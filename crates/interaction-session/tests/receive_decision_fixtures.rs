//! 三端共用的**接收端決策 fixtures**（`manifest.json` 的 `receiveDecisions` 段）。
//!
//! 為什麼需要它：同一則 `state` 訊息，v0.6.0 的三個實作會得到不同結論——桌面回 realign、
//! Rust／Swift 直接套用並靜默改寫本地 epoch（對抗審查 `427c806`）。差異發生在使用者看不見的
//! 地方，兩邊畫面都寫著「已同步」。這一段把 `docs/aip/character-session.md` §7 的決策表變成
//! 可執行的契約：每一筆都是「本地長這樣＋收到這則訊息＝這個決策」，三端必須答案相同。
//!
//! 這個檔案是**產生器兼驗證器**：案例定義在 Rust（下面的 `cases()`），
//! `AIP_UPDATE_FIXTURES=1 cargo test -p interaction-session --test receive_decision_fixtures`
//! 重生 `manifest.json` 的那一段；平常跑就是「磁碟上的內容 ＝ 產生器現在會寫的內容」。
//! 第二個**獨立消費者**（只讀 JSON、不認識 `cases()`）是 `receive_decisions_from_json.rs`。
//!
//! 超大訊息與壞 JSON 不在這一段：那是 typed boundary 的事，由 `envelopes`／`generated` 段
//! 涵蓋。這裡的輸入前提是「boundary 已經放行」。

use std::path::PathBuf;

use interaction_session::receive::{
    advance, decide_receive, decide_resume_batch, IncomingKind, IncomingState, RealignBudget,
    ReceiveDecision, ReceiverView, MAX_RESUME_PATCHES,
};
use interaction_session::{apply_patch, state_hash, REASON_RECOVERY, REASON_SESSION_RESET};
use serde_json::{json, Map, Value};

const SESSION: &str = "session.home";
const OTHER_SESSION: &str = "session.other-desktop";
/// 本地那條連線／請求的世代。
const GEN: u64 = 7;
/// 上一條連線的世代（它送出的東西現在才到）。
const OLD_GEN: u64 = 6;

fn state_a() -> Value {
    json!({"activity": "idle", "mood": {"kind": "neutral", "intensity": 0.0}})
}

fn state_b() -> Value {
    json!({"activity": "reacting", "mood": {"kind": "happy", "intensity": 0.5}})
}

/// A 套上一則真的 patch 之後的狀態（`hash` 欄位就是它的 SHA-256）。
fn state_merged() -> Value {
    apply_patch(&state_a(), &json!({"activity": "reacting"}))
}

fn hash_a() -> String {
    state_hash(&state_a())
}

fn hash_b() -> String {
    state_hash(&state_b())
}

fn hash_merged() -> String {
    state_hash(&state_merged())
}

/// 本地：已經同步到 epoch 5／revision 30，state 的 hash 是 A。
fn local() -> ReceiverView {
    ReceiverView {
        has_state: true,
        session_id: Some(SESSION.to_string()),
        epoch: 5,
        revision: 30,
        state_hash: Some(hash_a()),
        connection_generation: GEN,
    }
}

/// 本地：什麼都還沒有（第一次連上、或剛清空）。
fn empty() -> ReceiverView {
    ReceiverView::empty(GEN)
}

fn snapshot(revision: u64, epoch: u64, hash: &str) -> IncomingState {
    IncomingState {
        kind: IncomingKind::Snapshot,
        session_id: Some(SESSION.to_string()),
        epoch,
        revision,
        base_revision: None,
        reason: None,
        hash: Some(hash.to_string()),
        computed_hash: Some(hash.to_string()),
        state_present: true,
        arrived_on_generation: GEN,
        via_authoritative_reply: false,
    }
}

fn patch(revision: u64, base: u64, epoch: u64) -> IncomingState {
    IncomingState {
        kind: IncomingKind::Patch,
        session_id: Some(SESSION.to_string()),
        epoch,
        revision,
        base_revision: Some(base),
        reason: None,
        hash: Some(hash_merged()),
        computed_hash: Some(hash_merged()),
        state_present: false,
        arrived_on_generation: GEN,
        via_authoritative_reply: false,
    }
}

