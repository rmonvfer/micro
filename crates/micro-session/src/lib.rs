//! Durable session storage.

mod error;
mod meta;
mod tree;

pub use error::Result;
pub use error::SessionError;
pub use meta::SessionMeta;
pub use meta::MAX_TITLE_CHARS;
pub use tree::Compaction;
pub use tree::CustomEntry;
pub use tree::Entry;
pub use tree::LedgerLine;
pub use tree::Row;
pub use tree::Tree;

use micro_types::content_hash;
use micro_types::CompactionCost;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::Model;
use micro_types::PrefixSpan;
use micro_types::StopReason;
use micro_types::ToolDefinition;
use micro_types::Usage;
use micro_types::SCHEMA_VERSION;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::fs::File;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// The directory sessions live in, under whichever directory holds micro's data.
pub const SESSIONS_DIR: &str = "sessions";

/// How many ids to try when several sessions are created within one millisecond.
const MAX_ID_ATTEMPTS: u32 = 1000;

/// A directory of sessions.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// A store rooted at an explicit directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        SessionStore { root: root.into() }
    }

    /// A store under the `sessions` directory of micro's data directory.
    pub fn from_env() -> Result<Self> {
        Ok(SessionStore::new(default_root()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Starts a session and claims its id.
    pub async fn create(
        &self,
        workspace: impl AsRef<Path>,
        model_id: impl Into<String>,
    ) -> Result<Session> {
        create_private_dir(&self.root).await?;

        let (id, log) = self.claim_id().await?;
        let meta = SessionMeta::new(
            id.clone(),
            canonical(workspace.as_ref()).await,
            model_id.into(),
        );
        let session = Session {
            log,
            log_path: self.log_path(&id),
            meta_path: self.meta_path(&id),
            blobs_path: self.blobs_path(&id),
            meta,
            tree: Tree::new(),
            next_seq: 1,
        };
        session.write_meta().await?;
        Ok(session)
    }

    /// Reopens a session: its history, a handle ready for more appends, and a count of unreadable
    /// lines.
    pub async fn load(&self, id: &str) -> Result<LoadedSession> {
        validate_id(id)?;
        let raw = self.read_log(id).await?;
        let (lines, skipped_lines) = parse_log(&raw);
        let tree = checked_tree(&self.log_path(id), lines)?;

        let messages = tree.path();

        let meta = self.meta_for(id).await?;
        let log = self.open_log(id).await?;
        let mut session = Session {
            log,
            log_path: self.log_path(id),
            meta_path: self.meta_path(id),
            blobs_path: self.blobs_path(id),
            meta,

            next_seq: tree
                .ledger()
                .iter()
                .map(|recorded| recorded.seq)
                .max()
                .map_or(1, |highest| highest + 1),
            tree,
        };

        if !raw.is_empty() && !raw.ends_with('\n') {
            session.seal_partial_line().await?;
        }

        let entry_count = session.tree.entries().len();
        if session.meta.message_count != entry_count {
            session.meta.message_count = entry_count;
            session.write_meta().await?;
        }

        Ok(LoadedSession {
            session,
            messages,
            skipped_lines,
        })
    }

    /// What is recorded about one session, without reading its log.
    pub async fn meta(&self, id: &str) -> Result<SessionMeta> {
        validate_id(id)?;
        self.meta_for(id).await
    }

    pub async fn raw_log(&self, id: &str) -> Result<String> {
        validate_id(id)?;
        self.read_log(id).await
    }

    /// A piece of content a ledger event named by hash.
    pub async fn blob(&self, id: &str, hash: &str) -> Result<Vec<u8>> {
        validate_id(id)?;
        validate_id(hash)?;
        let path = self.blobs_path(id).join(hash);
        tokio::fs::read(&path)
            .await
            .map_err(|source| match source.kind() {
                ErrorKind::NotFound => SessionError::MissingBlob {
                    id: id.to_string(),
                    hash: hash.to_string(),
                },
                _ => SessionError::io(path, source),
            })
    }

    /// What the model was shown at one turn, put back together from the log.
    pub async fn reconstruct_turn(&self, id: &str, turn: u64) -> Result<ReconstructedTurn> {
        validate_id(id)?;
        let raw = self.read_log(id).await?;
        let (lines, _) = parse_log(&raw);
        checked_tree(&self.log_path(id), lines.clone())?;

        let mut tree = Tree::new();
        let mut request = None;
        let mut usage = None;
        for line in lines {
            if let tree::Line::Ledger(recorded) = &line {
                match &recorded.event {
                    LedgerEvent::TurnRequest { turn: at, .. } if *at == turn => {
                        let event = recorded.event.clone();
                        request = Some((event, tree.path(), tree.path_entry_ids()));
                    }
                    LedgerEvent::TurnUsage {
                        turn: at,
                        usage: reported,
                        stop_reason,
                        ..
                    } if *at == turn => usage = Some((*reported, *stop_reason)),
                    _ => {}
                }
            }
            tree.apply(line);
        }

        let Some((event, messages, entry_ids)) = request else {
            return Err(SessionError::NoSuchTurn {
                id: id.to_string(),
                turn,
            });
        };
        let LedgerEvent::TurnRequest {
            provider,
            model,
            prefix_hash,
            request_hash,
            request_body_blob,
            system_prompt_blob,
            tools_blob,
            model_blob,
            prefix_spans,
            attempt,
            ..
        } = event
        else {
            unreachable!("only a turn request is collected above");
        };

        let system_prompt = match &system_prompt_blob {
            Some(hash) => Some(self.text_blob(id, hash).await?),
            None => None,
        };
        let tools = self.parsed_blob(id, &tools_blob).await?;
        let described = self.parsed_blob(id, &model_blob).await?;
        let recorded_request_body = match &request_body_blob {
            Some(hash) => Some(self.blob(id, hash).await?),
            None => None,
        };

        Ok(ReconstructedTurn {
            turn,
            attempt,
            provider,
            model_id: model,
            model: described,
            prefix_hash,
            request_hash,
            recorded_request_body,
            prefix_spans,
            system_prompt,
            tools,
            messages,
            message_entry_ids: entry_ids,
            usage: usage.map(|(usage, _)| usage),
            stop_reason: usage.map(|(_, stop_reason)| stop_reason),
        })
    }

    /// A blob read as the text it was stored as.
    async fn text_blob(&self, id: &str, hash: &str) -> Result<String> {
        let raw = self.blob(id, hash).await?;
        String::from_utf8(raw).map_err(|_| SessionError::MissingBlob {
            id: id.to_string(),
            hash: hash.to_string(),
        })
    }

    async fn parsed_blob<T: serde::de::DeserializeOwned>(&self, id: &str, hash: &str) -> Result<T> {
        let raw = self.blob(id, hash).await?;
        serde_json::from_slice(&raw)
            .map_err(|source| SessionError::json(self.blobs_path(id).join(hash), source))
    }

    /// Every session in the store, newest first.
    pub async fn list(&self) -> Result<Vec<SessionMeta>> {
        self.collect(None).await
    }

    /// Sessions started in one workspace, newest first.
    pub async fn list_in(&self, workspace: impl AsRef<Path>) -> Result<Vec<SessionMeta>> {
        let workspace = canonical(workspace.as_ref()).await;
        self.collect(Some(workspace)).await
    }

    /// Removes a session's blobs, metadata, and log.
    pub async fn delete(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let log_path = self.log_path(id);
        let meta_path = self.meta_path(id);
        let blobs_path = self.blobs_path(id);
        let exists = path_exists(&log_path).await?
            || path_exists(&meta_path).await?
            || path_exists(&blobs_path).await?;
        if !exists {
            return Err(SessionError::NotFound(id.to_string()));
        }

        match tokio::fs::remove_dir_all(&blobs_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(SessionError::io(blobs_path, source)),
        }

        match tokio::fs::remove_file(&meta_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(SessionError::io(meta_path, source)),
        }

        match tokio::fs::remove_file(&log_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SessionError::io(log_path, source)),
        }
    }

    /// Branches a conversation: copies messages `0..=through_index` of `id` into a fresh session
    /// that records `id` as its parent.
    pub async fn fork(&self, id: &str, through_index: usize) -> Result<Session> {
        let source = self.load(id).await?;
        if through_index >= source.messages.len() {
            return Err(SessionError::IndexOutOfRange {
                index: through_index,
                len: source.messages.len(),
            });
        }

        let mut forked = self
            .create(
                &source.session.meta.workspace,
                source.session.meta.model_id.clone(),
            )
            .await?;
        forked.meta.parent = Some(id.to_string());
        forked
            .append_all(&source.messages[..=through_index])
            .await?;
        Ok(forked)
    }

    /// Takes a session log written elsewhere and files it here as a session of its own.
    pub async fn import(
        &self,
        source: impl AsRef<Path>,
        workspace: impl AsRef<Path>,
        model_id: impl Into<String>,
    ) -> Result<Imported> {
        let source = source.as_ref();
        let raw = tokio::fs::read_to_string(source)
            .await
            .map_err(|error| SessionError::io(source, error))?;

        let (lines, skipped_lines) = parse_log(&raw);
        let messages = checked_tree(source, lines)?.path();
        if messages.is_empty() {
            return Err(SessionError::NotFound(source.display().to_string()));
        }

        let mut session = self.create(workspace, model_id).await?;
        session.append_all(&messages).await?;
        Ok(Imported {
            session,
            messages,
            skipped_lines,
        })
    }

    async fn collect(&self, workspace: Option<PathBuf>) -> Result<Vec<SessionMeta>> {
        let mut dir = match tokio::fs::read_dir(&self.root).await {
            Ok(dir) => dir,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(SessionError::io(&self.root, source)),
        };

        let mut listed = Vec::new();
        loop {
            let entry = dir
                .next_entry()
                .await
                .map_err(|source| SessionError::io(&self.root, source))?;
            let Some(entry) = entry else { break };

            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if validate_id(id).is_err() {
                continue;
            }

            let meta = self.meta_for(id).await?;
            if workspace
                .as_ref()
                .is_some_and(|wanted| &meta.workspace != wanted)
            {
                continue;
            }
            listed.push(meta);
        }

        listed.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(listed)
    }

    async fn meta_for(&self, id: &str) -> Result<SessionMeta> {
        let path = self.meta_path(id);
        match tokio::fs::read(&path).await {
            Ok(raw) => {
                if let Ok(meta) = serde_json::from_slice::<SessionMeta>(&raw) {
                    return self.reconcile_meta(id, meta).await;
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(SessionError::io(path, source)),
        }
        self.rebuild_meta(id).await
    }

    /// Bring a readable log's derived title and entry count back into step with its sidecar.
    async fn reconcile_meta(&self, id: &str, mut meta: SessionMeta) -> Result<SessionMeta> {
        let raw = self.read_log(id).await?;
        let (lines, _) = parse_log(&raw);
        let tree = checked_tree(&self.log_path(id), lines)?;
        if tree.entries().is_empty() {
            return Ok(meta);
        }

        let mut derived = SessionMeta::new(
            id.to_string(),
            meta.workspace.clone(),
            meta.model_id.clone(),
        );
        for entry in tree.entries() {
            derived.record(&entry.message);
        }

        let mut changed = false;
        if meta.message_count != derived.message_count {
            meta.message_count = derived.message_count;
            changed = true;
        }
        if meta.title.trim().is_empty() && !derived.title.is_empty() {
            meta.title = derived.title;
            changed = true;
        }
        if let Ok(file_meta) = tokio::fs::metadata(self.log_path(id)).await {
            if let Some(modified) = file_meta.modified().ok().map(to_millis) {
                if modified > meta.updated_at {
                    meta.updated_at = modified;
                    changed = true;
                }
            }
        }
        if changed {
            write_meta(&self.meta_path(id), &meta).await?;
        }
        Ok(meta)
    }

    /// Reconstructs what the log and the filesystem still know: the title, the message count, and
    /// the timestamps.
    async fn rebuild_meta(&self, id: &str) -> Result<SessionMeta> {
        let raw = self.read_log(id).await?;
        let (lines, _) = parse_log(&raw);

        let messages: Vec<Message> = checked_tree(&self.log_path(id), lines)?
            .entries()
            .iter()
            .map(|entry| entry.message.clone())
            .collect();

        let mut meta = SessionMeta::new(id.to_string(), PathBuf::new(), String::new());
        for message in &messages {
            meta.record(message);
        }

        let log_path = self.log_path(id);
        if let Ok(file_meta) = tokio::fs::metadata(&log_path).await {
            let modified = file_meta.modified().ok().map(to_millis);
            let created = file_meta.created().ok().map(to_millis);
            meta.created_at = created.or(modified).unwrap_or(meta.created_at);
            meta.updated_at = modified.unwrap_or(meta.updated_at);
        }

        write_meta(&self.meta_path(id), &meta).await?;
        Ok(meta)
    }

    async fn read_log(&self, id: &str) -> Result<String> {
        let path = self.log_path(id);
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => Ok(raw),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Err(SessionError::NotFound(id.to_string()))
            }
            Err(source) => Err(SessionError::io(path, source)),
        }
    }

    async fn open_log(&self, id: &str) -> Result<File> {
        let path = self.log_path(id);
        open_private_file(&path, |options| {
            options.append(true);
        })
        .await
    }

    /// Takes the first free id at the current millisecond.
    async fn claim_id(&self) -> Result<(String, File)> {
        let stamp = micro_types::now_ms();
        for attempt in 0..MAX_ID_ATTEMPTS {
            let id = match attempt {
                0 => stamp.to_string(),

                _ => format!("{stamp}-{attempt:03}"),
            };
            let path = self.log_path(&id);
            match open_private_file(&path, |options| {
                options.create_new(true).append(true);
            })
            .await
            {
                Ok(log) => return Ok((id, log)),
                Err(SessionError::Io { source, .. })
                    if source.kind() == ErrorKind::AlreadyExists =>
                {
                    continue
                }
                Err(error) => return Err(error),
            }
        }
        Err(SessionError::io(
            &self.root,
            std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("no free session id at {stamp}"),
            ),
        ))
    }

    fn log_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.jsonl"))
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.meta.json"))
    }

    fn blobs_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.blobs"))
    }
}

