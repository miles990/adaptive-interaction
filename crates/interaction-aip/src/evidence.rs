//! §10 證據分類：fixture／simulator 永遠不得標成 real-device。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceClass {
    Unit,
    Contract,
    Fixture,
    Simulator,
    Integration,
    Browser,
    RealAgent,
    RealDevice,
    RealHardware,
    Unverified,
}

impl EvidenceClass {
    pub const ALL: [EvidenceClass; 10] = [
        EvidenceClass::Unit,
        EvidenceClass::Contract,
        EvidenceClass::Fixture,
        EvidenceClass::Simulator,
        EvidenceClass::Integration,
        EvidenceClass::Browser,
        EvidenceClass::RealAgent,
        EvidenceClass::RealDevice,
        EvidenceClass::RealHardware,
        EvidenceClass::Unverified,
    ];

    /// 是否算「真實」證據（真 agent／真機／真硬體）。fixture／simulator 一律 false。
    pub fn is_real(&self) -> bool {
        matches!(
            self,
            EvidenceClass::RealAgent | EvidenceClass::RealDevice | EvidenceClass::RealHardware
        )
    }
}