fn with_reason(mut incoming: IncomingState, reason: &str) -> IncomingState {
    incoming.reason = Some(reason.to_string());
    incoming
}

fn on_generation(mut incoming: IncomingState, generation: u64) -> IncomingState {
    incoming.arrived_on_generation = generation;
    incoming
}

fn from_session(mut incoming: IncomingState, session: &str) -> IncomingState {
    incoming.session_id = Some(session.to_string());
    incoming
}

fn authoritative(mut incoming: IncomingState) -> IncomingState {
    incoming.via_authoritative_reply = true;
    incoming
}

/// 一批 patch：從 `local().revision + 1` 起，連續 `count` 則。
/// （用來釘住 `MAX_RESUME_PATCHES` 的邊界，不必把幾百則訊息寫進 manifest。）
struct Chain {
    count: usize,
}

impl Chain {
    fn build(&self, view: &ReceiverView) -> Vec<IncomingState> {
        (0..self.count)
            .map(|i| {
                let base = view.revision + i as u64;
                let mut item = patch(base + 1, base, view.epoch);
                item.hash = None;
                item.computed_hash = None;
                item
            })
            .collect()
    }
}

/// fixture 的輸入形狀：單則訊息、一批 resume 回覆，或一條連續 patch 鏈。
enum Input {
    Single(IncomingState),
    Batch(Vec<IncomingState>),
    BatchChain(Chain),
}

struct Case {
    id: &'static str,
    note: &'static str,
    local: ReceiverView,
    input: Input,
    /// 這一則到達之前，接收端已經連續失敗幾次（realign 預算）。
    budget_before: u32,
}

fn single(
    id: &'static str,
    note: &'static str,
    local: ReceiverView,
    incoming: IncomingState,
) -> Case {
    Case {
        id,
        note,
        local,
        input: Input::Single(incoming),
        budget_before: 0,
    }
}

