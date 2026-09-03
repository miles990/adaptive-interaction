//! §13：command lifecycle、ack／completion 誠實性、cancel idempotency、duplicate messageId、expired message、
//! reconnect generation、adapter crash、bounded queue、payload size limit、偽造 verified、emergency priority、
//! 多實例安全去重、system.text 回退、rate limit、heartbeat 逾時。

mod common;

use common::*;
use interaction_character::*;

fn primary(manifest: &CharacterManifest) -> (Gateway, InstanceId) {
    let mut gw = Gateway::default();
    let id = connect(&mut gw, manifest, CharacterRole::PrimaryCompanion);
    (gw, id)
}

#[test]
fn hello_is_built_from_instance_and_limits() {
    let mut gw = Gateway::default();
    let id = gw.register_instance(text_manifest(), CharacterRole::Familiar);
    let hello = gw.hello_for(&id).expect("hello");
    assert_eq!(hello.protocol_version, "1.0");
    assert_eq!(hello.role, CharacterRole::Familiar);
    assert_eq!(hello.character_instance_id, id.as_str());
    assert_eq!(hello.requires.len(), 20);
    assert_eq!(hello.limits.max_message_bytes, 65_536);
    assert_eq!(hello.limits.max_messages_per_second, 50);
    assert_eq!(hello.limits.max_pending, 64);
    assert_eq!(id.as_str(), "text#1");
}

#[test]
fn dispatch_sends_intent_and_accepted_receipt() {
    let (mut gw, id) = primary(&text_manifest());
    let env = envelope(
        &id,
        "m1",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    let out = gw.dispatch(&id, env, t(1));
    assert_eq!(sent_intents(&out), vec!["m1"]);
    let r = receipts(&out);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].status, ReceiptStatus::Accepted);
    assert_eq!(r[0].resolution, Some(Resolution::Exact));
    assert_eq!(r[0].generation, 1);
    assert!(!r[0].duplicate);
    assert_eq!(gw.command_status(&id, "m1"), Some(ReceiptStatus::Accepted));
    // 送出的 envelope 保留 truthState（只有 runtime → adapter 方向才有）。
    match sends(&out)[0] {
        WireMessage::Intent { envelope } => assert_eq!(envelope.truth_state, TruthState::Working),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn command_lifecycle_legal_and_illegal_transitions() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    // accepted → completed 是非法的（必須經過 started）。
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Completed, t(2));
    assert!(receipts(&out).is_empty());
    assert!(audits(&out)
        .iter()
        .any(|a| a.contains("illegal transition")));
    assert_eq!(gw.command_status(&id, "m1"), Some(ReceiptStatus::Accepted));
    // 合法路徑。
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Scheduled, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Scheduled);
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(3));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Started);
    // 同狀態重送：冪等、無輸出。
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(3));
    assert!(out.is_empty());
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Completed, t(4));
    let r = receipts(&out);
    assert_eq!(r[0].status, ReceiptStatus::Completed);
    assert_eq!(r[0].resolution, Some(Resolution::Exact));
    assert_eq!(gw.command_status(&id, "m1"), Some(ReceiptStatus::Completed));
    // 終結後再回報：丟棄並記 audit。
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(5));
    assert!(receipts(&out).is_empty());
    assert!(audits(&out).iter().any(|a| a.contains("already terminal")));
    // 未知 messageId。
    let out = adapter_receipt(&mut gw, &id, "ghost", ReceiptStatus::Started, t(5));
    assert!(audits(&out).iter().any(|a| a.contains("unknown messageId")));
    // started 之後才能 completed；accepted 可直接 failed。
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(6),
        ),
        t(6),
    );
    let out = adapter_receipt(&mut gw, &id, "m2", ReceiptStatus::Failed, t(7));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Failed);
    assert_eq!(receipts(&out)[0].resolution, Some(Resolution::Failed));
}

#[test]
fn acknowledged_never_becomes_completed_and_sweeps_to_uncertain() {
    let (mut gw, id) = primary(&text_manifest());
    let mut env = envelope(
        &id,
        "m1",
        CharacterIntent::Notice,
        TruthState::None,
        10,
        t(1),
    );
    env.duration_hint = Some(DurationHint {
        ms: 2_000,
        looped: false,
    });
    gw.dispatch(&id, env, t(1));
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Acknowledged, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Acknowledged);
    // adapter 事後宣稱 completed：拒絕。
    let out = adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Completed, t(3));
    assert!(receipts(&out).is_empty());
    assert!(audits(&out)
        .iter()
        .any(|a| a.contains("illegal transition")));
    assert_eq!(
        gw.command_status(&id, "m1"),
        Some(ReceiptStatus::Acknowledged)
    );
    // 寬限內 sweep 不動；ack + duration(2 s) + grace(5 s) 之後 → uncertain。
    let out = gw.sweep(t(8));
    assert!(receipts(&out).is_empty());
    let out = gw.sweep(t(9));
    let r = receipts(&out);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].status, ReceiptStatus::Uncertain);
    assert_eq!(r[0].message_id, "m1");
    assert_eq!(gw.command_status(&id, "m1"), Some(ReceiptStatus::Uncertain));
}

#[test]
fn started_watchdog_marks_uncertain_not_completed() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(2));
    // expiresAt = t(61)；watchdog 60 s → t(121) 才判 uncertain（heartbeat 維持連線，排除逾時斷線）。
    gw.heartbeat(&id, t(119));
    assert!(receipts(&gw.sweep(t(120))).is_empty());
    gw.heartbeat(&id, t(120));
    let out = gw.sweep(t(121));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
}

