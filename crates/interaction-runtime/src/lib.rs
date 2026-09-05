//! The adaptive interaction runtime: one set of application services shared by
//! CLI, HTTP API and the Tauri desktop shell.

pub mod activity;
pub mod agents;
pub mod character;
pub mod character_session;
pub mod config;
pub mod curator;
pub mod domain_packs;
pub mod executor;
pub mod gateway;
pub mod hardware;
pub mod human;
pub mod knowledge;
pub mod lock;
pub mod memory;
pub mod mobile;
pub mod orchestrator;
pub mod presentation;
pub mod proactive;
pub mod providers;
pub mod runtime;
pub mod sensor_source;
pub mod sensors;
pub mod text;

pub use config::*;
pub use lock::*;
pub use runtime::*;
