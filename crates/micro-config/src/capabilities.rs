//! What each extension has been allowed to do.

use crate::config_dir;
use crate::ConfigError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

/// The file the decisions live in, beside the settings and the trust decisions.
pub const CAPABILITIES_FILE_NAME: &str = "capabilities.json";

/// What one extension was allowed to do, and when that was decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDecision {
    /// The capability names granted.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Milliseconds since the epoch, so a listing can say how old a decision is.
    pub decided_at: i64,
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityStore {
    #[serde(default)]
    extensions: BTreeMap<String, CapabilityDecision>,
}

impl CapabilityStore {
    /// Reads `capabilities.json` from the configuration directory.
    pub async fn load() -> Result<Self, ConfigError> {
        CapabilityStore::load_from(config_dir()?).await
    }

    pub async fn load_from(directory: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = directory.as_ref().join(CAPABILITIES_FILE_NAME);
        let raw = match tokio::fs::read(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CapabilityStore::default())
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
        let path = directory.join(CAPABILITIES_FILE_NAME);
        let encoded = serde_json::to_vec_pretty(self).map_err(|source| malformed(&path, source))?;
        tokio::fs::write(&path, encoded)
            .await
            .map_err(|source| unreadable(&path, source))
    }

    /// What was decided about an extension, if anything was.
    pub fn decision(&self, extension: impl AsRef<Path>) -> Option<&CapabilityDecision> {
        self.extensions.get(&key(extension.as_ref()))
    }

    /// Record a decision, replacing whatever was decided before.
    pub fn decide(&mut self, extension: impl AsRef<Path>, capabilities: Vec<String>) {
        self.extensions.insert(
            key(extension.as_ref()),
            CapabilityDecision {
                capabilities,
                decided_at: now_ms(),
            },
        );
    }

    /// Forget an extension, so the next run asks about it as if it were new.
    pub fn forget(&mut self, extension: impl AsRef<Path>) -> bool {
        self.extensions.remove(&key(extension.as_ref())).is_some()
    }

    /// Every extension with a decision, in path order.
    pub fn extensions(&self) -> impl Iterator<Item = (&str, &CapabilityDecision)> {
        self.extensions
            .iter()
            .map(|(path, decision)| (path.as_str(), decision))
    }
}

/// The key an extension is filed under: its canonical path when it can be resolved, and what was
/// asked for when it cannot.
fn key(extension: &Path) -> String {
    let resolved: PathBuf = extension
        .canonicalize()
        .unwrap_or_else(|_| extension.to_path_buf());
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
            "micro-capabilities-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn nothing_is_granted_until_it_is_said_so() {
        let store = CapabilityStore::default();
        assert_eq!(store.decision("/somewhere/thing.ts"), None);
    }

    
    #[test]
    fn granting_nothing_is_not_the_same_as_never_having_been_asked() {
        let mut store = CapabilityStore::default();
        store.decide("/x/thing.ts", Vec::new());

        let decided = store.decision("/x/thing.ts").expect("a decision");
        assert!(decided.capabilities.is_empty());
        assert_eq!(store.decision("/x/other.ts"), None);
    }

    #[tokio::test]
    async fn a_decision_survives_being_written_out() {
        let home = scratch("roundtrip");
        let mut store = CapabilityStore::default();
        store.decide(&home, vec!["exec".to_string(), "ui".to_string()]);
        store.save_to(&home).await.unwrap();

        let reopened = CapabilityStore::load_from(&home).await.unwrap();
        let decided = reopened.decision(&home).expect("a decision");
        assert_eq!(decided.capabilities, vec!["exec", "ui"]);
        assert_eq!(reopened.extensions().count(), 1);
    }

    #[tokio::test]
    async fn a_home_with_no_file_yet_has_decided_nothing() {
        let home = scratch("empty");
        let store = CapabilityStore::load_from(&home).await.unwrap();
        assert_eq!(store.extensions().count(), 0);
    }

    #[test]
    fn forgetting_an_extension_leaves_nothing_behind() {
        let mut store = CapabilityStore::default();
        store.decide("/x/thing.ts", vec!["ui".to_string()]);
        assert!(store.forget("/x/thing.ts"));
        assert!(!store.forget("/x/thing.ts"));
        assert_eq!(store.decision("/x/thing.ts"), None);
    }
}