/// What the model was shown at one turn, and what came back.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconstructedTurn {
    pub turn: u64,
    /// Which try produced this record.
    pub attempt: u32,
    pub provider: String,
    pub model_id: String,

    pub model: Model,
    pub prefix_hash: String,
    pub request_hash: String,
    /// Exact serialized provider body for ledgers written by versions that retain it.
    pub recorded_request_body: Option<Vec<u8>>,

    pub prefix_spans: Vec<PrefixSpan>,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
    /// The conversation as it stood when the request went out.
    pub messages: Vec<Message>,
    pub message_entry_ids: Vec<String>,
    /// What the provider said the turn cost, once it answered.
    pub usage: Option<Usage>,
    pub stop_reason: Option<StopReason>,
}

/// An open session.
#[derive(Debug)]
pub struct Session {
    log: File,
    log_path: PathBuf,
    meta_path: PathBuf,
    /// Where content a ledger event names by hash is kept.
    blobs_path: PathBuf,
    meta: SessionMeta,
    /// The shape of the conversation, so an appended message knows what it followed and a branch
    /// can be taken from anywhere in it.
    tree: Tree,
    /// The number the next fact recorded here is given.
    next_seq: u64,
}

impl Session {
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    /// The conversation's shape, for a caller showing or navigating its branches.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Continue from an earlier entry, keeping everything that came after it.
    pub async fn branch_from(&mut self, id: &str) -> Result<bool> {
        if !self.tree.branch_from(id) {
            return Ok(false);
        }
        self.append_event(LedgerEvent::HeadMoved {
            entry_id: id.to_string(),
        })
        .await?;
        Ok(true)
    }

