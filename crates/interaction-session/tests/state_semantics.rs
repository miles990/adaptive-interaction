//! `SemanticState` 的**字面語意**：同一個語意值只能有一種 canonical 文字。
//!
//! 為什麼獨立成一支測試：state 的 hash 是對 serde_json **寫出來的文字**取的
//! （AIP §6），所以「值相同、字面不同」就等於「hash 不同」。`-0.0` 是這條路上唯一
//! 會自然出現的分歧（`f64` 的兩個零），而 `(-0.0).clamp(0.0, 1.0)` 與
//! `(-0.0 * 1000.0).round() / 1000.0` 都會原樣回 `-0.0`——不正規化就會被 host 廣播出去。
//!
//! 這裡驗三件事：
//! 1. host **產生**的路徑（`Mood::new`／`SemanticState::new`）永遠寫 `0.0`，不寫 `-0.0`、不寫 `0`。
//! 2. host **接收**不可信 state 的路徑（`CharacterSession::restore`）拒絕 sign-negative 的零。
//! 3. `state-snapshot.json` conformance fixture 的 `state` 就是 host 真的寫得出來的形狀
//!    （三端共用的那份 snapshot：Swift 拿它比對 canonical hash）。

use chrono::{TimeZone, Utc};
use interaction_aip::{canonical_hash, canonical_json, Timestamp};
use interaction_session::{
    state_hash, CharacterSession, Mood, MoodKind, SemanticState, SessionConfig, SessionError,
    Snapshot,
};
use serde_json::Value;
use std::path::PathBuf;

const SESSION: &str = "session.home";

fn t0() -> Timestamp {
    Utc.with_ymd_and_hms(2026, 9, 5, 9, 0, 0)
        .single()
        .expect("fixed timestamp")
}

