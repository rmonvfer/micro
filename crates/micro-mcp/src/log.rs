//! What servers say about themselves while they run: the log messages they send with
//! `notifications/message`, and what stdio servers write to their standard error. Both are
//! appended to one file, `mcp.log` in micro's data directory. Several micro processes may write to
//! it, so every line is one append. The file moves to `mcp.log.1` once it grows past
//! [`MAX_LOG_BYTES`].

use serde_json::Value;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// What the log file is called.
pub const LOG_FILE_NAME: &str = "mcp.log";

/// Past this size the log is moved aside and started again.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// One file servers' messages are appended to. A failure to write is ignored: logging never
/// gets in the way of a server's tools.
pub struct ServerLog {
    path: PathBuf,
    /// The file's size as last seen, once it has been looked at.
    size: Mutex<Option<u64>>,
}

impl ServerLog {
    pub fn new(path: impl Into<PathBuf>) -> ServerLog {
        ServerLog {
            path: path.into(),
            size: Mutex::new(None),
        }
    }

    /// `mcp.log` in micro's data directory.
    pub fn in_data_dir() -> Option<ServerLog> {
        micro_dirs::data_dir().map(|directory| ServerLog::new(directory.join(LOG_FILE_NAME)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one `notifications/message` from `server`.
    pub fn message(&self, server: &str, params: &Value) {
        self.append(&message_line(server, params, SystemTime::now()));
    }

    /// Append one line `server` wrote to its standard error.
    pub fn stderr(&self, server: &str, line: &str) {
        let line = format!(
            "{} [{server}] stderr {}\n",
            timestamp(SystemTime::now()),
            line.trim_end()
        );
        self.append(&line);
    }

    fn append(&self, line: &str) {
        let mut size = self
            .size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if size.is_none() {
            if let Some(directory) = self.path.parent() {
                let _ = std::fs::create_dir_all(directory);
            }
            *size = Some(self.current_size());
        }
        if size.is_some_and(|size| size > MAX_LOG_BYTES) {
            // Another process may have moved it aside already.
            if self.current_size() > MAX_LOG_BYTES {
                let mut aside = self.path.clone().into_os_string();
                aside.push(".1");
                let _ = std::fs::rename(&self.path, aside);
            }
            *size = Some(self.current_size());
        }
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if appended.is_ok() {
            *size = size.map(|size| size + line.len() as u64);
        }
    }

    fn current_size(&self) -> u64 {
        std::fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    }
}

/// One `notifications/message` as a log line: `<time> [<server>] <level> <logger>: <data>`, with
/// continuation lines indented.
fn message_line(server: &str, params: &Value, now: SystemTime) -> String {
    let level = params
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("info");
    let logger = params
        .get("logger")
        .and_then(Value::as_str)
        .filter(|logger| !logger.is_empty())
        .map(|logger| format!(" {logger}:"))
        .unwrap_or_default();
    let data = match params.get("data") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None if params.is_object() => String::new(),
        None => params.to_string(),
    };
    let data = data.replace("\r\n", "\n").replace('\n', "\n    ");
    format!("{} [{server}] {level}{logger} {data}\n", timestamp(now))
}

/// The time in UTC, as `2026-10-02T09:30:00Z`.
fn timestamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = match month_index < 10 {
        true => month_index + 3,
        false => month_index - 9,
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn a_message_reads_as_one_line_per_message() {
        let at = UNIX_EPOCH + Duration::from_secs(1_790_933_400);
        assert_eq!(
            message_line(
                "docs",
                &json!({ "level": "warning", "logger": "index", "data": "slow\nvery slow" }),
                at
            ),
            "2026-10-02T09:30:00Z [docs] warning index: slow\n    very slow\n"
        );
        assert_eq!(
            message_line("docs", &json!({ "data": { "n": 1 } }), at),
            "2026-10-02T09:30:00Z [docs] info {\"n\":1}\n"
        );
    }

    #[test]
    fn the_log_moves_aside_once_it_grows_too_large() {
        let directory = std::env::temp_dir().join(format!("micro-mcp-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join(LOG_FILE_NAME);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();

        let log = ServerLog::new(&path);
        log.stderr("docs", "starting\n");
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.ends_with("[docs] stderr starting\n"), "{kept}");
        assert_eq!(kept.lines().count(), 1);
        assert!(directory.join("mcp.log.1").exists());
    }
}
