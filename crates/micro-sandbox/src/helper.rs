//! The contract between micro and the copy of itself that enforces the Linux sandbox.
//!
//! Derived from openai/codex codex-rs/linux-sandbox/src/linux_run_main.rs at commit
//! a8c7f5391c; modified.

use crate::WritableRoot;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

/// The first argument that turns a micro process into the sandbox helper.
///
/// Linux has no way to confine a command from the outside the way `sandbox-exec` does, so
/// the restrictions have to be applied by a process that then becomes the command. micro
/// re-runs its own executable with this marker, that process applies the rules to itself
/// and `execvp`s the command, and the command inherits them. Dispatching on the marker
/// has to happen before anything else in `main`, ahead of argument parsing.
pub const HELPER_ARG: &str = "__micro-sandbox-helper";

/// The exit code the helper uses when it could not run the command at all — the sandbox
/// would not apply, or the arguments made no sense. It is distinct from anything the
/// command itself could return, so a caller can tell "micro failed" from "the command
/// failed".
pub const HELPER_FAILURE_EXIT_CODE: i32 = 125;

/// What the helper is asked to enforce.
///
/// The rules are worked out by the parent, where the policy and the workspace are known,
/// and handed over whole. The helper resolves nothing itself, so what it enforces is
/// exactly what [`crate::Sandbox`] decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxRules {
    pub writable_roots: Vec<WritableRoot>,
    pub allow_network: bool,
    /// `None` keeps the command sandbox's normal read-anywhere behavior. `Some` is a
    /// fail-closed allowlist used for extension processes.
    pub readable_roots: Option<Vec<PathBuf>>,
    /// Empty means any executable. A non-empty list limits `process-exec` on macOS;
    /// Landlock applies the same limit through execute access on Linux.
    pub allowed_executables: Vec<PathBuf>,
}

/// Apply the sandbox to this process and become the command it was given.
///
/// `args` is everything after [`HELPER_ARG`], in the shape
/// `--rules <json> -- <program> <argument>...`. This never returns: on success the
/// process is replaced by the command, and on failure it exits with
/// [`HELPER_FAILURE_EXIT_CODE`].
pub fn run_linux_helper<I: IntoIterator<Item = String>>(args: I) -> ! {
    match parse(args) {
        Ok(invocation) => enforce_and_exec(invocation),
        Err(problem) => fail(&problem),
    }
}

/// Say why the command is not running, and stop.
pub(crate) fn fail(problem: &str) -> ! {
    eprintln!("micro-sandbox: {problem}");
    std::process::exit(HELPER_FAILURE_EXIT_CODE)
}

#[derive(Debug)]
struct Invocation {
    rules: SandboxRules,
    command: Vec<String>,
}

fn enforce_and_exec(invocation: Invocation) -> ! {
    #[cfg(target_os = "linux")]
    {
        if let Err(problem) = crate::linux::apply(&invocation.rules) {
            fail(&problem);
        }
        crate::linux::exec(&invocation.command)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let Invocation { rules, command } = invocation;
        fail(&format!(
            "the sandbox helper only enforces on Linux, so it will not run {} under {} writable root(s)",
            command[0],
            rules.writable_roots.len()
        ))
    }
}

fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Invocation, String> {
    let mut args = args.into_iter();
    let mut rules: Option<SandboxRules> = None;

    loop {
        let Some(argument) = args.next() else {
            return Err("expected `--rules <json> -- <command>`".to_string());
        };
        match argument.as_str() {
            "--rules" => {
                let Some(json) = args.next() else {
                    return Err("--rules needs the rules to enforce".to_string());
                };
                let parsed = serde_json::from_str(&json)
                    .map_err(|error| format!("the rules did not parse: {error}"))?;
                rules = Some(parsed);
            }
            "--" => break,
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }

    let Some(rules) = rules else {
        return Err("no rules were given to enforce".to_string());
    };
    let command: Vec<String> = args.collect();
    if command.is_empty() {
        return Err("no command was given to run".to_string());
    }
    Ok(Invocation { rules, command })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rules() -> SandboxRules {
        SandboxRules {
            writable_roots: vec![WritableRoot {
                root: PathBuf::from("/work"),
                read_only_subpaths: vec![PathBuf::from("/work/.git")],
            }],
            allow_network: false,
            readable_roots: None,
            allowed_executables: Vec::new(),
        }
    }

    #[test]
    fn the_helper_arguments_carry_the_rules_and_the_command_across_unchanged() {
        let json = serde_json::to_string(&rules()).unwrap();
        let invocation = parse(vec![
            "--rules".to_string(),
            json,
            "--".to_string(),
            "/bin/bash".to_string(),
            "-c".to_string(),
            "echo hi".to_string(),
        ])
        .unwrap();
        assert_eq!(invocation.rules, rules());
        assert_eq!(invocation.command, ["/bin/bash", "-c", "echo hi"]);
    }

    #[test]
    fn a_command_that_looks_like_a_flag_is_still_the_command() {
        let json = serde_json::to_string(&rules()).unwrap();
        let invocation = parse(vec![
            "--rules".to_string(),
            json,
            "--".to_string(),
            "/bin/echo".to_string(),
            "--rules".to_string(),
        ])
        .unwrap();
        assert_eq!(invocation.command, ["/bin/echo", "--rules"]);
    }

    #[test]
    fn the_helper_refuses_to_run_a_command_without_rules() {
        let problem = parse(vec!["--".to_string(), "/bin/echo".to_string()]).unwrap_err();
        assert!(problem.contains("no rules"), "{problem}");
    }

    #[test]
    fn the_helper_refuses_rules_without_a_command() {
        let json = serde_json::to_string(&rules()).unwrap();
        let problem = parse(vec!["--rules".to_string(), json, "--".to_string()]).unwrap_err();
        assert!(problem.contains("no command"), "{problem}");
    }
}