fn config() -> SessionConfig {
    SessionConfig {
        session_id: SESSION.to_string(),
        character_id: "ref-shape".to_string(),
        ..SessionConfig::default()
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("interaction-aip")
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// `Mood::new` 是 host 唯一寫 `intensity` 的入口：`-0.0` 進去，`0.0` 出來。
#[test]
fn mood_normalizes_negative_zero_to_positive_zero() {
    let mood = Mood::new(MoodKind::Neutral, -0.0);
    assert!(
        !mood.intensity.is_sign_negative(),
        "intensity 不得是 sign-negative 的零：{:?}",
        mood.intensity
    );
    let text = serde_json::to_string(&mood).expect("mood serializes");
    assert!(
        text.contains("\"intensity\":0.0"),
        "canonical 字面必須是 `0.0`，實際是 {text}"
    );

    // 夾取之後才變成零的輸入（負數、負無限）同樣不得留下負號。
    for input in [-0.4_f64, -1.0, f64::NEG_INFINITY, -0.0004] {
        let mood = Mood::new(MoodKind::Neutral, input);
        assert!(
            !mood.intensity.is_sign_negative(),
            "{input} 夾取後仍是 sign-negative 的零"
        );
    }
}

/// 全新狀態的字面回歸：JS 會寫 `0`、`-0.0` 會寫 `-0.0`，host 只寫 `0.0`。
#[test]
fn fresh_state_writes_intensity_as_zero_point_zero() {
    let text = serde_json::to_string(&SemanticState::new("ref-shape")).expect("state serializes");
    assert!(
        text.contains("\"intensity\":0.0"),
        "全新狀態的 canonical 字面必須含 `\"intensity\":0.0`，實際是 {text}"
    );
    assert!(!text.contains("\"intensity\":-0.0"));
    assert!(!text.contains("\"intensity\":0,"));
}

/// 不可信來源（持久化 snapshot）帶 `-0.0`：`restore` 必須拒絕，不「幫忙修正」。
///
/// 修正會讓還原後的狀態與 snapshot 的 hash 不符；沿用則會把一個 host 自己寫不出來的
/// 字面當成權威狀態廣播出去，成員各自算出不同的 hash。兩者都不誠實，所以拒絕。
#[test]
fn restore_rejects_a_sign_negative_zero_intensity() {
    // 從 host 真的寫得出來的那份 snapshot 出發，只把 `intensity` 的字面換成 `-0.0`
    // （並重算 hash，讓它自洽）——模擬「檔案被改過、但改得很像真的」。
    let mut snapshot = tampered_snapshot(serde_json::json!({"kind": "neutral", "intensity": -0.0}));
    assert!(
        canonical_json(&snapshot.state).contains("\"intensity\":-0.0"),
        "前提：serde_json 對 -0.0 寫的就是 `-0.0`"
    );
    snapshot.hash = state_hash(&snapshot.state);
    assert_eq!(
        CharacterSession::restore(config(), &snapshot, t0()).err(),
        Some(SessionError::InvalidState),
        "sign-negative 的零必須被當成不合法狀態拒絕"
    );
}

/// 把一份真實 snapshot 的 `mood` 換掉（hash 由呼叫端重算）。
fn tampered_snapshot(mood: Value) -> Snapshot {
    let mut snapshot = CharacterSession::new(config(), 1, t0()).snapshot();
    snapshot.state["mood"] = mood;
    snapshot
}

/// 同一份 snapshot 換成 `0.0` 就還原得回來（證明上面拒的是字面，不是整段路徑壞掉）。
#[test]
fn restore_accepts_the_same_snapshot_with_positive_zero() {
    let mut snapshot = tampered_snapshot(serde_json::json!({"kind": "neutral", "intensity": 0.0}));
    snapshot.hash = state_hash(&snapshot.state);
    let session = CharacterSession::restore(config(), &snapshot, t0()).expect("restore succeeds");
    assert_eq!(session.state().mood().intensity, 0.0);
}

/// `state-snapshot.json`（三端共用的權威 snapshot fixture）的 `state` 必須是 host 真的
/// 寫得出來的形狀：`intensity` 是 `0.0`、`members[]` 帶 `unsupportedIntents`、
/// 值為「無」的選填鍵省略而不是 `null`，而且 `payload.hash` 就是它的 canonical SHA-256。
#[test]
fn state_snapshot_fixture_is_what_the_host_writes() {
    let text = std::fs::read_to_string(fixture_path("state-snapshot.json"))
        .expect("state-snapshot.json readable");
    let doc: Value = serde_json::from_str(&text).expect("fixture is JSON");
    let state = doc["payload"]["state"].clone();
    let hash = doc["payload"]["hash"].as_str().expect("payload.hash");

    let parsed: SemanticState =
        serde_json::from_value(state.clone()).expect("host accepts the fixture state shape");
    let written = serde_json::to_value(&parsed).expect("state serializes");
    assert_eq!(
        canonical_json(&written),
        canonical_json(&state),
        "fixture 的 state 不是 host 重新序列化後的字面"
    );
    assert_eq!(canonical_hash(&state), hash, "payload.hash 與 state 不一致");
    assert!(
        canonical_json(&state).contains("\"intensity\":0.0"),
        "fixture 的 intensity 必須是 f64 字面 `0.0`"
    );
    assert!(
        canonical_json(&state).contains("\"unsupportedIntents\":"),
        "fixture 的 members[] 必須帶 unsupportedIntents（host 一定會寫）"
    );
}

/// `state-snapshot.json` → `state-patch.json` 這一對 fixture 必須串得起來：patch 套用在
/// snapshot 的 `state` 上，結果的 canonical hash 就是 patch fixture 記載的 `payload.hash`。
///
/// 這一對是三端共用的「snapshot 之後接 patch」證據（Swift 的
/// `testPatchWithMatchingBaseRevisionIsAppliedAndHashChainsFromTheSnapshot` 讀同一對）。
/// 改了其中一份卻沒改另一份，這裡就紅燈——而不是等到 iOS 模擬器跑起來才發現。
#[test]
fn state_patch_fixture_chains_from_the_snapshot_fixture() {
    let snapshot: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture_path("state-snapshot.json")).expect("snapshot readable"),
    )
    .expect("snapshot is JSON");
    let patch: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture_path("state-patch.json")).expect("patch readable"),
    )
    .expect("patch is JSON");
    assert_eq!(
        patch["baseRevision"], snapshot["payload"]["revision"],
        "patch 的 baseRevision 必須接在 snapshot 的 revision 之後"
    );

    let merged =
        interaction_session::apply_patch(&snapshot["payload"]["state"], &patch["payload"]["patch"]);
    assert_eq!(
        state_hash(&merged),
        patch["payload"]["hash"]
            .as_str()
            .expect("patch payload.hash"),
        "套用 patch 之後的 hash 與 state-patch.json 記載的不同"
    );
    // 合併結果仍然是 host 寫得出來的權威狀態（不是任意 JSON）。
    let parsed: SemanticState =
        serde_json::from_value(merged.clone()).expect("merged state is a valid SemanticState");
    assert_eq!(
        canonical_json(&serde_json::to_value(&parsed).expect("state serializes")),
        canonical_json(&merged)
    );
}

