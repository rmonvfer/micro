//! Which built-in tools a session starts with, as the `default_tools` setting chooses them.

use std::path::Path;
use std::sync::RwLock;

/// The `default_tools` selection in force, and what it chooses among.
#[derive(Debug, Clone)]
pub struct DefaultTools {
    /// Every built-in tool this run has, the set the setting chooses from.
    builtin: Vec<String>,
    /// The built-in tools the setting selected when it was last read.
    selected: Vec<String>,
    /// Whether `--tools` chose the tools instead, so the setting has no say.
    overridden: bool,
}

impl DefaultTools {
    /// The selection the user's and the project's settings make among `builtin`.
    pub fn new(
        builtin: Vec<String>,
        user: Option<Vec<String>>,
        workspace: &Path,
        trusted: bool,
        overridden: bool,
    ) -> Self {
        let selected = selection(&builtin, user, workspace, trusted);
        DefaultTools {
            builtin,
            selected,
            overridden,
        }
    }

    /// The tools to offer the model at startup, out of every tool it has; nothing when the setting
    /// keeps every one of them.
    pub fn initial_offer(&self, available: &[String]) -> Option<Vec<String>> {
        if self.overridden {
            return None;
        }
        let offered: Vec<String> = available
            .iter()
            .filter(|name| !self.builtin.contains(name) || self.selected.contains(name))
            .cloned()
            .collect();
        (offered.len() != available.len()).then_some(offered)
    }

    /// Read the setting again and offer the tools it newly names. Tools it no longer names stay
    /// on, and tools turned off since startup stay off unless newly named. Answers with the tools
    /// that were turned on.
    pub fn reload(
        &mut self,
        user: Option<Vec<String>>,
        workspace: &Path,
        trusted: bool,
        offered: &RwLock<Option<Vec<String>>>,
        available: &[String],
    ) -> Vec<String> {
        if self.overridden {
            return Vec::new();
        }
        let selected = selection(&self.builtin, user, workspace, trusted);
        let added: Vec<String> = selected
            .iter()
            .filter(|name| !self.selected.contains(name) && available.contains(name))
            .cloned()
            .collect();
        self.selected = selected;

        let mut offered = offered
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(names) = offered.as_mut() else {
            return Vec::new();
        };
        let enabled: Vec<String> = added
            .into_iter()
            .filter(|name| !names.contains(name))
            .collect();
        names.extend(enabled.iter().cloned());
        enabled
    }
}

/// The built-in tools the settings select: the user's list with the project's laid over it, or
/// every built-in tool when neither says anything.
fn selection(
    builtin: &[String],
    user: Option<Vec<String>>,
    workspace: &Path,
    trusted: bool,
) -> Vec<String> {
    let project = micro_config::ProjectConfig::load(workspace, trusted)
        .ok()
        .and_then(|project| project.default_tools);
    match micro_config::merge_default_tools(user, project) {
        Some(entries) => micro_config::resolve_default_tools(&entries, builtin),
        None => builtin.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    fn workspace(label: &str, project: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "micro-default-tools-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(micro_config::PROJECT_DIR)).unwrap();
        if let Some(settings) = project {
            std::fs::write(micro_config::ProjectConfig::path(&root), settings).unwrap();
        }
        root
    }

    #[test]
    fn the_setting_narrows_the_built_in_tools_and_keeps_the_rest() {
        let root = workspace("narrow", Some(r#"{"default_tools":["-bash"]}"#));
        let selection = DefaultTools::new(
            names(&["read", "bash", "edit"]),
            Some(names(&["read", "bash"])),
            &root,
            true,
            false,
        );
        let offered =
            selection.initial_offer(&names(&["read", "bash", "edit", "mcp__notes__find"]));
        assert_eq!(offered, Some(names(&["read", "mcp__notes__find"])));
    }

    #[test]
    fn an_untrusted_project_says_nothing() {
        let root = workspace("untrusted", Some(r#"{"default_tools":[]}"#));
        let selection = DefaultTools::new(names(&["read", "bash"]), None, &root, false, false);
        assert_eq!(selection.initial_offer(&names(&["read", "bash"])), None);
    }

    #[test]
    fn the_tools_flag_overrides_the_setting() {
        let root = workspace("overridden", None);
        let selection = DefaultTools::new(
            names(&["read", "bash"]),
            Some(Vec::new()),
            &root,
            true,
            true,
        );
        assert_eq!(selection.initial_offer(&names(&["read", "bash"])), None);
    }

    #[test]
    fn reloading_turns_on_only_newly_named_tools() {
        let root = workspace("reload", None);
        let available = names(&["read", "bash", "edit", "grep"]);
        let mut selection = DefaultTools::new(
            names(&["read", "bash", "edit", "grep"]),
            Some(names(&["read", "bash"])),
            &root,
            true,
            false,
        );
        let offered = RwLock::new(selection.initial_offer(&available));
        assert_eq!(*offered.read().unwrap(), Some(names(&["read", "bash"])));

        offered
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .retain(|name| name != "bash");

        let enabled = selection.reload(
            Some(names(&["read", "grep"])),
            &root,
            true,
            &offered,
            &available,
        );
        assert_eq!(enabled, names(&["grep"]));
        assert_eq!(*offered.read().unwrap(), Some(names(&["read", "grep"])));
    }
}
