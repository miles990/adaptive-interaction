//! Strongly-typed identifiers. All ids serialize as plain strings.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, schemars::JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Generate a fresh random id with the type's canonical prefix.
            pub fn generate() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::new_v4()))
            }

            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(
    /// Identifies a receptor instance, e.g. `system.time` or `mock.receptor`.
    ReceptorId, "rcp");
id_type!(
    /// Identifies an actuator instance, e.g. `conversation`.
    ActuatorId, "act");
id_type!(
    /// Identifies a single observation.
    ObservationId, "obs");
id_type!(
    /// Identifies a plan produced by the orchestrator.
    PlanId, "plan");
id_type!(
    /// Identifies a single bounded action / receipt.
    ActionId, "action");
id_type!(
    /// Identifies an interaction session (the consent boundary).
    SessionId, "session");
id_type!(
    /// Identifies a recipe.
    RecipeId, "recipe");
id_type!(
    /// Correlates observations, plans, actions and events end-to-end.
    CorrelationId, "corr");
id_type!(
    /// Identifies a runtime event on the event stream.
    EventId, "evt");
id_type!(
    /// Identifies a tool (namespace), e.g. `interaction`.
    ToolId, "tool");
id_type!(
    /// Identifies one operation of a tool, e.g. `interaction.observe`.
    OperationId, "op");
