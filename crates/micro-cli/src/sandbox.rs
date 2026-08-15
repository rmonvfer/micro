//! What the commands a session runs are allowed to touch.
//!
//! A policy can be said in three places, and they beat each other in the order a person
//! would expect: what this run was told on the command line, then what the project asks
//! for in its own settings — only once the project has been trusted — then what the user
//! settled in theirs. With nothing said anywhere the workspace is writable and nothing
//! else is, which is the default a coding agent can work under without being asked about
//! every file it writes.

use anyhow::anyhow;
use anyhow::Result;
use micro_sandbox::Sandbox;
use micro_sandbox::SandboxPolicy;
use std::path::Path;

/// A sandbox enforcing `policy` around `workspace`, with micro's own directories
/// protected.
///
/// A workspace that contains one of them — someone working on their own configuration —
/// would otherwise let a command rewrite the credentials and the settings that decide what
/// the next run may do, or the session logs that say what this one did. Both are named
/// because a fresh install keeps them apart, and either is inside a home directory
/// somebody could be standing in.
pub fn around(policy: SandboxPolicy, workspace: &Path) -> Sandbox {
    let mut sandbox = Sandbox::new(policy, workspace);
    for own in [micro_dirs::config_dir(), micro_dirs::data_dir()]
        .into_iter()
        .flatten()
        .filter(|own| !own.as_os_str().is_empty())
    {
        sandbox = sandbox.with_micro_home(own);
    }
    sandbox
}

/// The policy in force for this run, from whichever of the three places settled it.
///
/// A name nobody recognizes ends the run rather than falling back to the default: a policy
/// is what the rest of the session is judged against, and quietly using a different one
/// than was asked for is the one outcome nobody wants.
pub fn policy(
    flag: Option<&str>,
    workspace: &Path,
    trusted: bool,
    settings: &micro_config::Settings,
) -> Result<SandboxPolicy> {
    // Trust is what decides whether the project is read at all, and `ProjectConfig::load`
    // is where that is decided rather than here.
    let project = micro_config::ProjectConfig::load(workspace, trusted)
        .map_err(|error| anyhow!("{error}"))?
        .sandbox;

    let settled = flag
        .map(|written| (written_as(written), "--sandbox"))
        .or_else(|| project.map(|written| (Ok(written), "the project's settings")))
        .or_else(|| {
            settings
                .sandbox
                .clone()
                .map(|written| (Ok(written), "the settings"))
        });

    let Some((written, source)) = settled else {
        return Ok(SandboxPolicy::default());
    };
    let written = written.map_err(|error| anyhow!("{source}: {error}"))?;
    serde_json::from_value(written).map_err(|error| anyhow!("{source}: {error}"))
}

/// A policy as it was typed on the command line: one of the three names, or a table
/// spelling out what `workspace-write` grants beyond the default, which is the same thing
/// the settings take and so is written the same way.
fn written_as(flag: &str) -> Result<serde_json::Value, serde_json::Error> {
    match flag.trim_start().starts_with('{') {
        true => serde_json::from_str(flag),
        false => Ok(serde_json::Value::String(flag.to_string())),
    }
}

/// What a session would do with this command: `micro sandbox try -- <command>`.
///
/// Run against the policy this workspace would use, or the one named, and reported the way
/// the tools read it — what was actually spawned, whether anything is enforcing it, and
/// whether the outcome looks like a refusal rather than the command's own failure.
///
/// The report is what this command produces, so it succeeds whenever it managed to make
/// one: the command's own exit status is a line of the report rather than this one's, and
/// conflating the two would make a refused command indistinguishable from a broken micro.
pub async fn try_command(
    workspace: &Path,
    named: Option<&str>,
    settings: &micro_config::Settings,
    command: &[String],
) -> Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        return Err(anyhow!("nothing to run: micro sandbox try -- <command>"));
    };

    // Trusted outright: this is someone asking what a policy does, in a workspace they are
    // standing in, and the answer would be misleading if the project's own setting were
    // left out of it.
    let policy = policy(named, workspace, true, settings)?;
    let sandbox = around(policy, workspace);
    let wrapped = sandbox.wrap(program, arguments.to_vec(), workspace);

    println!("policy: {}", sandbox.policy());
    println!(
        "enforced: {}",
        match wrapped.enforced {
            true => enforcement(),
            false => match sandbox.policy().allows_all_writes() {
                true => "no, `full` confines nothing".to_string(),
                false => format!("no, {}", why_not()),
            },
        }
    );
    println!("running: {}", shown(&wrapped));

    // Collected rather than streamed: whether an outcome looks like a refusal is read out
    // of what the command printed as much as out of how it exited, and this is a question
    // asked about a command that has already finished.
    let finished = tokio::process::Command::from(wrapped.to_std_command())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|error| anyhow!("cannot start {program}: {error}"))?;
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&finished.stdout),
        String::from_utf8_lossy(&finished.stderr)
    );

    if !printed.trim().is_empty() {
        println!("output:");
        for line in printed.lines() {
            println!("  {line}");
        }
    }
    println!("exit: {}", finished.status);
    println!(
        "looks denied: {}",
        micro_sandbox::is_likely_denied(&finished.status, &printed)
    );
    Ok(())
}

