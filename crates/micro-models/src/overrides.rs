use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::catalog::Catalog;
use crate::error::{Error, Result};

/// The user catalog file, read from the micro configuration directory.
pub const USER_CATALOG_FILE: &str = "models.json";

/// The micro configuration directory, which is where the user's own catalog lives.
pub fn config_dir() -> Result<PathBuf> {
    micro_dirs::config_dir().ok_or(Error::NoHomeDir)
}


pub fn user_catalog_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(USER_CATALOG_FILE))
}

impl Catalog {
    /// The bundled catalog with the user's overrides applied.
    pub fn load() -> Result<Catalog> {
        Catalog::load_from(&user_catalog_path()?)
    }

    /// Load with the user catalog read from an explicit path.
    pub fn load_from(path: &Path) -> Result<Catalog> {
        let mut catalog = Catalog::bundled();
        if let Some(json) = read_optional(path)? {
            catalog.apply_overrides(&json)?;
        }
        Ok(catalog)
    }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::WireApi;
    use std::env;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory that removes itself, so tests never read or write the caller's real
    /// configuration directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = format!(
                "micro-models-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("create scratch directory");
            TempDir(path)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, contents).expect("write scratch file");
            path
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The user's catalog is one of the things they wrote, so it sits with the settings wherever
    /// those are.
    #[test]
    fn the_user_catalog_sits_in_the_configuration_directory() {
        assert_eq!(
            user_catalog_path().unwrap(),
            micro_dirs::config_dir().unwrap().join(USER_CATALOG_FILE)
        );
    }

    #[test]
    fn a_missing_user_catalog_leaves_the_bundle_untouched() {
        let dir = TempDir::new();
        let catalog = Catalog::load_from(&dir.path(USER_CATALOG_FILE)).unwrap();
        assert_eq!(catalog.len(), Catalog::bundled().len());
    }

    #[test]
    fn a_user_catalog_is_applied_over_the_bundle() {
        let dir = TempDir::new();
        let path = dir.write(
            USER_CATALOG_FILE,
            r#"{"providers": {
                "ollama": {
                    "base_url": "http://localhost:11434/v1",
                    "api": "openai-completions",
                    "models": [{"id": "qwen3-coder:30b", "name": "Qwen3 Coder 30B", "context_window": 262144}]
                },
                "anthropic": {"models": [{"id": "claude-opus-5", "max_output_tokens": 64000}]}
            }}"#,
        );

        let catalog = Catalog::load_from(&path).unwrap();

        let custom = catalog.get("ollama", "qwen3-coder:30b").unwrap();
        assert_eq!(custom.api, WireApi::OpenaiCompletions);
        assert_eq!(custom.context_window, 262_144);

        let opus = catalog.get("anthropic", "claude-opus-5").unwrap();
        assert_eq!(opus.max_output_tokens, 64_000);
        assert_eq!(opus.context_window, 1_000_000);
    }

    #[test]
    fn a_user_registered_model_is_resolvable() {
        let dir = TempDir::new();
        let path = dir.write(
            USER_CATALOG_FILE,
            r#"{"providers": {"ollama": {
                "base_url": "http://localhost:11434/v1",
                "api": "openai-completions",
                "models": [{"id": "qwen3-coder:30b", "aliases": ["local"]}]
            }}}"#,
        );

        let catalog = Catalog::load_from(&path).unwrap();
        assert_eq!(
            catalog.resolve("local").model().unwrap().qualified_id(),
            "ollama/qwen3-coder:30b"
        );
        assert_eq!(
            catalog
                .resolve("ollama/qwen3-coder:30b")
                .model()
                .unwrap()
                .base_url,
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn a_malformed_user_catalog_is_reported() {
        let dir = TempDir::new();
        let path = dir.write(USER_CATALOG_FILE, "{ not json");
        let error = Catalog::load_from(&path).unwrap_err();
        assert!(
            matches!(error, Error::Parse(_)),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unreadable_user_catalog_is_reported_with_its_path() {
        
        let dir = TempDir::new();
        fs::create_dir_all(dir.path(USER_CATALOG_FILE)).unwrap();
        let error = Catalog::load_from(&dir.path(USER_CATALOG_FILE)).unwrap_err();
        assert!(
            matches!(error, Error::Read { ref path, .. } if path.ends_with(USER_CATALOG_FILE)),
            "unexpected error: {error}"
        );
    }
}
