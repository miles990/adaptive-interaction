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
    bind_identity, canonical_hash, canonical_json, is_runtime_only_name, negotiate_capabilities,
    offline_policy, AipError, CapabilityAnnouncement, Envelope, ErrorCode, HostOffer,
    IdentityDecision, MessageType, NegotiatedCapabilities, Outcome, Party,
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

/// `stateHashes`：三端共用的 state-hash fixture。這裡是 **AIP 層的消費者**——只用
/// `canonical_json`／`canonical_hash` 把每一份 `state` 重算一次，與 `interaction-session`
/// 那支產生器（`tests/state_hash_fixtures.rs`）互相獨立。
///
/// 為什麼要兩支：產生器寫檔時用的是同一組函式，「自己驗自己」證明不了 canonical 規則本身
/// 是穩定的。這支不知道 fixture 怎麼來的，只讀檔案裡的 `state` 與 `hash`／`canonical` 對答案；
/// canonical 規則一改（鍵序、跳脫、數字字面），這裡立刻紅燈。
#[test]
fn state_hash_fixtures_agree_with_canonical_json_and_hash() {
    let m = manifest();
    let entries = section(&m, "stateHashes");
    assert!(
        entries.len() >= 9,
        "stateHashes 至少要涵蓋 9 個情境（含 -0.0、unicode、亂序輸入），實際 {}",
        entries.len()
    );
    let mut hashes: Vec<(String, String)> = Vec::new();
    for entry in &entries {
        let id = str_of(entry, "id");
        let file = str_of(entry, "file");
        let path = fixtures_dir().join(&file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read the state fixture {file}: {e}"));
        let doc: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("state fixture {file} is not JSON: {e}"));
        assert_eq!(doc["id"].as_str(), Some(id.as_str()), "{file}: id mismatch");
        let state = doc
            .get("state")
            .unwrap_or_else(|| panic!("{file}: no state"));
        let want_hash = doc["hash"].as_str().expect("hash");
        let want_canonical = doc["canonical"].as_str().expect("canonical");
        assert_eq!(
            canonical_json(state),
            want_canonical,
            "{file}: canonical JSON 與檔案記載的不同"
        );
        assert_eq!(
            canonical_hash(state),
            want_hash,
            "{file}: SHA-256 與檔案記載的不同"
        );
        // canonical 文字自己就足以決定 hash（消費端不必先解析成值）。
        assert_eq!(
            hex_sha256(want_canonical.as_bytes()),
            want_hash,
            "{file}: hash 不是 canonical 文字的 SHA-256"
        );
        hashes.push((id, want_hash.to_string()));
    }
    // 「同一份 state、不同的輸入排版」必須得到同一個 hash；其餘情境彼此不得相同。
    let find = |id: &str| -> String {
        hashes
            .iter()
            .find(|(k, _)| k == id)
            .unwrap_or_else(|| panic!("stateHashes 缺少 `{id}`"))
            .1
            .clone()
    };
    assert_eq!(find("fresh"), find("unsorted-input"));
    let distinct: BTreeSet<&String> = hashes.iter().map(|(_, h)| h).collect();
    assert_eq!(
        distinct.len(),
        hashes.len() - 1,
        "除了 fresh／unsorted-input 這一對，每個情境的 hash 都必須不同"
    );
}

/// `receiveDecisions`：接收端決策表的**形狀**檢查。
///
/// 決策本身屬於 `interaction-session`（AIP 層沒有 session 狀態的概念，也不能反向依賴它），
/// 所以這裡只驗「這一段長得對不對」：每一筆都要有 id／note／local／expect 與一種輸入，
/// 決策與 realign 原因都得是已知的穩定字串，id 不得重複。TypeScript 與 Swift 讀的是同一段，
/// 拼錯一個決策名在 Rust 這邊就要先擋下來，而不是等到另一個語言默默走進 default 分支。
#[test]
fn receive_decision_fixtures_have_the_documented_shape() {
    let m = manifest();
    let entries = section(&m, "receiveDecisions");
    assert!(
        entries.len() >= 32,
        "receiveDecisions 至少要 32 個具名案例，實際 {}",
        entries.len()
    );
    const DECISIONS: [&str; 9] = [
        "ignore-stale-connection",
        "reject-identity",
        "reject-invalid",
        "reset",
        "apply",
        "realign",
        "recover",
        "ignore-stale",
        "already-applied",
    ];
    const REASONS: [&str; 5] = [
        "no-local",
        "epoch-changed",
        "base-mismatch",
        "hash-mismatch",
        "resume-too-long",
    ];
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in &entries {
        let id = str_of(entry, "id");
        assert!(!str_of(entry, "note").is_empty(), "{id}: note 不得為空");
        assert!(ids.insert(id.clone()), "重複的案例 id：{id}");
        let local = entry
            .get("local")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{id}: 缺 local"));
        for key in ["hasState", "epoch", "revision", "connectionGeneration"] {
            assert!(local.contains_key(key), "{id}: local 缺 `{key}`");
        }
        let inputs = ["incoming", "incomingBatch", "incomingBatchChain"]
            .iter()
            .filter(|key| entry.get(**key).is_some())
            .count();
        assert_eq!(inputs, 1, "{id}: 必須剛好有一種輸入");
        if let Some(incoming) = entry.get("incoming") {
            for key in [
                "kind",
                "epoch",
                "revision",
                "statePresent",
                "arrivedOnGeneration",
                "viaAuthoritativeReply",
            ] {
                assert!(incoming.get(key).is_some(), "{id}: incoming 缺 `{key}`");
            }
            let kind = str_of(incoming, "kind");
            assert!(
                kind == "snapshot" || kind == "patch",
                "{id}: 未知的 state kind `{kind}`"
            );
        }
        let expect = entry
            .get("expect")
            .unwrap_or_else(|| panic!("{id}: 缺 expect"));
        let decision = str_of(expect, "decision");
        assert!(
            DECISIONS.contains(&decision.as_str()),
            "{id}: 未知的決策 `{decision}`"
        );
        seen.insert(decision.clone());
        match expect.get("reason").and_then(Value::as_str) {
            Some(reason) => {
                assert_eq!(decision, "realign", "{id}: 只有 realign 帶 reason");
                assert!(
                    REASONS.contains(&reason),
                    "{id}: 未知的 realign 原因 `{reason}`"
                );
            }
            None => assert_ne!(decision, "realign", "{id}: realign 必須說出原因"),
        }
        for key in ["revisionAfter", "epochAfter", "budgetAfter"] {
            assert!(
                expect.get(key).and_then(Value::as_u64).is_some(),
                "{id}: expect 缺 `{key}`"
            );
        }
        if let Some(session_after) = expect.get("sessionIdAfter") {
            assert!(
                session_after.as_str().is_some_and(|s| !s.is_empty()),
                "{id}: `sessionIdAfter` 有寫就必須是非空字串"
            );
        }
        let budget = str_of(expect, "budget");
        assert!(
            budget == "ok" || budget == "unrecoverable",
            "{id}: 未知的 realign 預算結論 `{budget}`"
        );
    }
    for decision in DECISIONS {
        assert!(
            seen.contains(decision),
            "沒有任何案例得到 `{decision}`：決策表有一條分支沒人測"
        );
    }
}

/// 直接對位元組取 SHA-256（十六進位小寫），用來確認 `canonical_hash` 沒有偷加料。
fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