#[test]
fn cancel_is_idempotent() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    let first = gw.cancel(&id, "m1", "user", t(2));
    assert_eq!(
        sent_cancels(&first),
        vec![("m1".to_string(), Some("user".to_string()))]
    );
    let r1 = receipts(&first)[0].clone();
    assert_eq!(r1.status, ReceiptStatus::Cancelled);
    assert_eq!(r1.reason.as_deref(), Some("user"));
    assert!(!r1.already_terminal);
    // 重複 cancel：同一結果、不再送 cancel。
    let second = gw.cancel(&id, "m1", "user", t(3));
    assert!(sent_cancels(&second).is_empty());
    assert_eq!(receipts(&second)[0], &r1);
    // 對 completed 的 command cancel：alreadyTerminal:true，不報錯。
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(4),
        ),
        t(4),
    );
    adapter_receipt(&mut gw, &id, "m2", ReceiptStatus::Started, t(5));
    adapter_receipt(&mut gw, &id, "m2", ReceiptStatus::Completed, t(6));
    let out = gw.cancel(&id, "m2", "user", t(7));
    let r = receipts(&out)[0].clone();
    assert_eq!(r.status, ReceiptStatus::Cancelled);
    assert!(r.already_terminal);
    assert!(sent_cancels(&out).is_empty());
    assert_eq!(receipts(&gw.cancel(&id, "m2", "user", t(8)))[0], &r);
    // 未知 messageId：alreadyTerminal:true、無錯誤。
    let out = gw.cancel(&id, "nope", "user", t(9));
    assert!(receipts(&out)[0].already_terminal);
    assert_eq!(
        gw.command_status(&id, "m2"),
        Some(ReceiptStatus::Completed),
        "cancel does not rewrite history"
    );
}

#[test]
fn duplicate_message_id_is_deduplicated_without_resend() {
    let (mut gw, id) = primary(&text_manifest());
    let env = envelope(
        &id,
        "m1",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    let first = gw.dispatch(&id, env.clone(), t(1));
    assert_eq!(sent_intents(&first).len(), 1);
    let second = gw.dispatch(&id, env.clone(), t(2));
    assert!(sent_intents(&second).is_empty());
    let r = receipts(&second)[0];
    assert_eq!(r.status, ReceiptStatus::Accepted);
    assert!(r.duplicate);
    assert_eq!(r.resolution, Some(Resolution::Exact));
    // 完成後的重複 id 仍去重。
    adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(3));
    adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Completed, t(4));
    let third = gw.dispatch(&id, env, t(5));
    assert!(receipts(&third)[0].duplicate);
    assert!(sent_intents(&third).is_empty());
    // 環 256：第 257 個新 id 之後，最舊的 id 可再次被接受。
    for i in 0..256 {
        gw.dispatch(
            &id,
            envelope(
                &id,
                &format!("fill{i}"),
                CharacterIntent::Work,
                TruthState::Working,
                10,
                t(6),
            ),
            t(6),
        );
        adapter_receipt(
            &mut gw,
            &id,
            &format!("fill{i}"),
            ReceiptStatus::Started,
            t(6),
        );
        adapter_receipt(
            &mut gw,
            &id,
            &format!("fill{i}"),
            ReceiptStatus::Completed,
            t(6),
        );
    }
    let again = gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(7),
        ),
        t(7),
    );
    assert!(!receipts(&again)[0].duplicate, "evicted from the ring");
}

#[test]
fn expired_message_is_not_sent() {
    let (mut gw, id) = primary(&text_manifest());
    let env = envelope(
        &id,
        "m1",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    let out = gw.dispatch(&id, env, t(61));
    assert!(sent_intents(&out).is_empty());
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Expired);
    assert_eq!(gw.command_status(&id, "m1"), Some(ReceiptStatus::Expired));
    // 送出後才過期：sweep 標 expired 並送 cancel。
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    gw.heartbeat(&id, t(61));
    let out = gw.sweep(t(62));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Expired);
    assert_eq!(sent_cancels(&out)[0].0, "m2");
    // 過期的 envelope 即使 expiresAt 在 timestamp 之前也被 normalize 擋下。
    let mut bad = envelope(
        &id,
        "m3",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(70),
    );
    bad.expires_at = t(69);
    let out = gw.dispatch(&id, bad, t(70));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Failed);
}

#[test]
fn reconnect_generation_rejects_stale_receipts() {
    let m = text_manifest();
    let (mut gw, id) = primary(&m);
    assert_eq!(gw.generation(&id), Some(1));
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    let out = gw.on_disconnect(&id, DisconnectReason::TransportClosed, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
    assert_eq!(gw.generation(&id), Some(2));
    assert!(gw.negotiated(&id).is_none());
    // 重連：重新 hello／negotiate → 新世代。
    let (n, out) = gw
        .on_negotiate(&id, Negotiate::from_manifest(&m, 2), t(3))
        .expect("re-negotiate");
    assert_eq!(n.generation, 3);
    assert!(matches!(sends(&out)[0], WireMessage::Negotiated(_)));
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(4),
        ),
        t(4),
    );
    // 舊世代（1）的回執與事件一律丟棄。
    let stale = CommandReceipt::new("m2", id.as_str(), 1, ReceiptStatus::Started, t(5));
    let out = gw.on_receipt(&id, stale, t(5));
    assert!(receipts(&out).is_empty());
    assert!(audits(&out).iter().any(|a| a.contains("stale generation")));
    assert_eq!(gw.command_status(&id, "m2"), Some(ReceiptStatus::Accepted));
    let stale_event = CharacterInputEvent {
        protocol_version: "1.0".into(),
        event_id: "e1".into(),
        character_instance_id: id.0.clone(),
        generation: 1,
        timestamp: t(5),
        kind: InputEventKind::Clicked,
        payload: Default::default(),
        privacy_class: PrivacyClass::Internal,
    };
    assert_eq!(
        gw.on_event(&id, stale_event.clone(), t(5)),
        InputDecision::Dropped(InputDropReason::StaleGeneration)
    );
    let mut fresh = stale_event;
    fresh.generation = 3;
    assert_eq!(gw.on_event(&id, fresh, t(5)), InputDecision::Queued);
    assert_eq!(gw.drain_input(&id).len(), 1);
    // 新世代回執正常。
    let out = adapter_receipt(&mut gw, &id, "m2", ReceiptStatus::Started, t(6));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Started);
    assert_eq!(receipts(&out)[0].generation, 3);
}

