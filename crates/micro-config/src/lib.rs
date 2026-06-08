//! Persisted settings: what micro does when nothing is said on the command line.

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

pub mod assignments;
mod capabilities;
mod project;
mod trust;

pub use capabilities::CapabilityDecision;
pub use capabilities::CapabilityStore;
pub use capabilities::CAPABILITIES_FILE_NAME;

pub use project::ProjectConfig;
pub use project::PROJECT_SETTINGS_FILE;

pub use trust::requires_decision;
pub use trust::ProjectTrust;
pub use trust::TrustDecision;
pub use trust::TrustStore;
pub use trust::PROJECT_DIR;
pub use trust::TRUST_FILE_NAME;

pub const MODEL_ENV: &str = "MICRO_MODEL";
pub const PROVIDER_ENV: &str = "MICRO_PROVIDER";
pub const THINKING_ENV: &str = "MICRO_THINKING";
pub const THEME_ENV: &str = "MICRO_THEME";
pub const LIVE_MODELS_ENV: &str = "MICRO_LIVE_MODELS";
/// The variable that turns on whatever is being tried out.
pub const EXPERIMENTAL_ENV: &str = "MICRO_EXPERIMENTAL";

/// How much of the terminal the interface takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TuiMode {
    /// A region at the cursor, as tall as the interface needs, leaving the conversation in the
    /// terminal's own scrollback.
    Regular,
    /// The whole screen, which scrolls internally and leaves the scrollback untouched.
    #[default]
    Fullscreen,
}

/// Whether this run has experimental behavior turned on.
pub fn experimental_enabled() -> bool {
    std::env::var(EXPERIMENTAL_ENV).is_ok_and(|value| value == "1")
}

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

    #[error("-c {assignment}: {message}")]
    Override { assignment: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Thinking {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// What a second escape does when the prompt is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoubleEscape {
    /// Show the conversation's branches, to go back to one.
    #[default]
    Tree,
    /// Branch from an earlier message.
    Fork,
    /// Nothing at all.
    None,
}

/// What happens to a prompt written while an answer is arriving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpMode {
    /// Hold it until the turn finishes, then send it.
    #[default]
    Queue,
    /// Send it as soon as it is written, interrupting what is running.
    Interrupt,
}

/// How many queued messages go at once when a turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SteeringMode {
    /// The oldest one, leaving the rest for the turns after it.
    #[default]
    OneAtATime,
    /// Every one of them, as a single message.
    All,
}

/// What the conversation tree shows before anything is asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeFilter {
    #[default]
    Default,
    /// The same, without what the tools did.
    NoTools,
    /// Only what the user wrote.
    UserOnly,
    /// Only what has been given a name.
    LabeledOnly,
    /// Everything there is.
    All,
}

/// What is left on the terminal after a full-screen session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExitOutput {
    #[default]
    Transcript,

    ResumeHint,
}

/// When the conversation shows how far through it you are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scrollbar {
    /// Only when there is more than fits.
    #[default]
    Auto,
    /// Whether or not there is.
    Always,
    /// Never.
    Hidden,
}

/// Whether a diagram written in a code block is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mermaid {
    /// Left as the code it was written as.
    Off,
    /// Drawn once the answer holding it is complete.
    Final,
    /// Drawn as it arrives.
    #[default]
    Streaming,
}

