//! Read-only hardware capability discovery contract.
//!
//! Discovery is metadata only: implementations MUST NOT open cameras,
//! microphones, raw input streams, radios, or device control channels. A scan
//! result is a candidate, not an installed/authorized runtime capability.

use crate::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HardwareClass {
    Camera,
    Microphone,
    AudioInput,
    AudioOutput,
    Keyboard,
    Mouse,
    Touchpad,
    Tablet,
    GameController,
    Midi,
    UsbSerial,
    BluetoothLe,
    Display,
    SystemNotification,
    OsSensor,
    MdnsDevice,
    Esp32Declaration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HardwareAvailability {
    Available,
    PermissionRequired,
    Busy,
    Unavailable,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoveredCapabilityKind {
    Receptor,
    Actuator,
    ToolOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredCapability {
    pub id: String,
    pub kind: DiscoveredCapabilityKind,
    /// Human-readable data/effect scope; no executable parameters at scan time.
    pub scope: String,
    pub read: bool,
    pub write: bool,
    pub requires_consent: bool,
    pub leaves_device: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHardware {
    pub class: HardwareClass,
    pub display_name: String,
    /// Stable device identity when the OS exposes one. None means the result
    /// MUST NOT be persisted or paired by a volatile path/name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
    pub identity_basis: String,
    pub availability: HardwareAvailability,
    #[serde(default)]
    pub permission_requirements: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<DiscoveredCapability>,
    pub source_adapter: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HardwareScanReport {
    pub platform: String,
    pub started_at: Timestamp,
    pub completed_at: Timestamp,
    /// Hard honesty proof: production scanners always return false. Tests
    /// assert this and adapters cannot omit the field.
    pub sensor_activation_attempted: bool,
    #[serde(default)]
    pub devices: Vec<DiscoveredHardware>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[async_trait::async_trait]
pub trait HardwareDiscoveryAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String>;
}
