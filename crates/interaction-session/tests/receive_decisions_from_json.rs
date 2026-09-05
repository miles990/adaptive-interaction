//! `receiveDecisions` 段的**第二個消費者**：只讀 JSON，不認識產生器。
//!
//! 產生器（`receive_decision_fixtures.rs`）證明的是「磁碟上的內容 ＝ 我現在會寫的內容」；
//! 一個只讀 JSON 的消費者證明的是另一件事——**別的語言照著這份檔案做，會得到同一個結論**。
//! 兩者都缺一不可：只有產生器的話，規則和期望一起改掉也永遠是綠的。
//!
//! TypeScript（桌面）與 Swift（iPhone）的接收端在 Wave 2 也會讀同一段，
//! 逐筆對同一個 `expect` 交答案（`docs/aip/conformance.md` §3）。

use std::path::PathBuf;

use interaction_session::receive::{
    advance, decide_receive, decide_resume_batch, IncomingKind, IncomingState, RealignBudget,
    RealignReason, ReceiveDecision, ReceiverView,
};
use serde_json::Value;

fn manifest() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("interaction-aip")
        .join("tests")
        .join("fixtures")
        .join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read the fixture index {path:?}: {e}"));
    serde_json::from_str(&text).expect("manifest.json must be valid JSON")
}

fn u64_at(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("fixture entry missing unsigned `{key}`: {value}"))
}

fn opt_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn parse_view(value: &Value) -> ReceiverView {
    ReceiverView {
        has_state: value
            .get("hasState")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("local 缺 hasState: {value}")),
        session_id: opt_string(value, "sessionId"),
        epoch: u64_at(value, "epoch"),
        revision: u64_at(value, "revision"),
        state_hash: opt_string(value, "hash"),
        connection_generation: u64_at(value, "connectionGeneration"),
    }
}