/// The config file, as it is written on disk.
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

    pub tui_mode: Option<TuiMode>,
    /// Merge live provider listings into the model catalog on startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_models: Option<bool>,

    /// Summarize the conversation on its own once the context fills up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact: Option<bool>,
    /// Check packaged installations for a newer signed release before an interactive session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
    /// Hours between automatic update checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_check_interval_hours: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_thinking: Option<bool>,
    /// Draw images in the terminal, where the terminal can.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_images: Option<bool>,
    /// The widest an image may be drawn, in cells.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_width_cells: Option<u16>,
    /// Shrink an image that would be wider than the room it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_resize_images: Option<bool>,
    /// Refuse to attach images at all, for a model or a workflow that cannot take them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_images: Option<bool>,
    /// Announce skills to the model, so it can reach for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_commands: Option<bool>,
    /// Columns of breathing room on each side of the input and lower interface components.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_padding: Option<u16>,
    /// Columns and rows kept clear between the terminal's edges and the interface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_padding: Option<u16>,
    pub steering_mode: Option<SteeringMode>,
    pub tree_filter_mode: Option<TreeFilter>,
    pub fullscreen_exit_output: Option<ExitOutput>,
    pub fullscreen_scrollbar: Option<Scrollbar>,
    pub clear_on_shrink: Option<bool>,
    pub mermaid: Option<Mermaid>,
    /// How many completions the command menu offers at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocomplete_max_items: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hardware_cursor: Option<bool>,
    /// Report progress to the terminal while a turn runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_progress: Option<bool>,
    /// Open without the introduction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_startup: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse_changelog: Option<bool>,
    /// Show warnings at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<bool>,
    /// Say when a request paid to write a cache it could have read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_miss_notices: Option<bool>,
    /// What a second escape does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_escape: Option<DoubleEscape>,
    /// What happens to a prompt written while an answer is arriving.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_up_mode: Option<FollowUpMode>,
    /// What to do about a project nobody has decided about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_project_trust: Option<ProjectTrust>,
    /// How long a request may go without producing anything, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_idle_timeout: Option<u64>,
    /// Models this workspace may use, when it should not have the whole catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_models: Option<Vec<String>>,
    /// Programs that provide tools over the Model Context Protocol, by the name their tools are
    /// announced under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Map<String, Value>>,
    /// How many tools beyond the built-in ones are described to the model up front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_search_threshold: Option<usize>,
    /// Warn that Anthropic subscription auth bills per token in a third-party harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_extra_usage: Option<bool>,
    /// How the ChatGPT Codex backend should answer: `sse`, or `auto` to let it decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Value>,
    /// What one session may spend before it stops, in US dollars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
    /// Extensions to load beyond the ones found in the project and the home directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,

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
    pub live_models: Option<bool>,
}

/// The settings actually in force, with every default applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub thinking: Thinking,
    pub theme: String,
    pub tui_mode: TuiMode,
    pub live_models: bool,

    pub auto_compact: bool,
    pub auto_update: bool,
    pub update_check_interval_hours: u64,
    pub hide_thinking: bool,
    pub show_images: bool,
    pub image_width_cells: u16,
    pub auto_resize_images: bool,
    pub block_images: bool,
    pub skill_commands: bool,
    pub content_padding: u16,
    pub interface_padding: u16,
    pub steering_mode: SteeringMode,
    pub tree_filter_mode: TreeFilter,
    pub fullscreen_exit_output: ExitOutput,
    pub fullscreen_scrollbar: Scrollbar,
    pub clear_on_shrink: bool,
    pub mermaid: Mermaid,
    pub autocomplete_max_items: usize,
    pub show_hardware_cursor: bool,
    pub terminal_progress: bool,
    pub quiet_startup: bool,
    pub collapse_changelog: bool,
    pub warnings: bool,
    pub cache_miss_notices: bool,
    pub double_escape: DoubleEscape,
    pub follow_up_mode: FollowUpMode,
    pub default_project_trust: ProjectTrust,
    pub http_idle_timeout: u64,
    pub scoped_models: Vec<String>,
    pub mcp_servers: Map<String, Value>,
    pub tool_search_threshold: usize,
    pub anthropic_extra_usage: bool,
    pub transport: String,
    /// The sandbox policy the user settled on, if they settled on one.
    pub sandbox: Option<Value>,
    /// What one session may spend before it stops, in US dollars.
    pub budget: f64,
    pub extensions: Vec<String>,
}

