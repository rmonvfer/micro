//! The macOS half of the enforcement: a Seatbelt profile handed to `sandbox-exec`.
//!
//! Derived from openai/codex codex-rs/core/src/seatbelt.rs at commit a8c7f5391c and
//! codex-rs/sandboxing/src/seatbelt.rs at commit 486df09a00; modified.

use crate::SandboxRules;
use crate::WritableRoot;
use std::path::PathBuf;

const BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");
const NETWORK_POLICY: &str = include_str!("seatbelt_network_policy.sbpl");

/// Only the copy in `/usr/bin` is ever run. Resolving `sandbox-exec` through PATH would
/// let anything that can write to a PATH directory decide what "sandboxed" means; if the
/// system copy itself has been replaced, the attacker already had root.
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Build the argument list that runs `command` under a profile allowing writes to
/// `roots` and nothing else.
///
/// Paths never reach the profile text. They travel as `-D` definitions and the profile
/// refers to them by name, so a directory named `") (allow default) ;` is a directory
/// name rather than a policy edit.
pub(crate) fn seatbelt_args(rules: &SandboxRules, command: Vec<String>) -> Vec<String> {
    let (write_policy, mut definitions) = write_policy(&rules.writable_roots);
    let (read_policy, read_definitions) = read_policy(rules.readable_roots.as_deref());
    definitions.extend(read_definitions);
    let (process_policy, process_definitions) = process_policy(&rules.allowed_executables);
    definitions.extend(process_definitions);
    let network_policy = if rules.allow_network {
        format!("(allow network-outbound)\n(allow network-inbound)\n{NETWORK_POLICY}")
    } else {
        String::new()
    };

    let profile = format!(
        "{BASE_POLICY}\n{process_policy}\n{read_policy}\n{write_policy}\n{network_policy}"
    );

    let mut args = vec!["-p".to_string(), profile];
    args.extend(
        definitions
            .into_iter()
            .map(|(name, path)| format!("-D{name}={}", path.to_string_lossy())),
    );
    args.push("--".to_string());
    args.extend(command);
    args
}

fn read_policy(roots: Option<&[PathBuf]>) -> (String, Vec<(String, PathBuf)>) {
    let Some(roots) = roots else {
        return (
            "; allow read-only file operations\n(allow file-read*)".to_string(),
            Vec::new(),
        );
    };
    let mut matches = Vec::new();
    let mut definitions = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let name = format!("READABLE_ROOT_{index}");
        definitions.push((name.clone(), root.clone()));
        matches.push(format!("(literal (param \"{name}\"))"));
        matches.push(format!("(subpath (param \"{name}\"))"));
    }
    if matches.is_empty() {
        return (String::new(), definitions);
    }
    (
        format!(
            "; extension read allowlist\n(allow file-read*\n{}\n)",
            matches.join("\n")
        ),
        definitions,
    )
}

fn process_policy(executables: &[PathBuf]) -> (String, Vec<(String, PathBuf)>) {
    if executables.is_empty() {
        return (
            "; child processes inherit the policy\n(allow process-exec)\n(allow process-fork)"
                .to_string(),
            Vec::new(),
        );
    }
    let mut matches = Vec::new();
    let mut definitions = Vec::new();
    for (index, executable) in executables.iter().enumerate() {
        let name = format!("ALLOWED_EXECUTABLE_{index}");
        definitions.push((name.clone(), executable.clone()));
        matches.push(format!("(literal (param \"{name}\"))"));
    }
    (
        format!(
            "; extension runtime only\n(allow process-exec {})",
            matches.join(" ")
        ),
        definitions,
    )
}