fn cases() -> Vec<Case> {
    vec![

    // -------------------------------------------------- 規則 0：連線／請求世代
    single(
        "stale-connection-reset",
        "規則 0：上一條連線送出的 session-reset 現在才到。它宣告的 epoch 一定與本地不同，任何 epoch 判斷都會被騙過去——世代檢查是唯一防線",
        local(),
        on_generation(
            with_reason(snapshot(1, 9, &hash_b()), REASON_SESSION_RESET),
            OLD_GEN,
        ),
    ),
    single(
        "stale-connection-newer-snapshot",
        "規則 0：舊連線上的較新 snapshot 也一樣丟掉（那條連線的權威性已經失效）",
        local(),
        on_generation(snapshot(31, 5, &hash_b()), OLD_GEN),
    ),
    single(
        "stale-request-generation-authoritative-reply",
        "規則 0 也涵蓋**請求**世代：上一次 GET／resume 的回覆在我們重發之後才到",
        local(),
        authoritative(on_generation(snapshot(40, 5, &hash_b()), OLD_GEN)),
    ),
    single(
        "stale-connection-recovery",
        "規則 0 先於規則 6：舊連線上的 recovery 宣告不得讓本地倒退",
        local(),
        on_generation(
            with_reason(snapshot(9, 5, &hash_b()), REASON_RECOVERY),
            OLD_GEN,
        ),
    ),

    // -------------------------------------------------- 規則 1：身分
    single(
        "identity-mismatch-snapshot",
        "規則 1：別的 session 的狀態不是「比較舊」，是不相干——不套用、不 realign（realign 只會再要一次別人的 session）",
        local(),
        from_session(snapshot(31, 5, &hash_b()), OTHER_SESSION),
    ),
    single(
        "identity-mismatch-beats-session-reset",
        "規則 1 先於規則 3：手機被重新配對到另一台桌面（sessionId 變了），對方的 session-reset 不得清掉這一份",
        local(),
        from_session(
            with_reason(snapshot(1, 6, &hash_b()), REASON_SESSION_RESET),
            OTHER_SESSION,
        ),
    ),
    single(
        "identity-mismatch-patch",
        "規則 1 對 patch 一樣成立",
        local(),
        from_session(patch(31, 30, 5), OTHER_SESSION),
    ),
    single(
        "identity-unknown-locally-is-not-a-mismatch",
        "本地還沒有狀態就沒有身分可比：不宣稱不符（規則 1 需要 local 有狀態）",
        empty(),
        snapshot(12, 5, &hash_b()),
    ),

    // -------------------------------------------------- 規則 2：形狀
    single(
        "snapshot-without-hash",
        "規則 2：AIP 1.0 的 snapshot 必帶 hash，沒有 legacy profile。超大／壞 JSON 由 typed boundary（envelopes／generated 段）擋下，不在這一段",
        local(),
        IncomingState {
            hash: None,
            computed_hash: None,
            ..snapshot(31, 5, &hash_b())
        },
    ),
    single(
        "snapshot-without-state",
        "規則 2：snapshot 的 payload 沒有 state 就沒有東西可套用",
        local(),
        IncomingState {
            state_present: false,
            ..snapshot(31, 5, &hash_b())
        },
    ),
    single(
        "patch-without-base-revision",
        "規則 2（patch 版）：缺 baseRevision 的 patch 接不上任何東西；typed boundary 已經擋過一次，這是第二道",
        local(),
        IncomingState {
            base_revision: None,
            ..patch(31, 30, 5)
        },
    ),
    Case {
        id: "boundary-rejected-authoritative-reply-costs-one-attempt",
        note: "壞掉的**權威回覆**（boundary 或形狀檢查擋下）算一次 realign 失敗：對方回答了，但答案沒用",
        local: local(),
        input: Input::Single(authoritative(IncomingState {
            hash: None,
            computed_hash: None,
            ..snapshot(31, 5, &hash_b())
        })),
        budget_before: 2,
    },

    // -------------------------------------------------- 規則 3／4／5：epoch
    single(
        "session-reset-with-a-larger-epoch",
        "規則 3：host 重建了 session（epoch 變大），revision 比本地小也要接受",
        local(),
        with_reason(snapshot(1, 6, &hash_b()), REASON_SESSION_RESET),
    ),
    single(
        "session-reset-with-a-smaller-epoch-host-reinstalled",
        "規則 3：host 重灌後 epoch 從 1 重新起跳，比本地記得的小——§7 寫的是「不同」不是「更大」，用「更大」會讓這台裝置永遠停在舊狀態",
        local(),
        with_reason(snapshot(1, 1, &hash_b()), REASON_SESSION_RESET),
    ),
    single(
        "session-reset-with-the-same-epoch-is-not-a-reset",
        "規則 3 的例外要求 epoch 不同：同 epoch 的 session-reset 只是換個說法宣稱 rollback，仍然忽略",
        local(),
        with_reason(snapshot(9, 5, &hash_b()), REASON_SESSION_RESET),
    ),
    single(
        "bootstrap-snapshot",
        "規則 4：本地什麼都沒有，第一份權威狀態直接收下",
        empty(),
        snapshot(12, 5, &hash_b()),
    ),
    single(
        "bootstrap-session-reset",
        "規則 3 對「local 無狀態」一樣成立：拿到的是重建宣告就記成 reset（epoch／revision 照收）",
        empty(),
        with_reason(snapshot(1, 6, &hash_b()), REASON_SESSION_RESET),
    ),
    single(
        "different-epoch-without-a-reset-declaration",
        "規則 5：epoch 不同又沒有重建宣告＝兩份狀態沒有可比的順序，不猜（v0.6.0 的 Rust／Swift 會直接套用並靜默改寫本地 epoch）。host 對 epoch 不同的 resume 一律回 session-reset，所以一次就收斂",
        local(),
        snapshot(31, 6, &hash_b()),
    ),
    single(
        "smaller-epoch-without-a-reset-declaration",
        "規則 5：epoch 變小又沒有宣告也一樣 realign（不得因為 revision 較大就採用）",
        local(),
        snapshot(40, 1, &hash_b()),
    ),

    // -------------------------------------------------- 規則 6：recovery
    single(
        "recovery-snapshot-moves-the-receiver-back",
        "規則 6：host 從較舊的快照還原過，同一個 session（epoch 不變）的權威狀態就是比本地舊。這是唯一合法的倒退說法——沒有它，host 的答覆會被 rollback 防護忽略，兩邊永久分歧而畫面都寫著「已同步」",
        local(),
        authoritative(with_reason(snapshot(9, 5, &hash_b()), REASON_RECOVERY)),
    ),
    single(
        "recovery-declared-on-a-newer-snapshot-is-just-an-apply",
        "規則 6 只在真的倒退時成立：recovery 不是特權，revision 較新就照常套用",
        local(),
        with_reason(snapshot(31, 5, &hash_b()), REASON_RECOVERY),
    ),
    single(
        "recovery-across-epochs-still-realigns",
        "規則 5 先於規則 6：recovery 是「同一個 session 內」的說法，epoch 不同就不算數",
        local(),
        with_reason(snapshot(9, 6, &hash_b()), REASON_RECOVERY),
    ),
    single(
        "unknown-reason-is-treated-as-no-reason",
        "未知的 reason 值不得被當成任何特權（舊接收端遇到新值也是這樣降級）",
        local(),
        with_reason(snapshot(9, 5, &hash_b()), "spring-cleaning"),
    ),

    // -------------------------------------------------- 規則 7／8／9：同 epoch 的 revision
    single(
        "newer-snapshot-applies",
        "規則 9：同 epoch、revision 較新、hash 核對過 → 套用",
        local(),
        snapshot(31, 5, &hash_b()),
    ),
    single(
        "older-snapshot-is-stale",
        "規則 7：同 epoch、revision 較舊 → 忽略（稽核 aip.state-rollback-ignored）",
        local(),
        snapshot(29, 5, &hash_b()),
    ),
    single(
        "replayed-snapshot-is-already-applied",
        "規則 8：同 revision、同 hash ＝ 重播，什麼都不做",
        local(),
        snapshot(30, 5, &hash_a()),
    ),
    single(
        "same-revision-different-hash-realigns",
        "規則 8：同一個 revision 卻有兩份不同的狀態——只能重新對齊，不得二選一",
        local(),
        snapshot(30, 5, &hash_b()),
    ),
    single(
        "snapshot-whose-hash-does-not-match-its-own-state",
        "規則 9：host 宣告的 hash 與接收端算出來的不同 → 不套用（AIP §6 的 hash 是套用前的門檻，不是事後說明）",
        local(),
        IncomingState {
            computed_hash: Some(hash_a()),
            ..snapshot(31, 5, &hash_b())
        },
    ),

    // -------------------------------------------------- 權威回覆與 SSE 的競態
    single(
        "http-reply-lands-after-a-newer-sse-snapshot",
        "同一個世代裡，HTTP 回覆比本地舊 → 仍然是 ignore-stale：「最新的回覆」不代表同一個 incarnation 的回退合法（桌面端 v0.6.x 的 allowRegression 因此取消）",
        local(),
        authoritative(snapshot(28, 5, &hash_b())),
    ),
    single(
        "authoritative-reply-that-is-behind-needs-an-explicit-recovery",
        "請求發出前本地就已經領先 host：沒有 recovery 宣告就忽略，有才採納（與上一筆是同一個情境的兩半）",
        local(),
        authoritative(snapshot(9, 5, &hash_b())),
    ),

    // -------------------------------------------------- 規則 10–15：patch
    single(
        "contiguous-patch-applies",
        "規則 15：baseRevision 接得上、merge 後 hash 相符 → 套用",
        local(),
        patch(31, 30, 5),
    ),
    single(
        "patch-with-a-gap-realigns",
        "規則 13：baseRevision 接不上本地 revision → 不得套用，改送 character.session.resume",
        local(),
        patch(32, 31, 5),
    ),
    single(
        "older-patch-is-stale",
        "規則 12：落後的 patch 直接忽略（先於 base 檢查）",
        local(),
        patch(29, 28, 5),
    ),
    single(
        "replayed-patch-is-already-applied",
        "規則 12：同 revision 的 patch 是重播",
        local(),
        patch(30, 29, 5),
    ),
    single(
        "patch-from-another-epoch-realigns",
        "規則 11：三端統一——v0.6.0 的 Rust patch 分支完全不看 epoch，只靠 baseRevision 恰巧不符去擋",
        local(),
        patch(31, 30, 6),
    ),
    single(
        "patch-without-a-local-copy-realigns",
        "規則 10：補丁不是完整狀態，本地沒有東西可以套上去",
        empty(),
        patch(31, 30, 5),
    ),
    single(
        "patch-whose-merged-hash-differs-realigns",
        "規則 14：merge 之後算出來的 hash 與 host 宣告的不同 → 不得假裝追上了",
        local(),
        IncomingState {
            computed_hash: Some(hash_a()),
            ..patch(31, 30, 5)
        },
    ),

    // -------------------------------------------------- realign 預算
    Case {
        id: "realign-budget-exhausted-after-three-attempts",
        note: "有界 realign：連續 3 次未能 apply → unrecoverable，照實說「狀態未知」，不再自動重試（無上限就是一個打不完的請求迴圈）",
        local: local(),
        input: Input::Single(snapshot(31, 6, &hash_b())),
        budget_before: 2,
    },
    Case {
        id: "an-apply-clears-the-realign-budget",
        note: "任一次成功套用（apply／reset／recover）把連續失敗清零",
        local: local(),
        input: Input::Single(snapshot(31, 5, &hash_b())),
        budget_before: 2,
    },

    // -------------------------------------------------- resume 回覆（逐則）
    Case {
        id: "resume-reply-mixes-applied-and-new-patches",
        note: "resume 回覆逐則走同一張表：host 回放的範圍與本地重疊是正常的，already-applied／ignore-stale 跳過不中止",
        local: local(),
        input: Input::Batch(vec![
            patch(29, 28, 5),
            patch(30, 29, 5),
            patch(31, 30, 5),
            patch(32, 31, 5),
        ]),
        budget_before: 0,
    },
    Case {
        id: "resume-reply-stops-at-the-first-gap",
        note: "第一個帶 effect 的 realign 中止整批：後面的補丁都建立在沒套用的那一則之上",
        local: local(),
        input: Input::Batch(vec![patch(31, 30, 5), patch(33, 32, 5), patch(34, 33, 5)]),
        budget_before: 0,
    },
    Case {
        id: "resume-reply-at-the-bound-applies-every-patch",
        note: "剛好 maxResumePatches 則（＝host 事件日誌環大小）仍然全部套用",
        local: local(),
        input: Input::BatchChain(Chain {
            count: MAX_RESUME_PATCHES,
        }),
        budget_before: 0,
    },
    Case {
        id: "resume-reply-beyond-the-bound-realigns",
        note: "超過上限**不**靜默截斷成「我以為我追上了」：整批不處理，改 realign",
        local: local(),
        input: Input::BatchChain(Chain {
            count: MAX_RESUME_PATCHES + 1,
        }),
        budget_before: 0,
    },

    ]
}