/// `SemanticState` 的序列化**永遠不含 `null`**：值為「無」的選填鍵一律省略。
///
/// `state.rs` 開頭的實作註記已經寫下這條規則（RFC 7396 的 `null` 是**刪除鍵**：host 寫 `null`、
/// 接收端刪鍵，兩邊的 canonical hash 就對不上），但在 v0.6.x 之前沒有任何測試擋得住它——
/// 拿掉一個欄位的 `skip_serializing_if` 之後，fixtures 會一起紅，而 `AIP_UPDATE_FIXTURES=1`
/// 會把它們全部重生成含 `null` 的樣子，重生之後又全綠（`docs/releases/v0.7.0-drills.md` §7 F5）。
#[test]
fn semantic_state_never_serializes_a_null() {
    // (a) 選填鍵全部是「無」的狀態：lastInteraction 與 truth.correlationId 都不該出現。
    let fresh = serde_json::to_value(SemanticState::new("ref-shape")).expect("state serializes");
    assert_eq!(first_null(&fresh, ""), None, "全新狀態不得寫出 null");
    assert!(fresh.get("lastInteraction").is_none());
    assert!(fresh["truth"].get("correlationId").is_none());

    // (b) 選填鍵全部有值的狀態（host 真的寫得出來的形狀，經 SemanticState round trip）。
    let full: SemanticState = serde_json::from_value(serde_json::json!({
        "characterId": "ref-shape",
        "mood": {"kind": "happy", "intensity": 0.4},
        "activity": "reacting",
        "attention": {"kind": "member", "id": "device:iphone-87b42264"},
        "truth": {"state": "claimed", "correlationId": "task_42"},
        "lastInteraction": {
            "name": "character.interaction.touch",
            "kind": "tap",
            "source": "device:iphone-87b42264",
            "at": "2026-09-05T09:00:00.100Z"
        },
        "members": [{
            "party": {"kind": "device", "id": "iphone-87b42264"},
            "role": "remote-renderer",
            "presence": "online",
            "lastSeenAt": "2026-09-05T09:00:00Z",
            "unsupportedIntents": []
        }],
        "reducedMotion": true
    }))
    .expect("host accepts the state shape");
    let written = serde_json::to_value(&full).expect("state serializes");
    assert_eq!(first_null(&written, ""), None, "有值的狀態也不得寫出 null");
    assert!(!canonical_json(&written).contains("null"));

    // (c) `null` 進來（RFC 7396 的刪除鍵）也不會被原樣寫回去：省略就是省略。
    let from_null: SemanticState = serde_json::from_value(serde_json::json!({
        "characterId": "ref-shape",
        "mood": {"kind": "neutral", "intensity": 0.0},
        "activity": "idle",
        "attention": {"kind": "none"},
        "truth": {"state": "none", "correlationId": null},
        "lastInteraction": null,
        "members": [],
        "reducedMotion": false
    }))
    .expect("null 的選填鍵反序列化成 None");
    let written = serde_json::to_value(&from_null).expect("state serializes");
    assert_eq!(first_null(&written, ""), None);
    assert_eq!(canonical_json(&written), canonical_json(&fresh));

    // (d) 反例：同一個斷言套在「寫成 null」的變體上必須抓得到——不然這條只是裝飾。
    let with_nulls = serde_json::json!({
        "characterId": "ref-shape",
        "lastInteraction": null,
        "truth": {"state": "none", "correlationId": null}
    });
    assert_eq!(
        first_null(&with_nulls, ""),
        Some("/lastInteraction".to_string())
    );
    assert_eq!(
        first_null(&serde_json::json!({"members": [{"party": null}]}), ""),
        Some("/members/0/party".to_string())
    );
}

/// 遞迴找出第一個 `null` 的 JSON pointer（沒有就回 `None`）。
fn first_null(value: &Value, pointer: &str) -> Option<String> {
    match value {
        Value::Null => Some(pointer.to_string()),
        Value::Object(map) => map
            .iter()
            .find_map(|(k, v)| first_null(v, &format!("{pointer}/{k}"))),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, v)| first_null(v, &format!("{pointer}/{i}"))),
        _ => None,
    }
}
