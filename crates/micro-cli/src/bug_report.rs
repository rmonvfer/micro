//! A bug report about micro: what it was running on and with, and what went wrong, with anything
//! that could unlock an account taken out.

use crate::archive::ArchiveFile;
use crate::archive::UtcTime;
use micro_session::LedgerLine;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::StopReason;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// What a removed value is replaced with.
pub const REDACTED: &str = "<redacted>";

/// The custom entry a session records a written report under.
pub const SESSION_ENTRY_TYPE: &str = "micro.bug-report";

const SCHEMA_VERSION: u32 = 1;

/// How many failures of each kind a report carries, newest kept.
const RECENT_FAILURES: usize = 20;

/// Words that mark a key whose value may unlock something.
const SENSITIVE_WORDS: [&str; 14] = [
    "apikey",
    "secret",
    "secrets",
    "token",
    "password",
    "passwords",
    "passwd",
    "credential",
    "credentials",
    "authorization",
    "cookie",
    "cookies",
    "bearer",
    "passphrase",
];

/// Settings keys that identify the installation rather than describe it.
const IDENTIFYING_KEYS: [&str; 2] = ["device_id", "tracking_id"];

/// Whether a key names something that may unlock an account, however it is cased or joined:
/// `api_key`, `apiKey`, `X-Api-Key` and `refreshToken` all do.
pub fn is_sensitive_key(key: &str) -> bool {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in key.chars() {
        if !character.is_ascii_alphanumeric() {
            words.push(std::mem::take(&mut current));
            previous_lower = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_lower {
            words.push(std::mem::take(&mut current));
        }
        previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
        current.push(character.to_ascii_lowercase());
    }
    words.push(current);
    words.retain(|word| !word.is_empty());

    let joined_api_key = words
        .windows(2)
        .any(|pair| pair[0] == "api" && pair[1] == "key");
    joined_api_key
        || words
            .iter()
            .any(|word| SENSITIVE_WORDS.contains(&word.as_str()))
}

/// A URL without its user name, password, or secret-looking query parameters. Anything that does
/// not parse as an absolute URL comes back as it was.
pub fn redact_url(value: &str) -> String {
    if let Some((prefix, inner)) = nested_url(value) {
        return format!("{prefix}{}", redact_url(inner));
    }
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return value.to_string();
    };
    if url.cannot_be_a_base() {
        return value.to_string();
    }

    let mut changed = false;
    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        changed = true;
    }
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if pairs.iter().any(|(key, _)| is_sensitive_key(key)) {
        url.query_pairs_mut()
            .clear()
            .extend_pairs(
                pairs
                    .iter()
                    .map(|(key, value)| match is_sensitive_key(key) {
                        true => (key.as_str(), REDACTED),
                        false => (key.as_str(), value.as_str()),
                    }),
            );
        changed = true;
    }
    match changed {
        true => url.to_string(),
        false => value.to_string(),
    }
}

/// A URL that names how it is fetched before the URL itself, as `git+https://` does not but
/// `git:https://` does.
fn nested_url(value: &str) -> Option<(&str, &str)> {
    let colon = value.find(':')?;
    let (scheme, inner) = (&value[..colon], &value[colon + 1..]);
    let scheme_like = |text: &str| {
        text.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    };
    let inner_scheme = inner.split_once("://")?.0;
    (scheme_like(scheme) && scheme_like(inner_scheme)).then(|| value.split_at(colon + 1))
}

/// Free text with every URL in it redacted, for error messages that quote the request they
/// failed on.
pub fn redact_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for character in text.chars() {
        if character.is_whitespace() {
            out.push_str(&redact_word(&word));
            word.clear();
            out.push(character);
        } else {
            word.push(character);
        }
    }
    out.push_str(&redact_word(&word));
    out
}

fn redact_word(word: &str) -> String {
    match word.contains("://") {
        true => redact_url(word),
        false => word.to_string(),
    }
}