fn parse_incoming(value: &Value) -> IncomingState {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .and_then(IncomingKind::parse)
        .unwrap_or_else(|| panic!("incoming 的 kind 不認得: {value}"));
    IncomingState {
        kind,
        session_id: opt_string(value, "sessionId"),
        epoch: u64_at(value, "epoch"),
        revision: u64_at(value, "revision"),
        base_revision: value.get("baseRevision").and_then(Value::as_u64),
        reason: opt_string(value, "reason"),
        hash: opt_string(value, "hash"),
        computed_hash: opt_string(value, "computedHash"),
        state_present: value
            .get("statePresent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        arrived_on_generation: u64_at(value, "arrivedOnGeneration"),
        via_authoritative_reply: value
            .get("viaAuthoritativeReply")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// `incomingBatchChain`：從本地 revision 起連續 `count` 則 patch（不必把幾百則寫進檔案）。
fn build_chain(view: &ReceiverView, spec: &Value) -> Vec<IncomingState> {
    let count = u64_at(spec, "count") as usize;
    assert_eq!(
        spec.get("kind").and_then(Value::as_str),
        Some("patch"),
        "目前只支援 patch 鏈"
    );
    (0..count)
        .map(|i| {
            let base = view.revision + i as u64;
            IncomingState {
                kind: IncomingKind::Patch,
                session_id: view.session_id.clone(),
                epoch: view.epoch,
                revision: base + 1,
                base_revision: Some(base),
                reason: None,
                hash: None,
                computed_hash: None,
                state_present: false,
                arrived_on_generation: view.connection_generation,
                via_authoritative_reply: false,
            }
        })
        .collect()
}

fn realign_reason(decision: &ReceiveDecision) -> Option<&'static str> {
    decision.realign_reason().map(RealignReason::as_str)
}

#[test]
fn every_receive_decision_fixture_reaches_the_documented_decision() {
    let manifest = manifest();
    let entries = manifest
        .get("receiveDecisions")
        .and_then(Value::as_array)
        .expect("manifest.json is missing the `receiveDecisions` section")
        .clone();
    assert!(
        entries.len() >= 32,
        "receiveDecisions 至少要 32 個具名案例，實際 {}",
        entries.len()
    );

    for entry in &entries {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("fixture entry missing `id`: {entry}"));
        let view = parse_view(entry.get("local").expect("local"));
        let expect = entry.get("expect").expect("expect");

        // 進來之前已經連續失敗幾次（有界 realign）。
        let mut budget = RealignBudget::new();
        for _ in 0..entry
            .get("budgetBefore")
            .and_then(Value::as_u64)
            .unwrap_or(0)
        {
            budget = budget.observe(
                &ReceiveDecision::Realign {
                    reason: RealignReason::EpochChanged,
                },
                true,
            );
        }

        let (decision, after, applied, skipped, stopped_at, via_authoritative) =
            if let Some(single) = entry.get("incoming") {
                let incoming = parse_incoming(single);
                let decision = decide_receive(&view, &incoming);
                let after = advance(&view, &incoming, &decision);
                let authoritative = incoming.via_authoritative_reply;
                (decision, after, None, None, None, authoritative)
            } else {
                let items: Vec<IncomingState> =
                    match (entry.get("incomingBatch"), entry.get("incomingBatchChain")) {
                        (Some(Value::Array(list)), _) => list.iter().map(parse_incoming).collect(),
                        (_, Some(spec)) => build_chain(&view, spec),
                        _ => panic!("{id}：案例既沒有 incoming 也沒有 incomingBatch"),
                    };
                let batch = decide_resume_batch(&view, &items);
                (
                    batch.outcome(),
                    batch.view.clone(),
                    Some(batch.applied),
                    Some(batch.skipped),
                    batch.stopped_at,
                    true,
                )
            };

        assert_eq!(
            decision.as_str(),
            expect["decision"].as_str().unwrap_or_default(),
            "{id}：決策不同"
        );
        assert_eq!(
            realign_reason(&decision),
            expect.get("reason").and_then(Value::as_str),
            "{id}：realign 原因不同"
        );
        assert_eq!(
            after.revision,
            u64_at(expect, "revisionAfter"),
            "{id}：套用後的 revision 不同"
        );
        assert_eq!(
            after.epoch,
            u64_at(expect, "epochAfter"),
            "{id}：套用後的 epoch 不同"
        );
        // 身分：`sessionIdAfter` 缺席＝「還是本地那一個」；有寫＝套用時記下了 incoming 的
        // sessionId（本地身分未知的那一格，規則 1 因此不宣稱不符）。
        assert_eq!(
            after.session_id,
            opt_string(expect, "sessionIdAfter").or_else(|| view.session_id.clone()),
            "{id}：套用後記下的 sessionId 不同"
        );
        if let Some(applied) = applied {
            assert_eq!(
                applied as u64,
                u64_at(expect, "applied"),
                "{id}：套用筆數不同"
            );
        }
        if let Some(skipped) = skipped {
            assert_eq!(
                skipped as u64,
                u64_at(expect, "skipped"),
                "{id}：跳過筆數不同"
            );
        }
        assert_eq!(
            stopped_at.map(|i| i as u64),
            expect.get("stoppedAt").and_then(Value::as_u64),
            "{id}：中止位置不同"
        );

        let budget = budget.observe(&decision, via_authoritative);
        assert_eq!(
            u64::from(budget.attempts()),
            u64_at(expect, "budgetAfter"),
            "{id}：realign 預算不同"
        );
        assert_eq!(
            if budget.is_unrecoverable() {
                "unrecoverable"
            } else {
                "ok"
            },
            expect["budget"].as_str().unwrap_or_default(),
            "{id}：realign 預算的結論不同"
        );

        // 不採用狀態的決策**不得**動到本地副本（這條規則值得單獨釘死）。
        if !decision.adopts_state() && entry.get("incoming").is_some() {
            assert_eq!(after, view, "{id}：不採用的決策不得改變本地狀態");
        }
    }
}

/// fixture 裡的 hash 必須是真的 SHA-256 十六進位字串（別的語言可以自己重算來核對）。
#[test]
fn the_hashes_in_the_fixtures_look_like_sha256() {
    let manifest = manifest();
    let entries = manifest["receiveDecisions"]
        .as_array()
        .expect("receiveDecisions")
        .clone();
    let mut seen = 0usize;
    for entry in &entries {
        for hash in [
            entry["local"].get("hash"),
            entry.get("incoming").and_then(|i| i.get("hash")),
            entry.get("incoming").and_then(|i| i.get("computedHash")),
        ]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        {
            assert_eq!(hash.len(), 64, "hash 不是 sha-256 hex：{hash}");
            assert!(
                hash.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "hash 不是小寫十六進位：{hash}"
            );
            seen += 1;
        }
    }
    assert!(seen >= 32, "案例裡的 hash 太少（{seen}）");
}
