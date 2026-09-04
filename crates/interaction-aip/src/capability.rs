//! §4.2 Capability 宣告與確定性協商（交集＋min）。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 參與方角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MemberRole {
    HostRenderer,
    RemoteRenderer,
    InputDevice,
    Observer,
}

/// 同步等級（§8／character-session.md §9）。1.0 只實作 `semantic`。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SyncClass {
    Semantic,
    Timeline,
    Realtime,
}

/// `capability` 宣告 payload（member → host）。未知鍵保留在 `extra`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityAnnouncement {
    #[serde(default)]
    pub spec_versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<MemberRole>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub sync_classes: Vec<SyncClass>,
    /// 可呈現的 Behavior Intent 名稱。
    #[serde(default)]
    pub intents: Vec<String>,
    /// 可產生的 event name。
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub features: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<CapabilityLimits>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_message_bytes: Option<usize>,
}

/// 單一 intent 的協商結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IntentSupport {
    Exact,
    Unsupported,
}

/// host 回的協商結果（`capability` payload，host → member）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NegotiatedCapabilities {
    pub spec_version: String,
    pub newer_minor: bool,
    pub role: MemberRole,
    pub sync_class: SyncClass,
    pub intents: BTreeMap<String, IntentSupport>,
    pub inputs: Vec<String>,
    // 對方宣告、host 不接受的 event name。**有界**：最多 `limits::MAX_UNSUPPORTED_INPUTS`
    // 個（超過就截斷，見下面的 `unsupported_inputs_truncated`）。
    // （刻意用 `//` 而不是 `///`：doc comment 會變成 JSON Schema 的 `description`，
    //   而 `schemas/aip-1.0.schema.json` 是 TS／Swift codegen 的單一來源。）
    pub unsupported_inputs: Vec<String>,
    pub limits: CapabilityLimits,
    // `unsupported_inputs` 被截斷過。**host 私有的診斷事實，不進 wire、不進 schema**
    // （`#[serde(skip)]`）：wire 形狀維持 1.0，截斷本身只留在 host 的稽核裡。
    #[serde(skip)]
    #[schemars(skip)]
    pub unsupported_inputs_truncated: bool,
}

/// host 端的本地能力（要求對方能呈現的 intent、接受的 input）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOffer {
    pub intents: Vec<String>,
    pub inputs: Vec<String>,
    pub sync_classes: Vec<SyncClass>,
}