#[test]
fn adapter_crash_marks_all_pending_uncertain() {
    let (mut gw, id) = primary(&text_manifest());
    for i in 0..3 {
        gw.dispatch(
            &id,
            envelope(
                &id,
                &format!("m{i}"),
                CharacterIntent::Work,
                TruthState::Working,
                10,
                t(1),
            ),
            t(1),
        );
    }
    adapter_receipt(&mut gw, &id, "m0", ReceiptStatus::Started, t(2));
    let out = gw.on_lifecycle(&id, AdapterLifecycleState::Crashed, t(3));
    let r = receipts(&out);
    assert_eq!(r.len(), 3);
    assert!(r.iter().all(|r| r.status == ReceiptStatus::Uncertain));
    assert!(r.iter().all(|r| r.reason.as_deref() == Some("crash")));
    let view = gw.instance(&id).expect("view");
    assert_eq!(view.lifecycle, AdapterLifecycleState::Crashed);
    assert!(!view.connected);
    assert_eq!(view.generation, 2);
    assert_eq!(view.pending, 0);
    // crash 後派送安全 intent：走 system.text，不遺失。
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "s1",
            CharacterIntent::Emergency,
            TruthState::Emergency,
            0,
            t(4),
        ),
        t(4),
    );
    assert_eq!(system_texts(&out), 1);
    assert!(sent_intents(&out).is_empty());
    // goodbye 也一樣。
    let m = text_manifest();
    let (mut gw, id) = primary(&m);
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    let out = gw.on_message(&id, WireMessage::Goodbye { reason: None }, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
}

#[test]
fn heartbeat_timeout_disconnects_after_45s() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    gw.on_message(
        &id,
        WireMessage::Heartbeat {
            generation: Some(1),
        },
        t(10),
    );
    assert!(
        receipts(&gw.sweep(t(55))).is_empty(),
        "45 s not yet elapsed since heartbeat"
    );
    let out = gw.sweep(t(56));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
    assert!(audits(&out).iter().any(|a| a.contains("heartbeat-timeout")));
    assert_eq!(gw.generation(&id), Some(2));
}

#[test]
fn bounded_pending_queue_never_drops_safety() {
    let (mut gw, id) = primary(&text_manifest());
    // 64 個非安全 command（adapter 都已 started，避免 outbound 上限先介入）。
    for i in 0..64 {
        let mid = format!("n{i}");
        let out = gw.dispatch(
            &id,
            envelope(
                &id,
                &mid,
                CharacterIntent::Work,
                TruthState::Working,
                10,
                t(1),
            ),
            t(1),
        );
        assert_eq!(sent_intents(&out), vec![mid.clone()], "{i}");
        adapter_receipt(&mut gw, &id, &mid, ReceiptStatus::Started, t(1));
    }
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64));
    // 第 65 個非安全：最舊的非安全被 cancelled{queue-full}，新的送出。
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "n64",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(2),
        ),
        t(2),
    );
    let cancelled: Vec<_> = receipts(&out)
        .into_iter()
        .filter(|r| r.status == ReceiptStatus::Cancelled)
        .collect();
    assert_eq!(cancelled[0].message_id, "n0");
    assert_eq!(cancelled[0].reason.as_deref(), Some("queue-full"));
    assert_eq!(sent_cancels(&out)[0].0, "n0");
    assert_eq!(sent_intents(&out), vec!["n64"]);
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64));

    // 安全 intent：擠掉非安全，自己一定送出。
    let mut safety = envelope(
        &id,
        "s0",
        CharacterIntent::Blocked,
        TruthState::Blocked,
        0,
        t(3),
    );
    safety.interrupt_policy = InterruptPolicy::Queue;
    let out = gw.dispatch(&id, safety, t(3));
    assert_eq!(sent_intents(&out), vec!["s0"]);
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64));

    // 全部換成安全 intent（queue 政策，避免互相搶占）。
    for i in 1..64 {
        let mut env = envelope(
            &id,
            &format!("s{i}"),
            CharacterIntent::Blocked,
            TruthState::Blocked,
            0,
            t(4),
        );
        env.interrupt_policy = InterruptPolicy::Queue;
        let out = gw.dispatch(&id, env, t(4));
        assert_eq!(sent_intents(&out), vec![format!("s{i}")]);
        adapter_receipt(&mut gw, &id, &format!("s{i}"), ReceiptStatus::Started, t(4));
    }
    adapter_receipt(&mut gw, &id, "s0", ReceiptStatus::Started, t(4));
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64));
    // 佇列全是安全 intent 時，再來一個安全 intent：不丟，交給 system.text。
    let mut env = envelope(
        &id,
        "s64",
        CharacterIntent::Emergency,
        TruthState::Emergency,
        0,
        t(5),
    );
    env.interrupt_policy = InterruptPolicy::Queue;
    let out = gw.dispatch(&id, env, t(5));
    assert_eq!(system_texts(&out), 1);
    assert!(
        receipts(&out)
            .iter()
            .all(|r| r.status != ReceiptStatus::Cancelled),
        "no safety intent cancelled"
    );
    assert_eq!(
        receipts(&out)
            .iter()
            .find(|r| r.message_id == "s64")
            .map(|r| r.status),
        Some(ReceiptStatus::Acknowledged)
    );
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64), "bounded");
    // 非安全此時被拒絕（cancelled{queue-full}），沒有無界成長。
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "n65",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(6),
        ),
        t(6),
    );
    assert!(sent_intents(&out).is_empty());
    assert_eq!(receipts(&out)[0].reason.as_deref(), Some("queue-full"));
    assert_eq!(gw.instance(&id).map(|v| v.pending), Some(64));
}

#[test]
fn outbound_cap_drops_oldest_unacknowledged_non_safety() {
    let (mut gw, id) = primary(&text_manifest());
    for i in 0..32 {
        let out = gw.dispatch(
            &id,
            envelope(
                &id,
                &format!("o{i}"),
                CharacterIntent::Work,
                TruthState::Working,
                10,
                t(1),
            ),
            t(1),
        );
        assert_eq!(sent_intents(&out).len(), 1);
    }
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "o32",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(2),
        ),
        t(2),
    );
    let cancelled: Vec<_> = receipts(&out)
        .into_iter()
        .filter(|r| r.status == ReceiptStatus::Cancelled)
        .collect();
    assert_eq!(cancelled[0].message_id, "o0");
    assert_eq!(cancelled[0].reason.as_deref(), Some("outbound-full"));
    assert_eq!(sent_intents(&out), vec!["o32"]);
    // 安全 intent 不受 outbound 上限影響（永遠送）。
    let mut safety = envelope(
        &id,
        "s1",
        CharacterIntent::Offline,
        TruthState::Offline,
        0,
        t(3),
    );
    safety.interrupt_policy = InterruptPolicy::Queue;
    let out = gw.dispatch(&id, safety, t(3));
    assert_eq!(sent_intents(&out), vec!["s1"]);
}

