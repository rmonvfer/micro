//! The list `/resume` opens: every recorded session, narrowed to this workspace's at first,
//! searched as it is typed into, and threaded so a fork sits under the session it came from.

use micro_commands::SessionChoices;
use micro_session::SessionMeta;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sessions shown at once before the list scrolls.
pub const MAX_VISIBLE: usize = 10;
/// How far page up and page down move.
const PAGE: usize = 10;

/// Which sessions the list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Those started in the workspace the agent is working in.
    CurrentFolder,
    All,
}

/// How the list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Forks under the session they came from, the most recently active thread first.
    Threaded,
    /// Most recently active first.
    Recent,
    /// Best match for the search first.
    Fuzzy,
}

impl Sort {
    pub fn name(self) -> &'static str {
        match self {
            Sort::Threaded => "Threaded",
            Sort::Recent => "Recent",
            Sort::Fuzzy => "Fuzzy",
        }
    }

    fn next(self) -> Sort {
        match self {
            Sort::Threaded => Sort::Recent,
            Sort::Recent => Sort::Fuzzy,
            Sort::Fuzzy => Sort::Threaded,
        }
    }
}

/// What the selector is in the middle of, beyond choosing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Choosing,
    /// Asking whether this session should be deleted.
    ConfirmingDelete {
        id: String,
    },
    /// Taking a new title for this session.
    Renaming {
        id: String,
        title: String,
    },
}

/// One row of the list: a session, and the lines that tie it into its thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub meta: &'a SessionMeta,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSelector {
    sessions: Vec<SessionMeta>,
    workspace: PathBuf,
    current: Option<String>,
    scope: Scope,
    sort: Sort,
    named_only: bool,
    show_path: bool,
    query: String,
    selected: usize,
    mode: Mode,
    /// What just happened, said in place of the key hints.
    status: Option<String>,
}

impl SessionSelector {
    pub fn new(choices: SessionChoices) -> Self {
        let mut selector = SessionSelector {
            sessions: Vec::new(),
            workspace: choices.workspace.clone(),
            current: None,
            scope: Scope::CurrentFolder,
            sort: Sort::Threaded,
            named_only: false,
            show_path: false,
            query: String::new(),
            selected: 0,
            mode: Mode::Choosing,
            status: None,
        };
        selector.update(choices);
        selector
    }

    /// Take the store's sessions as they now are, keeping the view, the search, and the row
    /// chosen when it is still there.
    pub fn update(&mut self, choices: SessionChoices) {
        let chosen = self.selected_meta().map(|meta| meta.id.clone());
        self.sessions = choices.sessions;
        self.workspace = choices.workspace;
        self.current = choices.current;
        self.selected = chosen
            .and_then(|id| self.rows().iter().position(|row| row.meta.id == id))
            .unwrap_or(0);
        self.clamp();
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn named_only(&self) -> bool {
        self.named_only
    }

    pub fn show_path(&self) -> bool {
        self.show_path
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn is_current(&self, meta: &SessionMeta) -> bool {
        self.current.as_deref() == Some(meta.id.as_str())
    }

    /// Whether a session in this folder holds anything at all, which decides what an empty list
    /// says.
    pub fn folder_is_empty(&self) -> bool {
        !self
            .sessions
            .iter()
            .any(|meta| meta.workspace == self.workspace)
    }

    pub fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            Scope::CurrentFolder => Scope::All,
            Scope::All => Scope::CurrentFolder,
        };
        self.selected = 0;
        self.status = None;
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.selected = 0;
        self.status = None;
    }

    pub fn toggle_named(&mut self) {
        self.named_only = !self.named_only;
        self.selected = 0;
        self.status = None;
    }

    pub fn toggle_path(&mut self) {
        self.show_path = !self.show_path;
    }

    pub fn push(&mut self, text: &str) {
        match &mut self.mode {
            Mode::Renaming { title, .. } => title.push_str(text),
            Mode::ConfirmingDelete { .. } => {}
            Mode::Choosing => {
                self.query.push_str(text);
                self.selected = 0;
                self.status = None;
            }
        }
    }

