use crate::config_dir;
use crate::ConfigError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

/// The file the decisions live in, beside the settings.
pub const TRUST_FILE_NAME: &str = "trust.json";

/// What a project keeps under its own directory that micro would run or be steered by.
const TRUST_REQUIRING: &[&str] = &[
    "settings.json",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

/// The directory a project keeps its own configuration in.
pub const PROJECT_DIR: &str = ".micro";

/// Whether this project carries anything that needs to be trusted before it is used.
pub fn requires_decision(workspace: impl AsRef<Path>) -> bool {
    let configured = workspace.as_ref().join(PROJECT_DIR);
    TRUST_REQUIRING
        .iter()
        .any(|entry| configured.join(entry).exists())
}

/// What to do about a project nobody has decided about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTrust {
    /// Put the question to the user, when there is someone to ask.
    #[default]
    Ask,
    /// Trust it without asking.
    Always,
    /// Do not trust it, and do not ask.
    Never,
}

/// What was decided about one project, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustDecision {
    pub trusted: bool,
    /// Milliseconds since the epoch, so a listing can say how old a decision is.
    pub decided_at: i64,
}

/// Every project decision, keyed by the workspace's canonical path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(default)]
    projects: BTreeMap<String, TrustDecision>,
}

impl TrustStore {
    /// Reads `trust.json` from the configuration directory.
    pub async fn load() -> Result<Self, ConfigError> {
        TrustStore::load_from(config_dir()?).await
    }

    pub async fn load_from(directory: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = directory.as_ref().join(TRUST_FILE_NAME);
        let raw = match tokio::fs::read(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TrustStore::default())
            }
            Err(source) => return Err(unreadable(&path, source)),
        };
        serde_json::from_slice(&raw).map_err(|source| malformed(&path, source))
    }

    pub async fn save(&self) -> Result<(), ConfigError> {
        self.save_to(config_dir()?).await
    }

    pub async fn save_to(&self, directory: impl AsRef<Path>) -> Result<(), ConfigError> {
        let directory = directory.as_ref();
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|source| unreadable(directory, source))?;
        let path = directory.join(TRUST_FILE_NAME);
        let encoded = serde_json::to_vec_pretty(self).map_err(|source| malformed(&path, source))?;
        tokio::fs::write(&path, encoded)
            .await
            .map_err(|source| unreadable(&path, source))
    }

    /// What was decided about a project, if anything was.
    pub fn decision(&self, workspace: impl AsRef<Path>) -> Option<&TrustDecision> {
        self.projects.get(&key(workspace.as_ref()))
    }

    pub fn is_trusted(&self, workspace: impl AsRef<Path>) -> bool {
        self.decision(workspace)
            .is_some_and(|decision| decision.trusted)
    }

    /// Record a decision, replacing whatever was decided before.
    pub fn decide(&mut self, workspace: impl AsRef<Path>, trusted: bool) {
        self.projects.insert(
            key(workspace.as_ref()),
            TrustDecision {
                trusted,
                decided_at: now_ms(),
            },
        );
    }

    /// Forget a project, so the next run asks about it as if it were new.
    pub fn forget(&mut self, workspace: impl AsRef<Path>) -> bool {
        self.projects.remove(&key(workspace.as_ref())).is_some()
    }

    /// Every project with a decision, in path order.
    pub fn projects(&self) -> impl Iterator<Item = (&str, &TrustDecision)> {
        self.projects
            .iter()
            .map(|(path, decision)| (path.as_str(), decision))
    }
}

/// The key a workspace is filed under: its canonical path when it can be resolved, and what was
/// asked for when it cannot.
fn key(workspace: &Path) -> String {
    let resolved: PathBuf = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    resolved.display().to_string()
}

fn unreadable(path: impl AsRef<Path>, source: std::io::Error) -> ConfigError {
    ConfigError::Malformed {
        path: path.as_ref().display().to_string(),
        message: source.to_string(),
    }
}

fn malformed(path: impl AsRef<Path>, source: serde_json::Error) -> ConfigError {
    ConfigError::Malformed {
        path: path.as_ref().display().to_string(),
        message: source.to_string(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "micro-trust-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn nothing_is_trusted_until_it_is_said_so() {
        let store = TrustStore::default();
        assert!(!store.is_trusted("/somewhere"));
        assert_eq!(store.decision("/somewhere"), None);
    }

    #[test]
    fn a_decision_is_remembered_either_way() {
        let mut store = TrustStore::default();
        store.decide("/project", true);
        assert!(store.is_trusted("/project"));

        store.decide("/project", false);
        assert!(!store.is_trusted("/project"));
        assert!(
            store.decision("/project").is_some(),
            "declining is a decision too, and asking again would ignore it"
        );
    }

    #[test]
    fn forgetting_a_project_leaves_nothing_behind() {
        let mut store = TrustStore::default();
        store.decide("/project", true);
        assert!(store.forget("/project"));
        assert!(!store.forget("/project"));
        assert_eq!(store.decision("/project"), None);
    }

    /// A project nobody has decided about is not trusted.
    #[test]
    fn an_undecided_project_is_not_a_refused_one() {
        let mut store = TrustStore::default();
        store.decide("/refused", false);

        assert!(!store.is_trusted("/refused"));
        assert!(store.decision("/refused").is_some());

        assert!(!store.is_trusted("/unknown"));
        assert_eq!(store.decision("/unknown"), None);
    }

    #[tokio::test]
    async fn a_decision_survives_being_written_out() {
        let home = scratch("roundtrip");
        let mut store = TrustStore::default();
        store.decide(&home, true);
        store.save_to(&home).await.unwrap();

        let reopened = TrustStore::load_from(&home).await.unwrap();
        assert!(reopened.is_trusted(&home));
        assert_eq!(reopened.projects().count(), 1);
    }

    #[tokio::test]
    async fn a_home_with_no_file_yet_has_decided_nothing() {
        let home = scratch("empty");
        let store = TrustStore::load_from(&home).await.unwrap();
        assert_eq!(store.projects().count(), 0);
    }
}