#[test]
fn payload_size_limits() {
    // parse_wire：> 64 KB 拒絕。
    let big = format!(
        "{{\"type\":\"heartbeat\",\"pad\":\"{}\"}}",
        "x".repeat(70_000)
    );
    assert!(matches!(
        parse_wire(big.as_bytes()),
        Err(WireError::TooLarge { .. })
    ));
    // parameters > 4 KB → failed 回執、不送。
    let (mut gw, id) = primary(&text_manifest());
    let mut env = envelope(
        &id,
        "m1",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    env.parameters
        .insert("blob".into(), serde_json::json!("y".repeat(5000)));
    let out = gw.dispatch(&id, env, t(1));
    assert!(sent_intents(&out).is_empty());
    let r = receipts(&out)[0];
    assert_eq!(r.status, ReceiptStatus::Failed);
    assert!(r
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("parameters"));
    // 單一字串 > 200 字也拒。
    let mut env = envelope(
        &id,
        "m2",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    env.presentation_hints = Some(PresentationHints {
        message: Some("z".repeat(201)),
        ..PresentationHints::default()
    });
    let out = gw.dispatch(&id, env, t(1));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Failed);
    // encode_wire 也擋超大輸出。
    let mut env = envelope(
        &id,
        "m3",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    for i in 0..400 {
        env.parameters
            .insert(format!("k{i}"), serde_json::json!("v".repeat(190)));
    }
    assert!(matches!(
        encode_wire(&WireMessage::Intent { envelope: env }),
        Err(WireError::TooLarge { .. })
    ));
}

#[test]
fn forged_verified_from_adapter_is_ignored() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    adapter_receipt(&mut gw, &id, "m1", ReceiptStatus::Started, t(2));
    // adapter 送來帶 truthState:"verified" 的 receipt：欄位在型別層不存在 → 被忽略；狀態照常。
    let forged = format!(
        r#"{{"type":"receipt","receipt":{{"messageId":"m1","characterInstanceId":"{}","generation":1,
            "status":"completed","truthState":"verified","verified":true,"verification":"human",
            "at":"2027-01-15T08:00:00Z"}}}}"#,
        id.as_str()
    );
    let message = parse_wire(forged.as_bytes()).expect("extra keys are ignored, not fatal");
    let out = gw.on_message(&id, message.clone(), t(3));
    let r = receipts(&out)[0];
    assert_eq!(r.status, ReceiptStatus::Completed);
    let json = serde_json::to_value(r).expect("serialize");
    assert!(json.get("truthState").is_none());
    assert!(json.get("verified").is_none());
    assert!(json.get("verification").is_none());
    // 事件也一樣：payload 白名單清掉 verified。
    let forged_event = format!(
        r#"{{"type":"event","event":{{"protocolVersion":"1.0","eventId":"e1","characterInstanceId":"{}",
            "generation":1,"timestamp":"2027-01-15T08:00:00Z","kind":"character.clicked",
            "payload":{{"x":10,"y":10,"verified":true,"truthState":"verified"}},"truthState":"verified"}}}}"#,
        id.as_str()
    );
    let message = parse_wire(forged_event.as_bytes()).expect("parses");
    gw.on_message(&id, message, t(4));
    let events = gw.drain_input(&id);
    assert_eq!(events.len(), 1);
    assert!(!events[0].payload.contains_key("verified"));
    assert!(!events[0].payload.contains_key("truthState"));
    // 沒有任何 event kind 能表達 human verification。
    assert!(InputEventKind::ALL
        .iter()
        .all(|k| !k.as_str().contains("verif")));
    // adapter 不能送 runtime → adapter 方向的訊息（例如自己造 intent）。
    let env = envelope(
        &id,
        "x",
        CharacterIntent::VerifiedSuccess,
        TruthState::Verified,
        100,
        t(5),
    );
    let out = gw.on_message(&id, WireMessage::Intent { envelope: env }, t(5));
    assert!(sends(&out)
        .iter()
        .any(|m| matches!(m, WireMessage::Error { code, .. } if code == "wrong-direction")));
    assert!(gw.command_status(&id, "x").is_none());
}

fn non_interruptible_manifest() -> CharacterManifest {
    let mut m = minimal_manifest("stiff", "text");
    // 只有 expression：play／notice／wait／emergency 全部落在同一個 channel 上。
    m.capabilities.insert(
        "visual.expression".into(),
        CapabilityDecl::supported().non_interruptible(),
    );
    m.intents = [
        "play",
        "notice",
        "emergency",
        "work",
        "blocked",
        "wait",
        "idle",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    m
}

#[test]
fn emergency_preempts_non_interruptible_non_safety() {
    let (mut gw, id) = primary(&non_interruptible_manifest());
    // play（非安全）在 expression 上演出中，且不可中斷。
    let mut play = envelope(
        &id,
        "play",
        CharacterIntent::Play,
        TruthState::None,
        40,
        t(1),
    );
    play.interrupt_policy = InterruptPolicy::Queue;
    gw.dispatch(&id, play, t(1));
    adapter_receipt(&mut gw, &id, "play", ReceiptStatus::Started, t(2));
    assert_eq!(
        gw.negotiated(&id)
            .and_then(|n| n.resolutions.get(&CharacterIntent::Play))
            .and_then(|r| r.via.clone())
            .map(|v| v.0),
        Some("visual.expression".into())
    );
    // notice（floor 0）priority 45 > 40，但 play 不可中斷且 notice floor < 75 → 不搶占。
    let notice = envelope(
        &id,
        "notice",
        CharacterIntent::Notice,
        TruthState::None,
        45,
        t(3),
    );
    let mut notice = notice;
    notice.interrupt_policy = InterruptPolicy::Preempt;
    let out = gw.dispatch(&id, notice, t(3));
    assert!(sent_cancels(&out).is_empty());
    assert_eq!(gw.command_status(&id, "play"), Some(ReceiptStatus::Started));
    // wait（floor 60 < 75）也不能搶占不可中斷的演出。
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "wait",
            CharacterIntent::Wait,
            TruthState::Queued,
            0,
            t(4),
        ),
        t(4),
    );
    assert!(sent_cancels(&out).is_empty());
    // emergency（floor 100 ≥ 75）搶占：play → cancelled{preempted}，送 cancel，emergency 送出。
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "em",
            CharacterIntent::Emergency,
            TruthState::Emergency,
            0,
            t(5),
        ),
        t(5),
    );
    let preempted: Vec<_> = receipts(&out)
        .into_iter()
        .filter(|r| r.reason.as_deref() == Some("preempted"))
        .collect();
    assert!(preempted.iter().any(|r| r.message_id == "play"));
    assert!(preempted
        .iter()
        .all(|r| r.status == ReceiptStatus::Cancelled));
    assert!(sent_cancels(&out)
        .iter()
        .any(|(m, r)| m == "play" && r.as_deref() == Some("preempted")));
    assert_eq!(sent_intents(&out), vec!["em"]);
    match sends(&out)
        .iter()
        .find(|m| matches!(m, WireMessage::Intent { .. }))
    {
        Some(WireMessage::Intent { envelope }) => assert_eq!(envelope.priority, 100),
        _ => panic!("emergency not sent"),
    }
    // custom channel 不能影響搶占：宣告 custom channel 的 command 仍照 canonical 規則。
    assert_eq!(
        gw.command_status(&id, "play"),
        Some(ReceiptStatus::Cancelled)
    );
}