    pub fn branch(&self) -> Vec<Message> {
        self.tree.path()
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }

    /// Record the model that will serve future turns in this session.
    pub async fn set_model_id(&mut self, model_id: impl Into<String>) -> Result<()> {
        self.meta.model_id = model_id.into();
        self.meta.updated_at = micro_types::now_ms();
        self.write_meta().await
    }

    /// Give the session a title of its own, in place of the one taken from the first message.
    pub async fn rename(&mut self, title: &str) -> Result<()> {
        self.meta.title = title.trim().to_string();
        self.meta.updated_at = micro_types::now_ms();
        self.write_meta().await
    }

    /// Record something beside the conversation.
    pub async fn append_custom(
        &mut self,
        custom_type: &str,
        data: serde_json::Value,
    ) -> Result<()> {
        let entry = self.tree.push_custom(custom_type, data);
        self.write_line(&entry).await
    }

    /// Record that a stretch of the conversation has been summarized.
    pub async fn compacted(
        &mut self,
        summary: &str,
        kept: usize,
        cost: CompactionCost,
    ) -> Result<()> {
        let compaction = self.tree.push_compaction(summary, kept);
        self.write_line(&compaction).await?;

        let summary_blob = self.store_blob(summary.as_bytes()).await?;
        self.append_event(LedgerEvent::Compaction {
            summary_blob,
            kept,
            message_entry_ids: Vec::new(),
            cost,
        })
        .await
        .map(|_| ())
    }