// ------------------------------------------------------------------ 渲染

fn view_json(view: &ReceiverView) -> Value {
    let mut map = Map::new();
    map.insert("hasState".into(), json!(view.has_state));
    if let Some(id) = &view.session_id {
        map.insert("sessionId".into(), json!(id));
    }
    map.insert("epoch".into(), json!(view.epoch));
    map.insert("revision".into(), json!(view.revision));
    if let Some(hash) = &view.state_hash {
        map.insert("hash".into(), json!(hash));
    }
    map.insert(
        "connectionGeneration".into(),
        json!(view.connection_generation),
    );
    Value::Object(map)
}

fn incoming_json(incoming: &IncomingState) -> Value {
    let mut map = Map::new();
    map.insert("kind".into(), json!(incoming.kind.as_str()));
    if let Some(id) = &incoming.session_id {
        map.insert("sessionId".into(), json!(id));
    }
    map.insert("epoch".into(), json!(incoming.epoch));
    map.insert("revision".into(), json!(incoming.revision));
    if let Some(base) = incoming.base_revision {
        map.insert("baseRevision".into(), json!(base));
    }
    if let Some(reason) = &incoming.reason {
        map.insert("reason".into(), json!(reason));
    }
    if let Some(hash) = &incoming.hash {
        map.insert("hash".into(), json!(hash));
    }
    if let Some(hash) = &incoming.computed_hash {
        map.insert("computedHash".into(), json!(hash));
    }
    map.insert("statePresent".into(), json!(incoming.state_present));
    map.insert(
        "arrivedOnGeneration".into(),
        json!(incoming.arrived_on_generation),
    );
    map.insert(
        "viaAuthoritativeReply".into(),
        json!(incoming.via_authoritative_reply),
    );
    Value::Object(map)
}

