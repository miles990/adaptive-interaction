//! Built-in receptors and actuators. None of them require special hardware;
//! the mock pair provides a full closed loop for tests and acceptance runs.

pub mod actuators;
pub mod outbox;
pub mod receptors;

pub use actuators::*;
pub use outbox::*;
pub use receptors::*;