/// The widest an image is drawn when nothing says otherwise.
pub const DEFAULT_IMAGE_WIDTH_CELLS: u16 = 60;
/// How far in from the terminal's edges the interface sits when nothing says otherwise.
pub const DEFAULT_PADDING: u16 = 0;
/// How many completions the command menu offers at once.
pub const DEFAULT_AUTOCOMPLETE_MAX_ITEMS: usize = 5;
/// How long a request may go without producing anything before it is given up on.
pub const DEFAULT_HTTP_IDLE_TIMEOUT: u64 = 120;
/// How the Codex backend answers when nothing says otherwise.
pub const DEFAULT_TRANSPORT: &str = "sse";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            model: None,
            provider: None,
            tui_mode: TuiMode::default(),
            thinking: Thinking::default(),
            theme: DEFAULT_THEME.to_string(),
            live_models: false,

            auto_compact: true,
            auto_update: true,
            update_check_interval_hours: 24,
            hide_thinking: true,
            show_images: true,
            image_width_cells: DEFAULT_IMAGE_WIDTH_CELLS,
            auto_resize_images: true,
            block_images: false,
            skill_commands: true,
            content_padding: 1,
            interface_padding: 0,
            steering_mode: SteeringMode::default(),
            tree_filter_mode: TreeFilter::default(),
            fullscreen_exit_output: ExitOutput::default(),
            fullscreen_scrollbar: Scrollbar::default(),
            clear_on_shrink: false,
            mermaid: Mermaid::default(),
            autocomplete_max_items: DEFAULT_AUTOCOMPLETE_MAX_ITEMS,
            show_hardware_cursor: false,
            terminal_progress: true,
            quiet_startup: false,
            collapse_changelog: false,
            warnings: true,
            cache_miss_notices: false,
            double_escape: DoubleEscape::default(),
            follow_up_mode: FollowUpMode::default(),
            default_project_trust: ProjectTrust::default(),
            http_idle_timeout: DEFAULT_HTTP_IDLE_TIMEOUT,
            scoped_models: Vec::new(),
            mcp_servers: Map::new(),
            tool_search_threshold: 15,
            anthropic_extra_usage: true,
            transport: DEFAULT_TRANSPORT.to_string(),
            sandbox: None,
            budget: 0.0,
            extensions: Vec::new(),
        }
    }
}

impl Config {
    /// Read the config from its default path.
    pub fn load() -> Result<Config> {
        Config::load_from(default_path()?)
    }