#[test]
fn resume_previous_after_safety_presentation() {
    let (mut gw, id) = primary(&text_manifest());
    let mut idle = envelope(
        &id,
        "idle",
        CharacterIntent::Rest,
        TruthState::None,
        10,
        t(1),
    );
    idle.resume_policy = ResumePolicy::ResumePrevious;
    gw.dispatch(&id, idle, t(1));
    adapter_receipt(&mut gw, &id, "idle", ReceiptStatus::Started, t(2));
    // blocked 走 textBubble；rest 走 presence（transform）— 不同 channel。改用 ask 搶占 rest？
    // rest 的能力鏈：pose ✗ expression ✗ presence ✓ → transform channel；用 offline（presence 在鏈上）驗證。
    let mut m2 = text_manifest();
    m2.capabilities.remove("visual.textBubble");
    let (mut gw2, id2) = primary(&m2);
    let mut rest = envelope(
        &id2,
        "rest",
        CharacterIntent::Rest,
        TruthState::None,
        10,
        t(1),
    );
    rest.resume_policy = ResumePolicy::ResumePrevious;
    gw2.dispatch(&id2, rest, t(1));
    adapter_receipt(&mut gw2, &id2, "rest", ReceiptStatus::Started, t(2));
    let out = gw2.dispatch(
        &id2,
        envelope(
            &id2,
            "off",
            CharacterIntent::Offline,
            TruthState::Offline,
            0,
            t(3),
        ),
        t(3),
    );
    assert!(receipts(&out)
        .iter()
        .any(|r| r.message_id == "rest" && r.reason.as_deref() == Some("preempted")));
    adapter_receipt(&mut gw2, &id2, "off", ReceiptStatus::Started, t(4));
    let out = adapter_receipt(&mut gw2, &id2, "off", ReceiptStatus::Completed, t(5));
    assert_eq!(sent_intents(&out), vec!["rest/resume1"]);
    assert!(audits(&out).iter().any(|a| a.contains("re-dispatching")));
    let _ = (&mut gw, &id);
}

#[test]
fn failed_safety_intent_falls_back_to_system_text() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "b1",
            CharacterIntent::Blocked,
            TruthState::Blocked,
            0,
            t(1),
        ),
        t(1),
    );
    let out = adapter_receipt(&mut gw, &id, "b1", ReceiptStatus::Failed, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Failed);
    assert_eq!(receipts(&out)[0].resolution, Some(Resolution::Failed));
    assert_eq!(system_texts(&out), 1);
    match out
        .iter()
        .find(|o| matches!(o, GatewayOutput::SystemText { .. }))
    {
        Some(GatewayOutput::SystemText {
            intent,
            truth_state,
            message,
            correlation_id,
            ..
        }) => {
            assert_eq!(*intent, CharacterIntent::Blocked);
            assert_eq!(*truth_state, TruthState::Blocked);
            assert!(message.contains("安全政策"));
            assert_eq!(correlation_id.as_deref(), Some("corr-1"));
        }
        _ => panic!("no system text"),
    }
    // 非安全 failed 不回退。
    gw.dispatch(
        &id,
        envelope(
            &id,
            "w1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(3),
        ),
        t(3),
    );
    let out = adapter_receipt(&mut gw, &id, "w1", ReceiptStatus::Failed, t(4));
    assert_eq!(system_texts(&out), 0);
}

#[test]
fn zero_capability_instance_uses_system_text_for_safety_only() {
    let m = minimal_manifest("nothing", "text");
    let (mut gw, id) = primary(&m);
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "s1",
            CharacterIntent::RequestConsent,
            TruthState::WaitingConsent,
            0,
            t(1),
        ),
        t(1),
    );
    assert!(sent_intents(&out).is_empty());
    assert_eq!(system_texts(&out), 1);
    let r = receipts(&out)[0];
    assert_eq!(r.status, ReceiptStatus::Acknowledged);
    assert_eq!(r.resolution, Some(Resolution::Substituted));
    // acknowledged → sweep → uncertain（Gateway 不知道文字是否真的被看見，不猜 completed）。
    let out = gw.sweep(t(7));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
    // 非安全 → unsupported。
    let out = gw.dispatch(
        &id,
        envelope(&id, "n1", CharacterIntent::Play, TruthState::None, 10, t(2)),
        t(2),
    );
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Unsupported);
    assert_eq!(system_texts(&out), 0);
    // 未協商的 instance：同樣規則。
    let mut gw = Gateway::default();
    let id = gw.register_instance(text_manifest(), CharacterRole::PrimaryCompanion);
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "s2",
            CharacterIntent::Emergency,
            TruthState::Emergency,
            0,
            t(1),
        ),
        t(1),
    );
    assert_eq!(system_texts(&out), 1);
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "n2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Unsupported);
    assert_eq!(
        receipts(&out)[0].detail.as_deref(),
        Some("instance not negotiated")
    );
}