/// A copy of a JSON value with the value of every sensitive key replaced, and every string that is
/// a URL stripped of credentials.
pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, child)| {
                    let redacted = match child.is_null() || !is_sensitive_key(key) {
                        true => redact_json(child),
                        false => Value::String(REDACTED.to_string()),
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        Value::String(text) => Value::String(redact_text(text)),
        other => other.clone(),
    }
}

/// A settings file as a report carries it: redacted, without the keys that identify the
/// installation, and saying so when it could not be read.
pub fn redacted_settings(path: &Path) -> Value {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Value::Null,
        Err(error) => return json!({ "unreadable": error.to_string() }),
    };
    if contents.trim().is_empty() {
        return Value::Object(Map::new());
    }
    match serde_json::from_str::<Value>(&contents) {
        Ok(Value::Object(mut fields)) => {
            for key in IDENTIFYING_KEYS {
                fields.remove(key);
            }
            redact_json(&Value::Object(fields))
        }
        Ok(other) => redact_json(&other),
        Err(error) => json!({ "unreadable": error.to_string() }),
    }
}

/// A path with the home directory written as `~`, so a report does not carry the user's name.
pub fn home_relative(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home.and_then(Path::to_str).filter(|home| !home.is_empty()) else {
        return path.to_string();
    };
    match path.strip_prefix(home) {
        Some(rest) if rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

/// What the report is about: the machine, the terminal, and the names of micro's variables.
pub fn environment() -> Value {
    let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let present = |name: &str| variable(name).is_some();
    let mut micro_variables: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.starts_with("MICRO_"))
        .collect();
    micro_variables.sort();

    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "shell": variable("SHELL").and_then(|shell| {
            Path::new(&shell).file_name().map(|name| name.to_string_lossy().into_owned())
        }),
        "terminal": {
            "term": variable("TERM"),
            "program": variable("TERM_PROGRAM"),
            "program_version": variable("TERM_PROGRAM_VERSION"),
            "colorterm": variable("COLORTERM"),
            "tmux": present("TMUX"),
            "ssh": present("SSH_CONNECTION") || present("SSH_CLIENT") || present("SSH_TTY"),
            "ci": present("CI"),
        },
        // Names help diagnose configuration; their values never go into a report.
        "micro_environment_variables": micro_variables,
    })
}

/// The extensions that loaded and the ones that would not, with paths under the home directory
/// shortened.
pub fn extensions(loaded: Option<&micro_extensions::Loaded>, home: Option<&Path>) -> Value {
    let Some(loaded) = loaded else {
        return json!({ "loaded": [], "errors": [] });
    };
    let names = |items: Vec<&String>| items.into_iter().cloned().collect::<Vec<_>>();
    json!({
        "loaded": loaded.extensions.iter().map(|extension| json!({
            "path": redact_url(&home_relative(&extension.path, home)),
            "tools": names(extension.tools.iter().map(|tool| &tool.name).collect()),
            "commands": names(extension.commands.iter().map(|command| &command.name).collect()),
            "events": extension.events,
            "providers": names(extension.providers.iter().map(|provider| &provider.name).collect()),
        })).collect::<Vec<_>>(),
        "errors": loaded.errors.iter().map(|failure| json!({
            "path": redact_url(&home_relative(&failure.path, home)),
            "error": redact_text(&failure.error),
        })).collect::<Vec<_>>(),
    })
}

