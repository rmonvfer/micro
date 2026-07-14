//! Answering the policy engine when a tool needs approval.

use async_trait::async_trait;
use micro_policy::Approval;
use micro_policy::ApprovalRequest;
use micro_policy::Approver;
use std::io::BufRead as _;
use std::io::IsTerminal as _;
use std::io::Write as _;

/// Asks on the terminal. Used by the non-interactive path, where the terminal is the only
/// place a question can go.
pub struct TerminalApprover;

#[async_trait]
impl Approver for TerminalApprover {
    async fn approve(&self, request: &ApprovalRequest) -> Approval {
        // Nothing can answer a prompt when input is piped, so the safe reading of silence
        // is refusal.
        if !std::io::stdin().is_terminal() {
            return Approval::Denied(format!(
                "{} was not approved: micro is not attached to a terminal, so it could not \
                 ask. Re-run interactively or lower the approval mode.",
                request.tool
            ));
        }

        eprintln!("\n  {} wants to run:", request.tool);
        if let Some(subject) = &request.subject {
            eprintln!("    {subject}");
        }
        eprintln!("    ({})", request.reason);
        eprint!("  allow? [y]es / [a]lways / [N]o: ");
        let _ = std::io::stderr().flush();

        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer).is_err() {
            return Approval::Denied("could not read an answer".into());
        }

        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => Approval::Once,
            "a" | "always" => Approval::Session,
            _ => Approval::Denied("the user declined".into()),
        }
    }
}
