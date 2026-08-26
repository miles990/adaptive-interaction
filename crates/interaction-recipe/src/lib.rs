//! Interaction recipes: declarative YAML/JSON descriptions of adaptive
//! interactions, plus pure evaluation logic (trigger matching, receptor
//! fusion) with explainable outcomes.

pub mod condition;
pub mod fusion;
pub mod model;
pub mod trigger;
pub mod validate;

pub use condition::*;
pub use fusion::*;
pub use model::*;
pub use trigger::*;
pub use validate::*;