/// What is doing the enforcing on this platform.
fn enforcement() -> String {
    match std::env::consts::OS {
        "macos" => "yes, by a Seatbelt profile".to_string(),
        "linux" => "yes, by Landlock and seccomp in a helper process".to_string(),
        other => format!("yes, on {other}"),
    }
}

/// Why nothing is enforcing the policy, where nothing is.
fn why_not() -> String {
    match std::env::consts::OS {
        "linux" => "micro cannot find its own executable to re-run as the helper".to_string(),
        other => format!("micro has no sandbox for {other} yet"),
    }
}

/// The command as it will be spawned, in one line a reader can take in.
///
/// One argument is not fit to print: the Seatbelt profile is a few hundred lines of policy
/// handed over as a string, and printing it buries the thing the reader came for. It is
/// summarized by its size, and the paths it is parameterized with are printed as they are —
/// those are what a reader is actually checking.
fn shown(wrapped: &micro_sandbox::WrappedCommand) -> String {
    let mut parts = vec![wrapped.program.display().to_string()];
    parts.extend(wrapped.args.iter().map(|argument| {
        if argument.contains('\n') {
            return format!("<{} lines of policy>", argument.lines().count());
        }
        match argument.contains(' ') {
            true => format!("'{argument}'"),
            false => argument.clone(),
        }
    }));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("micro-policy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join(micro_config::PROJECT_DIR)).unwrap();
        path
    }

    fn settled(policy: &str) -> micro_config::Settings {
        micro_config::Settings {
            sandbox: Some(serde_json::Value::from(policy)),
            ..micro_config::Settings::default()
        }
    }

    fn project(workspace: &Path, policy: &str) {
        std::fs::write(
            micro_config::ProjectConfig::path(workspace),
            format!(r#"{{"sandbox":"{policy}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn nothing_said_anywhere_leaves_the_workspace_writable_and_nothing_else() {
        let workspace = scratch("default");
        let resolved = policy(None, &workspace, true, &micro_config::Settings::default()).unwrap();
        assert_eq!(resolved, SandboxPolicy::workspace_write());
    }

    #[test]
    fn what_the_run_was_told_beats_the_project_which_beats_the_settings() {
        let workspace = scratch("order");
        project(&workspace, "read-only");

        assert_eq!(
            policy(Some("full"), &workspace, true, &settled("workspace-write")).unwrap(),
            SandboxPolicy::Full
        );
        assert_eq!(
            policy(None, &workspace, true, &settled("full")).unwrap(),
            SandboxPolicy::ReadOnly
        );
        assert_eq!(
            policy(None, &scratch("no-project"), true, &settled("read-only")).unwrap(),
            SandboxPolicy::ReadOnly
        );
    }

    /// An untrusted project has no say. Widening what a session may do is exactly what
    /// trust is asked about, so a checkout nobody vouched for cannot ask for it.
    #[test]
    fn an_untrusted_project_does_not_get_to_choose_the_policy() {
        let workspace = scratch("untrusted");
        project(&workspace, "full");

        assert_eq!(
            policy(None, &workspace, false, &settled("read-only")).unwrap(),
            SandboxPolicy::ReadOnly
        );
        assert_eq!(
            policy(None, &workspace, false, &micro_config::Settings::default()).unwrap(),
            SandboxPolicy::workspace_write()
        );
    }

    /// The grants `workspace-write` does not make by default — another writable directory,
    /// the network — are asked for by spelling the policy out, wherever it is written.
    #[test]
    fn a_spelled_out_policy_keeps_what_it_grants() {
        let workspace = scratch("spelled-out");
        let written =
            r#"{"mode":"workspace-write","writable_roots":["/srv/cache"],"allow_network":true}"#;

        let resolved = policy(
            Some(written),
            &workspace,
            true,
            &micro_config::Settings::default(),
        )
        .unwrap();
        assert_eq!(
            resolved,
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![PathBuf::from("/srv/cache")],
                allow_network: true,
            }
        );

        std::fs::write(
            micro_config::ProjectConfig::path(&workspace),
            format!("{{\"sandbox\":{written}}}"),
        )
        .unwrap();
        assert_eq!(
            policy(None, &workspace, true, &micro_config::Settings::default()).unwrap(),
            resolved,
            "a project spells it out the same way"
        );
    }

    #[test]
    fn a_policy_nobody_recognizes_ends_the_run_and_says_who_asked_for_it() {
        let workspace = scratch("unknown");
        let error = policy(
            Some("yolo"),
            &workspace,
            true,
            &micro_config::Settings::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("--sandbox"), "{error}");
        assert!(error.contains("yolo"), "{error}");
        assert!(error.contains("workspace-write"), "{error}");

        project(&workspace, "nonsense");
        let error = policy(None, &workspace, true, &micro_config::Settings::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("the project's settings"), "{error}");
    }
}