/// 確定性協商：版本 → role（缺省 observer）→ sync class（交集裡最保守）→ intents（host 需要的每個
/// intent：對方宣告有→exact，否則 unsupported）→ inputs（對方宣告 ∩ host 接受）→ limits（min）。
pub fn negotiate_capabilities(
    offer: &HostOffer,
    announcement: &CapabilityAnnouncement,
) -> Result<NegotiatedCapabilities, crate::AipError> {
    let version = crate::negotiate_versions(&announcement.spec_versions)?;
    let role = announcement.role.unwrap_or(MemberRole::Observer);
    let sync_class = offer
        .sync_classes
        .iter()
        .filter(|c| announcement.sync_classes.contains(c))
        .min()
        .copied()
        .unwrap_or(SyncClass::Semantic);
    let mut intents = BTreeMap::new();
    for intent in &offer.intents {
        let support = if announcement.intents.iter().any(|i| i == intent) {
            IntentSupport::Exact
        } else {
            IntentSupport::Unsupported
        };
        intents.insert(intent.clone(), support);
    }
    let mut inputs = Vec::new();
    let mut unsupported_inputs = Vec::new();
    // 對方宣告的 `inputs` 是外部輸入、本身無界。`inputs` 有 host offer 當天花板（交集），
    // 但 `unsupported_inputs` 沒有：不設上限，host 的協商回覆就會超過自己的 payload 上限
    // （session-integrity-060）。截斷是確定性的（取前 N 個），並記下截斷這件事。
    let mut unsupported_inputs_truncated = false;
    for input in &announcement.inputs {
        if offer.inputs.iter().any(|i| i == input) {
            if !inputs.contains(input) {
                inputs.push(input.clone());
            }
        } else if !unsupported_inputs.contains(input) {
            if unsupported_inputs.len() >= crate::limits::MAX_UNSUPPORTED_INPUTS {
                unsupported_inputs_truncated = true;
                continue;
            }
            unsupported_inputs.push(input.clone());
        }
    }
    let remote_max = announcement
        .limits
        .and_then(|l| l.max_message_bytes)
        .unwrap_or(crate::limits::MAX_MESSAGE_BYTES);
    let limits = CapabilityLimits {
        max_message_bytes: Some(remote_max.clamp(1024, crate::limits::MAX_MESSAGE_BYTES)),
    };
    Ok(NegotiatedCapabilities {
        spec_version: version.spec_version,
        newer_minor: version.newer_minor,
        role,
        sync_class,
        intents,
        inputs,
        unsupported_inputs,
        limits,
        unsupported_inputs_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> HostOffer {
        HostOffer {
            intents: vec![
                "react-happily-to-touch".into(),
                "celebrate".into(),
                "idle".into(),
            ],
            inputs: vec!["character.interaction.touch".into()],
            sync_classes: vec![SyncClass::Semantic],
        }
    }

    #[test]
    fn negotiation_is_intersection_and_min() {
        let ann = CapabilityAnnouncement {
            spec_versions: vec!["aip/1.2".into()],
            role: Some(MemberRole::RemoteRenderer),
            sync_classes: vec![SyncClass::Realtime, SyncClass::Semantic],
            intents: vec!["celebrate".into(), "fly".into()],
            inputs: vec!["character.interaction.touch".into(), "device.tilt".into()],
            limits: Some(CapabilityLimits {
                max_message_bytes: Some(1 << 20),
            }),
            ..Default::default()
        };
        let n = negotiate_capabilities(&offer(), &ann).unwrap();
        assert_eq!(n.spec_version, "aip/1.0");
        assert!(n.newer_minor);
        assert_eq!(n.sync_class, SyncClass::Semantic);
        assert_eq!(n.intents["celebrate"], IntentSupport::Exact);
        assert_eq!(
            n.intents["react-happily-to-touch"],
            IntentSupport::Unsupported
        );
        assert!(
            !n.intents.contains_key("fly"),
            "renderer cannot invent intents the host never offers"
        );
        assert_eq!(n.inputs, vec!["character.interaction.touch".to_string()]);
        assert_eq!(n.unsupported_inputs, vec!["device.tilt".to_string()]);
        assert_eq!(
            n.limits.max_message_bytes,
            Some(crate::limits::MAX_MESSAGE_BYTES)
        );
    }

    /// 對方宣告的 `inputs` 是外部輸入（無界）。協商結果會被 host 序列化成一則要送上線的
    /// `capability` 回覆，所以 `unsupported_inputs` 必須是**有界集合**：超過上限就截斷，
    /// 並誠實記下「這份清單被截斷過」，不假裝自己列完了（session-integrity-060）。
    #[test]
    fn unsupported_inputs_are_bounded_and_truncation_is_recorded() {
        let announced: Vec<String> = (0..500).map(|i| format!("device.sensor-{i}")).collect();
        let ann = CapabilityAnnouncement {
            spec_versions: vec!["aip/1.0".into()],
            role: Some(MemberRole::RemoteRenderer),
            inputs: announced.clone(),
            ..Default::default()
        };
        let n = negotiate_capabilities(&offer(), &ann).unwrap();
        assert_eq!(
            n.unsupported_inputs.len(),
            crate::limits::MAX_UNSUPPORTED_INPUTS
        );
        assert!(n.unsupported_inputs_truncated, "截斷過就要說截斷過");
        assert_eq!(
            n.unsupported_inputs,
            announced[..crate::limits::MAX_UNSUPPORTED_INPUTS].to_vec(),
            "截斷是取前 N 個（確定性），不是隨機取樣"
        );
        // host 送得出去：協商回覆本身要進得了 payload 上限。
        let payload = serde_json::to_value(&n).unwrap();
        assert!(
            crate::canonical_json(&payload).len() <= crate::limits::MAX_PAYLOAD_BYTES,
            "host 自己送出的協商回覆不得超過自己的 payload 上限"
        );
        // 沒截斷時旗標為 false，且不進 wire（wire 形狀不變）。
        let small = negotiate_capabilities(
            &offer(),
            &CapabilityAnnouncement {
                spec_versions: vec!["aip/1.0".into()],
                inputs: vec!["device.tilt".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!small.unsupported_inputs_truncated);
        assert!(
            serde_json::to_value(&small)
                .unwrap()
                .get("unsupportedInputsTruncated")
                .is_none(),
            "這是 host 私有的診斷事實，不是新的 wire 欄位"
        );
    }

    #[test]
    fn unknown_keys_round_trip() {
        let raw = r#"{"specVersions":["aip/1.0"],"role":"input-device","futureThing":{"x":1}}"#;
        let ann: CapabilityAnnouncement = serde_json::from_str(raw).unwrap();
        assert_eq!(ann.extra["futureThing"]["x"], 1);
        let back = serde_json::to_value(&ann).unwrap();
        assert_eq!(back["futureThing"]["x"], 1);
        let err = negotiate_capabilities(
            &offer(),
            &CapabilityAnnouncement {
                spec_versions: vec!["aip/2.0".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, crate::ErrorCode::UnsupportedVersion);
    }
}
