//! Extensions: someone else's code, running beside micro rather than inside it.
//!
//! An extension is a TypeScript or JavaScript file with a default export. micro finds it
//! ([`discover`]), starts a Bun process to hold it ([`host::Host`]), and turns what it
//! registers into things micro already knows how to do: a tool the model may call, a
//! command a user may type, a handler that runs when something happens.

mod discover;
mod host;
mod tool;

pub use discover::discover;
pub use discover::in_directory;
pub use discover::PROJECT_DIR;
pub use host::install_host;
pub use host::which_bun;
pub use host::FromHost;
pub use host::Host;
pub use host::LoadFailure;
pub use host::Loaded;
pub use host::Registered;
pub use host::RegisteredCommand;
pub use host::RegisteredFlag;
pub use host::RegisteredShortcut;
pub use host::RegisteredTool;
pub use tool::ExtensionTool;