/// The failures a session recorded, without any of the conversation: replies that ended in an
/// error or were aborted, and requests that had to be retried.
pub fn failures(messages: &[Message], events: &[LedgerLine]) -> Value {
    let mut replies: Vec<Value> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            Message::Assistant(reply)
                if reply.error.is_some()
                    || matches!(reply.stop_reason, StopReason::Error | StopReason::Aborted) =>
            {
                Some(json!({
                    "index": index,
                    "timestamp": reply.timestamp,
                    "provider": reply.provider,
                    "model": reply.model,
                    "stop_reason": reply.stop_reason,
                    "error": reply.error.as_deref().map(redact_text),
                }))
            }
            _ => None,
        })
        .collect();
    let mut attempts: Vec<Value> = events
        .iter()
        .filter_map(|line| match &line.event {
            LedgerEvent::RequestAttemptFailed {
                turn,
                attempt,
                error,
                ..
            } => Some(json!({
                "timestamp": line.ts,
                "turn": turn,
                "attempt": attempt,
                "error": redact_text(error),
            })),
            _ => None,
        })
        .collect();

    let assistant_messages = messages
        .iter()
        .filter(|message| matches!(message, Message::Assistant(_)))
        .count();
    json!({
        "schema_version": SCHEMA_VERSION,
        "message_count": messages.len(),
        "assistant_message_count": assistant_messages,
        "failed_replies": newest(&mut replies),
        "failed_request_attempts": newest(&mut attempts),
    })
}

/// The last [`RECENT_FAILURES`] of a list, oldest first.
fn newest(items: &mut Vec<Value>) -> Vec<Value> {
    let skip = items.len().saturating_sub(RECENT_FAILURES);
    items.drain(..skip);
    std::mem::take(items)
}

/// The id a report is filed under, such as `20261002-093000-1a2b`: when it was written, and enough
/// of the moment to tell two written in the same second apart.
pub fn report_id(now: SystemTime) -> String {
    let time = UtcTime::of(now);
    let fraction = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}-{:04x}",
        time.year,
        time.month,
        time.day,
        time.hour,
        time.minute,
        time.second,
        (fraction ^ std::process::id()) & 0xffff
    )
}

/// The name a report's archive is written under.
pub fn archive_name(id: &str) -> String {
    format!("micro-bug-report-{id}.zip")
}

/// A report ready to be written.
pub struct BugReport {
    pub id: String,
    pub created: SystemTime,
    /// What the report is about, as `report.json`.
    pub metadata: Value,
    /// What went wrong, as `diagnostics.json`.
    pub diagnostics: Value,
    /// The session log, when the user chose to include it.
    pub transcript: Option<String>,
}

impl BugReport {
    /// The files the archive holds.
    pub fn files(&self) -> Vec<ArchiveFile> {
        let pretty = |value: &Value| {
            let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
            text.push('\n');
            text
        };
        let mut files = vec![
            ArchiveFile::new("report.json", pretty(&self.metadata)),
            ArchiveFile::new("diagnostics.json", pretty(&self.diagnostics)),
        ];
        if let Some(transcript) = &self.transcript {
            files.push(ArchiveFile::new("session.jsonl", transcript.as_bytes()));
        }
        files
    }

    /// Write the archive into `directory`, and say where it went.
    pub fn write_to(&self, directory: &Path) -> std::io::Result<std::path::PathBuf> {
        let bytes = crate::archive::zip(&self.files(), self.created)?;
        let path = directory.join(archive_name(&self.id));
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// What the session records about this report, so a later look at the log shows one was made.
    pub fn session_entry(&self, path: &Path) -> Value {
        json!({
            "id": self.id,
            "created_at": UtcTime::of(self.created).rfc3339(),
            "transcript_included": self.transcript.is_some(),
            "path": path.display().to_string(),
        })
    }
}

/// Everything about the running session a report describes.
pub struct ReportInputs<'a> {
    pub description: Option<&'a str>,
    pub session_id: &'a str,
    pub workspace: &'a Path,
    pub model: &'a micro_models::ModelDef,
    pub thinking: micro_types::ThinkingLevel,
    pub tool_names: &'a [String],
    pub extensions: Option<&'a micro_extensions::Loaded>,
    pub messages: &'a [Message],
    pub events: &'a [LedgerLine],
    pub global_settings: &'a Path,
    pub project_settings: &'a Path,
    /// The raw session log, present only when the user chose to share it.
    pub transcript: Option<String>,
    pub home: Option<&'a Path>,
    pub now: SystemTime,
}