    /// Read the config from its default path, then apply `key=value` assignments.
    pub fn load_with(overrides: &[String]) -> Result<Config> {
        Config::load_from_with(default_path()?, overrides)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Config> {
        Config::load_from_with(path, &[])
    }

    /// Read the config, then apply the settings named on the command line.
    pub fn load_from_with(path: impl AsRef<Path>, overrides: &[String]) -> Result<Config> {
        let path = path.as_ref();
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(io_error(path, error)),
        };

        let mut value: Value = if contents.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&contents).map_err(|error| ConfigError::Malformed {
                path: path.display().to_string(),
                message: error.to_string(),
            })?
        };

        let written = assignments::apply_all(&mut value, overrides)?;

        Config::from_value(value, path).map_err(|error| match &error {
            ConfigError::Field { field, message, .. } => written
                .iter()
                .find(|written| &written.key == field)
                .map(|written| ConfigError::Override {
                    assignment: written.assignment.clone(),
                    message: message.clone(),
                })
                .unwrap_or(error),
            _ => error,
        })
    }

    /// Write the config to its default path, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        self.save_to(default_path()?)
    }

    /// Write the config through a temporary file.
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

    /// The settings in force.
    pub fn resolve(
        &self,
        arguments: &Overrides,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Settings> {
        let defaults = Settings::default();
        let read = |variable: &str| {
            environment(variable)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };

        Ok(Settings {
            tui_mode: self.tui_mode.unwrap_or_default(),
            steering_mode: self.steering_mode.unwrap_or(defaults.steering_mode),
            tree_filter_mode: self.tree_filter_mode.unwrap_or(defaults.tree_filter_mode),
            fullscreen_exit_output: self
                .fullscreen_exit_output
                .unwrap_or(defaults.fullscreen_exit_output),
            fullscreen_scrollbar: self
                .fullscreen_scrollbar
                .unwrap_or(defaults.fullscreen_scrollbar),
            clear_on_shrink: self.clear_on_shrink.unwrap_or(defaults.clear_on_shrink),
            mermaid: self.mermaid.unwrap_or(defaults.mermaid),
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
            live_models: layered(
                arguments.live_models,
                from_env::<BoolSetting>(LIVE_MODELS_ENV, read(LIVE_MODELS_ENV))?.map(bool::from),
                self.live_models,
            )
            .unwrap_or(false),

            auto_compact: self.auto_compact.unwrap_or(defaults.auto_compact),
            auto_update: self.auto_update.unwrap_or(defaults.auto_update),
            update_check_interval_hours: self
                .update_check_interval_hours
                .unwrap_or(defaults.update_check_interval_hours)
                .max(1),
            hide_thinking: self.hide_thinking.unwrap_or(defaults.hide_thinking),
            show_images: self.show_images.unwrap_or(defaults.show_images),
            image_width_cells: self
                .image_width_cells
                .unwrap_or(defaults.image_width_cells)
                .max(1),
            auto_resize_images: self
                .auto_resize_images
                .unwrap_or(defaults.auto_resize_images),
            block_images: self.block_images.unwrap_or(defaults.block_images),
            skill_commands: self.skill_commands.unwrap_or(defaults.skill_commands),
            content_padding: self.content_padding.unwrap_or(defaults.content_padding),
            interface_padding: self.interface_padding.unwrap_or(defaults.interface_padding),
            autocomplete_max_items: self
                .autocomplete_max_items
                .unwrap_or(defaults.autocomplete_max_items)
                .max(1),
            show_hardware_cursor: self
                .show_hardware_cursor
                .unwrap_or(defaults.show_hardware_cursor),
            terminal_progress: self.terminal_progress.unwrap_or(defaults.terminal_progress),
            quiet_startup: self.quiet_startup.unwrap_or(defaults.quiet_startup),
            collapse_changelog: self
                .collapse_changelog
                .unwrap_or(defaults.collapse_changelog),
            warnings: self.warnings.unwrap_or(defaults.warnings),
            cache_miss_notices: self
                .cache_miss_notices
                .unwrap_or(defaults.cache_miss_notices),
            double_escape: self.double_escape.unwrap_or(defaults.double_escape),
            follow_up_mode: self.follow_up_mode.unwrap_or(defaults.follow_up_mode),
            default_project_trust: self
                .default_project_trust
                .unwrap_or(defaults.default_project_trust),
            http_idle_timeout: self
                .http_idle_timeout
                .unwrap_or(defaults.http_idle_timeout)
                .max(1),
            scoped_models: self.scoped_models.clone().unwrap_or(defaults.scoped_models),
            mcp_servers: self.mcp_servers.clone().unwrap_or(defaults.mcp_servers),
            tool_search_threshold: self
                .tool_search_threshold
                .unwrap_or(defaults.tool_search_threshold),
            anthropic_extra_usage: self
                .anthropic_extra_usage
                .unwrap_or(defaults.anthropic_extra_usage),
            transport: self.transport.clone().unwrap_or(defaults.transport),

            sandbox: self.sandbox.clone().or(defaults.sandbox),

            budget: self.budget.unwrap_or(defaults.budget).max(0.0),
            extensions: self.extensions.clone().unwrap_or(defaults.extensions),
        })
    }

    fn from_value(value: Value, path: &Path) -> Result<Config> {
        let Value::Object(mut fields) = value else {
            return Err(ConfigError::NotAnObject {
                path: path.display().to_string(),
            });
        };

        let config = Config {
            tui_mode: take(&mut fields, "tui_mode", path)?,
            steering_mode: take(&mut fields, "steering_mode", path)?,
            tree_filter_mode: take(&mut fields, "tree_filter_mode", path)?,
            fullscreen_exit_output: take(&mut fields, "fullscreen_exit_output", path)?,
            fullscreen_scrollbar: take(&mut fields, "fullscreen_scrollbar", path)?,
            clear_on_shrink: take(&mut fields, "clear_on_shrink", path)?,
            mermaid: take(&mut fields, "mermaid", path)?,
            model: take(&mut fields, "model", path)?,
            provider: take(&mut fields, "provider", path)?,
            thinking: take(&mut fields, "thinking", path)?,
            theme: take(&mut fields, "theme", path)?,
            live_models: take(&mut fields, "live_models", path)?,
            auto_compact: take(&mut fields, "auto_compact", path)?,
            auto_update: take(&mut fields, "auto_update", path)?,
            update_check_interval_hours: take(&mut fields, "update_check_interval_hours", path)?,
            hide_thinking: take(&mut fields, "hide_thinking", path)?,
            show_images: take(&mut fields, "show_images", path)?,
            image_width_cells: take(&mut fields, "image_width_cells", path)?,
            auto_resize_images: take(&mut fields, "auto_resize_images", path)?,
            block_images: take(&mut fields, "block_images", path)?,
            skill_commands: take(&mut fields, "skill_commands", path)?,
            content_padding: take(&mut fields, "content_padding", path)?,
            interface_padding: take(&mut fields, "interface_padding", path)?,
            autocomplete_max_items: take(&mut fields, "autocomplete_max_items", path)?,
            show_hardware_cursor: take(&mut fields, "show_hardware_cursor", path)?,
            terminal_progress: take(&mut fields, "terminal_progress", path)?,
            quiet_startup: take(&mut fields, "quiet_startup", path)?,
            collapse_changelog: take(&mut fields, "collapse_changelog", path)?,
            warnings: take(&mut fields, "warnings", path)?,
            cache_miss_notices: take(&mut fields, "cache_miss_notices", path)?,
            double_escape: take(&mut fields, "double_escape", path)?,
            follow_up_mode: take(&mut fields, "follow_up_mode", path)?,
            default_project_trust: take(&mut fields, "default_project_trust", path)?,
            http_idle_timeout: take(&mut fields, "http_idle_timeout", path)?,
            scoped_models: take(&mut fields, "scoped_models", path)?,
            mcp_servers: take(&mut fields, "mcp_servers", path)?,
            tool_search_threshold: take(&mut fields, "tool_search_threshold", path)?,
            anthropic_extra_usage: take(&mut fields, "anthropic_extra_usage", path)?,
            transport: take(&mut fields, "transport", path)?,
            sandbox: take(&mut fields, "sandbox", path)?,
            budget: take(&mut fields, "budget", path)?,
            extensions: take(&mut fields, "extensions", path)?,
            extra: fields,
        };
        Ok(config)
    }
}

