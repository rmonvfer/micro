//! Persisted settings: what micro does when nothing is said on the command line.
//!
//! Settings live in `~/.micro/config.json`. Every field is optional, so a missing file is
//! the same as an empty one, and a key this version does not know is carried through a
//! save untouched rather than dropped.
//!
//! Three layers decide what is in force, each beating the one below it: an explicit
//! command-line argument, then an environment variable, then the config file.
//!
//! ```no_run
//! use micro_config::{Config, Overrides};
//!
//! let config = Config::load()?;
//! let settings = config.resolve_from_env(&Overrides {
//!     model: Some("opus".to_string()),
//!     ..Overrides::default()
//! })?;
//! println!("{:?} at {:?}", settings.model, settings.approval);
//! # Ok::<(), micro_config::ConfigError>(())
//! ```

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

pub const FILE_NAME: &str = "config.json";
pub const MICRO_DIR_ENV: &str = "MICRO_DIR";

pub const MODEL_ENV: &str = "MICRO_MODEL";
pub const PROVIDER_ENV: &str = "MICRO_PROVIDER";
pub const THINKING_ENV: &str = "MICRO_THINKING";
pub const THEME_ENV: &str = "MICRO_THEME";
pub const APPROVAL_ENV: &str = "MICRO_APPROVAL";
pub const LIVE_MODELS_ENV: &str = "MICRO_LIVE_MODELS";

/// The palette to use when the config names none.
pub const DEFAULT_THEME: &str = "dark";

pub type Result<T, E = ConfigError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path} is not valid JSON: {message}")]
    Malformed { path: String, message: String },

    #[error("{path}: the file must hold a JSON object")]
    NotAnObject { path: String },

    #[error("{path}: field `{field}` {message}")]
    Field {
        path: String,
        field: String,
        message: String,
    },

    #[error("{variable}: {message}")]
    Environment { variable: String, message: String },

    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

/// How much reasoning to ask a model for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Thinking {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

/// How much the agent may do without being asked. Mirrors the modes the policy layer
/// enforces; this is only where the choice is remembered between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Reading is free; changing a file or running a command is asked about.
    #[default]
    Cautious,
    /// Reading and editing inside the workspace are free; commands are still asked about.
    Workspace,
    /// Everything is allowed except what cannot be undone.
    Unrestricted,
}

/// The config file, as it is written on disk.
///
/// Every field is optional: absent means "no preference", and the default applies. Keys
/// this version does not recognize are kept in `extra` so that saving from an older
/// binary does not discard what a newer one wrote.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
    /// Merge live provider listings into the model catalog on startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_models: Option<bool>,

    /// Keys written by a version that knew more than this one.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Values supplied on the command line, each overriding everything below it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub thinking: Option<Thinking>,
    pub theme: Option<String>,
    pub approval: Option<ApprovalMode>,
    pub live_models: Option<bool>,
}

/// The settings actually in force, with every default applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// A model query — an id, a qualified id, a prefix, or an alias. The catalog resolves
    /// it; nothing here assumes a particular model exists.
    pub model: Option<String>,
    pub provider: Option<String>,
    pub thinking: Thinking,
    pub theme: String,
    pub approval: ApprovalMode,
    pub live_models: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            model: None,
            provider: None,
            thinking: Thinking::default(),
            theme: DEFAULT_THEME.to_string(),
            approval: ApprovalMode::default(),
            live_models: false,
        }
    }
}

