//! interaction-core: domain model for the cross-AI adaptive interaction platform.
//!
//! This crate is dependency-light on purpose: no Tokio runtime, no HTTP, no
//! storage, no device protocols. Everything here is the shared contract used by
//! the runtime, registry, policy governor, recipe engine, API, CLI, Tauri shell
//! and adapters.

pub mod action;
pub mod capability;
pub mod error;
pub mod event;
pub mod health;
pub mod ids;
pub mod manifest;
pub mod message;
pub mod observation;
pub mod policy;
pub mod receipt;
pub mod session;
pub mod traits;

pub use action::*;
pub use capability::*;
pub use error::*;
pub use event::*;
pub use health::*;
pub use ids::*;
pub use manifest::*;
pub use message::*;
pub use observation::*;
pub use policy::*;
pub use receipt::*;
pub use session::*;
pub use traits::*;

/// Version stamped on every serialized domain object.
pub const SCHEMA_VERSION: &str = "1.0";

/// UTC timestamp alias used across the domain.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
