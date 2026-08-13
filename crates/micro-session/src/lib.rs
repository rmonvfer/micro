//! Durable session storage.
//!
//! A session is a JSONL file holding one serialized [`Message`] per line, appended as
//! each message is produced, plus a sidecar metadata file listings read instead of
//! replaying the log. The log is never rewritten, so a crash costs at most the line
//! being written and [`SessionStore::load`] skips whatever it cannot parse.

mod error;
mod tree;
mod meta;

pub use error::Result;
pub use error::SessionError;
pub use meta::SessionMeta;
pub use tree::Compaction;
pub use tree::CustomEntry;
pub use tree::Entry;
pub use tree::Row;
pub use tree::Tree;
pub use meta::MAX_TITLE_CHARS;

use micro_types::Message;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::fs::File;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

/// Environment variable naming micro's home directory. Sessions live in its
/// `sessions` subdirectory.
pub const MICRO_DIR_ENV: &str = "MICRO_DIR";

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

    /// A store under `$MICRO_DIR/sessions`, falling back to `~/.micro/sessions`.
    pub fn from_env() -> Result<Self> {
        Ok(SessionStore::new(default_root()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Starts a session and claims its id. The workspace is canonicalized so listings
    /// match regardless of which symlinked spelling of the path the caller used.
    pub async fn create(
        &self,
        workspace: impl AsRef<Path>,
        model_id: impl Into<String>,
    ) -> Result<Session> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|source| SessionError::io(&self.root, source))?;

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
            meta,
            tree: Tree::new(),
        };
        session.write_meta().await?;
        Ok(session)
    }

    /// Reopens a session: its history, a handle ready for more appends, and a count of
    /// unreadable lines.
    pub async fn load(&self, id: &str) -> Result<LoadedSession> {
        validate_id(id)?;
        let raw = self.read_log(id).await?;
        let (lines, skipped_lines) = parse_log(&raw);
        let tree = Tree::from_lines(lines);
        // What the model is shown is the branch in use, not every message ever written.
        let messages = tree.path();

        let meta = self.meta_for(id).await?;
        let log = self.open_log(id).await?;
        let mut session = Session {
            log,
            log_path: self.log_path(id),
            meta_path: self.meta_path(id),
            meta,
            tree,
        };

        // A crash can leave the log ending mid-line. Terminating it now keeps the next
        // append on its own line instead of gluing it onto the unreadable fragment.
        if !raw.is_empty() && !raw.ends_with('\n') {
            session.seal_partial_line().await?;
        }

        // A crash between the log write and the metadata write, or a skipped line, leaves
        // the recorded count ahead of what the log actually yields. The log is the truth.
        if session.meta.message_count != messages.len() {
            session.meta.message_count = messages.len();
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

    /// Every session in the store, newest first.
    pub async fn list(&self) -> Result<Vec<SessionMeta>> {
        self.collect(None).await
    }

    /// Sessions started in one workspace, newest first.
    pub async fn list_in(&self, workspace: impl AsRef<Path>) -> Result<Vec<SessionMeta>> {
        let workspace = canonical(workspace.as_ref()).await;
        self.collect(Some(workspace)).await
    }

    /// Removes a session's log and metadata.
    pub async fn delete(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let log_path = self.log_path(id);
        match tokio::fs::remove_file(&log_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(SessionError::NotFound(id.to_string()))
            }
            Err(source) => return Err(SessionError::io(log_path, source)),
        }

        let meta_path = self.meta_path(id);
        match tokio::fs::remove_file(&meta_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SessionError::io(meta_path, source)),
        }
    }

    /// Branches a conversation: copies messages `0..=through_index` of `id` into a fresh
    /// session that records `id` as its parent. The source session is left untouched.
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
    ///
    /// The conversation is copied rather than adopted in place: the file that was imported
    /// keeps whatever it was, and the session that comes back is written to like any
    /// other. Lines that cannot be read are counted rather than aborting the import, so a
    /// log that was cut short still brings back everything before the damage.
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
        let messages = Tree::from_lines(lines).path();
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

    /// Reads a session's metadata, rebuilding it from the log when the sidecar is missing
    /// or unreadable so a lost sidecar never hides a session.
    async fn meta_for(&self, id: &str) -> Result<SessionMeta> {
        let path = self.meta_path(id);
        match tokio::fs::read(&path).await {
            Ok(raw) => {
                if let Ok(meta) = serde_json::from_slice::<SessionMeta>(&raw) {
                    return Ok(meta);
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(SessionError::io(path, source)),
        }
        self.rebuild_meta(id).await
    }

    /// Reconstructs what the log and the filesystem still know: the title, the message
    /// count, and the timestamps. The workspace and model are not recoverable.
    async fn rebuild_meta(&self, id: &str) -> Result<SessionMeta> {
        let raw = self.read_log(id).await?;
        let (lines, _) = parse_log(&raw);
        // Every message counts toward the metadata, not only the branch in use: the count
        // describes the file, and the title comes from the first thing ever asked.
        let messages: Vec<Message> = Tree::from_lines(lines)
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
        OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .map_err(|source| SessionError::io(path, source))
    }

    /// Takes the first free id at the current millisecond. `create_new` makes the claim
    /// atomic, so concurrent creators never land on the same session file.
    async fn claim_id(&self) -> Result<(String, File)> {
        let stamp = micro_types::now_ms();
        for attempt in 0..MAX_ID_ATTEMPTS {
            let id = match attempt {
                0 => stamp.to_string(),
                // Zero padded so ids stay in creation order under a plain string sort.
                _ => format!("{stamp}-{attempt:03}"),
            };
            let path = self.log_path(&id);
            match OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(log) => return Ok((id, log)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(SessionError::io(path, source)),
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
}

/// An open session. Appends land at the end of the log immediately; nothing earlier in
/// the file is ever rewritten.
#[derive(Debug)]
pub struct Session {
    log: File,
    log_path: PathBuf,
    meta_path: PathBuf,
    meta: SessionMeta,
    /// The shape of the conversation, so an appended message knows what it followed and a
    /// branch can be taken from anywhere in it.
    tree: Tree,
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
    ///
    /// The next message appended hangs off that entry instead of the end, which is what
    /// makes a second answer to the same question a branch rather than a replacement.
    pub fn branch_from(&mut self, id: &str) -> bool {
        self.tree.branch_from(id)
    }

    /// The conversation along the current branch, which is what the model is shown.
    pub fn branch(&self) -> Vec<Message> {
        self.tree.path()
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }

    /// Give the session a title of its own, in place of the one taken from the first
    /// message. It sticks, because a title that is set is never derived over.
    pub async fn rename(&mut self, title: &str) -> Result<()> {
        self.meta.title = title.trim().to_string();
        self.meta.updated_at = micro_types::now_ms();
        self.write_meta().await
    }

    /// Record something beside the conversation. It is written to the log and read back
    /// on the next open, and the model never sees it.
    pub async fn append_custom(
        &mut self,
        custom_type: &str,
        data: serde_json::Value,
    ) -> Result<()> {
        let entry = self.tree.push_custom(custom_type, data);
        self.write_line(&entry).await
    }

    /// Record that a stretch of the conversation has been summarized.
    ///
    /// `kept` is how many of the most recent messages are still part of the conversation.
    /// Nothing is removed from the log; what is written is where the conversation now
    /// starts reading from, so reopening the session costs no summarizing.
    pub async fn compacted(&mut self, summary: &str, kept: usize) -> Result<()> {
        let compaction = self.tree.push_compaction(summary, kept);
        self.write_line(&compaction).await
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
        let mut line =
            serde_json::to_vec(value).map_err(|source| SessionError::json(&self.log_path, source))?;
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

    /// Appends a run of messages as one write, then republishes the metadata. The log is
    /// written first: a crash in between leaves a stale count that [`SessionStore::load`]
    /// repairs, whereas the reverse order would claim messages that were never stored.
    pub async fn append_all(&mut self, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut lines = Vec::new();
        for message in messages {
            // Written as an entry rather than a bare message, so what it followed is on
            // disk and a branch taken later still knows where it came from.
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

    /// Closes off a line a crash left unfinished, so it stays one skipped line rather
    /// than swallowing whatever is appended next.
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

/// `$MICRO_DIR/sessions`, or `~/.micro/sessions` when the variable is unset.
pub fn default_root() -> Result<PathBuf> {
    let micro_dir = std::env::var(MICRO_DIR_ENV).ok();
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok());
    root_from(micro_dir.as_deref(), home.as_deref())
}

fn root_from(micro_dir: Option<&str>, home: Option<&str>) -> Result<PathBuf> {
    if let Some(dir) = micro_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir).join("sessions"));
    }
    let home = home
        .map(str::trim)
        .filter(|home| !home.is_empty())
        .ok_or(SessionError::NoHome { env: MICRO_DIR_ENV })?;
    Ok(PathBuf::from(home).join(".micro").join("sessions"))
}

/// Parses a log, returning the messages it yields and how many lines were unreadable.
/// Blank lines are not corruption and are not counted.
/// Read a log into its lines, counting the ones that could not be read.
///
/// A line is either an entry with its place in the tree or, in a session written before
/// sessions had one, a bare message. Both are accepted, so an old log still opens.
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

/// Replaces the metadata sidecar through a temporary file, so a reader never observes a
/// half-written record.
async fn write_meta(path: &Path, meta: &SessionMeta) -> Result<()> {
    let encoded = serde_json::to_vec(meta).map_err(|source| SessionError::json(path, source))?;
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, &encoded)
        .await
        .map_err(|source| SessionError::io(&temporary, source))?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|source| SessionError::io(path, source))
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

    /// An imported log becomes a session of micro's own: what it held is copied in, the
    /// file it came from is left alone, and damage in it costs only the lines affected.
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

        // The imported session stands on its own, and the file it came from is untouched.
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

    /// A branch taken in one run is still there in the next: what came after the branch
    /// point is on disk, and the conversation reopens on the branch that was in use.
    #[tokio::test]
    async fn a_branch_survives_being_reopened() {
        let store = SessionStore::new(scratch("branching"));
        let mut session = store.create("/work", "m").await.expect("created");
        session.append(&Message::user("question")).await.expect("wrote");
        session.append(&Message::user("first answer")).await.expect("wrote");

        // Go back to the question and answer it differently.
        assert!(session.branch_from("1"));
        session.append(&Message::user("second answer")).await.expect("wrote");
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

        // A crash mid-write leaves a partial JSON object with no terminating newline.
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
        // The fragment stayed one unreadable line instead of swallowing the next message.
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

        // Forking must not disturb the session it branched from.
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

        // Replacing the log with garbage would break any listing that parsed messages.
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

    #[test]
    fn the_root_follows_the_environment() {
        assert_eq!(
            root_from(Some("/opt/micro"), Some("/home/ramon")).unwrap(),
            PathBuf::from("/opt/micro/sessions")
        );
        assert_eq!(
            root_from(None, Some("/home/ramon")).unwrap(),
            PathBuf::from("/home/ramon/.micro/sessions")
        );
        assert_eq!(
            root_from(Some("  "), Some("/home/ramon")).unwrap(),
            PathBuf::from("/home/ramon/.micro/sessions")
        );
        assert!(matches!(
            root_from(None, None).unwrap_err(),
            SessionError::NoHome { .. }
        ));
    }
}
