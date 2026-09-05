//! 三端共用的 **state hash fixtures**（AIP §6「`hash` = SHA-256(canonical JSON of `state`)」）。
//!
//! 為什麼需要它：桌面端到 v0.6.0 為止刻意不核對 hash——「JS 的 number 留不住 `0.0` 的字面」。
//! 這組 fixture 把 canonical 規則變成可執行的契約：每一份都是 **host 真的寫得出來的**
//! `SemanticState`（經 `CharacterSession` 或 `SemanticState` 反序列化→序列化 round trip），
//! 檔案裡的 `state` 字面就是 host 的 serde_json 輸出，`hash` 是 `state_hash(state)`，
//! `canonical` 是拿去做 SHA-256 的那一串文字（除錯用；也是「數字怎麼寫」的白紙黑字）。
//!
//! Rust／TypeScript／Swift 都讀 `crates/interaction-aip/tests/fixtures/manifest.json` 的
//! `stateHashes` 段（Swift 由 `scripts/aip-codegen.mjs` 內嵌）。三端對同一份 `state` 必須算出
//! 同一個 `hash`；算不出來就是那一端的 canonical 實作有漏洞，不是 fixture 的問題。
//!
//! 重生：`AIP_UPDATE_FIXTURES=1 cargo test -p interaction-session --test state_hash_fixtures`
//! 之後在 `apps/interaction-desktop` 跑 `pnpm aip:codegen`（Swift 內嵌）。

use chrono::{Duration, TimeZone, Utc};
use interaction_aip::{
    canonical_json, CapabilityAnnouncement, Envelope, MemberRole, MessageType, Party, SyncClass,
    Timestamp,
};
use interaction_session::{
    state_hash, CharacterSession, RuntimeFact, SemanticState, SessionConfig, EVENT_DISMISS,
    EVENT_TOUCH,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const SESSION: &str = "session.home";
const PREFIX: &str = "state-hash-";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("interaction-aip")
        .join("tests")
        .join("fixtures")
}

fn t0() -> Timestamp {
    Utc.with_ymd_and_hms(2026, 9, 5, 9, 0, 0)
        .single()
        .expect("fixed timestamp")
}

fn at(ms: i64) -> Timestamp {
    t0() + Duration::milliseconds(ms)
}

fn config() -> SessionConfig {
    SessionConfig {
        session_id: SESSION.to_string(),
        character_id: "ref-shape".to_string(),
        ..SessionConfig::default()
    }
}

