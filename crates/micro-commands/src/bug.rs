//! `/bug`: a report about micro itself, written to a file the user attaches to an issue.

use crate::outcome::BugReportAction;
use crate::CommandOutcome;
use crate::Picker;
use crate::PickerItem;

/// Where micro's issues are filed.
pub const ISSUES_URL: &str = "https://github.com/rmonvfer/micro/issues/new";

const INCLUDE_TRANSCRIPT: &str = "--transcript";
const WITHOUT_TRANSCRIPT: &str = "--no-transcript";
const OPEN_ISSUE: &str = "--open-issue";

const DISCLAIMER: &str = "The archive stays on this machine until you attach it. It holds micro's \
                          version, the platform and terminal, settings with secrets removed, \
                          loaded extensions, and errors recorded in this session.";

const TRANSCRIPT_NOTE: &str = "The transcript holds your messages, model output, tool calls and \
                               their results, including file contents and command output.";

/// `/bug [description]` asks whether to include the transcript; the rows it offers carry the
/// answer back as a flag.
pub fn command(argument: Option<&str>) -> CommandOutcome {
    let argument = argument.map(str::trim).unwrap_or_default();
    let (flag, rest) = match argument.split_once(char::is_whitespace) {
        Some((flag, rest)) => (flag, rest.trim()),
        None => (argument, ""),
    };

    match flag {
        OPEN_ISSUE => CommandOutcome::ReportBug(BugReportAction::OpenIssue),
        INCLUDE_TRANSCRIPT | WITHOUT_TRANSCRIPT => {
            CommandOutcome::ReportBug(BugReportAction::Export {
                transcript: flag == INCLUDE_TRANSCRIPT,
                description: Some(rest.to_string()).filter(|text| !text.is_empty()),
            })
        }
        _ => CommandOutcome::Choose(choice(argument)),
    }
}

/// The two ways a report can be written, each remembering the description it was given.
fn choice(description: &str) -> Picker {
    let line = |flag: &str| match description.is_empty() {
        true => format!("/bug {flag}"),
        false => format!("/bug {flag} {description}"),
    };
    Picker::new(
        "Report a bug",
        vec![
            PickerItem::new(
                "Diagnostics only",
                "no conversation content",
                line(WITHOUT_TRANSCRIPT),
            )
            .noting(DISCLAIMER),
            PickerItem::new(
                "Diagnostics and transcript",
                "adds this session's log",
                line(INCLUDE_TRANSCRIPT),
            )
            .noting(TRANSCRIPT_NOTE),
        ],
    )
    .titled()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exported(outcome: CommandOutcome) -> (bool, Option<String>) {
        match outcome {
            CommandOutcome::ReportBug(BugReportAction::Export {
                transcript,
                description,
            }) => (transcript, description),
            other => panic!("expected an export, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_bug_asks_about_the_transcript_and_keeps_the_description() {
        let CommandOutcome::Choose(picker) = command(Some("the editor froze")) else {
            panic!("expected a choice");
        };
        assert_eq!(
            picker.command_at(0),
            Some("/bug --no-transcript the editor froze")
        );
        assert_eq!(
            picker.command_at(1),
            Some("/bug --transcript the editor froze")
        );
    }

    #[test]
    fn a_flag_says_whether_the_transcript_goes_in() {
        assert_eq!(
            exported(command(Some("--transcript it hung"))),
            (true, Some("it hung".to_string()))
        );
        assert_eq!(exported(command(Some("--no-transcript"))), (false, None));
    }

    #[test]
    fn the_issue_tracker_can_be_opened() {
        assert!(matches!(
            command(Some("--open-issue")),
            CommandOutcome::ReportBug(BugReportAction::OpenIssue)
        ));
    }
}
