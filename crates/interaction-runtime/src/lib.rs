//! The adaptive interaction runtime: one set of application services shared by
//! CLI, HTTP API and the Tauri desktop shell.

pub mod agents;
pub mod config;
pub mod executor;
pub mod gateway;
pub mod human;
pub mod knowledge;
pub mod lock;
pub mod memory;
pub mod orchestrator;
pub mod presentation;
pub mod proactive;
pub mod providers;
pub mod runtime;
pub mod sensors;
pub mod text;

pub use config::*;
pub use lock::*;
pub use runtime::*;