/// The value in force for one setting: an explicit argument beats the environment, which beats the
/// config file.
pub fn layered<T>(argument: Option<T>, environment: Option<T>, configured: Option<T>) -> Option<T> {
    argument.or(environment).or(configured)
}

/// Read a setting out of one environment variable, naming the variable if its value cannot be read.
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

/// The directory micro keeps everything the user has settled in.
pub fn config_dir() -> Result<PathBuf> {
    micro_dirs::config_dir().ok_or_else(|| ConfigError::Io {
        path: "micro's configuration directory".into(),
        message: format!("no home directory; set {}", micro_dirs::MICRO_DIR_ENV),
    })
}

/// The settings file, under whichever directory holds the configuration.
pub fn default_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(FILE_NAME))
}

/// Read one field, naming it if it does not fit.
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
            "minimal" => Ok(Thinking::Minimal),
            "low" => Ok(Thinking::Low),
            "medium" => Ok(Thinking::Medium),
            "high" => Ok(Thinking::High),
            "xhigh" => Ok(Thinking::XHigh),
            "max" => Ok(Thinking::Max),
            other => Err(format!(
                "unknown thinking level `{other}` - expected off, minimal, low, medium, high, xhigh, or max"
            )),
        }
    }
}

impl fmt::Display for Thinking {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Thinking::Off => "off",
            Thinking::Minimal => "minimal",
            Thinking::Low => "low",
            Thinking::Medium => "medium",
            Thinking::High => "high",
            Thinking::XHigh => "xhigh",
            Thinking::Max => "max",
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
    fn the_settings_file_sits_in_the_configuration_directory() {
        let directory = config_dir().expect("a home directory");
        assert_eq!(default_path().unwrap(), directory.join(FILE_NAME));
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
                "live_models": true
            }"#,
        )
        .unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.provider.as_deref(), Some("openrouter"));
        assert_eq!(config.thinking, Some(Thinking::High));
        assert_eq!(config.theme.as_deref(), Some("light"));
        assert_eq!(config.live_models, Some(true));
        assert!(config.extra.is_empty());
    }

    #[test]
    fn a_settled_sandbox_policy_survives_resolution() {
        let path = scratch("sandbox").join("config.json");
        fs::write(&path, r#"{"sandbox": "read-only"}"#).unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.sandbox, Some(Value::from("read-only")));
        assert!(config.extra.is_empty(), "it is a key this build knows");

        let settings = config.resolve(&Overrides::default(), |_| None).unwrap();
        assert_eq!(settings.sandbox, Some(Value::from("read-only")));
        assert_eq!(
            Config::default()
                .resolve(&Overrides::default(), |_| None)
                .unwrap()
                .sandbox,
            None,
            "nobody having said anything is not the same as having said the default"
        );
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
            ..Config::default()
        };
        config.save_to(&path).unwrap();

        assert_eq!(Config::load_from(&path).unwrap(), config);
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"model\": \"opus\""), "{written}");

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
            theme: Some("light".into()),
            provider: Some("anthropic".into()),
            live_models: Some(false),
            ..Config::default()
        };
        let environment = environment(&[
            (THINKING_ENV, "medium"),
            (THEME_ENV, "dark"),
            (PROVIDER_ENV, "openrouter"),
            (LIVE_MODELS_ENV, "true"),
        ]);

        let from_env = config.resolve(&Overrides::default(), &environment).unwrap();
        assert_eq!(from_env.thinking, Thinking::Medium);
        assert_eq!(from_env.theme, "dark");
        assert_eq!(from_env.provider.as_deref(), Some("openrouter"));
        assert!(from_env.live_models);

        let arguments = Overrides {
            thinking: Some(Thinking::High),
            theme: Some("light".into()),
            provider: Some("gemini".into()),
            live_models: Some(false),
            ..Overrides::default()
        };
        let from_arguments = config.resolve(&arguments, &environment).unwrap();
        assert_eq!(from_arguments.thinking, Thinking::High);
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
        assert_eq!(settings.theme, "dark");
        assert!(!settings.live_models);
        assert_eq!(settings.model, None);
    }

    #[test]
    fn update_settings_default_to_a_daily_automatic_check() {
        let settings = Config::default()
            .resolve(&Overrides::default(), no_environment)
            .unwrap();
        assert!(settings.auto_update);
        assert_eq!(settings.update_check_interval_hours, 24);

        let configured = Config {
            auto_update: Some(false),
            update_check_interval_hours: Some(0),
            ..Config::default()
        }
        .resolve(&Overrides::default(), no_environment)
        .unwrap();
        assert!(!configured.auto_update);
        assert_eq!(configured.update_check_interval_hours, 1);
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
        assert!(
            error.contains("off, minimal, low, medium, high, xhigh, or max"),
            "{error}"
        );
    }

    #[test]
    fn settings_are_written_the_way_a_shell_writes_them() {
        assert_eq!("HIGH".parse::<Thinking>().unwrap(), Thinking::High);
        assert_eq!("none".parse::<Thinking>().unwrap(), Thinking::Off);
        assert!("extreme".parse::<Thinking>().is_err());

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