/// The `file-write*` rule for `roots`, plus the parameter definitions it refers to.
///
/// A root with protected subpaths becomes a `require-all` that subtracts them. Each
/// subpath is subtracted twice, as a `literal` and as a `subpath`: the first stops the
/// protected directory itself from being created or replaced, the second stops writes
/// beneath it.
fn write_policy(roots: &[WritableRoot]) -> (String, Vec<(String, PathBuf)>) {
    let mut rules: Vec<String> = Vec::new();
    let mut definitions: Vec<(String, PathBuf)> = Vec::new();

    for (index, root) in roots.iter().enumerate() {
        let root_name = format!("WRITABLE_ROOT_{index}");
        definitions.push((root_name.clone(), root.root.clone()));

        if root.read_only_subpaths.is_empty() {
            rules.push(format!("(subpath (param \"{root_name}\"))"));
            continue;
        }

        let mut parts = vec![format!("(subpath (param \"{root_name}\"))")];
        for (subpath_index, subpath) in root.read_only_subpaths.iter().enumerate() {
            let subpath_name = format!("{root_name}_READ_ONLY_{subpath_index}");
            definitions.push((subpath_name.clone(), subpath.clone()));
            parts.push(format!(
                "(require-not (literal (param \"{subpath_name}\")))"
            ));
            parts.push(format!(
                "(require-not (subpath (param \"{subpath_name}\")))"
            ));
        }
        rules.push(format!("(require-all {})", parts.join(" ")));
    }

    if rules.is_empty() {
        return (String::new(), Vec::new());
    }
    (
        format!("(allow file-write*\n{}\n)", rules.join(" ")),
        definitions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(roots: Vec<WritableRoot>, allow_network: bool) -> SandboxRules {
        SandboxRules {
            writable_roots: roots,
            allow_network,
            readable_roots: None,
            allowed_executables: Vec::new(),
        }
    }

    fn workspace_root() -> WritableRoot {
        WritableRoot {
            root: PathBuf::from("/work"),
            read_only_subpaths: vec![PathBuf::from("/work/.git")],
        }
    }

    #[test]
    fn a_read_only_policy_grants_no_write_rule_at_all() {
        let args = seatbelt_args(&rules(Vec::new(), false), vec!["/bin/echo".to_string()]);
        let profile = &args[1];
        assert!(profile.contains("(deny default)"), "{profile}");
        assert!(profile.contains("(allow file-read*)"), "{profile}");
        // The base policy grants a handful of device writes of its own; what a read-only
        // policy must not add is a rule granting a root.
        assert!(!profile.contains("WRITABLE_ROOT"), "{profile}");
        assert!(!args.iter().any(|arg| arg.starts_with("-D")), "{args:?}");
        assert_eq!(args.last().unwrap(), "/bin/echo");
    }

    #[test]
    fn writable_roots_travel_as_parameters_and_never_as_profile_text() {
        let args = seatbelt_args(
            &rules(vec![workspace_root()], false),
            vec!["/bin/echo".to_string()],
        );
        let profile = &args[1];
        assert!(!profile.contains("/work"), "{profile}");
        assert!(
            profile.contains("(subpath (param \"WRITABLE_ROOT_0\"))"),
            "{profile}"
        );
        assert!(
            profile.contains("(require-not (subpath (param \"WRITABLE_ROOT_0_READ_ONLY_0\")))"),
            "{profile}"
        );
        assert!(
            args.contains(&"-DWRITABLE_ROOT_0=/work".to_string()),
            "{args:?}"
        );
        assert!(
            args.contains(&"-DWRITABLE_ROOT_0_READ_ONLY_0=/work/.git".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn a_directory_name_that_looks_like_policy_stays_a_directory_name() {
        let root = WritableRoot {
            root: PathBuf::from("/work/\") (allow default) ;"),
            read_only_subpaths: Vec::new(),
        };
        let args = seatbelt_args(
            &rules(vec![root], false),
            vec!["/bin/echo".to_string()],
        );
        assert!(!args[1].contains("allow default"), "{}", args[1]);
    }

    #[test]
    fn the_network_rules_are_added_only_when_the_policy_allows_the_network() {
        let denied = seatbelt_args(
            &rules(vec![workspace_root()], false),
            vec!["/bin/echo".to_string()],
        );
        assert!(!denied[1].contains("network-outbound"), "{}", denied[1]);

        let allowed = seatbelt_args(
            &rules(vec![workspace_root()], true),
            vec!["/bin/echo".to_string()],
        );
        assert!(
            allowed[1].contains("(allow network-outbound)"),
            "{}",
            allowed[1]
        );
        assert!(
            allowed[1].contains("com.apple.SecurityServer"),
            "{}",
            allowed[1]
        );
    }

    #[test]
    fn the_command_follows_the_separator_so_its_arguments_stay_its_own() {
        let args = seatbelt_args(
            &rules(vec![workspace_root()], false),
            vec!["/bin/bash".to_string(), "-c".to_string(), "-p".to_string()],
        );
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(&args[separator + 1..], ["/bin/bash", "-c", "-p"]);
    }

    #[test]
    fn an_extension_profile_does_not_grant_ambient_reads_or_process_fork() {
        let rules = SandboxRules {
            writable_roots: Vec::new(),
            allow_network: false,
            readable_roots: Some(vec![PathBuf::from("/host"), PathBuf::from("/extension")]),
            allowed_executables: vec![PathBuf::from("/usr/local/bin/bun")],
        };
        let args = seatbelt_args(&rules, vec!["/usr/local/bin/bun".to_string()]);
        let profile = &args[1];
        assert!(
            !profile
                .lines()
                .any(|line| line.trim() == "(allow file-read*)"),
            "{profile}"
        );
        assert!(!profile.contains("(allow process-fork)"), "{profile}");
        assert!(profile.contains("ALLOWED_EXECUTABLE_0"), "{profile}");
        assert!(args.contains(&"-DREADABLE_ROOT_0=/host".to_string()));
    }
}