    /// Every fact recorded beside the conversation, oldest first.
    pub fn events(&self) -> &[LedgerLine] {
        self.tree.ledger()
    }

    /// Record one fact about the run, in the envelope every ledger line is written in.
    pub async fn append_event(&mut self, event: LedgerEvent) -> Result<u64> {
        let mut event = event;
        self.stamp(&mut event);

        let seq = self.next_seq;
        let recorded = LedgerLine {
            v: SCHEMA_VERSION,
            seq,
            ts: micro_types::now_ms(),
            event,
        };
        self.write_line(&recorded).await?;
        self.next_seq += 1;
        self.tree.apply(tree::Line::Ledger(recorded));
        Ok(seq)
    }

    /// Fill in what only the session can know.
    fn stamp(&self, event: &mut LedgerEvent) {
        let message_entry_ids = match event {
            LedgerEvent::TurnRequest {
                message_entry_ids, ..
            }
            | LedgerEvent::Compaction {
                message_entry_ids, ..
            } => Some(message_entry_ids),
            _ => None,
        };
        if let Some(message_entry_ids) = message_entry_ids.filter(|ids| ids.is_empty()) {
            *message_entry_ids = self.tree.path_entry_ids();
        }
    }

    /// File a piece of content under the hash of its bytes, and answer with that name.
    pub async fn store_blob(&self, content: &[u8]) -> Result<String> {
        let hash = content_hash(content);
        let path = self.blobs_path.join(&hash);
        if tokio::fs::metadata(&path).await.is_ok() {
            return Ok(hash);
        }

        create_private_dir(&self.blobs_path).await?;
        let temporary = path.with_extension("tmp");
        write_private_file(&temporary, content).await?;
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(|source| SessionError::io(&path, source))?;
        Ok(hash)
    }

    /// Name an entry, or take its name away.
    pub async fn set_label(&mut self, entry_id: &str, label: Option<String>) -> Result<bool> {
        if !self.tree.set_label(entry_id, label.clone()) {
            return Ok(false);
        }
        let written = crate::tree::Label {
            entry_id: entry_id.to_string(),
            label,
            timestamp: micro_types::now_ms(),
        };
        self.write_line(&written).await.map(|()| true)
    }

    /// Append one line to the log, whatever kind of line it is.
    async fn write_line(&mut self, value: &impl serde::Serialize) -> Result<()> {
        let mut line = serde_json::to_vec(value)
            .map_err(|source| SessionError::json(&self.log_path, source))?;
        line.push(b'\n');
        self.log
            .write_all(&line)
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))?;
        self.log
            .flush()
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))
    }

    /// The JSONL log backing this session.
    pub fn path(&self) -> &Path {
        &self.log_path
    }

    pub async fn append(&mut self, message: &Message) -> Result<()> {
        self.append_all(std::slice::from_ref(message)).await
    }

    /// Appends a run of messages as one write, then republishes the metadata.
    pub async fn append_all(&mut self, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut lines = Vec::new();
        for message in messages {
            let entry = self.tree.push(message.clone());
            serde_json::to_writer(&mut lines, &entry)
                .map_err(|source| SessionError::json(&self.log_path, source))?;
            lines.push(b'\n');
        }
        self.log
            .write_all(&lines)
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))?;
        self.log
            .flush()
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))?;

        for message in messages {
            self.meta.record(message);
        }
        self.write_meta().await
    }

    async fn write_meta(&self) -> Result<()> {
        write_meta(&self.meta_path, &self.meta).await
    }

    async fn seal_partial_line(&mut self) -> Result<()> {
        self.log
            .write_all(b"\n")
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))?;
        self.log
            .flush()
            .await
            .map_err(|source| SessionError::io(&self.log_path, source))
    }
}

/// A session brought in from a log written elsewhere.
#[derive(Debug)]
pub struct Imported {
    pub session: Session,
    pub messages: Vec<Message>,
    /// Lines the imported file held that could not be read.
    pub skipped_lines: usize,
}

/// A session reopened from disk.
#[derive(Debug)]
pub struct LoadedSession {
    /// A handle ready to append the rest of the conversation.
    pub session: Session,
    /// The full history, in order, ready to hand to the agent.
    pub messages: Vec<Message>,
    /// Lines the log held that could not be parsed, such as a write cut short by a crash.
    pub skipped_lines: usize,
}

/// Where sessions are written: the `sessions` directory under micro's data directory.
pub fn default_root() -> Result<PathBuf> {
    let data = micro_dirs::data_dir().ok_or(SessionError::NoHome {
        env: micro_dirs::MICRO_DIR_ENV,
    })?;
    Ok(data.join(SESSIONS_DIR))
}

/// Build a conversation tree only after confirming that its parent links are safe to traverse.
fn checked_tree(path: &Path, lines: Vec<tree::Line>) -> Result<Tree> {
    let tree = Tree::from_lines(lines);
    tree.validate()
        .map_err(|reason| SessionError::InvalidGraph {
            path: path.to_path_buf(),
            reason,
        })?;
    Ok(tree)
}

/// Parses a log, returning the messages it yields and how many lines were unreadable.
fn parse_log(raw: &str) -> (Vec<tree::Line>, usize) {
    let mut lines = Vec::new();
    let mut skipped = 0;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<tree::Line>(line) {
            Ok(parsed) => lines.push(parsed),
            Err(_) => skipped += 1,
        }
    }
    (lines, skipped)
}

/// Replaces the metadata sidecar through a temporary file, so a reader never observes a half-
/// written record.
async fn write_meta(path: &Path, meta: &SessionMeta) -> Result<()> {
    let encoded = serde_json::to_vec(meta).map_err(|source| SessionError::json(path, source))?;
    let temporary = path.with_extension("tmp");
    write_private_file(&temporary, &encoded).await?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|source| SessionError::io(path, source))
}

