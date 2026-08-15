//! OS-level confinement for the commands micro runs on the model's behalf.
//!
//! One policy, two enforcers. A command that micro spawns is wrapped in the platform
//! sandbox — a Seatbelt profile on macOS, Landlock and seccomp on Linux — so the kernel
//! decides what it may touch. The file tools do not spawn anything, so they ask
//! [`Sandbox::check_read`] and [`Sandbox::check_write`] about the same policy before they
//! open a path. The two answers are meant to agree; the kernel is what makes the first
//! one binding. They can differ in one place: Landlock can only rule on a path that
//! exists, so a protected directory that has not been created yet — a workspace with no
//! `.git` in it — is refused in process but not by the Linux kernel rules. Seatbelt takes
//! the path as a string and has no such gap.
//!
//! Nothing here spawns a process. [`Sandbox::wrap`] describes a command — the program to
//! run, the arguments to run it with, the environment to add — and the caller owns
//! spawning it, pumping its output, and timing it out:
//!
//! ```no_run
//! # use micro_sandbox::Sandbox;
//! # use micro_sandbox::SandboxPolicy;
//! # use std::path::Path;
//! let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), "/work");
//! let wrapped = sandbox.wrap("/bin/bash", ["-c", "ls"], Path::new("/work"));
//! let command = wrapped.to_std_command();
//! ```
//!
//! `tokio::process::Command` converts from that, so an async caller adds its stdio and
//! spawns.
//!
//! On a platform micro cannot confine, and under [`SandboxPolicy::Full`], `wrap` hands
//! back the command as it was given and [`Sandbox::is_enforced`] is false. A caller that
//! must not run unconfined checks that before spawning.
//!
//! # The Linux helper
//!
//! macOS can confine a command from the outside; Linux cannot. The restrictions have to
//! be applied by a process that then becomes the command, so on Linux the wrapped program
//! is micro's own executable, re-run with [`HELPER_ARG`] as its first argument:
//!
//! ```text
//! /path/to/micro __micro-sandbox-helper --rules <json> -- /bin/bash -c ls
//! ```
//!
//! The binary that hosts this crate is responsible for spotting that first argument at
//! the top of `main`, before it parses anything else, and handing the rest to
//! [`run_linux_helper`], which never returns. A binary that does not do this leaves the
//! sandbox unenforced on Linux.

mod denial;
mod helper;
#[cfg(target_os = "linux")]
mod linux;
mod paths;
mod policy;
#[cfg(target_os = "macos")]
mod seatbelt;

pub use denial::is_likely_denied;
pub use helper::run_linux_helper;
pub use helper::SandboxRules;
pub use helper::HELPER_ARG;
pub use helper::HELPER_FAILURE_EXIT_CODE;
pub use policy::SandboxPolicy;
pub use policy::UnknownPolicy;
pub use policy::WritableRoot;
pub use policy::PROTECTED_NAMES;

use std::path::Path;
use std::path::PathBuf;

/// The environment variable a confined command can read to learn it is confined, and by
/// what.
pub const SANDBOX_ENV_VAR: &str = "MICRO_SANDBOX";

/// The policy in force for a workspace, and the answers that follow from it.
#[derive(Debug, Clone)]
pub struct Sandbox {
    policy: SandboxPolicy,
    workspace: PathBuf,
    micro_homes: Vec<PathBuf>,
    helper_program: Option<PathBuf>,
    readable_roots: Option<Vec<PathBuf>>,
    allowed_executables: Vec<PathBuf>,
}

/// A command described so the caller can spawn it: what to run, where, and what to add to
/// its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,

    /// Whether this argument list actually confines the command. False when the policy
    /// asks for nothing, and on a platform micro cannot confine.
    pub enforced: bool,
}

impl WrappedCommand {
    /// The command as it was handed over, to be run without confinement.
    fn plain(mut command: Vec<String>, cwd: &Path) -> Self {
        let program = command.remove(0);
        WrappedCommand {
            program: PathBuf::from(program),
            args: command,
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            enforced: false,
        }
    }

