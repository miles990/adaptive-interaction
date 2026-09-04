//! 跨語言 conformance：Rust 端逐檔跑 `tests/fixtures/manifest.json`。
//!
//! 同一份 `manifest.json` 也被 TypeScript（`apps/interaction-desktop/src/test/aip-conformance.test.ts`）
//! 與 Swift（`AIPConformanceTests.swift`，經 `scripts/aip-codegen.mjs` 內嵌）讀取，
//! 三個實作對同一組 fixture 必須得到同一個結論（ok 或同一個 ErrorCode）。
//!
//! 契約：`docs/aip/README.md` §14；跑法與重生：`docs/aip/conformance.md`。

use std::collections::BTreeSet;
use std::path::PathBuf;

use interaction_aip::{
    bind_identity, canonical_json, is_runtime_only_name, negotiate_capabilities, offline_policy,
    AipError, CapabilityAnnouncement, Envelope, ErrorCode, HostOffer, IdentityDecision,
    MessageType, NegotiatedCapabilities, Outcome, Party,
};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn manifest() -> Value {
    let path = fixtures_dir().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read the fixture index {path:?}: {e}"));
    serde_json::from_str(&text).expect("manifest.json must be valid JSON")
}

fn section(m: &Value, key: &str) -> Vec<Value> {
    m.get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("manifest.json is missing the `{key}` section"))
        .clone()
}

fn str_of(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("fixture entry missing `{key}`: {entry}"))
        .to_string()
}

/// 一則 wire bytes 走完整條檢查：大小 → 解析 → profile／上限／版本驗證。
fn evaluate(bytes: &[u8]) -> Result<Envelope, AipError> {
    let envelope = Envelope::parse(bytes)?;
    envelope.validate()?;
    Ok(envelope)
}

/// 依 manifest 的 `expect`／`code` 斷言結論一致，並確保錯誤訊息不回顯輸入。
fn assert_expectation(id: &str, entry: &Value, bytes: &[u8]) -> Option<Envelope> {
    let expect = str_of(entry, "expect");
    let outcome = evaluate(bytes);
    match (expect.as_str(), outcome) {
        ("ok", Ok(envelope)) => Some(envelope),
        ("ok", Err(e)) => panic!("fixture {id} should be accepted but failed: {:?}", e.code),
        ("error", Ok(_)) => panic!("fixture {id} should be rejected but passed validation"),
        ("error", Err(e)) => {
            let want = str_of(entry, "code");
            assert_eq!(
                e.code.as_str(),
                want,
                "fixture {id} produced the wrong ErrorCode"
            );
            for token in entry
                .get("mustNotEcho")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let token = token.as_str().unwrap_or_default();
                assert!(
                    !e.message.contains(token),
                    "fixture {id}: error message echoes caller input"
                );
            }
            // 訊息可以提到 `aip/1.x` 這種版本字串，但不得洩漏檔案路徑。
            for leak in ["/Users", "/private", "/home", ".json", ".rs", "\\", "://"] {
                assert!(
                    !e.message.contains(leak),
                    "fixture {id}: error message leaks a path-like fragment"
                );
            }
            assert!(
                e.message.chars().count() <= 200,
                "fixture {id}: error message exceeds 200 chars"
            );
            None
        }
        (other, _) => panic!("fixture {id} has an unknown expect value `{other}`"),
    }
}

/// `generated` 區段：超大訊息與壞 JSON 在測試內生成，不存大檔。
fn synthesize(entry: &Value) -> Vec<u8> {
    if let Some(raw) = entry.get("raw").and_then(Value::as_str) {
        return raw.as_bytes().to_vec();
    }
    let base = str_of(entry, "base");
    let text = std::fs::read_to_string(fixtures_dir().join(&base))
        .unwrap_or_else(|e| panic!("cannot read base fixture {base}: {e}"));
    let mut value: Value = serde_json::from_str(&text).expect("base fixture must be valid JSON");
    let chars = entry
        .get("inflatePayloadChars")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("generated entry needs `raw` or `inflatePayloadChars`"))
        as usize;
    value["payload"]["blob"] = Value::String("x".repeat(chars));
    serde_json::to_vec(&value).expect("synthesized fixture serializes")
}