    pub fn backspace(&mut self) {
        match &mut self.mode {
            Mode::Renaming { title, .. } => {
                title.pop();
            }
            Mode::ConfirmingDelete { .. } => {}
            Mode::Choosing => {
                self.query.pop();
                self.selected = 0;
            }
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        self.selected += 1;
        self.clamp();
    }

    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE);
    }

    pub fn page_down(&mut self) {
        self.selected += PAGE;
        self.clamp();
    }

    fn clamp(&mut self) {
        let count = self.rows().len();
        self.selected = self.selected.min(count.saturating_sub(1));
    }

    pub fn selected_meta(&self) -> Option<&SessionMeta> {
        self.rows().get(self.selected).map(|row| row.meta)
    }

    /// The command line that continues the chosen session, or nothing when it is the one already
    /// open.
    pub fn commit(&self) -> Option<String> {
        self.selected_meta()
            .filter(|meta| !self.is_current(meta))
            .map(|meta| format!("/resume {}", meta.id))
    }

    /// Ask whether the chosen session should be deleted; the open one never is.
    pub fn begin_delete(&mut self) {
        let Some(meta) = self.selected_meta() else {
            return;
        };
        if self.is_current(meta) {
            self.status = Some("Cannot delete the currently active session".to_string());
            return;
        }
        self.mode = Mode::ConfirmingDelete {
            id: meta.id.clone(),
        };
    }

    /// Delete the session asked about: it leaves the list now, and the command line removes it
    /// from the store.
    pub fn confirm_delete(&mut self) -> Option<String> {
        let Mode::ConfirmingDelete { id } = std::mem::replace(&mut self.mode, Mode::Choosing)
        else {
            return None;
        };
        self.sessions.retain(|meta| meta.id != id);
        self.clamp();
        self.status = Some("Session deleted".to_string());
        Some(format!("/resume --delete {id}"))
    }

    /// Start taking a new title for the chosen session.
    pub fn begin_rename(&mut self) {
        let Some(meta) = self.selected_meta() else {
            return;
        };
        self.mode = Mode::Renaming {
            id: meta.id.clone(),
            title: match meta.named {
                true => meta.title.clone(),
                false => String::new(),
            },
        };
    }

    /// Give the session the title taken: it shows now, and the command line keeps it. A blank
    /// title changes nothing.
    pub fn commit_rename(&mut self) -> Option<String> {
        let Mode::Renaming { id, title } = std::mem::replace(&mut self.mode, Mode::Choosing) else {
            return None;
        };
        let title = title.trim().to_string();
        if title.is_empty() {
            return None;
        }
        if let Some(meta) = self.sessions.iter_mut().find(|meta| meta.id == id) {
            meta.title = title.clone();
            meta.named = true;
        }
        Some(format!("/resume --rename {id} {title}"))
    }

    /// Back out of deleting or renaming, reporting whether there was anything to back out of.
    pub fn back_out(&mut self) -> bool {
        let busy = self.mode != Mode::Choosing;
        self.mode = Mode::Choosing;
        busy
    }

    /// The rows the list shows, in order.
    pub fn rows(&self) -> Vec<Row<'_>> {
        let candidates: Vec<&SessionMeta> = self
            .sessions
            .iter()
            .filter(|meta| self.scope == Scope::All || meta.workspace == self.workspace)
            .filter(|meta| !self.named_only || meta.named)
            .collect();

        let query = Query::parse(&self.query);
        let mut scored: Vec<(f64, &SessionMeta)> = candidates
            .into_iter()
            .filter_map(|meta| query.score(&searched(meta)).map(|score| (score, meta)))
            .collect();

        if self.sort == Sort::Threaded && query.is_empty() {
            return threaded(scored.into_iter().map(|(_, meta)| meta).collect());
        }
        match self.sort {
            Sort::Fuzzy => scored.sort_by(|(left_score, left), (right_score, right)| {
                left_score
                    .total_cmp(right_score)
                    .then_with(|| right.updated_at.cmp(&left.updated_at))
            }),
            Sort::Threaded | Sort::Recent => {
                scored.sort_by_key(|(_, meta)| std::cmp::Reverse(meta.updated_at))
            }
        }
        scored
            .into_iter()
            .map(|(_, meta)| Row {
                meta,
                prefix: String::new(),
            })
            .collect()
    }
}

/// What a search looks through for a session.
fn searched(meta: &SessionMeta) -> String {
    format!("{} {} {}", meta.id, meta.title, meta.workspace.display())
}

/// A search as it was typed: a pattern after `re:`, phrases in quotes that must appear as they
/// are, and words matched loosely.
enum Query {
    Everything,
    Pattern(Option<regex::Regex>),
    Words {
        phrases: Vec<String>,
        words: Vec<String>,
    },
}