#[test]
fn multi_instance_safety_dedupe_per_role_class_plus_notification_only() {
    let m = text_manifest();
    let mut gw = Gateway::default();
    let a = connect(&mut gw, &m, CharacterRole::PrimaryCompanion);
    let b = connect(&mut gw, &m, CharacterRole::PrimaryCompanion);
    let fam = connect(&mut gw, &m, CharacterRole::Familiar);
    let notify = connect(&mut gw, &m, CharacterRole::NotificationOnly);
    let env = |id: &InstanceId, mid: &str| {
        envelope(
            id,
            mid,
            CharacterIntent::Blocked,
            TruthState::Blocked,
            0,
            t(1),
        )
    };
    assert_eq!(
        sent_intents(&gw.dispatch(&a, env(&a, "a1"), t(1))),
        vec!["a1"]
    );
    let out = gw.dispatch(&b, env(&b, "b1"), t(1));
    assert!(
        sent_intents(&out).is_empty(),
        "same role class → suppressed"
    );
    assert_eq!(
        receipts(&out)[0].reason.as_deref(),
        Some("safety-deduplicated")
    );
    assert_eq!(
        sent_intents(&gw.dispatch(&fam, env(&fam, "f1"), t(1))),
        vec!["f1"],
        "other role class delivered"
    );
    assert_eq!(
        sent_intents(&gw.dispatch(&notify, env(&notify, "n1"), t(1))),
        vec!["n1"],
        "notification-only always delivered"
    );
    // 同一 instance 後續同 correlation 的安全 intent 仍送給它。
    let follow = envelope(
        &a,
        "a2",
        CharacterIntent::Failed,
        TruthState::Failed,
        0,
        t(2),
    );
    assert_eq!(sent_intents(&gw.dispatch(&a, follow, t(2))), vec!["a2"]);
    // A 斷線後，B 接手。
    gw.on_disconnect(&a, DisconnectReason::Crash, t(3));
    let follow = envelope(
        &b,
        "b2",
        CharacterIntent::Failed,
        TruthState::Failed,
        0,
        t(4),
    );
    assert_eq!(sent_intents(&gw.dispatch(&b, follow, t(4))), vec!["b2"]);
    // 非安全 intent 不去重（每個角色都可以各自表現）。
    let play_a = envelope(
        &a,
        "pa",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(5),
    );
    let play_b = envelope(
        &b,
        "pb",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(5),
    );
    let _ = gw.dispatch(&a, play_a, t(5));
    assert_eq!(sent_intents(&gw.dispatch(&b, play_b, t(5))), vec!["pb"]);
    // 沒有 correlationId 的安全 intent 不去重。
    let mut no_corr = envelope(
        &b,
        "nc",
        CharacterIntent::Unknown,
        TruthState::Unknown,
        0,
        t(6),
    );
    no_corr.correlation_id = None;
    assert_eq!(sent_intents(&gw.dispatch(&b, no_corr, t(6))), vec!["nc"]);
}

#[test]
fn merge_and_drop_if_busy_policies() {
    let (mut gw, id) = primary(&text_manifest());
    let mut first = envelope(
        &id,
        "w1",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(1),
    );
    first.interrupt_policy = InterruptPolicy::Merge;
    gw.dispatch(&id, first, t(1));
    let mut second = envelope(
        &id,
        "w2",
        CharacterIntent::Work,
        TruthState::Working,
        10,
        t(2),
    );
    second.interrupt_policy = InterruptPolicy::Merge;
    let out = gw.dispatch(&id, second, t(2));
    assert!(sent_intents(&out).is_empty());
    let r = receipts(&out)[0];
    assert_eq!(r.status, ReceiptStatus::Cancelled);
    assert_eq!(r.reason.as_deref(), Some("merged"));
    assert!(r.detail.as_deref().unwrap_or_default().contains("w1"));
    // drop-if-busy：bubble channel 被 w1 佔用（accepted 即算 busy）。
    let mut busy = envelope(
        &id,
        "a1",
        CharacterIntent::Ask,
        TruthState::WaitingInput,
        0,
        t(3),
    );
    busy.interrupt_policy = InterruptPolicy::DropIfBusy;
    let out = gw.dispatch(&id, busy, t(3));
    assert!(sent_intents(&out).is_empty());
    assert_eq!(receipts(&out)[0].reason.as_deref(), Some("busy"));
}

#[test]
fn rate_limit_and_wrong_direction_are_enforced() {
    let (mut gw, id) = primary(&text_manifest());
    let mut limited = 0;
    for _ in 0..60 {
        let out = gw.on_message(&id, WireMessage::Heartbeat { generation: None }, t(10));
        if sends(&out)
            .iter()
            .any(|m| matches!(m, WireMessage::Error { code, .. } if code == "rate-limited"))
        {
            limited += 1;
        }
    }
    // 協商用掉 1 個 token（t(0) 到 t(10) 已補滿），60 則中 50 則通過。
    assert_eq!(limited, 10);
    // 1 秒後又可以送。
    let out = gw.on_message(&id, WireMessage::Heartbeat { generation: None }, t(11));
    assert!(sends(&out).is_empty());
}

#[test]
fn role_filtering_and_input_flow_through_gateway() {
    let m = text_manifest();
    let mut gw = Gateway::default();
    let observer = connect(&mut gw, &m, CharacterRole::Observer);
    let companion = connect(&mut gw, &m, CharacterRole::PrimaryCompanion);
    let event = |id: &InstanceId, generation: u64| CharacterInputEvent {
        protocol_version: "1.0".into(),
        event_id: "e".into(),
        character_instance_id: id.0.clone(),
        generation,
        timestamp: t(1),
        kind: InputEventKind::TextSubmitted,
        payload: [("text".to_string(), serde_json::json!("hi"))]
            .into_iter()
            .collect(),
        privacy_class: PrivacyClass::Internal,
    };
    assert_eq!(
        gw.on_event(&observer, event(&observer, 1), t(1)),
        InputDecision::Dropped(InputDropReason::RoleFiltered)
    );
    assert!(gw.drain_input(&observer).is_empty());
    assert_eq!(
        gw.on_event(&companion, event(&companion, 1), t(1)),
        InputDecision::Queued
    );
    let drained = gw.drain_input(&companion);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].privacy_class, PrivacyClass::Personal);
    // 送錯 instance id 的事件被丟。
    assert!(matches!(
        gw.on_event(&companion, event(&observer, 1), t(1)),
        InputDecision::Dropped(InputDropReason::InvalidPayload { .. })
    ));
    // 透過 on_message 的丟棄會留下 audit。
    let out = gw.on_message(
        &observer,
        WireMessage::Event {
            event: event(&observer, 1),
        },
        t(2),
    );
    assert!(audits(&out).iter().any(|a| a.contains("role-filtered")));
}

