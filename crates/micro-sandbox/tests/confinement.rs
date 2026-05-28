//! What the kernel actually does with a command micro wraps.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use micro_sandbox::is_likely_denied;
use micro_sandbox::Sandbox;
use micro_sandbox::SandboxPolicy;
use std::path::PathBuf;
use std::process::ExitStatus;

/// A workspace with a source directory and a `.git`, and the directory it sits in.
fn workspace(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("micro-sandbox-confinement-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::create_dir_all(workspace.join(".git")).unwrap();
    let dir = dir.canonicalize().unwrap();
    (dir.clone(), dir.join("workspace"))
}

fn sandbox(policy: SandboxPolicy, workspace: &PathBuf) -> Sandbox {
    Sandbox::new(policy, workspace).with_helper_program(env!("CARGO_BIN_EXE_micro-sandbox-helper"))
}

/// Run `script` under the sandbox, answering with how it ended and everything it said.
fn run(sandbox: &Sandbox, script: &str) -> (ExitStatus, String) {
    let wrapped = sandbox.wrap("/bin/bash", ["-c", script], sandbox.workspace());
    assert!(wrapped.enforced, "the command was not confined");
    let output = wrapped.to_std_command().output().unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status, said)
}

#[test]
fn a_write_inside_the_workspace_goes_through() {
    let (_dir, workspace) = workspace("write-inside");
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);

    let (status, said) = run(&sandbox, "echo written > src/main.rs");
    assert!(status.success(), "{said}");
    assert_eq!(
        std::fs::read_to_string(workspace.join("src/main.rs")).unwrap(),
        "written\n"
    );
}

#[test]
fn a_write_outside_the_workspace_is_refused_by_the_kernel() {
    let (dir, workspace) = workspace("write-outside");
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);
    let loot = dir.join("loot.txt");

    let (status, said) = run(&sandbox, &format!("echo taken > {}", loot.display()));
    assert!(!status.success(), "{said}");
    assert!(!loot.exists(), "{said}");
    assert!(is_likely_denied(&status, &said), "{said}");
}

#[test]
fn a_write_to_the_git_directory_is_refused_by_the_kernel() {
    let (_dir, workspace) = workspace("write-git");
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);

    let (status, said) = run(&sandbox, "echo hooked > .git/hooks");
    assert!(!status.success(), "{said}");
    assert!(!workspace.join(".git/hooks").exists(), "{said}");
}

#[test]
fn reading_outside_the_workspace_still_works() {
    let (dir, workspace) = workspace("read-outside");
    std::fs::write(dir.join("notes.md"), "readable").unwrap();
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);

    let (status, said) = run(&sandbox, &format!("cat {}", dir.join("notes.md").display()));
    assert!(status.success(), "{said}");
    assert!(said.contains("readable"), "{said}");
}

#[test]
fn a_read_only_policy_refuses_a_write_inside_the_workspace_too() {
    let (_dir, workspace) = workspace("read-only");
    let sandbox = sandbox(SandboxPolicy::ReadOnly, &workspace);

    let (status, said) = run(&sandbox, "echo written > src/main.rs");
    assert!(!status.success(), "{said}");
    assert!(!workspace.join("src/main.rs").exists(), "{said}");
    assert!(is_likely_denied(&status, &said), "{said}");
}

#[test]
fn the_network_is_out_of_reach_when_the_policy_does_not_allow_it() {
    let (_dir, workspace) = workspace("network");
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);

    
    let (status, said) = run(&sandbox, "exec 3<>/dev/tcp/1.1.1.1/80");
    assert!(!status.success(), "{said}");
    assert!(is_likely_denied(&status, &said), "{said}");
}

#[test]
fn a_command_under_the_sandbox_can_tell_that_it_is_confined() {
    let (_dir, workspace) = workspace("env");
    let sandbox = sandbox(SandboxPolicy::workspace_write(), &workspace);

    let (status, said) = run(&sandbox, "echo $MICRO_SANDBOX");
    assert!(status.success(), "{said}");
    assert!(!said.trim().is_empty(), "{said}");
}