impl Query {
    fn parse(typed: &str) -> Query {
        let typed = typed.trim();
        if typed.is_empty() {
            return Query::Everything;
        }
        if let Some(pattern) = typed.strip_prefix("re:") {
            return Query::Pattern(
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .ok(),
            );
        }

        let mut phrases = Vec::new();
        let mut rest = String::new();
        let mut quoted = typed.split('"');
        if let Some(first) = quoted.next() {
            rest.push_str(first);
        }
        for (index, part) in quoted.enumerate() {
            match index % 2 {
                0 => phrases.push(normalized(part)),
                _ => {
                    rest.push(' ');
                    rest.push_str(part);
                }
            }
        }
        Query::Words {
            phrases: phrases
                .into_iter()
                .filter(|phrase| !phrase.is_empty())
                .collect(),
            words: rest.split_whitespace().map(str::to_string).collect(),
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Query::Everything)
    }

    /// How well `text` answers the search, lower being better, or nothing when it does not.
    fn score(&self, text: &str) -> Option<f64> {
        match self {
            Query::Everything => Some(0.0),
            Query::Pattern(pattern) => pattern
                .as_ref()
                .filter(|pattern| pattern.is_match(text))
                .map(|_| 0.0),
            Query::Words { phrases, words } => {
                let flattened = normalized(text);
                if !phrases.iter().all(|phrase| flattened.contains(phrase)) {
                    return None;
                }
                words.iter().try_fold(0.0, |total, word| {
                    micro_models::fuzzy::match_score(word, text).map(|found| total + found.score)
                })
            }
        }
    }
}

/// Lowercased, with every run of whitespace one space.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The sessions as threads: each fork under the session it came from, threads and forks alike
/// ordered by the latest activity anywhere in them.
fn threaded(sessions: Vec<&SessionMeta>) -> Vec<Row<'_>> {
    let present: HashMap<&str, &SessionMeta> = sessions
        .iter()
        .map(|meta| (meta.id.as_str(), *meta))
        .collect();
    let mut children: HashMap<Option<&str>, Vec<&SessionMeta>> = HashMap::new();
    for meta in &sessions {
        let parent = meta
            .parent
            .as_deref()
            .filter(|parent| present.contains_key(parent));
        children.entry(parent).or_default().push(meta);
    }

    let mut latest: HashMap<&str, i64> = HashMap::new();
    fn activity<'a>(
        meta: &'a SessionMeta,
        children: &HashMap<Option<&'a str>, Vec<&'a SessionMeta>>,
        latest: &mut HashMap<&'a str, i64>,
    ) -> i64 {
        if let Some(known) = latest.get(meta.id.as_str()) {
            return *known;
        }
        latest.insert(meta.id.as_str(), meta.updated_at);
        let newest = children
            .get(&Some(meta.id.as_str()))
            .map(|kids| {
                kids.iter()
                    .map(|kid| activity(kid, children, latest))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
            .max(meta.updated_at);
        latest.insert(meta.id.as_str(), newest);
        newest
    }
    for meta in &sessions {
        activity(meta, &children, &mut latest);
    }
    for kids in children.values_mut() {
        kids.sort_by(|left, right| {
            latest[right.id.as_str()]
                .cmp(&latest[left.id.as_str()])
                .then_with(|| right.id.cmp(&left.id))
        });
    }

    let mut rows = Vec::new();
    let mut visited = std::collections::HashSet::new();
    fn walk<'a>(
        parent: Option<&'a str>,
        continues: &mut Vec<bool>,
        children: &HashMap<Option<&'a str>, Vec<&'a SessionMeta>>,
        visited: &mut std::collections::HashSet<&'a str>,
        rows: &mut Vec<Row<'a>>,
    ) {
        let Some(kids) = children.get(&parent) else {
            return;
        };
        for (index, kid) in kids.iter().enumerate() {
            if !visited.insert(kid.id.as_str()) {
                continue;
            }
            let last = index + 1 == kids.len();
            let prefix = match parent {
                None => String::new(),
                Some(_) => {
                    let mut prefix: String = continues
                        .iter()
                        .map(|goes_on| match goes_on {
                            true => "│  ",
                            false => "   ",
                        })
                        .collect();
                    prefix.push_str(match last {
                        true => "└─ ",
                        false => "├─ ",
                    });
                    prefix
                }
            };
            rows.push(Row { meta: kid, prefix });
            let pushed = parent.is_some();
            if pushed {
                continues.push(!last);
            }
            walk(Some(kid.id.as_str()), continues, children, visited, rows);
            if pushed {
                continues.pop();
            }
        }
    }
    walk(None, &mut Vec::new(), &children, &mut visited, &mut rows);
    rows
}