impl Config {
    /// Read the config from its default path. A missing file is an empty config.
    pub fn load() -> Result<Config> {
        Config::load_from(default_path()?)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Config> {
        let path = path.as_ref();
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default())
            }
            Err(error) => return Err(io_error(path, error)),
        };

        // An empty file is what an editor leaves behind after clearing it out; treat it
        // the same as no file rather than as a syntax error.
        if contents.trim().is_empty() {
            return Ok(Config::default());
        }

        let value: Value =
            serde_json::from_str(&contents).map_err(|error| ConfigError::Malformed {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
        Config::from_value(value, path)
    }

    /// Write the config to its default path, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        self.save_to(default_path()?)
    }

    /// Write the config through a temporary file, so an interrupted save cannot leave a
    /// half-written file where a readable one used to be.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(directory).map_err(|error| io_error(path, error))?;

        let mut contents = serde_json::to_string_pretty(self).map_err(|error| ConfigError::Io {
            path: path.display().to_string(),
            message: format!("could not be encoded: {error}"),
        })?;
        contents.push('\n');

        let temporary = directory.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&temporary);

        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        };
        write().map_err(|error| io_error(path, error))?;

        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            io_error(path, error)
        })
    }

    /// The settings in force, reading overrides from the process environment.
    pub fn resolve_from_env(&self, arguments: &Overrides) -> Result<Settings> {
        self.resolve(arguments, |variable| std::env::var(variable).ok())
    }

    /// The settings in force. `environment` supplies the middle layer, so a caller can
    /// hand in its own for a test.
    ///
    /// A variable set to nothing counts as unset, which is what an exported-but-empty
    /// shell variable is meant to say.
    pub fn resolve(
        &self,
        arguments: &Overrides,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Settings> {
        let read = |variable: &str| {
            environment(variable)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };

        Ok(Settings {
            model: layered(arguments.model.clone(), read(MODEL_ENV), self.model.clone()),
            provider: layered(
                arguments.provider.clone(),
                read(PROVIDER_ENV),
                self.provider.clone(),
            ),
            thinking: layered(
                arguments.thinking,
                from_env(THINKING_ENV, read(THINKING_ENV))?,
                self.thinking,
            )
            .unwrap_or_default(),
            theme: layered(arguments.theme.clone(), read(THEME_ENV), self.theme.clone())
                .unwrap_or_else(|| DEFAULT_THEME.to_string()),
            approval: layered(
                arguments.approval,
                from_env(APPROVAL_ENV, read(APPROVAL_ENV))?,
                self.approval,
            )
            .unwrap_or_default(),
            live_models: layered(
                arguments.live_models,
                from_env::<BoolSetting>(LIVE_MODELS_ENV, read(LIVE_MODELS_ENV))?.map(bool::from),
                self.live_models,
            )
            .unwrap_or(false),
        })
    }

    fn from_value(value: Value, path: &Path) -> Result<Config> {
        let Value::Object(mut fields) = value else {
            return Err(ConfigError::NotAnObject {
                path: path.display().to_string(),
            });
        };

        let config = Config {
            model: take(&mut fields, "model", path)?,
            provider: take(&mut fields, "provider", path)?,
            thinking: take(&mut fields, "thinking", path)?,
            theme: take(&mut fields, "theme", path)?,
            approval: take(&mut fields, "approval", path)?,
            live_models: take(&mut fields, "live_models", path)?,
            extra: fields,
        };
        Ok(config)
    }
}

/// The value in force for one setting: an explicit argument beats the environment, which
/// beats the config file.
pub fn layered<T>(argument: Option<T>, environment: Option<T>, configured: Option<T>) -> Option<T> {
    argument.or(environment).or(configured)
}

/// Read a setting out of one environment variable, naming the variable if its value
/// cannot be read.
fn from_env<T: FromStr<Err = String>>(variable: &str, value: Option<String>) -> Result<Option<T>> {
    value
        .map(|value| {
            value.parse().map_err(|message| ConfigError::Environment {
                variable: variable.to_string(),
                message,
            })
        })
        .transpose()
}

/// `$MICRO_DIR/config.json`, falling back to `~/.micro/config.json`.
pub fn default_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok());
    path_from(
        std::env::var(MICRO_DIR_ENV).ok().as_deref(),
        home.as_deref(),
    )
    .ok_or_else(|| ConfigError::Io {
        path: "~/.micro".into(),
        message: format!("no home directory; set {MICRO_DIR_ENV}"),
    })
}

fn path_from(micro_dir: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = micro_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir).join(FILE_NAME));
    }
    home.map(str::trim)
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".micro").join(FILE_NAME))
}