#[test]
fn remove_instance_marks_pending_uncertain_and_forgets_instance() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    let out = gw.remove_instance(&id, t(2));
    assert!(receipts(&out)
        .iter()
        .any(|r| r.message_id == "m1" && r.status == ReceiptStatus::Uncertain));
    assert!(gw.instance(&id).is_none());
    assert!(audits(&gw.dispatch(
        &id,
        envelope(
            &id,
            "m2",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(3)
        ),
        t(3)
    ))
    .iter()
    .any(|a| a.contains("unknown instance")));
}

#[test]
fn wire_round_trip_of_gateway_outputs() {
    let (mut gw, id) = primary(&text_manifest());
    let out = gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Ask,
            TruthState::WaitingInput,
            0,
            t(1),
        ),
        t(1),
    );
    for o in &out {
        let json = serde_json::to_string(o).expect("serialize");
        let back: GatewayOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, o);
    }
    let sent = sends(&out)[0];
    let bytes = encode_wire(sent).expect("encode");
    let parsed = parse_wire(&bytes).expect("parse");
    assert_eq!(&parsed, sent);
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["type"], "intent");
    assert_eq!(v["envelope"]["intent"], "ask");
    assert_eq!(v["envelope"]["truthState"], "waiting-input");
    assert_eq!(v["envelope"]["priority"], 60);
}

// ---------------------------------------------------------------------------
// Reduced Motion 只有一個主人（每個 instance 由可信 host 設定）＋回執只能誠實變差
// ---------------------------------------------------------------------------

#[test]
fn reduced_motion_is_per_instance_and_drives_negotiation() {
    let m = shu_manifest();
    let mut gw = Gateway::default();
    let id = gw.register_instance(m.clone(), CharacterRole::PrimaryCompanion);
    // 預設（config）：非 reduced。
    assert_eq!(gw.reduced_motion(&id), Some(false));
    assert!(!gw.hello_for(&id).expect("hello").reduced_motion);

    // 可信 host 回報 Reduced Motion → hello 與協商都跟著變。
    assert!(gw.set_reduced_motion(&id, true));
    assert_eq!(gw.reduced_motion(&id), Some(true));
    assert!(gw.hello_for(&id).expect("hello").reduced_motion);
    let (negotiated, _) = gw
        .on_negotiate(&id, Negotiate::from_manifest(&m, 1), t(0))
        .expect("negotiates");
    assert!(negotiated.reduced_motion, "negotiated 必須回報真值");
    assert_eq!(
        negotiated
            .resolutions
            .get(&CharacterIntent::Notice)
            .map(|r| r.resolution),
        Some(Resolution::Reduced),
        "reducedMotionBehavior=reduced 的能力必須解析成 reduced，不能假裝 exact"
    );
    // 不存在的 instance：誠實回 false／None，不 panic。
    assert!(!gw.set_reduced_motion(&InstanceId("nope".into()), true));
    assert_eq!(gw.reduced_motion(&InstanceId("nope".into())), None);
}

#[test]
fn receipt_resolution_may_degrade_but_never_upgrade() {
    let m = shu_manifest();
    let mut gw = Gateway::default();
    let id = gw.register_instance(m.clone(), CharacterRole::PrimaryCompanion);
    gw.set_reduced_motion(&id, true);
    gw.on_negotiate(&id, Negotiate::from_manifest(&m, 1), t(0))
        .expect("negotiates");
    let generation = gw.generation(&id).unwrap_or(0);
    gw.dispatch(
        &id,
        envelope(
            &id,
            "m1",
            CharacterIntent::Notice,
            TruthState::None,
            10,
            t(1),
        ),
        t(1),
    );
    // adapter 謊報 exact：協商是 reduced → 仍回 reduced。
    let out = gw.on_receipt(
        &id,
        CommandReceipt::new("m1", id.as_str(), generation, ReceiptStatus::Started, t(2))
            .with_resolution(Resolution::Exact),
        t(2),
    );
    assert_eq!(receipts(&out)[0].resolution, Some(Resolution::Reduced));

    // adapter 誠實降級（substituted 比協商的 reduced 好 → 取較差者；unsupported 更差 → 採用）。
    let out = gw.on_receipt(
        &id,
        CommandReceipt::new(
            "m1",
            id.as_str(),
            generation,
            ReceiptStatus::Completed,
            t(3),
        )
        .with_resolution(Resolution::Substituted),
        t(3),
    );
    assert_eq!(receipts(&out)[0].resolution, Some(Resolution::Reduced));

    // 非 reduced 的實例：adapter 回 reduced → 誠實採用（不是覆蓋成 exact）。
    let mut gw2 = Gateway::default();
    let id2 = connect(&mut gw2, &m, CharacterRole::PrimaryCompanion);
    let generation2 = gw2.generation(&id2).unwrap_or(0);
    gw2.dispatch(
        &id2,
        envelope(
            &id2,
            "m2",
            CharacterIntent::Notice,
            TruthState::None,
            10,
            t(1),
        ),
        t(1),
    );
    let out = gw2.on_receipt(
        &id2,
        CommandReceipt::new(
            "m2",
            id2.as_str(),
            generation2,
            ReceiptStatus::Started,
            t(2),
        )
        .with_resolution(Resolution::Reduced),
        t(2),
    );
    assert_eq!(receipts(&out)[0].resolution, Some(Resolution::Reduced));
}

// ---------------------------------------------------------------------------
// 速率限制對所有入口一致（HTTP／WS 共用）＋畸形訊息先計費、稽核有界
// ---------------------------------------------------------------------------

