//! The micro runtime: everything that resolves models, credentials, tools, extensions and
//! sessions into a ready agent, shared by the `micro` binary and other hosts.

pub mod access;
pub mod archive;
pub mod bug_report;
pub mod capabilities;
pub mod codemode;
pub mod codemode_models;
pub mod commands;
pub mod default_tools;
pub mod extension_broker;
pub mod extensions;
pub mod headless;
pub mod llama;
pub mod mcp;
pub mod model_registry;
pub mod remote;
pub mod runtime;
pub mod sandbox;
pub mod share;
pub mod subcommands;
pub mod update;
pub mod virtual_models;
