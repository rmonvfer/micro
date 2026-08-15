//! What the commands a session runs are allowed to touch.

use anyhow::anyhow;
use anyhow::Result;
use micro_sandbox::Sandbox;
use micro_sandbox::SandboxPolicy;
use std::path::Path;

/// A sandbox enforcing `policy` around `workspace`, with micro's own directories protected.
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
pub fn policy(
    flag: Option<&str>,
    workspace: &Path,
    trusted: bool,
    settings: &micro_config::Settings,
) -> Result<SandboxPolicy> {
    
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


fn written_as(flag: &str) -> Result<serde_json::Value, serde_json::Error> {
    match flag.trim_start().starts_with('{') {
        true => serde_json::from_str(flag),
        false => Ok(serde_json::Value::String(flag.to_string())),
    }
}

/// What a session would do with this command: `micro sandbox try -- <command>`.
pub async fn try_command(
    workspace: &Path,
    named: Option<&str>,
    settings: &micro_config::Settings,
    command: &[String],
) -> Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        return Err(anyhow!("nothing to run: micro sandbox try -- <command>"));
    };

    
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

    /// An untrusted project has no say.
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

    /// The grants `workspace-write` does not make by default.
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
            "project policy should be explicit"
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