    /// The same command as a [`std::process::Command`], which
    /// [`tokio::process::Command`](https://docs.rs/tokio/latest/tokio/process/struct.Command.html)
    /// converts from. Stdio, timeouts and process groups stay the caller's business.
    pub fn to_std_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args).current_dir(&self.cwd);
        for (name, value) in &self.env {
            command.env(name, value);
        }
        command
    }
}

/// Whether an operation is allowed, and how to say why it was not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,

    /// A sentence the model can act on, phrased for a tool result.
    pub reason: String,
}

impl Decision {
    fn allow(reason: impl Into<String>) -> Self {
        Decision {
            allowed: true,
            reason: reason.into(),
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Decision {
            allowed: false,
            reason: reason.into(),
        }
    }
}

impl Sandbox {
    /// A sandbox enforcing `policy` around `workspace`.
    ///
    /// The workspace is resolved through symlinks once, here, so every later comparison
    /// is against the path the kernel sees.
    pub fn new(policy: SandboxPolicy, workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let workspace = paths::canonicalize_deepest_existing(&workspace);
        Sandbox {
            policy,
            workspace,
            micro_homes: Vec::new(),
            helper_program: std::env::current_exe().ok(),
            readable_roots: None,
            allowed_executables: Vec::new(),
        }
    }

    /// A fail-closed process sandbox for an extension runtime.
    ///
    /// Unlike the command sandbox, this grants no ambient filesystem reads. The runtime
    /// can read only its own executable, the supplied extension/host roots, and the small
    /// set of operating-system files required to start. It cannot write or use the
    /// network. Callers must still check [`Sandbox::is_enforced`] before spawning.
    pub fn extension_host<I, P>(runtime: impl Into<PathBuf>, readable_roots: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let runtime = paths::canonicalize_deepest_existing(&runtime.into());
        let mut readable_roots: Vec<PathBuf> = readable_roots
            .into_iter()
            .map(Into::into)
            .map(|path| paths::canonicalize_deepest_existing(&path))
            .collect();
        readable_roots.push(runtime.clone());
        readable_roots.extend(platform_runtime_roots());
        readable_roots.sort();
        readable_roots.dedup();

        let workspace = readable_roots
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("/"));
        let mut sandbox = Sandbox::new(SandboxPolicy::ReadOnly, workspace);
        sandbox.readable_roots = Some(readable_roots);
        sandbox.allowed_executables = vec![runtime];
        sandbox
    }

    /// Protect one of micro's own directories, so a command confined to a workspace that
    /// contains it cannot rewrite the configuration, credentials or session history that
    /// decide what the next run is allowed to do.
    ///
    /// Called once per directory: micro keeps what the user wrote apart from what it
    /// produced, and both are worth the same protection.
    pub fn with_micro_home(mut self, micro_home: impl Into<PathBuf>) -> Self {
        let micro_home = paths::canonicalize_deepest_existing(&micro_home.into());
        if !self.micro_homes.contains(&micro_home) {
            self.micro_homes.push(micro_home);
        }
        self
    }