#[test]
fn every_message_type_has_at_least_one_valid_fixture() {
    let m = manifest();
    assert_eq!(
        m["specVersion"], "aip/1.0",
        "the fixture index must name the spec version it was written for"
    );
    let mut covered = BTreeSet::new();
    for entry in section(&m, "envelopes") {
        if entry["expect"] != "ok" {
            continue;
        }
        let file = str_of(&entry, "file");
        let text = std::fs::read_to_string(fixtures_dir().join(&file))
            .unwrap_or_else(|e| panic!("missing fixture file {file}: {e}"));
        let value: Value = serde_json::from_str(&text).expect("fixture must be valid JSON");
        covered.insert(
            value["messageType"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
    for known in MessageType::KNOWN {
        assert!(
            covered.contains(known.as_str()),
            "no valid fixture covers messageType {}",
            known.as_str()
        );
    }
}

#[test]
fn envelope_fixtures_agree_with_the_index() {
    let m = manifest();
    let entries = section(&m, "envelopes");
    assert!(
        entries.len() >= 20,
        "the fixture index is suspiciously thin"
    );
    let mut ids = BTreeSet::new();
    for entry in entries {
        let id = str_of(&entry, "id");
        assert!(ids.insert(id.clone()), "duplicate fixture id {id}");
        let file = str_of(&entry, "file");
        let bytes = std::fs::read(fixtures_dir().join(&file))
            .unwrap_or_else(|e| panic!("missing fixture file {file}: {e}"));
        assert_expectation(&id, &entry, &bytes);
    }
}

#[test]
fn generated_fixtures_cover_the_oversized_and_malformed_cases() {
    let m = manifest();
    for entry in section(&m, "generated") {
        let id = str_of(&entry, "id");
        let bytes = synthesize(&entry);
        assert_expectation(&id, &entry, &bytes);
    }
}

#[test]
fn accepted_fixtures_round_trip_without_losing_unknown_fields() {
    let m = manifest();
    for entry in section(&m, "envelopes") {
        if entry["expect"] != "ok" {
            continue;
        }
        let id = str_of(&entry, "id");
        let file = str_of(&entry, "file");
        let bytes = std::fs::read(fixtures_dir().join(&file)).expect("fixture file");
        let parsed = Envelope::parse(&bytes).expect("fixture parses");
        let encoded = parsed.encode().expect("fixture re-encodes");
        let reparsed = Envelope::parse(&encoded).expect("re-encoded fixture parses");
        assert_eq!(parsed, reparsed, "fixture {id} is not round-trip stable");
        assert_eq!(
            encoded,
            reparsed.encode().expect("stable"),
            "fixture {id}: encode/parse is not symmetric"
        );
        // canonical JSON 必須穩定（同一則訊息 → 同一份文字）。
        let a: Value = serde_json::from_slice(&encoded).expect("json");
        let b: Value = serde_json::from_slice(&reparsed.encode().expect("stable")).expect("json");
        assert_eq!(canonical_json(&a), canonical_json(&b));
        if entry["roundTrip"] == Value::Bool(true) {
            let original: Value = serde_json::from_slice(&bytes).expect("json");
            assert!(
                !parsed.extra.is_empty(),
                "fixture {id} is flagged roundTrip but carries no unknown top-level field"
            );
            for (key, value) in &parsed.extra {
                assert!(
                    !KNOWN_TOP_LEVEL.contains(&key.as_str()),
                    "fixture {id}: a known field {key} leaked into `extra`"
                );
                assert_eq!(
                    &original[key], value,
                    "fixture {id}: unknown field {key} changed across round-trip"
                );
            }
        }
    }
}

const KNOWN_TOP_LEVEL: [&str; 14] = [
    "specVersion",
    "messageId",
    "messageType",
    "name",
    "source",
    "target",
    "sessionId",
    "occurredAt",
    "correlationId",
    "causationId",
    "sequence",
    "baseRevision",
    "expiresAt",
    "consentGrantId",
];

#[test]
fn negotiation_fixtures_are_deterministic() {
    let m = manifest();
    for entry in section(&m, "negotiations") {
        let id = str_of(&entry, "id");
        let offer_value = &entry["offer"];
        let offer = HostOffer {
            intents: serde_json::from_value(offer_value["intents"].clone()).expect("intents"),
            inputs: serde_json::from_value(offer_value["inputs"].clone()).expect("inputs"),
            sync_classes: serde_json::from_value(offer_value["syncClasses"].clone())
                .expect("syncClasses"),
        };
        let announcement: CapabilityAnnouncement =
            serde_json::from_value(entry["announcement"].clone()).expect("announcement");
        match str_of(&entry, "expect").as_str() {
            "ok" => {
                let got = negotiate_capabilities(&offer, &announcement)
                    .unwrap_or_else(|e| panic!("negotiation {id} failed: {:?}", e.code));
                let want: NegotiatedCapabilities =
                    serde_json::from_value(entry["negotiated"].clone()).expect("negotiated");
                assert_eq!(got, want, "negotiation {id} drifted from the index");
                // 兩次協商必須完全相同（確定性）。
                assert_eq!(got, negotiate_capabilities(&offer, &announcement).unwrap());
            }
            "error" => {
                let err = negotiate_capabilities(&offer, &announcement)
                    .expect_err("negotiation should fail");
                assert_eq!(err.code.as_str(), str_of(&entry, "code"));
            }
            other => panic!("negotiation {id} has an unknown expect `{other}`"),
        }
    }
}

#[test]
fn identity_decision_table() {
    let m = manifest();
    for entry in section(&m, "identity") {
        let id = str_of(&entry, "id");
        let bound: Party = serde_json::from_value(entry["bound"].clone()).expect("bound");
        let claimed: Party = serde_json::from_value(entry["claimed"].clone()).expect("claimed");
        let decision = bind_identity(&bound, &claimed);
        match str_of(&entry, "expect").as_str() {
            "accept" => assert_eq!(decision, IdentityDecision::Accept, "identity {id}"),
            "reject" => assert!(
                matches!(decision, IdentityDecision::Reject { .. }),
                "identity {id} must be rejected, never normalised"
            ),
            other => panic!("identity {id} has an unknown expect `{other}`"),
        }
    }
}

#[test]
fn offline_policy_table() {
    let m = manifest();
    for entry in section(&m, "offlinePolicy") {
        let name = str_of(&entry, "name");
        let has_grant = entry["hasConsentGrant"].as_bool().unwrap_or(false);
        let got = offline_policy(&name, has_grant);
        let want = str_of(&entry, "expect");
        let got_str = serde_json::to_value(got).expect("serialize");
        assert_eq!(got_str, Value::String(want), "offline policy for {name}");
    }
}

#[test]
fn outcome_migration_table() {
    let m = manifest();
    for entry in section(&m, "outcomeTransitions") {
        let from: Outcome =
            serde_json::from_value(entry["from"].clone()).expect("from is a known Outcome");
        let to: Outcome =
            serde_json::from_value(entry["to"].clone()).expect("to is a known Outcome");
        let want = entry["allowed"].as_bool().expect("allowed");
        assert_eq!(
            from.can_transition_to(to),
            want,
            "transition {} -> {} disagrees with the index",
            from.as_str(),
            to.as_str()
        );
    }
    for entry in section(&m, "outcomeProfiles") {
        let status: Outcome = serde_json::from_value(entry["status"].clone()).expect("status");
        let want = entry["allowed"].as_bool().expect("allowed");
        let got = match str_of(&entry, "profile").as_str() {
            "event" => status.allowed_for_event(),
            "command" => status.allowed_for_command(),
            "state" => status.allowed_for_state(),
            other => panic!("unknown outcome profile {other}"),
        };
        assert_eq!(got, want, "outcome {} in profile", status.as_str());
    }
    assert!(
        Outcome::Verified.is_runtime_only(),
        "verified must stay runtime-only"
    );
}

#[test]
fn name_scope_table() {
    let m = manifest();
    for entry in section(&m, "nameScope") {
        let name = str_of(&entry, "name");
        let want = entry["runtimeOnly"].as_bool().expect("runtimeOnly");
        assert_eq!(is_runtime_only_name(&name), want, "name scope for {name}");
    }
}

#[test]
fn error_codes_in_the_index_are_all_known() {
    let m = manifest();
    let known: BTreeSet<&str> = ErrorCode::KNOWN.iter().map(|c| c.as_str()).collect();
    for key in ["envelopes", "generated", "negotiations"] {
        for entry in section(&m, key) {
            if let Some(code) = entry.get("code").and_then(Value::as_str) {
                assert!(known.contains(code), "unknown ErrorCode `{code}` in {key}");
            }
        }
    }
}