/// Read one field, naming it if it does not fit. An explicit `null` is read as "unset",
/// so a field can be cleared without deleting the line.
fn take<T: serde::de::DeserializeOwned>(
    fields: &mut Map<String, Value>,
    key: &str,
    path: &Path,
) -> Result<Option<T>> {
    let Some(value) = fields.remove(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| ConfigError::Field {
            path: path.display().to_string(),
            field: key.to_string(),
            message: error.to_string(),
        })
}

fn io_error(path: &Path, error: std::io::Error) -> ConfigError {
    ConfigError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

impl FromStr for Thinking {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "off" | "none" => Ok(Thinking::Off),
            "low" => Ok(Thinking::Low),
            "medium" => Ok(Thinking::Medium),
            "high" => Ok(Thinking::High),
            other => Err(format!(
                "unknown thinking level `{other}` - expected off, low, medium, or high"
            )),
        }
    }
}

impl FromStr for ApprovalMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "cautious" => Ok(ApprovalMode::Cautious),
            "workspace" => Ok(ApprovalMode::Workspace),
            "unrestricted" => Ok(ApprovalMode::Unrestricted),
            other => Err(format!(
                "unknown approval mode `{other}` - expected cautious, workspace, or unrestricted"
            )),
        }
    }
}

impl fmt::Display for Thinking {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Thinking::Off => "off",
            Thinking::Low => "low",
            Thinking::Medium => "medium",
            Thinking::High => "high",
        })
    }
}

impl fmt::Display for ApprovalMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            ApprovalMode::Cautious => "cautious",
            ApprovalMode::Workspace => "workspace",
            ApprovalMode::Unrestricted => "unrestricted",
        })
    }
}

/// A boolean as a person writes one in a shell.
impl FromStr for BoolSetting {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(BoolSetting(true)),
            "0" | "false" | "no" | "off" => Ok(BoolSetting(false)),
            other => Err(format!("`{other}` is not a yes or no value")),
        }
    }
}

/// Wrapper that gives `bool` the spellings a shell variable uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoolSetting(pub bool);

impl From<BoolSetting> for bool {
    fn from(value: BoolSetting) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    /// A directory of this process's own, so no test reads or writes a real config.
    fn scratch(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "micro-config-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |variable: &str| map.get(variable).cloned()
    }