    /// Run the Linux helper from this executable rather than the current one. The current
    /// executable is the right answer whenever micro spawns micro; a test harness or a
    /// wrapper binary says so here.
    pub fn with_helper_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.helper_program = Some(program.into());
        self
    }

    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Whether commands wrapped by this sandbox are actually confined.
    ///
    /// False under [`SandboxPolicy::Full`], on a platform without an enforcement
    /// mechanism micro speaks, and on Linux when the helper cannot be located.
    pub fn is_enforced(&self) -> bool {
        if self.policy.allows_all_writes() {
            return false;
        }
        if cfg!(target_os = "macos") {
            return true;
        }
        cfg!(target_os = "linux") && self.helper_program.is_some()
    }

    /// The directories this policy makes writable, each with the paths inside it that
    /// stay read-only.
    ///
    /// Protected paths are listed whether or not they exist: creating a `.git` is as much
    /// a way to change what the next run does as editing one.
    pub fn writable_roots(&self) -> Vec<WritableRoot> {
        match &self.policy {
            SandboxPolicy::Full => {
                return vec![WritableRoot {
                    root: PathBuf::from("/"),
                    read_only_subpaths: Vec::new(),
                }];
            }
            SandboxPolicy::ReadOnly => return Vec::new(),
            SandboxPolicy::WorkspaceWrite { .. } => {}
        }

        let mut roots = vec![self.workspace.clone()];
        roots.extend(
            self.policy
                .granted_roots()
                .iter()
                .map(|root| paths::resolve(&self.workspace, root)),
        );
        roots.dedup();

        roots
            .into_iter()
            .map(|root| {
                let mut read_only_subpaths: Vec<PathBuf> =
                    PROTECTED_NAMES.iter().map(|name| root.join(name)).collect();
                for micro_home in &self.micro_homes {
                    if micro_home.starts_with(&root) && !read_only_subpaths.contains(micro_home) {
                        read_only_subpaths.push(micro_home.clone());
                    }
                }
                WritableRoot {
                    root,
                    read_only_subpaths,
                }
            })
            .collect()
    }

    /// The rules the Linux helper is handed: the same decisions as everything else here,
    /// in the form that survives the trip to another process.
    pub fn rules(&self) -> SandboxRules {
        SandboxRules {
            writable_roots: self.writable_roots(),
            allow_network: self.policy.allows_network(),
            readable_roots: self.readable_roots.clone(),
            allowed_executables: self.allowed_executables.clone(),
        }
    }

    /// Describe `program` and `args` as a command confined by this policy, to be run in
    /// `cwd`.
    ///
    /// `cwd` is where the command runs; it grants nothing. A command that runs somewhere
    /// outside the workspace still writes only where the policy says it may.
    pub fn wrap<I, S>(&self, program: &str, args: I, cwd: &Path) -> WrappedCommand
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut command = vec![program.to_string()];
        command.extend(args.into_iter().map(Into::into));
        if !self.is_enforced() {
            return WrappedCommand::plain(command, cwd);
        }

        #[cfg(target_os = "macos")]
        {
            WrappedCommand {
                program: PathBuf::from(seatbelt::SANDBOX_EXEC),
                args: seatbelt::seatbelt_args(&self.rules(), command),
                cwd: cwd.to_path_buf(),
                env: vec![(SANDBOX_ENV_VAR.to_string(), "seatbelt".to_string())],
                enforced: true,
            }
        }

        #[cfg(target_os = "linux")]
        {
            let (Some(helper), Ok(rules)) = (
                self.helper_program.clone(),
                serde_json::to_string(&self.rules()),
            ) else {
                return WrappedCommand::plain(command, cwd);
            };
            let mut args = vec![
                HELPER_ARG.to_string(),
                "--rules".to_string(),
                rules,
                "--".to_string(),
            ];
            args.extend(command);
            WrappedCommand {
                program: helper,
                args,
                cwd: cwd.to_path_buf(),
                env: vec![(SANDBOX_ENV_VAR.to_string(), "landlock".to_string())],
                enforced: true,
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            WrappedCommand::plain(command, cwd)
        }
    }

    /// Whether `path` may be read.
    ///
    /// Every policy grants full read access, so this always allows. It is the seam the
    /// file tools ask, so that reads and writes are decided in one place rather than two.
    pub fn check_read(&self, path: &Path) -> Decision {
        let resolved = paths::resolve(&self.workspace, path);
        Decision::allow(format!(
            "{} is readable under {}",
            resolved.display(),
            self.policy
        ))
    }

    /// Whether `path` may be written to, created, or removed.
    ///
    /// The path is resolved through symlinks first, so a link inside the workspace
    /// pointing somewhere else is judged by where it points.
    pub fn check_write(&self, path: &Path) -> Decision {
        let resolved = paths::resolve(&self.workspace, path);
        let shown = resolved.display();

        match &self.policy {
            SandboxPolicy::Full => Decision::allow(format!("{shown} is writable under full")),
            SandboxPolicy::ReadOnly => Decision::deny(format!(
                "cannot write {shown}: the sandbox policy is read-only"
            )),
            SandboxPolicy::WorkspaceWrite { .. } => {
                let roots = self.writable_roots();
                if roots.iter().any(|root| root.is_path_writable(&resolved)) {
                    return Decision::allow(format!("{shown} is writable under workspace-write"));
                }
                let protected = roots.iter().find_map(|root| root.protecting(&resolved));
                match protected {
                    Some(subpath) => Decision::deny(format!(
                        "cannot write {shown}: {} stays read-only under workspace-write",
                        subpath.display()
                    )),
                    None => Decision::deny(format!(
                        "cannot write {shown}: workspace-write allows writes under {} only",
                        self.workspace.display()
                    )),
                }
            }
        }
    }
}