fn announcement(role: MemberRole, intents: &[&str], inputs: &[&str]) -> CapabilityAnnouncement {
    CapabilityAnnouncement {
        spec_versions: vec!["aip/1.0".to_string()],
        role: Some(role),
        profiles: vec!["character-session".to_string()],
        sync_classes: vec![SyncClass::Semantic],
        intents: intents.iter().map(|s| s.to_string()).collect(),
        inputs: inputs.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn full_device() -> CapabilityAnnouncement {
    announcement(
        MemberRole::RemoteRenderer,
        &["react-happily-to-touch", "celebrate", "settle", "idle"],
        &[EVENT_TOUCH, EVENT_DISMISS],
    )
}

fn desktop() -> CapabilityAnnouncement {
    announcement(
        MemberRole::HostRenderer,
        &["react-happily-to-touch", "celebrate", "settle", "idle"],
        &[EVENT_TOUCH, EVENT_DISMISS],
    )
}

fn touch(id: &str, source: &Party, intensity: f64, now: Timestamp) -> Envelope {
    Envelope::new(MessageType::Event, EVENT_TOUCH, source.clone(), id, now)
        .with_session(SESSION)
        .with_expiry(now + Duration::seconds(5))
        .with_payload(json!({"kind": "tap", "intensity": intensity}))
}

/// 一份 fixture：`state` 一定是 host 寫得出來的 canonical 形狀（經 `SemanticState` round trip）。
struct Fixture {
    id: &'static str,
    note: &'static str,
    /// host 會不會把這份 state 當成合法權威狀態（`SemanticState` 反序列化＋不變量）。
    /// `false` 的 fixture 只驗 hash 函式本身（例如 `-0.0`：hash 算得出來，但 host 拒絕它）。
    semantic_valid: bool,
    /// 檔案裡 `state` 的**原始文字**：`None` = 由 `state` 值的 serde_json 輸出決定；
    /// `Some` = 故意寫成非排序／帶空白的文字（測試消費端會不會自己排序）。
    raw_state_text: Option<String>,
    state: Value,
}

fn round_trip(value: Value) -> Value {
    let state: SemanticState = serde_json::from_value(value).expect("fixture state deserializes");
    serde_json::to_value(&state).expect("state serializes")
}

fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();

    // 1. 全新 session：intensity 0.0（serde_json 寫 `0.0`，JS 一般會寫 `0`——這正是桌面端
    //    過去不核對 hash 的理由）。
    let session = CharacterSession::new(config(), 1, t0());
    let fresh = serde_json::to_value(session.state()).expect("state serializes");
    out.push(Fixture {
        id: "fresh",
        note: "全新 session：mood.intensity 0.0 必須寫成 `0.0`（serde_json f64），不是 `0`",
        semantic_valid: true,
        raw_state_text: None,
        state: fresh.clone(),
    });

    // 2. 同一份狀態，但檔案裡的鍵故意亂序＋帶空白：消費端必須自己做鍵排序與去空白。
    let unsorted = {
        let Value::Object(map) = &fresh else {
            unreachable!()
        };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        keys.reverse();
        let mut text = String::from("{\n");
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                text.push_str(",\n");
            }
            text.push_str("    ");
            text.push_str(&serde_json::to_string(key).expect("key"));
            text.push_str(" : ");
            text.push_str(&serde_json::to_string_pretty(&map[*key]).expect("value"));
        }
        text.push_str("\n  }");
        text
    };
    out.push(Fixture {
        id: "unsorted-input",
        note: "與 fresh 同一份 state，但輸入鍵反序、帶空白與換行：hash 必須與 fresh 相同",
        semantic_valid: true,
        raw_state_text: Some(unsorted),
        state: fresh.clone(),
    });

    // 3. 成員與協商結果：桌面（全支援 → `unsupportedIntents: []`）＋只會 idle 的手機
    //    （三個 unsupported，排序穩定）。這是 v0.6.0 已知限制 21 提到、當時 fixtures 沒涵蓋的欄位。
    let mut session = CharacterSession::new(config(), 1, t0());
    session
        .join(Party::human_surface("desktop"), &desktop(), t0())
        .expect("desktop joins");
    session
        .join(
            Party::device("iphone-87b42264"),
            &announcement(MemberRole::RemoteRenderer, &["idle"], &[EVENT_TOUCH]),
            at(10),
        )
        .expect("device joins");
    out.push(Fixture {
        id: "members-unsupported-intents",
        note: "members[].unsupportedIntents：全支援＝空陣列（不是缺鍵）；部分支援＝排序後的清單",
        semantic_valid: true,
        raw_state_text: None,
        state: serde_json::to_value(session.state()).expect("state serializes"),
    });

    // 4. 互動之後：mood 變、attention 指向成員、lastInteraction 出現（`"<kind>:<id>"` 字串形）。
    let mut session = CharacterSession::new(config(), 1, t0());
    let phone = Party::device("iphone-87b42264");
    session
        .join(phone.clone(), &full_device(), t0())
        .expect("device joins");
    let _ = session.submit(touch("msg_touch_1", &phone, 0.4, at(100)), &phone, at(100));
    out.push(Fixture {
        id: "after-touch",
        note: "touch 之後：mood.intensity 是 0..1 的小數、attention.kind=member、lastInteraction 存在",
        semantic_valid: true,
        raw_state_text: None,
        state: serde_json::to_value(session.state()).expect("state serializes"),
    });

    // 5. 真相與 reducedMotion：truth.correlationId、attention.kind=task、reducedMotion=true。
    let mut session = CharacterSession::new(config(), 1, t0());
    session
        .join(Party::human_surface("desktop"), &desktop(), t0())
        .expect("desktop joins");
    let _ = session.submit_runtime(RuntimeFact::ReducedMotion(true), None, at(5));
    let _ = session.submit_runtime(
        RuntimeFact::TaskState {
            truth: interaction_character::TruthState::Claimed,
            correlation_id: Some("task_42".to_string()),
        },
        Some("task_42".to_string()),
        at(20),
    );
    out.push(Fixture {
        id: "task-truth-reduced-motion",
        note: "truth 帶 correlationId、attention.kind=task、reducedMotion=true（布林與可選鍵）",
        semantic_valid: true,
        raw_state_text: None,
        state: serde_json::to_value(session.state()).expect("state serializes"),
    });

    // 6. intensity 剛好 1.0：整數值的 f64 必須寫成 `1.0`（JS 的 `1` 會讓 hash 對不上）。
    let mut one = fresh.clone();
    one["mood"] = json!({"kind": "happy", "intensity": 1.0});
    out.push(Fixture {
        id: "intensity-one",
        note: "mood.intensity 1.0：整數值的 f64 仍寫成 `1.0`",
        semantic_valid: true,
        raw_state_text: None,
        state: round_trip(one),
    });

    // 7. 三位小數：0.123（`clamp_unit` 的精度上限）。
    let mut frac = fresh.clone();
    frac["mood"] = json!({"kind": "playful", "intensity": 0.123});
    out.push(Fixture {
        id: "intensity-three-decimals",
        note: "mood.intensity 0.123：最短 round-trip 十進位，沒有指數記法",
        semantic_valid: true,
        raw_state_text: None,
        state: round_trip(frac),
    });

    // 8. 負零：hash 函式本身要有確定答案（serde_json 寫 `-0.0`），但 host **不得**產生它、
    //    restore 也必須拒絕它（`-0.0` 與 `0.0` 語意相同、字面不同，是 hash 分歧的溫床）。
    let mut negative_zero = fresh.clone();
    negative_zero["mood"] = json!({"kind": "neutral", "intensity": -0.0});
    out.push(Fixture {
        id: "intensity-negative-zero",
        note: "-0.0：canonical 文字是 `-0.0`；host 永不產生它，restore 拒絕（semanticValid=false）",
        semantic_valid: false,
        raw_state_text: None,
        state: negative_zero,
    });

    // 9. 非 ASCII 與需要跳脫的字串：serde_json 對非 ASCII 原樣輸出、`/` 不跳脫、控制字元
    //    寫 `\u00XX`、`"`／`\` 跳脫；`\n`／`\t` 用短寫。
    let mut unicode = fresh.clone();
    unicode["characterId"] = json!("角色／測試 \"quoted\" back\\slash\ttab\nnewline \u{1}ctrl 🎈");
    out.push(Fixture {
        id: "unicode-and-escapes",
        note: "非 ASCII 原樣（不 \\u 跳脫）、`/` 不跳脫、控制字元 \\u0001、emoji 原樣",
        semantic_valid: true,
        raw_state_text: None,
        state: round_trip(unicode),
    });

    out
}