    fn no_environment(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn micro_dir_overrides_the_home_directory() {
        assert_eq!(
            path_from(Some("/tmp/micro"), Some("/home/x")),
            Some(PathBuf::from("/tmp/micro/config.json"))
        );
        assert_eq!(
            path_from(None, Some("/home/x")),
            Some(PathBuf::from("/home/x/.micro/config.json"))
        );
        assert_eq!(
            path_from(Some("  "), Some("/home/x")),
            Some(PathBuf::from("/home/x/.micro/config.json"))
        );
        assert_eq!(path_from(None, None), None);
    }

    #[test]
    fn a_missing_or_empty_file_is_the_default_config() {
        let directory = scratch("absent");
        assert_eq!(
            Config::load_from(directory.join("config.json")).unwrap(),
            Config::default()
        );

        let blank = directory.join("blank.json");
        fs::write(&blank, "   \n").unwrap();
        assert_eq!(Config::load_from(&blank).unwrap(), Config::default());
    }

    #[test]
    fn every_field_is_read() {
        let path = scratch("full").join("config.json");
        fs::write(
            &path,
            r#"{
                "model": "opus",
                "provider": "openrouter",
                "thinking": "high",
                "theme": "light",
                "approval": "workspace",
                "live_models": true
            }"#,
        )
        .unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.provider.as_deref(), Some("openrouter"));
        assert_eq!(config.thinking, Some(Thinking::High));
        assert_eq!(config.theme.as_deref(), Some("light"));
        assert_eq!(config.approval, Some(ApprovalMode::Workspace));
        assert_eq!(config.live_models, Some(true));
        assert!(config.extra.is_empty());
    }

    #[test]
    fn a_field_of_the_wrong_type_is_named() {
        let path = scratch("bad-type").join("config.json");
        fs::write(&path, r#"{"model": 7}"#).unwrap();

        let error = Config::load_from(&path).unwrap_err().to_string();
        assert!(error.contains("field `model`"), "{error}");
        assert!(error.contains("config.json"), "{error}");
    }

    #[test]
    fn an_unknown_setting_value_names_the_field_that_holds_it() {
        let path = scratch("bad-variant").join("config.json");
        fs::write(&path, r#"{"thinking": "extreme"}"#).unwrap();

        let error = Config::load_from(&path).unwrap_err().to_string();
        assert!(error.contains("field `thinking`"), "{error}");
        assert!(error.contains("extreme"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_json_reports_where_it_broke() {
        let path = scratch("broken").join("config.json");
        fs::write(&path, "{ not json").unwrap();

        let error = Config::load_from(&path).unwrap_err();
        assert!(matches!(error, ConfigError::Malformed { .. }), "{error}");
        assert!(error.to_string().contains("line 1"), "{error}");
    }

    #[test]
    fn a_file_holding_something_other_than_an_object_is_rejected() {
        let path = scratch("array").join("config.json");
        fs::write(&path, "[1, 2]").unwrap();

        assert!(matches!(
            Config::load_from(&path).unwrap_err(),
            ConfigError::NotAnObject { .. }
        ));
    }

    #[test]
    fn a_null_field_reads_as_unset() {
        let path = scratch("null").join("config.json");
        fs::write(&path, r#"{"model": null, "thinking": "low"}"#).unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.model, None);
        assert_eq!(config.thinking, Some(Thinking::Low));
    }

    #[test]
    fn a_key_from_a_later_version_survives_a_save() {
        let path = scratch("forward").join("config.json");
        fs::write(
            &path,
            r#"{"model": "opus", "telepathy": {"enabled": true}}"#,
        )
        .unwrap();

        let mut config = Config::load_from(&path).unwrap();
        assert_eq!(config.extra["telepathy"]["enabled"], Value::Bool(true));

        config.model = Some("sonnet".into());
        config.save_to(&path).unwrap();

        let reloaded = Config::load_from(&path).unwrap();
        assert_eq!(reloaded.model.as_deref(), Some("sonnet"));
        assert_eq!(reloaded.extra["telepathy"]["enabled"], Value::Bool(true));
    }

    #[test]
    fn saving_creates_the_directory_and_round_trips() {
        let path = scratch("save").join("nested").join("config.json");
        let config = Config {
            model: Some("opus".into()),
            approval: Some(ApprovalMode::Unrestricted),
            ..Config::default()
        };
        config.save_to(&path).unwrap();

        assert_eq!(Config::load_from(&path).unwrap(), config);
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("\"approval\": \"unrestricted\""),
            "{written}"
        );
        // Fields nobody set stay out of the file rather than being written as null.
        assert!(!written.contains("theme"), "{written}");
    }

    #[test]
    fn an_argument_beats_the_environment_which_beats_the_file() {
        let config = Config {
            model: Some("from-file".into()),
            ..Config::default()
        };
        let environment = environment(&[(MODEL_ENV, "from-env")]);

        let arguments = Overrides {
            model: Some("from-argument".into()),
            ..Overrides::default()
        };
        assert_eq!(
            config.resolve(&arguments, &environment).unwrap().model,
            Some("from-argument".into())
        );

        assert_eq!(
            config
                .resolve(&Overrides::default(), &environment)
                .unwrap()
                .model,
            Some("from-env".into())
        );

        assert_eq!(
            config
                .resolve(&Overrides::default(), no_environment)
                .unwrap()
                .model,
            Some("from-file".into())
        );
    }

    #[test]
    fn precedence_holds_for_every_setting() {
        let config = Config {
            thinking: Some(Thinking::Low),
            approval: Some(ApprovalMode::Cautious),
            theme: Some("light".into()),
            provider: Some("anthropic".into()),
            live_models: Some(false),
            ..Config::default()
        };
        let environment = environment(&[
            (THINKING_ENV, "medium"),
            (APPROVAL_ENV, "workspace"),
            (THEME_ENV, "dark"),
            (PROVIDER_ENV, "openrouter"),
            (LIVE_MODELS_ENV, "true"),
        ]);

        let from_env = config.resolve(&Overrides::default(), &environment).unwrap();
        assert_eq!(from_env.thinking, Thinking::Medium);
        assert_eq!(from_env.approval, ApprovalMode::Workspace);
        assert_eq!(from_env.theme, "dark");
        assert_eq!(from_env.provider.as_deref(), Some("openrouter"));
        assert!(from_env.live_models);

        let arguments = Overrides {
            thinking: Some(Thinking::High),
            approval: Some(ApprovalMode::Unrestricted),
            theme: Some("light".into()),
            provider: Some("gemini".into()),
            live_models: Some(false),
            ..Overrides::default()
        };
        let from_arguments = config.resolve(&arguments, &environment).unwrap();
        assert_eq!(from_arguments.thinking, Thinking::High);
        assert_eq!(from_arguments.approval, ApprovalMode::Unrestricted);
        assert_eq!(from_arguments.theme, "light");
        assert_eq!(from_arguments.provider.as_deref(), Some("gemini"));
        assert!(!from_arguments.live_models);
    }

    #[test]
    fn an_empty_environment_variable_counts_as_unset() {
        let config = Config {
            model: Some("from-file".into()),
            ..Config::default()
        };
        let settings = config
            .resolve(&Overrides::default(), environment(&[(MODEL_ENV, "  ")]))
            .unwrap();

        assert_eq!(settings.model, Some("from-file".into()));
    }

    #[test]
    fn defaults_apply_when_nothing_says_otherwise() {
        let settings = Config::default()
            .resolve(&Overrides::default(), no_environment)
            .unwrap();

        assert_eq!(settings, Settings::default());
        assert_eq!(settings.thinking, Thinking::Off);
        assert_eq!(settings.approval, ApprovalMode::Cautious);
        assert_eq!(settings.theme, "dark");
        assert!(!settings.live_models);
        assert_eq!(settings.model, None);
    }

    #[test]
    fn an_unreadable_environment_variable_names_itself() {
        let error = Config::default()
            .resolve(
                &Overrides::default(),
                environment(&[(THINKING_ENV, "extreme")]),
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains(THINKING_ENV), "{error}");
        assert!(error.contains("off, low, medium, or high"), "{error}");
    }

    #[test]
    fn settings_are_written_the_way_a_shell_writes_them() {
        assert_eq!("HIGH".parse::<Thinking>().unwrap(), Thinking::High);
        assert_eq!("none".parse::<Thinking>().unwrap(), Thinking::Off);
        assert!("extreme".parse::<Thinking>().is_err());

        assert_eq!(
            "Unrestricted".parse::<ApprovalMode>().unwrap(),
            ApprovalMode::Unrestricted
        );
        assert!("yolo".parse::<ApprovalMode>().is_err());

        for yes in ["1", "true", "YES", "on"] {
            assert_eq!(yes.parse::<BoolSetting>().unwrap(), BoolSetting(true));
        }
        for no in ["0", "false", "NO", "off"] {
            assert_eq!(no.parse::<BoolSetting>().unwrap(), BoolSetting(false));
        }
        assert!("maybe".parse::<BoolSetting>().is_err());
    }

    #[test]
    fn a_setting_reads_back_as_it_is_written() {
        assert_eq!(Thinking::Medium.to_string(), "medium");
        assert_eq!(ApprovalMode::Workspace.to_string(), "workspace");
        assert_eq!(
            Thinking::Medium.to_string().parse::<Thinking>().unwrap(),
            Thinking::Medium
        );
    }

    #[test]
    fn layering_prefers_the_nearest_source() {
        assert_eq!(layered(Some(1), Some(2), Some(3)), Some(1));
        assert_eq!(layered(None, Some(2), Some(3)), Some(2));
        assert_eq!(layered(None, None, Some(3)), Some(3));
        assert_eq!(layered::<u8>(None, None, None), None);
    }
}