async fn create_private_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| SessionError::io(path, source))?;
    #[cfg(unix)]
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|source| SessionError::io(path, source))?;
    Ok(())
}

async fn open_private_file(path: &Path, configure: impl FnOnce(&mut OpenOptions)) -> Result<File> {
    let mut options = OpenOptions::new();
    configure(&mut options);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .await
        .map_err(|source| SessionError::io(path, source))?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|source| SessionError::io(path, source))?;
    Ok(file)
}

async fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = open_private_file(path, |options| {
        options.create(true).truncate(true).write(true);
    })
    .await?;
    file.write_all(content)
        .await
        .map_err(|source| SessionError::io(path, source))?;
    file.flush()
        .await
        .map_err(|source| SessionError::io(path, source))
}

async fn path_exists(path: &Path) -> Result<bool> {
    match tokio::fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SessionError::io(path, source)),
    }
}

/// Ids become file names, so only characters that cannot leave the store are accepted.
fn validate_id(id: &str) -> Result<()> {
    let acceptable = !id.is_empty()
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        });
    if acceptable {
        Ok(())
    } else {
        Err(SessionError::InvalidId(id.to_string()))
    }
}

async fn canonical(path: &Path) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

fn to_millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::AssistantMessage;
    use micro_types::ContentBlock;
    use micro_types::StopReason;
    use micro_types::Usage;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-session-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assistant(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text(text)],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })
    }

    #[tokio::test]
    async fn importing_copies_a_log_written_elsewhere() {
        let root = scratch("import");
        let store = SessionStore::new(root.join("sessions"));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let source = root.join("elsewhere.jsonl");
        let mut written = String::new();
        for message in [Message::user("first"), assistant("second")] {
            written.push_str(&serde_json::to_string(&message).unwrap());
            written.push('\n');
        }
        written.push_str("{ cut short\n");
        tokio::fs::write(&source, &written).await.unwrap();

        let imported = store
            .import(&source, &workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        assert_eq!(imported.messages.len(), 2);
        assert_eq!(imported.skipped_lines, 1);

        let reopened = store.load(imported.session.id()).await.unwrap();
        assert_eq!(reopened.messages.len(), 2);
        assert_eq!(tokio::fs::read_to_string(&source).await.unwrap(), written);
    }

    #[tokio::test]
    async fn importing_a_log_with_nothing_in_it_is_refused() {
        let root = scratch("import-empty");
        let store = SessionStore::new(root.join("sessions"));
        let source = root.join("empty.jsonl");
        tokio::fs::write(&source, "\n\n").await.unwrap();

        assert!(store
            .import(&source, &root, "anthropic/claude-opus-5")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_branch_survives_being_reopened() {
        let store = SessionStore::new(scratch("branching"));
        let mut session = store.create("/work", "m").await.expect("created");
        session
            .append(&Message::user("question"))
            .await
            .expect("wrote");
        session
            .append(&Message::user("first answer"))
            .await
            .expect("wrote");

        assert!(session.branch_from("1").await.expect("moved"));
        session
            .append(&Message::user("second answer"))
            .await
            .expect("wrote");
        let id = session.id().to_string();
        drop(session);

        let loaded = store.load(&id).await.expect("loaded");
        let texts: Vec<String> = loaded
            .messages
            .iter()
            .map(|message| message.content()[0].as_text().to_string())
            .collect();
        assert_eq!(texts, vec!["question", "second answer"]);
        assert_eq!(
            loaded.session.tree().entries().len(),
            3,
            "the abandoned answer is still on disk"
        );
    }

    #[tokio::test]
    async fn a_session_round_trips_its_history() {
        let store = SessionStore::new(scratch("round-trip"));
        let mut session = store.create("/work", "claude-opus-5").await.unwrap();
        let id = session.id().to_string();

        let written = vec![
            Message::user("port the session layer"),
            assistant("on it"),
            Message::tool_result("call_1", "read", "contents", false),
        ];
        for message in &written {
            session.append(message).await.unwrap();
        }

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.messages, written);
        assert_eq!(loaded.skipped_lines, 0);
        assert_eq!(loaded.session.meta().message_count, 3);
        assert_eq!(loaded.session.meta().model_id, "claude-opus-5");
        assert_eq!(loaded.session.meta().title, "port the session layer");
    }

    #[tokio::test]
    async fn appending_adds_one_line_and_leaves_earlier_bytes_untouched() {
        let store = SessionStore::new(scratch("append-only"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let path = session.path().to_path_buf();

        session.append(&Message::user("first")).await.unwrap();
        let after_first = std::fs::read(&path).unwrap();

        session.append(&assistant("second")).await.unwrap();
        let after_second = std::fs::read(&path).unwrap();

        assert!(after_second.starts_with(&after_first));
        assert_eq!(String::from_utf8(after_second).unwrap().lines().count(), 2);
    }

    #[tokio::test]
    async fn appended_messages_are_visible_without_reopening_the_session() {
        let store = SessionStore::new(scratch("append-then-load"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        session.append(&Message::user("first")).await.unwrap();
        assert_eq!(store.load(&id).await.unwrap().messages.len(), 1);

        session.append(&assistant("second")).await.unwrap();
        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[1], assistant("second"));
    }

    #[tokio::test]
    async fn a_truncated_final_line_is_skipped_and_the_rest_still_loads() {
        let store = SessionStore::new(scratch("truncated"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let path = session.path().to_path_buf();

        session.append(&Message::user("intact")).await.unwrap();
        session.append(&assistant("also intact")).await.unwrap();

        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{\"role\":\"user\",\"content\":[{\"type\":\"tex");
        std::fs::write(&path, raw).unwrap();

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.skipped_lines, 1);
        assert_eq!(loaded.session.meta().message_count, 2);
    }

    #[tokio::test]
    async fn a_malformed_line_in_the_middle_does_not_hide_later_messages() {
        let store = SessionStore::new(scratch("malformed-middle"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let path = session.path().to_path_buf();

        session.append(&Message::user("before")).await.unwrap();
        session.append(&assistant("after")).await.unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = raw.lines().collect();
        lines.insert(1, "{ not json at all");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.skipped_lines, 1);
    }

    #[tokio::test]
    async fn a_session_stays_appendable_after_loading_a_corrupted_log() {
        let store = SessionStore::new(scratch("append-after-corruption"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let path = session.path().to_path_buf();

        session.append(&Message::user("intact")).await.unwrap();
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{\"role\":\"assis");
        std::fs::write(&path, raw).unwrap();

        let mut loaded = store.load(&id).await.unwrap();
        loaded
            .session
            .append(&assistant("recovered"))
            .await
            .unwrap();

        let reloaded = store.load(&id).await.unwrap();
        assert_eq!(reloaded.messages.len(), 2);
        assert_eq!(reloaded.messages[1], assistant("recovered"));

        assert_eq!(reloaded.skipped_lines, 1);
    }

    #[tokio::test]
    async fn forking_copies_history_through_the_index() {
        let store = SessionStore::new(scratch("fork"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        let written = vec![
            Message::user("one"),
            assistant("two"),
            Message::user("three"),
        ];
        for message in &written {
            session.append(message).await.unwrap();
        }

        let forked = store.fork(&id, 1).await.unwrap();
        let forked_id = forked.id().to_string();
        assert_ne!(forked_id, id);
        assert_eq!(forked.meta().parent.as_deref(), Some(id.as_str()));
        assert_eq!(forked.meta().model_id, "opus");
        assert_eq!(forked.meta().title, "one");

        let branch = store.load(&forked_id).await.unwrap();
        assert_eq!(branch.messages, written[..2]);

        let original = store.load(&id).await.unwrap();
        assert_eq!(original.messages.len(), 3);
        assert!(original.session.meta().parent.is_none());
    }

    #[tokio::test]
    async fn forking_can_keep_the_whole_conversation() {
        let store = SessionStore::new(scratch("fork-all"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        session.append(&Message::user("one")).await.unwrap();
        session.append(&assistant("two")).await.unwrap();

        let forked = store.fork(&id, 1).await.unwrap();
        assert_eq!(forked.meta().message_count, 2);
    }

    #[tokio::test]
    async fn forking_past_the_end_is_rejected() {
        let store = SessionStore::new(scratch("fork-range"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        session.append(&Message::user("one")).await.unwrap();

        let error = store.fork(&id, 5).await.unwrap_err();
        assert!(matches!(
            error,
            SessionError::IndexOutOfRange { index: 5, len: 1 }
        ));
    }

    #[tokio::test]
    async fn listing_returns_the_newest_session_first() {
        let store = SessionStore::new(scratch("list-order"));
        let first = store.create("/work", "opus").await.unwrap();
        let second = store.create("/work", "opus").await.unwrap();
        let third = store.create("/work", "opus").await.unwrap();

        let listed: Vec<String> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|meta| meta.id)
            .collect();
        assert_eq!(
            listed,
            vec![
                third.id().to_string(),
                second.id().to_string(),
                first.id().to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn listing_can_be_scoped_to_one_workspace() {
        let root = scratch("list-workspace");
        let alpha = root.join("alpha");
        let beta = root.join("beta");
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();

        let store = SessionStore::new(root.join("sessions"));
        let in_alpha = store.create(&alpha, "opus").await.unwrap();
        let _in_beta = store.create(&beta, "opus").await.unwrap();

        let scoped = store.list_in(&alpha).await.unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, in_alpha.id());
        assert_eq!(store.list().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn listing_answers_from_the_sidecar_alone() {
        let store = SessionStore::new(scratch("list-metadata"));
        let mut session = store.create("/work", "claude-opus-5").await.unwrap();
        session
            .append(&Message::user("write the store"))
            .await
            .unwrap();
        session.append(&assistant("done")).await.unwrap();

        std::fs::write(session.path(), "not json\nnot json either\n").unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "write the store");
        assert_eq!(listed[0].message_count, 2);
        assert_eq!(listed[0].model_id, "claude-opus-5");
        assert_eq!(listed[0].workspace, PathBuf::from("/work"));
        assert!(listed[0].updated_at >= listed[0].created_at);
    }

    #[tokio::test]
    async fn listing_repairs_stale_title_and_message_count() {
        let store = SessionStore::new(scratch("reconcile-metadata"));
        let mut session = store.create("/work", "claude-opus-5").await.unwrap();
        let id = session.id().to_string();
        session
            .append(&Message::user("repair the explorer"))
            .await
            .unwrap();
        session.append(&assistant("done")).await.unwrap();

        let stale = SessionMeta {
            v: micro_types::SCHEMA_VERSION,
            id: id.clone(),
            created_at: 1,
            updated_at: 1,
            workspace: PathBuf::from("/work"),
            model_id: "claude-opus-5".into(),
            title: String::new(),
            message_count: 0,
            parent: None,
            org_id: None,
            agent_id: None,
        };
        write_meta(&store.meta_path(&id), &stale).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed[0].title, "repair the explorer");
        assert_eq!(listed[0].message_count, 2);

        let repaired: SessionMeta =
            serde_json::from_slice(&std::fs::read(store.meta_path(&id)).unwrap()).unwrap();
        assert_eq!(repaired.title, "repair the explorer");
        assert_eq!(repaired.message_count, 2);
    }

    #[tokio::test]
    async fn listing_an_empty_or_missing_store_yields_nothing() {
        let store = SessionStore::new(scratch("empty").join("never-created"));
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn metadata_lost_from_disk_is_rebuilt_from_the_log() {
        let store = SessionStore::new(scratch("rebuild-meta"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        session.append(&Message::user("rebuild me")).await.unwrap();
        session.append(&assistant("sure")).await.unwrap();

        std::fs::remove_file(store.meta_path(&id)).unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].title, "rebuild me");
        assert_eq!(listed[0].message_count, 2);
        assert!(store.meta_path(&id).exists());
    }

    #[tokio::test]
    async fn corrupted_metadata_is_rebuilt_from_the_log() {
        let store = SessionStore::new(scratch("corrupt-meta"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        session.append(&Message::user("still here")).await.unwrap();

        std::fs::write(store.meta_path(&id), "{ truncated").unwrap();

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.session.meta().title, "still here");
        assert_eq!(loaded.session.meta().message_count, 1);
    }

    #[tokio::test]
    async fn deleting_removes_the_log_and_its_metadata() {
        let store = SessionStore::new(scratch("delete"));
        let session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let log_path = session.path().to_path_buf();

        store.delete(&id).await.unwrap();
        assert!(!log_path.exists());
        assert!(!store.meta_path(&id).exists());
        assert!(store.list().await.unwrap().is_empty());
        assert!(matches!(
            store.delete(&id).await.unwrap_err(),
            SessionError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn deleting_keeps_the_session_retryable_when_blob_cleanup_fails() {
        let store = SessionStore::new(scratch("delete-blob-error"));
        let session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let log_path = session.path().to_path_buf();
        let meta_path = store.meta_path(&id);
        let blobs_path = store.blobs_path(&id);
        std::fs::write(&blobs_path, "not a directory").unwrap();

        assert!(store.delete(&id).await.is_err());
        assert!(log_path.exists());
        assert!(meta_path.exists());

        std::fs::remove_file(blobs_path).unwrap();
        store.delete(&id).await.unwrap();
        assert!(!log_path.exists());
        assert!(!meta_path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn session_files_are_private() {
        let root = scratch("private-permissions").join("sessions");
        let store = SessionStore::new(&root);
        let session = store.create("/work", "opus").await.unwrap();
        session.store_blob(b"secret").await.unwrap();

        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(session.path()), 0o600);
        assert_eq!(mode(&store.meta_path(session.id())), 0o600);
        assert_eq!(mode(&store.blobs_path(session.id())), 0o700);
        let blob = store.blobs_path(session.id()).join(content_hash(b"secret"));
        assert_eq!(mode(&blob), 0o600);
    }

    #[tokio::test]
    async fn an_unknown_session_is_reported_as_missing() {
        let store = SessionStore::new(scratch("missing"));
        assert!(matches!(
            store.load("1784552052027").await.unwrap_err(),
            SessionError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn ids_that_could_leave_the_store_are_rejected() {
        let store = SessionStore::new(scratch("bad-id"));
        for id in ["../escape", "a/b", "", "with space"] {
            assert!(matches!(
                store.load(id).await.unwrap_err(),
                SessionError::InvalidId(_)
            ));
        }
    }

    #[tokio::test]
    async fn appending_nothing_leaves_the_session_alone() {
        let store = SessionStore::new(scratch("append-empty"));
        let mut session = store.create("/work", "opus").await.unwrap();
        session.append_all(&[]).await.unwrap();
        assert_eq!(session.meta().message_count, 0);
        assert_eq!(std::fs::read(session.path()).unwrap().len(), 0);
    }

    /// A ledger line is written between the messages and read back as what it was, and the
    /// conversation around it is untouched by it.
    #[tokio::test]
    async fn a_recorded_fact_survives_being_reopened() {
        let store = SessionStore::new(scratch("ledger-round-trip"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        session.append(&Message::user("go")).await.unwrap();
        session
            .append_event(LedgerEvent::Marker {
                data: serde_json::json!({ "note": "sandbox off" }),
            })
            .await
            .unwrap();
        session.append(&assistant("done")).await.unwrap();

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.skipped_lines, 0);
        assert_eq!(loaded.messages.len(), 2, "the ledger stays out of the talk");
        assert_eq!(loaded.session.events().len(), 1);
        assert_eq!(loaded.session.events()[0].v, micro_types::SCHEMA_VERSION);
        assert_eq!(
            loaded.session.events()[0].event,
            LedgerEvent::Marker {
                data: serde_json::json!({ "note": "sandbox off" }),
            }
        );
    }

    /// Numbering is what orders facts against each other.
    #[tokio::test]
    async fn sequence_numbers_carry_on_after_a_reload() {
        let store = SessionStore::new(scratch("ledger-seq"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        for _ in 0..3 {
            session
                .append_event(LedgerEvent::Marker {
                    data: serde_json::Value::Null,
                })
                .await
                .unwrap();
        }
        drop(session);

        let mut reopened = store.load(&id).await.unwrap();
        assert_eq!(
            reopened
                .session
                .append_event(LedgerEvent::Marker {
                    data: serde_json::Value::Null
                })
                .await
                .unwrap(),
            4
        );

        let seqs: Vec<u64> = store
            .load(&id)
            .await
            .unwrap()
            .session
            .events()
            .iter()
            .map(|recorded| recorded.seq)
            .collect();
        assert_eq!(seqs, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn a_session_written_before_the_ledger_still_loads() {
        let store = SessionStore::new(scratch("legacy"));
        let session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let path = session.path().to_path_buf();
        drop(session);

        let mut written = String::new();
        for message in [Message::user("first"), assistant("second")] {
            written.push_str(&serde_json::to_string(&message).unwrap());
            written.push('\n');
        }
        std::fs::write(&path, written).unwrap();

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.skipped_lines, 0);
        assert!(loaded.session.events().is_empty());
        assert!(matches!(
            store.reconstruct_turn(&id, 1).await.unwrap_err(),
            SessionError::NoSuchTurn { .. }
        ));
    }

    #[tokio::test]
    async fn content_is_stored_once_under_the_hash_of_its_bytes() {
        let store = SessionStore::new(scratch("blobs"));
        let session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        let first = session.store_blob(b"you are micro").await.unwrap();
        let again = session.store_blob(b"you are micro").await.unwrap();
        let other = session.store_blob(b"you are something else").await.unwrap();

        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(store.blob(&id, &first).await.unwrap(), b"you are micro");
        assert!(matches!(
            store.blob(&id, "0badc0de").await.unwrap_err(),
            SessionError::MissingBlob { .. }
        ));
    }

    #[tokio::test]
    async fn a_turn_is_rebuilt_as_the_conversation_stood_then() {
        let store = SessionStore::new(scratch("reconstruct"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        let asked = Message::user("first");
        session.append(&asked).await.unwrap();
        let system_prompt_blob = session.store_blob(b"you are micro").await.unwrap();
        let tools_blob = session.store_blob(b"[]").await.unwrap();
        let request_body_blob = session.store_blob(br#"{"messages":[]}"#).await.unwrap();
        let model = Model::anthropic("claude-opus-5");
        let model_blob = session
            .store_blob(&serde_json::to_vec(&model).unwrap())
            .await
            .unwrap();
        session
            .append_event(LedgerEvent::TurnRequest {
                turn: 1,
                provider: "anthropic".into(),
                model: "claude-opus-5".into(),
                pricing: None,
                prefix_hash: "aa".into(),
                request_hash: "bb".into(),
                request_body_blob: Some(request_body_blob),
                system_prompt_blob: Some(system_prompt_blob),
                tools_blob,
                model_blob,
                prefix_spans: Vec::new(),
                message_entry_ids: Vec::new(),
                attempt: 1,
            })
            .await
            .unwrap();
        session.append(&assistant("answered")).await.unwrap();
        session
            .append_event(LedgerEvent::TurnUsage {
                turn: 1,
                usage: Usage {
                    input: 7,
                    output: 3,
                    cache_read: 0,
                    cache_write: 0,
                },
                stop_reason: StopReason::Stop,
                provider: "anthropic".into(),
                model: "claude-opus-5".into(),
            })
            .await
            .unwrap();

        session.branch_from("1").await.unwrap();
        session.append(&Message::user("second")).await.unwrap();

        let rebuilt = store.reconstruct_turn(&id, 1).await.unwrap();
        assert_eq!(rebuilt.messages, vec![asked]);
        assert_eq!(rebuilt.message_entry_ids, vec!["1".to_string()]);
        assert_eq!(rebuilt.system_prompt.as_deref(), Some("you are micro"));
        assert!(rebuilt.tools.is_empty());
        assert_eq!(rebuilt.model, model);
        assert_eq!(
            rebuilt.recorded_request_body.as_deref(),
            Some(br#"{"messages":[]}"#.as_slice())
        );
        assert_eq!(rebuilt.usage.map(|usage| usage.input), Some(7));
        assert_eq!(rebuilt.stop_reason, Some(StopReason::Stop));
    }

    /// Moving the conversation back to an earlier entry is a fact about the session.
    #[tokio::test]
    async fn moving_the_head_is_recorded() {
        let store = SessionStore::new(scratch("head-moved"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        session.append(&Message::user("question")).await.unwrap();
        session.append(&assistant("answer")).await.unwrap();
        assert!(session.branch_from("1").await.unwrap());
        assert!(
            !session.branch_from("nowhere").await.unwrap(),
            "a stale id records nothing"
        );
        drop(session);

        let events = store.load(&id).await.unwrap();
        assert_eq!(
            events
                .session
                .events()
                .iter()
                .map(|recorded| recorded.event.clone())
                .collect::<Vec<_>>(),
            vec![LedgerEvent::HeadMoved {
                entry_id: "1".into()
            }]
        );
    }

    #[tokio::test]
    async fn reopening_restores_the_last_recorded_head() {
        let store = SessionStore::new(scratch("replay-head-moved"));
        let mut session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();

        session.append(&Message::user("question")).await.unwrap();
        session.append(&assistant("first answer")).await.unwrap();
        session.branch_from("1").await.unwrap();
        drop(session);

        let loaded = store.load(&id).await.unwrap();
        assert_eq!(loaded.session.tree().head(), Some("1"));
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content()[0].as_text(), "question");
    }

    #[tokio::test]
    async fn loading_a_cyclic_or_duplicate_graph_is_refused() {
        let store = SessionStore::new(scratch("invalid-graphs"));
        let session = store.create("/work", "opus").await.unwrap();
        let id = session.id().to_string();
        let path = session.path().to_path_buf();
        drop(session);

        let cycle = [
            Entry::new("1", Some("2".into()), Message::user("first")),
            Entry::new("2", Some("1".into()), Message::user("second")),
        ];
        let raw = cycle
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        std::fs::write(&path, format!("{raw}\n")).unwrap();

        let error = store.load(&id).await.unwrap_err();
        assert!(matches!(error, SessionError::InvalidGraph { .. }));

        let duplicate = [
            Entry::new("1", None, Message::user("first")),
            Entry::new("1", None, Message::user("second")),
        ];
        let raw = duplicate
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        std::fs::write(&path, format!("{raw}\n")).unwrap();

        let error = store.load(&id).await.unwrap_err();
        assert!(matches!(error, SessionError::InvalidGraph { .. }));
    }

    #[test]
    fn the_root_sits_under_the_data_directory() {
        assert_eq!(
            default_root().unwrap(),
            micro_dirs::data_dir().unwrap().join(SESSIONS_DIR)
        );
    }
}