fn file_name(id: &str) -> String {
    format!("{PREFIX}{id}.json")
}

/// fixture 檔的原始文字：`state` 段用 host 的 serde_json 輸出（或指定的 raw 文字）逐字寫入。
fn render(fixture: &Fixture) -> String {
    let state_text = match &fixture.raw_state_text {
        Some(raw) => raw.clone(),
        None => serde_json::to_string(&fixture.state).expect("state serializes"),
    };
    let canonical = canonical_json(&fixture.state);
    let hash = state_hash(&fixture.state);
    format!(
        "{{\n  \"id\": {id},\n  \"note\": {note},\n  \"semanticValid\": {valid},\n  \"hash\": \"{hash}\",\n  \"canonical\": {canonical},\n  \"state\": {state}\n}}\n",
        id = serde_json::to_string(fixture.id).expect("id"),
        note = serde_json::to_string(fixture.note).expect("note"),
        valid = fixture.semantic_valid,
        canonical = serde_json::to_string(&canonical).expect("canonical text"),
        state = state_text,
    )
}

fn manifest_entries(list: &[Fixture]) -> Vec<Value> {
    list.iter()
        .map(|f| {
            json!({
                "id": f.id,
                "file": file_name(f.id),
                "semanticValid": f.semantic_valid,
                "note": f.note,
            })
        })
        .collect()
}

/// manifest.json 是手寫、有固定排版的索引檔（其他段落的鍵序與縮排都有意義），所以
/// 不能整份 `to_string_pretty` 重寫；只以文字拼接置換／插入 `stateHashes` 段。
fn manifest_section_text(list: &[Fixture]) -> String {
    let mut out = String::from("\"stateHashes\": [\n");
    for (i, f) in list.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!(
            "    {{ \"id\": {id}, \"file\": {file}, \"semanticValid\": {valid}, \"note\": {note} }}",
            id = serde_json::to_string(f.id).expect("id"),
            file = serde_json::to_string(&file_name(f.id)).expect("file"),
            valid = f.semantic_valid,
            note = serde_json::to_string(f.note).expect("note"),
        ));
    }
    out.push_str("\n  ]");
    out
}

