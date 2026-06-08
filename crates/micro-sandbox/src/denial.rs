//! Telling a sandbox denial apart from an ordinary command failure.

use std::process::ExitStatus;

pub fn is_likely_denied(status: &ExitStatus, output: &str) -> bool {
    if status.success() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        const SIGNALLED_EXIT_BASE: i32 = 128;
        if status.signal() == Some(libc::SIGSYS)
            || status.code() == Some(SIGNALLED_EXIT_BASE + libc::SIGSYS)
        {
            return true;
        }
    }

    const DENIAL_WORDING: [&str; 6] = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "seccomp",
        "sandbox",
        "landlock",
    ];
    let output = output.to_lowercase();
    DENIAL_WORDING
        .iter()
        .any(|wording| output.contains(wording))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn a_command_that_succeeded_was_not_denied() {
        assert!(!is_likely_denied(
            &ExitStatus::from_raw(0),
            "operation not permitted"
        ));
    }

    #[test]
    fn a_process_killed_by_sigsys_was_denied() {
        assert!(is_likely_denied(&ExitStatus::from_raw(libc::SIGSYS), ""));
    }

    #[test]
    fn a_shell_reporting_sigsys_as_an_exit_code_was_denied() {
        let status = ExitStatus::from_raw((128 + libc::SIGSYS) << 8);
        assert!(is_likely_denied(&status, ""));
    }

    #[test]
    fn a_command_that_was_not_found_is_not_read_as_a_denial() {
        let status = ExitStatus::from_raw(127 << 8);
        assert!(!is_likely_denied(&status, "bash: nope: command not found"));
    }

    #[test]
    fn the_wording_the_platform_uses_for_a_refusal_counts_as_a_denial() {
        let status = ExitStatus::from_raw(1 << 8);
        assert!(is_likely_denied(
            &status,
            "touch: /etc/passwd: Operation not permitted"
        ));
        assert!(is_likely_denied(
            &status,
            "sandbox-exec: deny file-write-create"
        ));
        assert!(!is_likely_denied(&status, "error: no such table"));
    }
}