/// Put a report together.
pub fn build(inputs: ReportInputs<'_>) -> BugReport {
    let id = report_id(inputs.now);
    let included = inputs.transcript.is_some();
    let mut session = json!({
        "id": inputs.session_id,
        "transcript_included": included,
        "message_count": inputs.messages.len(),
    });
    if included {
        session["workspace"] = Value::String(home_relative(
            &inputs.workspace.display().to_string(),
            inputs.home,
        ));
    }
    let model = inputs.model;

    let metadata = json!({
        "schema_version": SCHEMA_VERSION,
        "id": id,
        "created_at": UtcTime::of(inputs.now).rfc3339(),
        "description": inputs.description.map(str::trim).filter(|text| !text.is_empty()),
        "environment": environment(),
        "session": session,
        "model": {
            "provider": model.provider,
            "id": model.id,
            "name": model.name,
            "api": model.api,
            "base_url": redact_url(&model.base_url),
            "reasoning": model.reasoning,
            "context_window": model.context_window,
            "max_output_tokens": model.max_output_tokens,
            "header_names": model.headers.keys().collect::<Vec<_>>(),
        },
        "thinking": inputs.thinking,
        "tools": inputs.tool_names,
        "extensions": extensions(inputs.extensions, inputs.home),
        "settings": {
            "global": redacted_settings(inputs.global_settings),
            "project": redacted_settings(inputs.project_settings),
        },
    });

    BugReport {
        id,
        created: inputs.now,
        metadata,
        diagnostics: failures(inputs.messages, inputs.events),
        transcript: inputs.transcript,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::AssistantMessage;
    use micro_types::Usage;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn keys_that_name_secrets_are_recognized_however_they_are_written() {
        for key in [
            "api_key",
            "apiKey",
            "X-Api-Key",
            "OPENAI_API_KEY",
            "refreshToken",
            "access_token",
            "Authorization",
            "client_secret",
            "password",
            "cookie",
            "credentials",
        ] {
            assert!(is_sensitive_key(key), "{key}");
        }
        for key in [
            "theme",
            "model",
            "max_tokens",
            "tokenizer",
            "keyboard",
            "auto_compact",
            "authority",
        ] {
            assert!(!is_sensitive_key(key), "{key}");
        }
    }

    #[test]
    fn a_url_loses_its_credentials_and_secret_parameters() {
        assert_eq!(
            redact_url("https://user:hunter2@example.com/v1?api_key=abc&region=eu"),
            "https://example.com/v1?api_key=%3Credacted%3E&region=eu"
        );
        assert_eq!(
            redact_url("git:https://bot:ghp_secret@github.com/acme/tools"),
            "git:https://github.com/acme/tools"
        );
        assert_eq!(
            redact_url("https://api.example.com/v1?region=eu"),
            "https://api.example.com/v1?region=eu"
        );
        assert_eq!(
            redact_url("anthropic/claude-opus-5"),
            "anthropic/claude-opus-5"
        );
    }

    #[test]
    fn error_text_keeps_its_words_but_not_the_keys_in_its_urls() {
        assert_eq!(
            redact_text("request to https://x.dev/v1?token=t0p failed: 401"),
            "request to https://x.dev/v1?token=%3Credacted%3E failed: 401"
        );
    }

    #[test]
    fn settings_lose_secrets_device_ids_and_tokens_but_keep_the_rest() {
        let settings = json!({
            "theme": "dark",
            "device_id": "6f1c0d8e-0000-4000-8000-000000000000",
            "tracking_id": "abc",
            "anthropic_api_key": "sk-ant-secret",
            "transport": "sse",
            "providers": {
                "local": {
                    "baseUrl": "http://admin:pw@localhost:8080/v1?key=x&apiKey=y",
                    "headers": { "Authorization": "Bearer sk-live", "X-Trace": "on" },
                    "apiKey": "sk-live",
                },
            },
            "extensions": ["~/ext/a.ts"],
            "budget": 3.5,
            "accessToken": null,
        });
        let path = std::env::temp_dir().join(format!(
            "micro-bug-report-settings-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, settings.to_string()).unwrap();
        let redacted = redacted_settings(&path);
        let _ = std::fs::remove_file(&path);

        let text = redacted.to_string();
        for secret in ["sk-ant-secret", "sk-live", "6f1c0d8e", "admin", "pw@"] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        assert!(redacted.get("device_id").is_none());
        assert!(redacted.get("tracking_id").is_none());
        assert_eq!(redacted["anthropic_api_key"], REDACTED);
        assert_eq!(redacted["providers"]["local"]["apiKey"], REDACTED);
        assert_eq!(
            redacted["providers"]["local"]["headers"]["Authorization"],
            REDACTED
        );
        assert_eq!(redacted["providers"]["local"]["headers"]["X-Trace"], "on");
        assert_eq!(
            redacted["providers"]["local"]["baseUrl"],
            "http://localhost:8080/v1?key=x&apiKey=%3Credacted%3E"
        );
        assert_eq!(redacted["theme"], "dark");
        assert_eq!(redacted["budget"], 3.5);
        assert!(redacted["accessToken"].is_null());
    }

    #[test]
    fn missing_and_broken_settings_are_described_rather_than_failing() {
        let missing = PathBuf::from("/nonexistent/micro/config.json");
        assert!(redacted_settings(&missing).is_null());

        let path = std::env::temp_dir().join(format!(
            "micro-bug-report-broken-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{ not json").unwrap();
        let described = redacted_settings(&path);
        let _ = std::fs::remove_file(&path);
        assert!(described.get("unreadable").is_some());
    }

    #[test]
    fn paths_under_home_are_shortened() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(
            home_relative("/Users/someone/.micro/extensions/a.ts", Some(&home)),
            "~/.micro/extensions/a.ts"
        );
        assert_eq!(
            home_relative("/Users/someoneelse/a.ts", Some(&home)),
            "/Users/someoneelse/a.ts"
        );
        assert_eq!(home_relative("/opt/a.ts", None), "/opt/a.ts");
    }

    fn reply(stop_reason: StopReason, error: Option<&str>) -> Message {
        Message::Assistant(AssistantMessage {
            content: Vec::new(),
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason,
            error: error.map(str::to_string),
            timestamp: 7,
        })
    }

    #[test]
    fn failures_carry_errors_but_no_conversation() {
        let messages = vec![
            Message::user("my private question"),
            reply(StopReason::Stop, None),
            reply(
                StopReason::Error,
                Some("401 from https://api.x.dev/v1?api_key=k3y"),
            ),
        ];
        let events = vec![LedgerLine {
            v: 1,
            seq: 1,
            ts: 9,
            event: LedgerEvent::RequestAttemptFailed {
                turn: 2,
                attempt: 1,
                error: "connection reset".into(),
                usage_unknown: false,
            },
        }];

        let diagnostics = failures(&messages, &events);
        let text = diagnostics.to_string();
        assert!(!text.contains("my private question"));
        assert!(!text.contains("k3y"));
        assert_eq!(diagnostics["message_count"], 3);
        assert_eq!(diagnostics["assistant_message_count"], 2);
        assert_eq!(diagnostics["failed_replies"][0]["index"], 2);
        assert_eq!(diagnostics["failed_replies"][0]["stop_reason"], "error");
        assert_eq!(
            diagnostics["failed_request_attempts"][0]["error"],
            "connection reset"
        );
    }

    #[test]
    fn only_the_newest_failures_are_kept() {
        let messages: Vec<Message> = (0..30).map(|_| reply(StopReason::Aborted, None)).collect();
        let diagnostics = failures(&messages, &[]);
        let kept = diagnostics["failed_replies"].as_array().unwrap();
        assert_eq!(kept.len(), RECENT_FAILURES);
        assert_eq!(kept[0]["index"], 10);
    }

    #[test]
    fn a_report_names_its_archive_after_when_it_was_written() {
        let moment = UNIX_EPOCH + Duration::from_secs(1_790_933_400);
        let id = report_id(moment);
        assert!(id.starts_with("20261002-093000-"), "{id}");
        assert_eq!(archive_name(&id), format!("micro-bug-report-{id}.zip"));
    }
}