/// How long ago a moment in milliseconds was, in the fewest characters: `now`, `5m`, `3h`, `2d`,
/// `1w`, `4mo`, `1y`.
pub fn age(then_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - then_ms).max(0) / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match () {
        _ if minutes < 1 => "now".to_string(),
        _ if hours < 1 => format!("{minutes}m"),
        _ if days < 1 => format!("{hours}h"),
        _ if days < 7 => format!("{days}d"),
        _ if days < 30 => format!("{}w", days / 7),
        _ if days < 365 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, workspace: &str, updated_at: i64) -> SessionMeta {
        SessionMeta {
            v: 1,
            id: id.to_string(),
            created_at: updated_at,
            updated_at,
            workspace: PathBuf::from(workspace),
            model_id: "anthropic/claude-opus-5".to_string(),
            title: format!("about {id}"),
            named: false,
            message_count: 2,
            parent: None,
            org_id: None,
            agent_id: None,
        }
    }

    fn selector(sessions: Vec<SessionMeta>) -> SessionSelector {
        SessionSelector::new(SessionChoices {
            sessions,
            workspace: PathBuf::from("/here"),
            current: Some("open".to_string()),
        })
    }

    fn ids(selector: &SessionSelector) -> Vec<String> {
        selector
            .rows()
            .iter()
            .map(|row| format!("{}{}", row.prefix, row.meta.id))
            .collect()
    }

    #[test]
    fn this_folder_shows_first_and_tab_shows_every_folder() {
        let mut selector = selector(vec![meta("a", "/here", 3), meta("b", "/there", 2)]);
        assert_eq!(ids(&selector), ["a"]);
        selector.toggle_scope();
        assert_eq!(ids(&selector), ["a", "b"]);
    }

    #[test]
    fn a_fork_sits_under_the_session_it_came_from() {
        let fork = SessionMeta {
            parent: Some("a".to_string()),
            ..meta("a1", "/here", 9)
        };
        let second_fork = SessionMeta {
            parent: Some("a".to_string()),
            ..meta("a2", "/here", 5)
        };
        let selector = selector(vec![
            meta("a", "/here", 1),
            meta("b", "/here", 4),
            fork,
            second_fork,
        ]);
        assert_eq!(ids(&selector), ["a", "├─ a1", "└─ a2", "b"]);
    }

    #[test]
    fn a_search_narrows_the_list_and_flattens_it() {
        let mut selector = selector(vec![meta("alpha", "/here", 1), meta("beta", "/here", 2)]);
        selector.push("alp");
        assert_eq!(ids(&selector), ["alpha"]);

        selector.query.clear();
        selector.push("\"about beta\"");
        assert_eq!(ids(&selector), ["beta"]);

        selector.query.clear();
        selector.push("re:^ALPHA");
        assert_eq!(ids(&selector), ["alpha"]);
    }

    #[test]
    fn named_sessions_can_be_shown_alone() {
        let named = SessionMeta {
            named: true,
            ..meta("kept", "/here", 1)
        };
        let mut selector = selector(vec![named, meta("other", "/here", 2)]);
        selector.toggle_named();
        assert_eq!(ids(&selector), ["kept"]);
    }

    #[test]
    fn the_open_session_is_never_deleted() {
        let mut selector = selector(vec![meta("open", "/here", 2), meta("old", "/here", 1)]);
        selector.begin_delete();
        assert_eq!(selector.mode(), &Mode::Choosing);
        assert!(selector.status().unwrap().contains("Cannot delete"));

        selector.select_next();
        selector.begin_delete();
        assert_eq!(
            selector.confirm_delete().as_deref(),
            Some("/resume --delete old")
        );
        assert_eq!(ids(&selector), ["open"]);
    }

    #[test]
    fn a_rename_shows_at_once_and_is_kept_by_its_command() {
        let mut selector = selector(vec![meta("old", "/here", 1)]);
        selector.begin_rename();
        selector.push("the plan");
        assert_eq!(
            selector.commit_rename().as_deref(),
            Some("/resume --rename old the plan")
        );
        let row = &selector.rows()[0];
        assert_eq!(row.meta.title, "the plan");
        assert!(row.meta.named);
    }

    #[test]
    fn choosing_the_open_session_does_nothing() {
        let selector = selector(vec![meta("open", "/here", 1)]);
        assert_eq!(selector.commit(), None);
    }

    #[test]
    fn ages_are_said_in_the_fewest_characters() {
        let minute = 60_000;
        assert_eq!(age(0, 30_000), "now");
        assert_eq!(age(0, 5 * minute), "5m");
        assert_eq!(age(0, 3 * 60 * minute), "3h");
        assert_eq!(age(0, 2 * 24 * 60 * minute), "2d");
        assert_eq!(age(0, 8 * 24 * 60 * minute), "1w");
        assert_eq!(age(0, 120 * 24 * 60 * minute), "4mo");
        assert_eq!(age(0, 400 * 24 * 60 * minute), "1y");
    }
}
