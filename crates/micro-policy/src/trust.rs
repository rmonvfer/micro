//! Whether a project has been vouched for.
//!
//! A trusted project is one the user has said they are willing to have edited without
//! being asked about every file. The decision is kept per workspace and read when a run
//! starts, so it survives the session that made it. It is deliberately not a way to skip
//! approval for shell commands: a command can reach outside the workspace, and trusting a
//! directory says nothing about that.

use crate::error::PolicyError;
use crate::error::Result;
use crate::policy::micro_home;
use crate::policy::Mode;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

/// The file the decisions live in, beside the policy.
pub const TRUST_FILE_NAME: &str = "trust.json";

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
    /// Reads `$MICRO_DIR/trust.json`. A missing file means nothing has been decided yet,
    /// which is not an error.
    pub async fn load() -> Result<Self> {
        TrustStore::load_from(micro_home()?).await
    }

    pub async fn load_from(directory: impl AsRef<Path>) -> Result<Self> {
        let path = directory.as_ref().join(TRUST_FILE_NAME);
        let raw = match tokio::fs::read(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TrustStore::default())
            }
            Err(source) => return Err(PolicyError::io(path, source)),
        };
        serde_json::from_slice(&raw).map_err(|source| PolicyError::json(path, source))
    }

    pub async fn save(&self) -> Result<()> {
        self.save_to(micro_home()?).await
    }

    pub async fn save_to(&self, directory: impl AsRef<Path>) -> Result<()> {
        let directory = directory.as_ref();
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|source| PolicyError::io(directory, source))?;
        let path = directory.join(TRUST_FILE_NAME);
        let encoded =
            serde_json::to_vec_pretty(self).map_err(|source| PolicyError::json(&path, source))?;
        tokio::fs::write(&path, encoded)
            .await
            .map_err(|source| PolicyError::io(path, source))
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

    /// How a run should start in this workspace.
    ///
    /// Trusting a project buys freedom to edit inside it and nothing more, so a mode that
    /// already grants at least that much is left alone.
    pub fn mode_for(&self, workspace: impl AsRef<Path>, requested: Mode) -> Mode {
        match (requested, self.is_trusted(workspace)) {
            (Mode::Cautious, true) => Mode::Workspace,
            (mode, _) => mode,
        }
    }
}

/// The key a workspace is filed under: its canonical path when it can be resolved, and
/// what was asked for when it cannot, so a project that has since moved is still findable.
fn key(workspace: &Path) -> String {
    let resolved: PathBuf = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    resolved.display().to_string()
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

    /// Trust widens the cautious default and nothing else: a mode the user asked for
    /// explicitly is theirs, and an untrusted project is left as it was.
    #[test]
    fn trust_only_widens_the_cautious_default() {
        let mut store = TrustStore::default();
        store.decide("/project", true);

        assert_eq!(store.mode_for("/project", Mode::Cautious), Mode::Workspace);
        assert_eq!(
            store.mode_for("/project", Mode::Unrestricted),
            Mode::Unrestricted
        );
        assert_eq!(store.mode_for("/elsewhere", Mode::Cautious), Mode::Cautious);
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