/// 案例的期望：直接由 `decide_receive`／`decide_resume_batch` 算出來（產生器不手寫答案，
/// 但驗證時比對的是**磁碟上的內容**，所以規則一改、磁碟沒重生就紅燈）。
fn expectation(case: &Case) -> Value {
    let mut map = Map::new();
    let budget = RealignBudget::new();
    let budget = (0..case.budget_before).fold(budget, |b, _| {
        b.observe(
            &ReceiveDecision::Realign {
                reason: interaction_session::receive::RealignReason::EpochChanged,
            },
            true,
        )
    });
    let (decision, view, extra) = match &case.input {
        Input::Single(incoming) => {
            let decision = decide_receive(&case.local, incoming);
            let view = advance(&case.local, incoming, &decision);
            (decision, view, None)
        }
        Input::Batch(items) => {
            let batch = decide_resume_batch(&case.local, items);
            (batch.outcome(), batch.view.clone(), Some(batch))
        }
        Input::BatchChain(chain) => {
            let items = chain.build(&case.local);
            let batch = decide_resume_batch(&case.local, &items);
            (batch.outcome(), batch.view.clone(), Some(batch))
        }
    };
    let via_authoritative = match &case.input {
        Input::Single(incoming) => incoming.via_authoritative_reply,
        _ => true,
    };
    let budget = budget.observe(&decision, via_authoritative);
    map.insert("decision".into(), json!(decision.as_str()));
    if let Some(reason) = decision.realign_reason() {
        map.insert("reason".into(), json!(reason.as_str()));
    }
    map.insert("revisionAfter".into(), json!(view.revision));
    map.insert("epochAfter".into(), json!(view.epoch));
    if let Some(batch) = extra {
        map.insert("applied".into(), json!(batch.applied));
        map.insert("skipped".into(), json!(batch.skipped));
        if let Some(index) = batch.stopped_at {
            map.insert("stoppedAt".into(), json!(index));
        }
    }
    map.insert("budgetAfter".into(), json!(budget.attempts()));
    map.insert(
        "budget".into(),
        json!(if budget.is_unrecoverable() {
            "unrecoverable"
        } else {
            "ok"
        }),
    );
    Value::Object(map)
}

