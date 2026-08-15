//! What a project asks for on its own behalf.
//!
//! A project keeps its settings in `<workspace>/.micro/settings.json`, beside the
//! extensions, skills and prompts it ships. It is read only once the project has been
//! trusted, for the same reason those are: a file in a repository is written by whoever
//! wrote the repository, and what it asks for here decides how much of the machine the
//! session may touch.
//!
//! Only the keys a project has business setting live here. A user's own preferences —
//! which model, what the interface looks like — stay in the user's config; a checkout
//! that could set those would be deciding things about someone else's machine.

use crate::ConfigError;
use crate::Result;
use crate::PROJECT_DIR;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;

/// The file a project keeps its own settings in, inside [`PROJECT_DIR`].
pub const PROJECT_SETTINGS_FILE: &str = "settings.json";

/// The settings a project supplies, each absent unless the project said something.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// What commands this project's sessions may do: `read-only`, `workspace-write`, or
    /// `full`, or a table spelling out what `workspace-write` grants. Held as written,
    /// since resolving one to a policy is the sandbox's job rather than this crate's.
    pub sandbox: Option<Value>,
}

impl ProjectConfig {
    /// What the project at `workspace` asks for.
    ///
    /// An untrusted project asks for nothing: the file is not read at all rather than read
    /// and then second-guessed. A project with no file asks for nothing either, which is
    /// the ordinary case and not an error.
    pub fn load(workspace: impl AsRef<Path>, trusted: bool) -> Result<ProjectConfig> {
        if !trusted {
            return Ok(ProjectConfig::default());
        }
        ProjectConfig::read(&Self::path(workspace))
    }

    /// Where a project's settings live.
    pub fn path(workspace: impl AsRef<Path>) -> PathBuf {
        workspace
            .as_ref()
            .join(PROJECT_DIR)
            .join(PROJECT_SETTINGS_FILE)
    }

    fn read(path: &Path) -> Result<ProjectConfig> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProjectConfig::default())
            }
            Err(error) => {
                return Err(ConfigError::Io {
                    path: path.display().to_string(),
                    message: error.to_string(),
                })
            }
        };
        if contents.trim().is_empty() {
            return Ok(ProjectConfig::default());
        }
        serde_json::from_str(&contents).map_err(|error| ConfigError::Malformed {
            path: path.display().to_string(),
            message: error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("micro-project-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join(PROJECT_DIR)).unwrap();
        path
    }

    fn write(workspace: &Path, contents: &str) {
        std::fs::write(ProjectConfig::path(workspace), contents).unwrap();
    }

    #[test]
    fn a_trusted_project_is_read() {
        let workspace = scratch("trusted");
        write(&workspace, r#"{"sandbox":"read-only"}"#);

        let project = ProjectConfig::load(&workspace, true).unwrap();
        assert_eq!(project.sandbox, Some(Value::from("read-only")));
    }

    /// An untrusted project is not read at all. What it asks for is exactly the kind of
    /// thing trust is asked about, so a checkout nobody vouched for gets no say.
    #[test]
    fn an_untrusted_project_is_not_read() {
        let workspace = scratch("untrusted");
        write(&workspace, r#"{"sandbox":"full"}"#);

        assert_eq!(
            ProjectConfig::load(&workspace, false).unwrap(),
            ProjectConfig::default()
        );
    }

    #[test]
    fn a_project_with_no_settings_asks_for_nothing() {
        let workspace = scratch("absent");
        assert_eq!(
            ProjectConfig::load(&workspace, true).unwrap(),
            ProjectConfig::default()
        );
    }

    /// A key this build does not know is stepped over rather than taken as a reason to
    /// ignore the file, so a project written against a later micro still configures this
    /// one as far as it can.
    #[test]
    fn a_key_from_a_later_build_does_not_cost_the_rest_of_the_file() {
        let workspace = scratch("unknown-key");
        write(&workspace, r#"{"sandbox":"read-only","telepathy":true}"#);

        let project = ProjectConfig::load(&workspace, true).unwrap();
        assert_eq!(project.sandbox, Some(Value::from("read-only")));
    }

    #[test]
    fn a_file_that_is_not_json_is_reported_rather_than_ignored() {
        let workspace = scratch("malformed");
        write(&workspace, "{ this is not json");

        let error = ProjectConfig::load(&workspace, true).unwrap_err();
        assert!(error.to_string().contains("settings.json"), "{error}");
    }
}