fn platform_runtime_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        [
            "/lib",
            "/lib64",
            "/usr/lib",
            "/usr/lib64",
            "/etc/ld.so.cache",
            "/etc/localtime",
            "/usr/share/zoneinfo",
            "/dev/null",
            "/dev/urandom",
            "/proc/self",
        ]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect()
    }
    #[cfg(target_os = "macos")]
    {
        [
            "/System/Library",
            "/usr/lib",
            "/etc/localtime",
            "/usr/share/zoneinfo",
            "/dev/null",
            "/dev/urandom",
        ]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-sandbox-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn workspace(name: &str) -> (PathBuf, PathBuf) {
        let dir = scratch(name);
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        (dir, workspace)
    }

    #[test]
    fn workspace_write_allows_a_write_inside_the_workspace() {
        let (_dir, workspace) = workspace("inside");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let decision = sandbox.check_write(&workspace.join("src/main.rs"));
        assert!(decision.allowed, "{}", decision.reason);
    }

    #[test]
    fn workspace_write_refuses_a_write_outside_the_workspace() {
        let (dir, workspace) = workspace("outside");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let decision = sandbox.check_write(&dir.join("elsewhere.txt"));
        assert!(!decision.allowed);
        assert!(
            decision
                .reason
                .contains("workspace-write allows writes under"),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn workspace_write_keeps_the_git_directory_read_only() {
        let (_dir, workspace) = workspace("git");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let decision = sandbox.check_write(&workspace.join(".git/hooks/pre-commit"));
        assert!(!decision.allowed);
        assert!(
            decision.reason.contains("stays read-only"),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn workspace_write_refuses_to_create_a_protected_directory_that_is_not_there_yet() {
        let (_dir, workspace) = workspace("protected-missing");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let decision = sandbox.check_write(&workspace.join(".micro/settings.json"));
        assert!(!decision.allowed, "{}", decision.reason);
    }

    #[test]
    fn workspace_write_keeps_the_micro_home_read_only_when_it_sits_in_the_workspace() {
        let (_dir, workspace) = workspace("micro-home");
        let micro_home = workspace.join("home/.micro-data");
        std::fs::create_dir_all(&micro_home).unwrap();
        let sandbox =
            Sandbox::new(SandboxPolicy::workspace_write(), &workspace).with_micro_home(&micro_home);
        let decision = sandbox.check_write(&micro_home.join("auth.json"));
        assert!(!decision.allowed, "{}", decision.reason);
        assert!(
            sandbox
                .check_write(&workspace.join("home/notes.md"))
                .allowed
        );
    }

    #[test]
    fn a_symlink_out_of_the_workspace_is_judged_by_where_it_points() {
        let (dir, workspace) = workspace("symlink");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();

        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let decision = sandbox.check_write(&workspace.join("escape/loot.txt"));
        assert!(!decision.allowed, "{}", decision.reason);
    }

    #[test]
    fn an_explicitly_granted_root_becomes_writable() {
        let (dir, workspace) = workspace("granted");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![cache.clone()],
            allow_network: false,
        };
        let sandbox = Sandbox::new(policy, &workspace);
        assert!(sandbox.check_write(&cache.join("blob")).allowed);
        assert!(!sandbox.check_write(&dir.join("other")).allowed);
    }

    #[test]
    fn read_only_refuses_every_write_and_allows_every_read() {
        let (dir, workspace) = workspace("read-only");
        let sandbox = Sandbox::new(SandboxPolicy::ReadOnly, &workspace);
        assert!(!sandbox.check_write(&workspace.join("src/main.rs")).allowed);
        assert!(!sandbox.check_write(&dir.join("elsewhere.txt")).allowed);
        assert!(sandbox.check_read(&workspace.join("src/main.rs")).allowed);
        assert!(sandbox.check_read(Path::new("/etc/hosts")).allowed);
    }

    #[test]
    fn read_only_hands_the_kernel_no_writable_root_either() {
        let (_dir, workspace) = workspace("read-only-roots");
        let sandbox = Sandbox::new(SandboxPolicy::ReadOnly, &workspace);
        assert!(sandbox.writable_roots().is_empty());
        assert!(sandbox.rules().writable_roots.is_empty());
    }

    #[test]
    fn an_extension_host_has_an_explicit_read_and_execute_allowlist() {
        let (_dir, workspace) = workspace("extension-host");
        let runtime = workspace.join("bun");
        std::fs::write(&runtime, "runtime").unwrap();
        let package = workspace.join("extension");
        std::fs::create_dir_all(&package).unwrap();

        let sandbox = Sandbox::extension_host(&runtime, [&package]);
        let rules = sandbox.rules();
        assert!(!rules.allow_network);
        assert!(rules.writable_roots.is_empty());
        let readable = rules.readable_roots.expect("extension reads are allowlisted");
        assert!(readable.contains(&std::fs::canonicalize(&runtime).unwrap()));
        assert!(readable.contains(&std::fs::canonicalize(&package).unwrap()));
        assert_eq!(
            rules.allowed_executables,
            [std::fs::canonicalize(runtime).unwrap()]
        );
    }

    #[test]
    fn full_access_allows_everything_and_confines_nothing() {
        let (dir, workspace) = workspace("full");
        let sandbox = Sandbox::new(SandboxPolicy::Full, &workspace);
        assert!(sandbox.check_write(&dir.join("elsewhere.txt")).allowed);
        assert!(!sandbox.is_enforced());
        let wrapped = sandbox.wrap("/bin/echo", ["hi"], &workspace);
        assert_eq!(wrapped.program, PathBuf::from("/bin/echo"));
        assert_eq!(wrapped.args, ["hi"]);
        assert!(!wrapped.enforced);
    }

    #[test]
    fn a_relative_path_is_judged_against_the_workspace() {
        let (_dir, workspace) = workspace("relative");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        assert!(sandbox.check_write(Path::new("src/main.rs")).allowed);
        assert!(!sandbox.check_write(Path::new("../elsewhere.txt")).allowed);
    }

    #[test]
    fn the_rules_handed_to_the_helper_carry_the_protected_paths() {
        let (_dir, workspace) = workspace("rules");
        let sandbox = Sandbox::new(SandboxPolicy::workspace_write(), &workspace);
        let rules = sandbox.rules();
        assert!(!rules.allow_network);
        assert_eq!(rules.writable_roots.len(), 1);
        assert_eq!(rules.writable_roots[0].root, workspace);
        assert!(rules.writable_roots[0]
            .read_only_subpaths
            .contains(&workspace.join(".git")));
    }
}
