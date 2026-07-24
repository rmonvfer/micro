use crate::helper::SandboxRules;
use landlock::path_beneath_rules;
use landlock::Access;
use landlock::AccessFs;
use landlock::CompatLevel;
use landlock::Compatible;
use landlock::RestrictionStatus;
use landlock::Ruleset;
use landlock::RulesetAttr;
use landlock::RulesetCreatedAttr;
use landlock::RulesetError;
use landlock::RulesetStatus;
use landlock::ABI;
use seccompiler::apply_filter;
use seccompiler::BpfProgram;
use seccompiler::SeccompAction;
use seccompiler::SeccompCmpArgLen;
use seccompiler::SeccompCmpOp;
use seccompiler::SeccompCondition;
use seccompiler::SeccompFilter;
use seccompiler::SeccompRule;
use seccompiler::TargetArch;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::Path;

/// Confine the current thread to `rules`.
pub(crate) fn apply(rules: &SandboxRules) -> Result<(), String> {
    set_no_new_privs()?;
    install_seccomp_filter(rules.allow_network)?;
    install_filesystem_rules(rules)
}

/// Replace this process with `command`, keeping the restrictions just applied.
pub(crate) fn exec(command: &[String]) -> ! {
    let program = match CString::new(command[0].as_str()) {
        Ok(program) => program,
        Err(_) => crate::helper::fail("the command name contains a null byte"),
    };
    let mut arguments: Vec<CString> = Vec::with_capacity(command.len());
    for argument in command {
        match CString::new(argument.as_str()) {
            Ok(argument) => arguments.push(argument),
            Err(_) => crate::helper::fail("a command argument contains a null byte"),
        }
    }

    let mut pointers: Vec<*const libc::c_char> =
        arguments.iter().map(|argument| argument.as_ptr()).collect();
    pointers.push(std::ptr::null());

    // SAFETY: the pointers outlive the call — `arguments` owns them for the rest of this
    // function — and the array is null-terminated as execvp requires.
    unsafe {
        libc::execvp(program.as_ptr(), pointers.as_ptr());
    }

    let error = std::io::Error::last_os_error();
    crate::helper::fail(&format!("could not run {}: {error}", command[0]))
}

fn set_no_new_privs() -> Result<(), String> {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes no pointers and touches no memory
    // this process owns.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        return Err(format!("could not set no_new_privs: {error}"));
    }
    Ok(())
}

/// Allow reading the whole filesystem and writing only under the roots the policy grants.
fn install_filesystem_rules(rules: &SandboxRules) -> Result<(), String> {
    let writable: Vec<&Path> = rules
        .writable_roots
        .iter()
        .map(|root| root.root.as_path())
        .filter(|root| root.exists())
        .collect();

    let readable: Option<Vec<&Path>> = rules.readable_roots.as_ref().map(|roots| {
        roots
            .iter()
            .map(|path| path.as_path())
            .filter(|root| root.exists())
            .collect()
    });
    let status = restrict_current_thread(&writable, readable.as_deref())
        .map_err(|error| format!("the filesystem ruleset would not apply: {error}"))?;
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err("this kernel does not enforce Landlock".to_string());
    }
    Ok(())
}

fn restrict_current_thread(
    writable: &[&Path],
    readable: Option<&[&Path]>,
) -> Result<RestrictionStatus, RulesetError> {
    let abi = ABI::V5;
    let access_rw = AccessFs::from_all(abi);
    let access_ro = AccessFs::from_read(abi);

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(access_rw)?
        .create()?
        .add_rules(path_beneath_rules(["/dev/null"], access_rw))?
        .no_new_privs(true);

    match readable {
        Some(roots) if !roots.is_empty() => {
            ruleset = ruleset.add_rules(path_beneath_rules(roots, access_ro))?;
        }
        Some(_) => {}
        None => {
            ruleset = ruleset.add_rules(path_beneath_rules(["/"], access_ro))?;
        }
    }

    if !writable.is_empty() {
        ruleset = ruleset.add_rules(path_beneath_rules(writable, access_rw))?;
    }

    ruleset.restrict_self()
}

/// Prevent changes to the sandbox mount topology and, unless allowed, network access.
fn install_seccomp_filter(allow_network: bool) -> Result<(), String> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    let mut deny = |syscall: i64| {
        rules.insert(syscall, Vec::new());
    };

    deny(libc::SYS_mount);
    deny(libc::SYS_umount2);
    deny(libc::SYS_unshare);
    deny(libc::SYS_setns);

    if !allow_network {
        deny(libc::SYS_connect);
        deny(libc::SYS_accept);
        deny(libc::SYS_accept4);
        deny(libc::SYS_bind);
        deny(libc::SYS_listen);
        deny(libc::SYS_getpeername);
        deny(libc::SYS_getsockname);
        deny(libc::SYS_shutdown);
        deny(libc::SYS_sendto);
        deny(libc::SYS_sendmmsg);
        deny(libc::SYS_recvmmsg);
        deny(libc::SYS_getsockopt);
        deny(libc::SYS_setsockopt);

        deny(libc::SYS_ptrace);
        deny(libc::SYS_process_vm_readv);
        deny(libc::SYS_process_vm_writev);
        deny(libc::SYS_io_uring_setup);
        deny(libc::SYS_io_uring_enter);
        deny(libc::SYS_io_uring_register);

        let domain_is_not_unix = SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Ne,
            libc::AF_UNIX as u64,
        )
        .map_err(|error| format!("the socket-domain condition would not build: {error}"))?;
        let anything_but_unix = SeccompRule::new(vec![domain_is_not_unix])
            .map_err(|error| format!("the socket rule would not build: {error}"))?;
        rules.insert(libc::SYS_socket, vec![anything_but_unix.clone()]);
        rules.insert(libc::SYS_socketpair, vec![anything_but_unix]);
    }

    let target = if cfg!(target_arch = "x86_64") {
        TargetArch::x86_64
    } else if cfg!(target_arch = "aarch64") {
        TargetArch::aarch64
    } else {
        return Err("no seccomp filter is built for this architecture".to_string());
    };

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        target,
    )
    .map_err(|error| format!("the network filter would not build: {error}"))?;
    let program: BpfProgram = filter
        .try_into()
        .map_err(|error| format!("the network filter would not compile: {error}"))?;
    apply_filter(&program)
        .map_err(|error| format!("the network filter would not install: {error}"))?;
    Ok(())
}