fn splice_manifest(text: &str, section: &str) -> String {
    const KEY: &str = "\"stateHashes\": [";
    if let Some(start) = text.find(KEY) {
        let close = text[start..]
            .find("\n  ]")
            .map(|i| start + i + "\n  ]".len())
            .expect("stateHashes section closes with a two-space-indented `]`");
        format!("{}{}{}", &text[..start], section, &text[close..])
    } else {
        let end = text
            .trim_end()
            .strip_suffix('}')
            .expect("manifest ends with `}`")
            .trim_end()
            .len();
        format!("{},\n  {}\n}}\n", &text[..end], section)
    }
}

fn update_requested() -> bool {
    std::env::var("AIP_UPDATE_FIXTURES").is_ok_and(|v| v == "1")
}

#[test]
fn state_hash_fixtures_are_what_the_host_writes() {
    let list = fixtures();
    let dir = fixtures_dir();
    let manifest_path = dir.join("manifest.json");
    let mut manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("manifest.json readable"),
    )
    .expect("manifest.json is JSON");

    if update_requested() {
        for fixture in &list {
            std::fs::write(dir.join(file_name(fixture.id)), render(fixture))
                .expect("fixture written");
        }
        let text = std::fs::read_to_string(&manifest_path).expect("manifest.json readable");
        let spliced = splice_manifest(&text, &manifest_section_text(&list));
        std::fs::write(&manifest_path, &spliced).expect("manifest written");
        manifest = serde_json::from_str(&spliced).expect("spliced manifest is still JSON");
    }

    // (a) 索引與磁碟上的檔案一致。
    let entries = manifest
        .get("stateHashes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!("manifest.json 缺 `stateHashes` 段：用 AIP_UPDATE_FIXTURES=1 重生")
        });
    assert_eq!(
        entries,
        manifest_entries(&list),
        "manifest.json 的 stateHashes 段與產生器不一致：AIP_UPDATE_FIXTURES=1 重生"
    );

    for fixture in &list {
        let path = dir.join(file_name(fixture.id));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{} 讀不到（{e}）：AIP_UPDATE_FIXTURES=1 重生",
                path.display()
            )
        });
        // (b) 檔案內容就是產生器現在會寫的內容（state 字面逐字相同）。
        assert_eq!(
            text,
            render(fixture),
            "{} 與產生器輸出不同：AIP_UPDATE_FIXTURES=1 重生",
            path.display()
        );
        let doc: Value = serde_json::from_str(&text).expect("fixture is JSON");
        // (c) hash 就是 `state_hash(state)`；canonical 就是拿去 hash 的文字。
        let state = doc.get("state").cloned().expect("state");
        assert_eq!(state_hash(&state), doc["hash"].as_str().expect("hash"));
        assert_eq!(
            canonical_json(&state),
            doc["canonical"].as_str().expect("canonical")
        );
        // (d) host 寫得出來：反序列化成 SemanticState 再序列化，canonical 文字逐字相同。
        //     （`unsorted-input` 的檔案文字亂序，但 Value 一樣；這一步證明的是「數字字面」。）
        if fixture.semantic_valid {
            let parsed: SemanticState =
                serde_json::from_value(state.clone()).expect("host accepts the state shape");
            let written = serde_json::to_value(&parsed).expect("state serializes");
            assert_eq!(
                canonical_json(&written),
                canonical_json(&state),
                "{}：host 重新序列化後的 canonical 文字必須與 fixture 相同",
                fixture.id
            );
        }
    }

    // (e) `unsorted-input` 與 `fresh` 是同一份狀態：hash 必須相同（消費端必須自己排序）。
    let hash_of = |id: &str| {
        let doc: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(file_name(id))).expect("fixture readable"),
        )
        .expect("fixture JSON");
        doc["hash"].as_str().expect("hash").to_string()
    };
    assert_eq!(hash_of("fresh"), hash_of("unsorted-input"));
}

/// `-0.0` 不是 host 會產生的值：`Mood::new` 正規化、restore 拒絕。這裡只固定「hash 函式對
/// `-0.0` 的答案是 `-0.0` 的 canonical」，讓三端在遇到它時至少不會各算各的。
#[test]
fn negative_zero_intensity_has_a_deterministic_canonical_text() {
    let state = json!({"mood": {"kind": "neutral", "intensity": -0.0}});
    assert!(canonical_json(&state).contains("\"intensity\":-0.0"));
    let mut map = Map::new();
    map.insert("intensity".into(), json!(0.0));
    assert!(canonical_json(&Value::Object(map)).contains("\"intensity\":0.0"));
}