#[test]
fn rate_budget_is_shared_by_every_entry_point() {
    let (mut gw, id) = primary(&text_manifest());
    // 50 則/s：前 50 則過，第 51 則起被擋。
    for i in 0..50 {
        assert!(gw.allow_message(&id, t(1)), "第 {i} 則應在預算內");
    }
    assert!(!gw.allow_message(&id, t(1)));
    // on_message 走同一個預算（已用完 → rate-limited）。
    let out = gw.on_message(&id, WireMessage::Heartbeat { generation: None }, t(1));
    assert!(audits(&out).iter().any(|a| a.contains("rate-limited")));
    // 下一秒回補。
    assert!(gw.allow_message(&id, t(2)));
    // 未知 instance：不 panic，交給呼叫端回 404。
    assert!(gw.allow_message(&InstanceId("nope".into()), t(1)));
}

#[test]
fn malformed_frames_are_charged_first_and_audited_at_most_once_per_window() {
    let (mut gw, id) = primary(&text_manifest());
    let first = gw.note_wire_rejected(&id, t(1));
    assert!(first.within_rate);
    assert!(first.audit, "第一則畸形訊息要留下稽核");
    assert_eq!(first.suppressed, 0);
    // 同一個 5 秒視窗內：只計數、不再寫稽核。
    for _ in 0..40 {
        let v = gw.note_wire_rejected(&id, t(1));
        assert!(!v.audit);
    }
    // 一直丟垃圾也會吃掉 50 則/s 的預算（不能用畸形訊息繞過限制）。
    for _ in 0..20 {
        gw.note_wire_rejected(&id, t(1));
    }
    assert!(
        !gw.note_wire_rejected(&id, t(1)).within_rate,
        "畸形訊息必須先扣速率預算"
    );
    // 視窗過了：寫一列稽核，並帶出被壓下的次數。
    let later = gw.note_wire_rejected(&id, t(7));
    assert!(later.audit);
    assert!(later.suppressed >= 40, "suppressed={}", later.suppressed);
    assert_eq!(
        gw.note_wire_rejected(&id, t(7)).suppressed,
        0,
        "同一視窗第二則不再寫稽核"
    );
}

#[test]
fn disconnect_hands_in_flight_safety_intent_to_system_text() {
    let (mut gw, id) = primary(&text_manifest());
    gw.dispatch(
        &id,
        envelope(
            &id,
            "e1",
            CharacterIntent::Emergency,
            TruthState::Emergency,
            100,
            t(1),
        ),
        t(1),
    );
    adapter_receipt(&mut gw, &id, "e1", ReceiptStatus::Started, t(2));
    // adapter 在演出中掛掉：uncertain（不猜 completed）＋安全訊息以 system.text 補送。
    let out = gw.on_disconnect(&id, DisconnectReason::Crash, t(3));
    let uncertain: Vec<_> = receipts(&out)
        .into_iter()
        .filter(|r| r.status == ReceiptStatus::Uncertain)
        .collect();
    assert_eq!(uncertain.len(), 1);
    assert_eq!(uncertain[0].message_id, "e1");
    assert_eq!(uncertain[0].reason.as_deref(), Some("crash"));
    assert_eq!(system_texts(&out), 1, "安全 intent 不得因斷線而遺失");
    let handed = out.iter().find_map(|o| match o {
        GatewayOutput::SystemText {
            message_id, intent, ..
        } => Some((message_id.clone(), *intent)),
        _ => None,
    });
    let (message_id, intent) = handed.expect("system.text output");
    assert_eq!(intent, CharacterIntent::Emergency);
    assert_eq!(message_id, "e1/system-text");
    assert!(audits(&out)
        .iter()
        .any(|a| a.contains("falling back to system.text")));

    // 非安全 intent 不補送（只有 uncertain）。
    let (mut gw2, id2) = primary(&text_manifest());
    gw2.dispatch(
        &id2,
        envelope(
            &id2,
            "w1",
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t(1),
        ),
        t(1),
    );
    let out = gw2.on_disconnect(&id2, DisconnectReason::TransportClosed, t(2));
    assert_eq!(receipts(&out)[0].status, ReceiptStatus::Uncertain);
    assert_eq!(system_texts(&out), 0);
}

#[test]
fn merged_cancel_receipt_from_the_desktop_settles_without_waiting_for_expiry() {
    // 桌面 TS Gateway 把「併入既有演出」回報成 cancelled{merged}（不是 completed）。
    // Rust 端必須接受這個轉移並就地終結，不能當成非法轉移丟掉、讓命令一路掛到過期。
    let (mut gw, id) = primary(&text_manifest());
    let mut first = envelope(
        &id,
        "n1",
        CharacterIntent::Notice,
        TruthState::None,
        40,
        t(1),
    );
    first.correlation_id = Some("receptor:a".into());
    first.interrupt_policy = InterruptPolicy::Merge;
    gw.dispatch(&id, first, t(1));
    let mut second = envelope(
        &id,
        "n2",
        CharacterIntent::Notice,
        TruthState::None,
        40,
        t(2),
    );
    second.correlation_id = Some("receptor:b".into());
    second.interrupt_policy = InterruptPolicy::Merge;
    let out = gw.dispatch(&id, second, t(2));
    assert_eq!(
        sent_intents(&out),
        vec!["n2".to_string()],
        "不同 correlation 不是同一件事：Rust 也會送出去"
    );

    let generation = gw.generation(&id).unwrap_or(0);
    let out = gw.on_receipt(
        &id,
        CommandReceipt::new(
            "n2",
            id.as_str(),
            generation,
            ReceiptStatus::Cancelled,
            t(3),
        )
        .with_reason("merged"),
        t(3),
    );
    assert!(
        !audits(&out)
            .iter()
            .any(|a| a.contains("illegal transition")),
        "cancelled{{merged}} 是合法轉移：{:?}",
        audits(&out)
    );
    let r = receipts(&out);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].status, ReceiptStatus::Cancelled);
    assert_eq!(r[0].reason.as_deref(), Some("merged"));
    assert_eq!(gw.command_status(&id, "n2"), Some(ReceiptStatus::Cancelled));

    // 過期掃描不會再對它做任何事（不會補一筆 expired，也不會送 cancel 給 adapter）。
    let out = gw.sweep(t(200));
    assert!(
        receipts(&out).iter().all(|r| r.message_id != "n2"),
        "已終結的命令不該再被 sweep 記一次"
    );
    assert!(sent_cancels(&out).iter().all(|(id, _)| id != "n2"));
}