fn entry(case: &Case) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), json!(case.id));
    map.insert("note".into(), json!(case.note));
    map.insert("local".into(), view_json(&case.local));
    match &case.input {
        Input::Single(incoming) => {
            map.insert("incoming".into(), incoming_json(incoming));
        }
        Input::Batch(items) => {
            map.insert(
                "incomingBatch".into(),
                Value::Array(items.iter().map(incoming_json).collect()),
            );
        }
        Input::BatchChain(chain) => {
            map.insert(
                "incomingBatchChain".into(),
                json!({"kind": "patch", "count": chain.count}),
            );
        }
    }
    if case.budget_before > 0 {
        map.insert("budgetBefore".into(), json!(case.budget_before));
    }
    map.insert("expect".into(), expectation(case));
    Value::Object(map)
}

fn entries() -> Vec<Value> {
    cases().iter().map(entry).collect()
}

fn section_text(list: &[Value]) -> String {
    let rendered: Vec<String> = list
        .iter()
        .map(|entry| {
            serde_json::to_string_pretty(entry)
                .expect("entry serializes")
                .lines()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    format!("\"receiveDecisions\": [\n{}\n  ]", rendered.join(",\n"))
}

fn splice_manifest(text: &str, section: &str) -> String {
    const KEY: &str = "\"receiveDecisions\": [";
    if let Some(start) = text.find(KEY) {
        let close = text[start..]
            .find("\n  ]")
            .map(|i| start + i + "\n  ]".len())
            .expect("receiveDecisions 段以兩格縮排的 `]` 結束");
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

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("interaction-aip")
        .join("tests")
        .join("fixtures")
        .join("manifest.json")
}

fn update_requested() -> bool {
    std::env::var("AIP_UPDATE_FIXTURES").is_ok_and(|v| v == "1")
}

#[test]
fn receive_decision_fixtures_match_the_decision_table() {
    let list = entries();
    let path = manifest_path();
    let mut text = std::fs::read_to_string(&path).expect("manifest.json readable");

    if update_requested() {
        text = splice_manifest(&text, &section_text(&list));
        std::fs::write(&path, &text).expect("manifest written");
    }

    let manifest: Value = serde_json::from_str(&text).expect("manifest.json is JSON");
    let on_disk = manifest
        .get("receiveDecisions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!("manifest.json 缺 `receiveDecisions` 段：用 AIP_UPDATE_FIXTURES=1 重生")
        });
    assert_eq!(
        on_disk, list,
        "manifest.json 的 receiveDecisions 段與決策表不一致：AIP_UPDATE_FIXTURES=1 重生"
    );
    // 重生是冪等的（跑兩次不會多出空白或重複段落）。
    assert_eq!(
        splice_manifest(&text, &section_text(&list)),
        text,
        "重生不是冪等的：splice 會讓 manifest 每跑一次就變一次"
    );
}

/// 案例數與涵蓋面：少了任何一類，跨語言就有一個沒人測到的分支。
#[test]
fn the_decision_table_fixtures_cover_every_branch() {
    let list = cases();
    assert!(
        list.len() >= 32,
        "決策表至少要 32 個具名案例，實際 {}",
        list.len()
    );
    let mut ids: Vec<&str> = list.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "案例 id 不得重複");

    let decisions: Vec<String> = entries()
        .iter()
        .map(|e| {
            e["expect"]["decision"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    for decision in [
        "ignore-stale-connection",
        "reject-identity",
        "reject-invalid",
        "reset",
        "apply",
        "realign",
        "recover",
        "ignore-stale",
        "already-applied",
    ] {
        assert!(
            decisions.iter().any(|d| d == decision),
            "沒有任何案例得到 `{decision}`"
        );
    }
    let reasons: Vec<String> = entries()
        .iter()
        .filter_map(|e| e["expect"]["reason"].as_str().map(str::to_string))
        .collect();
    for reason in [
        "no-local",
        "epoch-changed",
        "base-mismatch",
        "hash-mismatch",
        "resume-too-long",
    ] {
        assert!(
            reasons.iter().any(|r| r == reason),
            "沒有任何案例得到 realign reason `{reason}`"
        );
    }
    assert!(
        entries()
            .iter()
            .any(|e| e["expect"]["budget"] == json!("unrecoverable")),
        "沒有案例走到有界 realign 的終點"
    );
}

/// fixture 裡的 hash 是真的 SHA-256（別的語言可以自己重算來核對），
/// patch 案例的 hash 是**merge 之後**的結果。
#[test]
fn the_hashes_in_the_fixtures_are_real() {
    assert_eq!(hash_a().len(), 64);
    assert_ne!(hash_a(), hash_b());
    assert_eq!(state_merged()["activity"], json!("reacting"));
    assert_eq!(hash_merged(), state_hash(&state_merged()));
    assert_ne!(hash_merged(), hash_a());
}
