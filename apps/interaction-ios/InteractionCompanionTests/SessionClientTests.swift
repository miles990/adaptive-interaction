//
//  SessionClientTests.swift
//  InteractionCompanionTests
//
//  AIP Character Session 手機端的純決策測試（`SessionDecisions`／`SemanticJSON`／
//  `CharacterPresentation`）。全部是純函式：不開連線、不碰時鐘、不需要模擬器以外的東西。
//
//  跨語言證據：state 的 canonical hash 直接對 Rust 那份 conformance fixture
//  （`crates/interaction-aip/tests/fixtures/state-*.json`，由 codegen 內嵌成 `AIPFixtures`）
//  驗證——同一份 state 在 Rust 與 Swift 必須算出同一個 SHA-256，patch 串接後也必須續得上。
//

import SwiftUI
import XCTest

@testable import InteractionCompanion

final class SessionClientTests: XCTestCase {

    // MARK: - 工具

    private func fixture(_ name: String) throws -> SemanticJSON {
        let text = try XCTUnwrap(AIPFixtures.files[name], "缺少內嵌 fixture \(name)")
        return try XCTUnwrap(SemanticJSON.parse(text), "fixture \(name) 解析失敗")
    }

    private func payload(_ name: String) throws -> SemanticJSON {
        try XCTUnwrap(fixture(name)["payload"], "fixture \(name) 沒有 payload")
    }

    private func json(_ text: String) throws -> [String: Any] {
        let data = try XCTUnwrap(text.data(using: .utf8))
        return try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// 從 JSON 文字做一個已通過完整 AIP 驗證的信封。
    private func envelope(_ text: String) throws -> AIPEnvelope {
        switch AIPCheck.evaluate(Data(text.utf8)) {
        case .success(let envelope):
            return envelope
        case .failure(let error):
            XCTFail("信封應該通過驗證，卻回 \(error.code.rawValue)")
            throw error
        }
    }

    private func iso(_ date: Date) -> String { WireTime.nowISO8601(date) }

    /// 一則 Behavior Intent command。
    private func intentEnvelope(
        messageId: String = "aip-1-1",
        intent: String = "react-happily-to-touch",
        intensity: Double = 0.45,
        expiresAt: Date
    ) throws -> AIPEnvelope {
        try envelope(
            """
            {"specVersion":"aip/1.0","messageId":"\(messageId)","messageType":"command",
             "name":"character.behavior.request",
             "source":{"kind":"runtime","id":"runtime"},
             "target":{"kind":"device","id":"iphone-1"},
             "sessionId":"session.home","correlationId":"flow-1",
             "occurredAt":"2026-09-04T12:30:00.000Z","expiresAt":"\(iso(expiresAt))",
             "sequence":9,
             "payload":{"intent":"\(intent)","intensity":\(intensity),"interruptible":true,
                        "origin":"interaction","hints":{}}}
            """)
    }

    // MARK: - Canonical hash（跨語言）

    func testSnapshotStateHashMatchesTheRustFixture() throws {
        let snapshot = try payload("state-snapshot.json")
        let state = try XCTUnwrap(snapshot["state"])
        let expected = try XCTUnwrap(snapshot["hash"]?.stringValue)
        XCTAssertEqual(
            state.canonicalSHA256, expected,
            "Swift 與 Rust 對同一份 state 必須算出同一個 canonical SHA-256")
    }

    /// 數字字面必須逐字保留：`0` 與 `0.0` 是同一個值、不同的 canonical 文字。
    /// 這正是 host（serde_json 的 f64 會寫 `0.0`）與 App 之間 hash 對不對得上的關鍵。
    func testNumberLiteralsAreKeptVerbatimSoHashesMatchTheHost() throws {
        let integerForm = try XCTUnwrap(SemanticJSON.parse(#"{"intensity":0}"#))
        let floatForm = try XCTUnwrap(SemanticJSON.parse(#"{"intensity":0.0}"#))
        XCTAssertEqual(integerForm.canonicalJSON, #"{"intensity":0}"#)
        XCTAssertEqual(floatForm.canonicalJSON, #"{"intensity":0.0}"#)
        XCTAssertNotEqual(integerForm.canonicalSHA256, floatForm.canonicalSHA256)
        // 值本身仍然讀得出來（呈現層用的是值，不是字面）。
        XCTAssertEqual(integerForm["intensity"]?.doubleValue, 0)
        XCTAssertEqual(floatForm["intensity"]?.doubleValue, 0)
    }

    func testCanonicalJsonSortsKeysByUtf8AndOmitsWhitespace() throws {
        let value = try XCTUnwrap(
            SemanticJSON.parse(#"{ "b" : { "y" : 1 , "x" : [ 3 , { "q" : 1 , "p" : 2 } ] } , "a" : "z" }"#))
        XCTAssertEqual(value.canonicalJSON, #"{"a":"z","b":{"x":[3,{"p":2,"q":1}],"y":1}}"#)
    }

    // MARK: - Snapshot／Patch／Rollback／Epoch／Hash

    func testSnapshotIsAppliedWholeAndBecomesTheLocalState() throws {
        let message = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-snapshot.json")))
        guard case .applied(let revision, let epoch, let state) = SessionDecisions.apply(
            message, to: SessionSyncLocal())
        else {
            return XCTFail("全新的 session 收到 snapshot 應該整份套用")
        }
        XCTAssertEqual(revision, 204)
        XCTAssertEqual(epoch, 3)
        let projected = try XCTUnwrap(CharacterSemanticState.project(state))
        XCTAssertEqual(projected.characterId, "ref-shape")
        XCTAssertEqual(projected.mood, .neutral)
        XCTAssertEqual(projected.activity, .idle)
        XCTAssertEqual(projected.truth, CharacterTruth.none)
        XCTAssertEqual(projected.members.count, 1)
        XCTAssertEqual(projected.members.first?.presence, "online")
    }

    /// snapshot → patch 串接：baseRevision 對得上就套用，而且套用後的 hash 必須等於
    /// host 算出來的那一個（Rust fixture 的 `a3954c…`）。
    func testPatchWithMatchingBaseRevisionIsAppliedAndHashChainsFromTheSnapshot() throws {
        var local = SessionSyncLocal()
        let snapshot = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-snapshot.json")))
        guard case .applied(let revision, let epoch, let state) = SessionDecisions.apply(
            snapshot, to: local)
        else { return XCTFail("snapshot 應該套用") }
        local.revision = revision
        local.epoch = epoch
        local.state = state

        let patch = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-patch.json"), baseRevision: 204))
        guard case .applied(let next, _, let merged) = SessionDecisions.apply(patch, to: local) else {
            return XCTFail("baseRevision 對得上的 patch 應該套用（hash 也必須對得上）")
        }
        XCTAssertEqual(next, 205)
        let projected = try XCTUnwrap(CharacterSemanticState.project(merged))
        XCTAssertEqual(projected.mood, .happy)
        XCTAssertEqual(projected.moodIntensity, 0.45, accuracy: 0.0001)
        XCTAssertEqual(projected.activity, .reacting)
        XCTAssertEqual(projected.lastInteractionKind, "tap")
    }

    func testPatchWithAWrongBaseRevisionIsNotAppliedAndAsksForResume() throws {
        var local = SessionSyncLocal()
        local.revision = 199  // 落後：baseRevision 204 接不上
        local.state = try XCTUnwrap(fixture("state-snapshot.json")["payload"]?["state"])
        let patch = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-patch.json"), baseRevision: 204))
        XCTAssertEqual(SessionDecisions.apply(patch, to: local), .needsResume)
    }

    func testAPatchAppliedOntoTheWrongContentIsRejectedByTheHash() throws {
        var local = SessionSyncLocal()
        local.revision = 204
        // 內容被動過（activity 不是 idle）：baseRevision 對得上，但套用後 hash 一定不同。
        local.state = try XCTUnwrap(
            SemanticJSON.parse(#"{"characterId":"ref-shape","activity":"working"}"#))
        let patch = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-patch.json"), baseRevision: 204))
        XCTAssertEqual(
            SessionDecisions.apply(patch, to: local), .needsSnapshot,
            "hash 對不上就要丟掉本地狀態、重新要一份完整快照")
    }

    func testStateOlderThanOrEqualToTheLocalOneIsIgnored() throws {
        var local = SessionSyncLocal()
        local.revision = 300
        local.epoch = 3
        local.state = try XCTUnwrap(fixture("state-snapshot.json")["payload"]?["state"])
        let snapshot = try XCTUnwrap(
            SessionDecisions.stateMessage(payload: try payload("state-snapshot.json")))
        XCTAssertEqual(
            SessionDecisions.apply(snapshot, to: local), .ignoredRollback,
            "比本地舊的狀態一律忽略（rollback 防護）")

        local.revision = 204
        XCTAssertEqual(SessionDecisions.apply(snapshot, to: local), .ignoredAlreadyApplied)
    }

    /// host 明說重建了 session（`reason: session-reset` 且 epoch 更新）：
    /// 即使 revision 比本地小也要接受，並丟棄本地狀態。
    func testSessionResetWithANewerEpochIsAcceptedEvenThoughTheRevisionGoesBackwards() throws {
        var local = SessionSyncLocal()
        local.revision = 300
        local.epoch = 3
        let resetPayload = try XCTUnwrap(
            SemanticJSON.parse(
                """
                {"kind":"snapshot","reason":"session-reset","revision":1,"sequence":1,
                 "sessionEpoch":4,
                 "state":{"characterId":"ref-shape","mood":{"kind":"neutral","intensity":0.0},
                          "activity":"idle","attention":{"kind":"none"},"truth":{"state":"none"},
                          "members":[],"reducedMotion":false}}
                """))
        let message = try XCTUnwrap(SessionDecisions.stateMessage(payload: resetPayload))
        guard case .applied(let revision, let epoch, _) = SessionDecisions.apply(message, to: local)
        else { return XCTFail("session-reset ＋ 新 epoch 必須被接受") }
        XCTAssertEqual(revision, 1)
        XCTAssertEqual(epoch, 4)

        // 沒有 `reason` 的舊 epoch 快照仍然是 rollback，不得接受。
        let stale = try XCTUnwrap(
            SessionDecisions.stateMessage(
                payload: try XCTUnwrap(
                    SemanticJSON.parse(
                        #"{"kind":"snapshot","revision":1,"sessionEpoch":4,"state":{"characterId":"x"}}"#
                    ))))
        XCTAssertEqual(SessionDecisions.apply(stale, to: local), .ignoredRollback)
    }

    // MARK: - resume 連敗

    func testThreeResumeAttemptsWithoutSyncingBecomeUnrecoverable() {
        var local = SessionSyncLocal()
        SessionDecisions.noteResumeAttempt(&local)
        XCTAssertFalse(local.unrecoverable)
        SessionDecisions.noteResumeAttempt(&local)
        XCTAssertFalse(local.unrecoverable)
        SessionDecisions.noteResumeAttempt(&local)
        XCTAssertTrue(local.unrecoverable, "連續三次沒補齊就必須誠實說「無法恢復」")
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: local, connected: true, negotiated: true, hasUnsupportedIntents: false,
                resuming: true),
            .unrecoverable)
        XCTAssertEqual(SessionSyncStatus.unrecoverable.text, "無法恢復，請重新連接")

        // 只要成功對齊過一次，計數就歸零。
        SessionDecisions.noteSyncSucceeded(&local)
        XCTAssertFalse(local.unrecoverable)
        XCTAssertEqual(local.resumeFailures, 0)
    }

    // MARK: - Behavior Intent

    func testAnExpiredIntentIsNotPlayed() throws {
        let now = Date()
        let envelope = try intentEnvelope(expiresAt: now.addingTimeInterval(-1))
        XCTAssertEqual(
            SessionDecisions.intentOutcome(
                envelope: envelope, now: now, negotiated: true, alreadySeen: false),
            .expired)
    }

    func testTheSameIntentMessageIsNeverPlayedTwice() throws {
        let now = Date()
        let envelope = try intentEnvelope(expiresAt: now.addingTimeInterval(10))
        var ring = AIPDedupeRing()
        XCTAssertTrue(ring.note(envelope.messageId))
        XCTAssertEqual(
            SessionDecisions.intentOutcome(
                envelope: envelope, now: now, negotiated: true,
                alreadySeen: !ring.note(envelope.messageId)),
            .duplicate)
    }

    func testAnUnsupportedIntentIsRejectedAndNeverReportedAsObserved() throws {
        let now = Date()
        let envelope = try intentEnvelope(
            intent: "dance-a-jig", expiresAt: now.addingTimeInterval(10))
        XCTAssertEqual(
            SessionDecisions.intentOutcome(
                envelope: envelope, now: now, negotiated: true, alreadySeen: false),
            .unsupported("dance-a-jig"))

        let result = try XCTUnwrap(
            SessionDecisions.resultEnvelope(
                causationId: envelope.messageId, status: .rejected, code: .unsupportedCapability,
                deviceId: "iphone-1", sessionId: "session.home", messageId: "ios-res-1", now: now))
        let body = try json(try XCTUnwrap(String(data: try encode(result), encoding: .utf8)))
        let payload = try XCTUnwrap(body["payload"] as? [String: Any])
        XCTAssertEqual(payload["status"] as? String, "rejected")
        XCTAssertEqual(payload["code"] as? String, "unsupported-capability")
        XCTAssertNotEqual(payload["status"] as? String, "observed")
        XCTAssertNil(result.validate(), "result 信封必須通過 AIP 驗證")
    }

    func testASupportedIntentIsPlayedWithTheRequestedIntensity() throws {
        let now = Date()
        let envelope = try intentEnvelope(
            intent: "celebrate", intensity: 0.8, expiresAt: now.addingTimeInterval(10))
        guard case .play(let playing) = SessionDecisions.intentOutcome(
            envelope: envelope, now: now, negotiated: true, alreadySeen: false)
        else { return XCTFail("celebrate 是本版支援的 intent") }
        XCTAssertEqual(playing.intent, .celebrate)
        XCTAssertEqual(playing.intensity, 0.8, accuracy: 0.0001)
        XCTAssertTrue(playing.interruptible)
    }

    func testAnIntentThatArrivesBeforeNegotiationIsNotPlayed() throws {
        let now = Date()
        let envelope = try intentEnvelope(expiresAt: now.addingTimeInterval(10))
        XCTAssertEqual(
            SessionDecisions.intentOutcome(
                envelope: envelope, now: now, negotiated: false, alreadySeen: false),
            .notNegotiated)
    }

    /// App 永遠不得宣告 `verified`（那只能來自 Runtime 的人類驗證路徑）。
    func testTheAppCanNeverBuildAVerifiedResult() {
        XCTAssertNil(
            SessionDecisions.resultEnvelope(
                causationId: "aip-1-1", status: .verified, code: nil, deviceId: "iphone-1",
                sessionId: "session.home", messageId: "ios-res-1", now: Date()))
    }

    // MARK: - Capability 宣告（golden）

    func testCapabilityAnnouncementIsExactlyThisShape() throws {
        let announcement = SessionDecisions.capabilityAnnouncement(reducedMotion: false)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let text = try XCTUnwrap(String(data: try encoder.encode(announcement), encoding: .utf8))
        XCTAssertEqual(
            text,
            """
            {"features":{"haptic":false,"reducedMotion":false},\
            "inputs":["character.interaction.touch","character.interaction.dismiss"],\
            "intents":["react-happily-to-touch","celebrate","settle","idle"],\
            "limits":{"maxMessageBytes":65536},\
            "profiles":["character-session"],\
            "role":"remote-renderer",\
            "specVersions":["aip/1.0"],\
            "syncClasses":["semantic"]}
            """)

        // Reduced Motion 是唯一會變的欄位（haptic 永遠 false：震動只走 haptic.pulse 動器）。
        let reduced = SessionDecisions.capabilityAnnouncement(reducedMotion: true)
        XCTAssertEqual(reduced.features?["reducedMotion"], .bool(true))
        XCTAssertEqual(reduced.features?["haptic"], .bool(false))
    }

    func testCapabilityEnvelopePassesAipValidation() throws {
        let envelope = try XCTUnwrap(
            SessionDecisions.capabilityEnvelope(
                deviceId: "iphone-1", sessionId: "session.home", messageId: "ios-cap-1",
                now: Date(), reducedMotion: false))
        XCTAssertNil(envelope.validate())
        XCTAssertEqual(envelope.messageType, .capability)
        XCTAssertEqual(envelope.name, "character.session.capability")
        XCTAssertEqual(envelope.source, AIPParty(kind: .device, id: "iphone-1"))
    }

    // MARK: - 觸摸事件的信封形狀

    func testTouchEventCarriesSessionDeadlineAndClaimedDeviceSource() throws {
        let now = Date()
        let envelope = SessionDecisions.touchEnvelope(
            kind: "longpress", deviceId: "iphone-87b42264", sessionId: "session.home",
            messageId: "ios-touch-1", now: now)

        XCTAssertNil(envelope.validate(), "互動事件必須通過 AIP 驗證（含 expiresAt 必填）")
        XCTAssertEqual(envelope.messageType, .event)
        XCTAssertEqual(envelope.name, "character.interaction.touch")
        XCTAssertEqual(envelope.sessionId, "session.home")
        XCTAssertEqual(envelope.source, AIPParty(kind: .device, id: "iphone-87b42264"))
        XCTAssertEqual(envelope.target, AIPParty(kind: .session, id: "session.home"))
        XCTAssertEqual(envelope.payload, .object(["kind": .string("longpress")]))

        let deadline = try XCTUnwrap(envelope.expiresAt.flatMap(AIPTime.parse))
        XCTAssertEqual(
            deadline.timeIntervalSince(now), 5, accuracy: 0.05,
            "§7 建議互動事件 5 秒 deadline")
        XCTAssertFalse(envelope.isExpired(now: now))
        XCTAssertTrue(envelope.isExpired(now: now.addingTimeInterval(6)))
    }

    func testDismissEventIsAlsoAValidInteractionEvent() throws {
        let envelope = SessionDecisions.dismissEnvelope(
            deviceId: "iphone-1", sessionId: "session.home", messageId: "ios-dismiss-1",
            now: Date())
        XCTAssertNil(envelope.validate())
        XCTAssertEqual(envelope.name, "character.interaction.dismiss")
        XCTAssertNotNil(envelope.expiresAt)
    }

    func testResumeQueryCarriesTheKeysTheHostReads() throws {
        var local = SessionSyncLocal()
        local.revision = 12
        local.sequence = 30
        local.epoch = 2
        let envelope = SessionDecisions.resumeEnvelope(
            local: local, deviceId: "iphone-1", sessionId: "session.home",
            messageId: "ios-resume-1", now: Date())
        XCTAssertNil(envelope.validate())
        XCTAssertEqual(envelope.messageType, .query)
        XCTAssertEqual(envelope.name, "character.session.resume")
        let body = try json(try XCTUnwrap(String(data: try encode(envelope), encoding: .utf8)))
        let payload = try XCTUnwrap(body["payload"] as? [String: Any])
        XCTAssertEqual(payload["lastRevision"] as? Int, 12)
        XCTAssertEqual(payload["lastSequence"] as? Int, 30)
        XCTAssertEqual(payload["sessionEpoch"] as? Int, 2)
    }

    // MARK: - 同步狀態文案

    func testSyncStatusCopyIsTheHumanWordingWithNoTechnicalTerms() {
        let clean = SessionSyncLocal()
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: clean, connected: false, negotiated: true, hasUnsupportedIntents: false,
                resuming: false), .offline)
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: clean, connected: true, negotiated: false, hasUnsupportedIntents: false,
                resuming: false), .notNegotiated)
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: clean, connected: true, negotiated: true, hasUnsupportedIntents: false,
                resuming: true), .resuming)
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: clean, connected: true, negotiated: true, hasUnsupportedIntents: true,
                resuming: false), .partialCapabilities)
        XCTAssertEqual(
            SessionDecisions.syncStatus(
                local: clean, connected: true, negotiated: true, hasUnsupportedIntents: false,
                resuming: false), .synced)

        XCTAssertEqual(SessionSyncStatus.partialCapabilities.text, "部分能力目前不可用")
        XCTAssertEqual(SessionSyncStatus.resuming.text, "同步尚未完成")

        // 一般模式不得洩漏技術詞。
        let forbidden = [
            "revision", "sequence", "epoch", "token", "provider", "lease", "schema", "transport",
            "UUID",
        ]
        for status in [
            SessionSyncStatus.offline, .notNegotiated, .synced, .partialCapabilities, .resuming,
            .unrecoverable,
        ] {
            for term in forbidden {
                XCTAssertFalse(
                    status.text.lowercased().contains(term.lowercased()),
                    "同步狀態文案不得出現「\(term)」：\(status.text)")
            }
        }
    }

    // MARK: - 呈現投影（誠實階梯）

    func testGreenCheckOnlyAppearsForVerifiedTruth() throws {
        var state = try semanticState(truth: "claimed")
        var presentation = CharacterPresentation.resolve(
            session: state, negotiated: true, legacy: .idle)
        XCTAssertFalse(presentation.showsVerifiedCheck, "claimed 不是 verified")
        XCTAssertEqual(presentation.detail, "宣稱完成（尚未驗證）")

        state = try semanticState(truth: "verified")
        presentation = CharacterPresentation.resolve(session: state, negotiated: true, legacy: .idle)
        XCTAssertTrue(presentation.showsVerifiedCheck)
        XCTAssertEqual(presentation.detail, "已驗證成功")
    }

    /// 安全訊息只能加嚴：任一條路徑說緊急停止，畫面就必須是緊急停止的固定文案。
    func testEmergencyIsTheUnionOfBothPathsAndKeepsTheFixedCopy() throws {
        let calm = try semanticState(truth: "none")
        let fromLegacy = CharacterPresentation.resolve(
            session: calm, negotiated: true, legacy: .emergency)
        XCTAssertTrue(fromLegacy.isEmergency, "舊路徑的緊急停止不得被語意狀態淡化")
        XCTAssertEqual(fromLegacy.headline, "緊急停止中")
        XCTAssertFalse(fromLegacy.showsVerifiedCheck)

        let fromSession = CharacterPresentation.resolve(
            session: try semanticState(truth: "emergency"), negotiated: true, legacy: .idle)
        XCTAssertTrue(fromSession.isEmergency)
        XCTAssertEqual(fromSession.headline, "緊急停止中")
    }

    /// 還沒協商（舊桌面）時退回既有的 `character.present` 路徑，不假裝有語意狀態。
    func testWithoutNegotiationThePresentationFallsBackToTheLegacyPath() throws {
        let presentation = CharacterPresentation.resolve(
            session: try semanticState(truth: "verified"), negotiated: false, legacy: .working)
        XCTAssertFalse(presentation.fromSession)
        XCTAssertEqual(presentation.headline, "工作中")
        XCTAssertFalse(presentation.showsVerifiedCheck)
    }

    func testUnknownVocabularyIsShownAsUnknownRatherThanGuessed() throws {
        let state = try XCTUnwrap(
            CharacterSemanticState.project(
                try XCTUnwrap(
                    SemanticJSON.parse(
                        """
                        {"characterId":"ref-shape","mood":{"kind":"sparkly","intensity":0.5},
                         "activity":"orbiting","truth":{"state":"teleporting"},
                         "members":[],"reducedMotion":false}
                        """))))
        XCTAssertEqual(state.mood, .unknown("sparkly"))
        XCTAssertEqual(state.activity, .unknown("orbiting"))
        XCTAssertEqual(state.truth, .unrecognized("teleporting"))
        let presentation = CharacterPresentation.resolve(
            session: state, negotiated: true, legacy: .idle)
        XCTAssertEqual(presentation.headline, "未知")
        XCTAssertEqual(presentation.tone, .unknown)
        XCTAssertFalse(presentation.showsVerifiedCheck)
    }

    func testAStateWithTooManyMembersIsRejectedRatherThanTruncated() throws {
        let members = (0..<(AIPLimits.maxMembers + 1)).map {
            """
            {"party":{"kind":"device","id":"iphone-\($0)"},"role":"remote-renderer",
             "presence":"online","lastSeenAt":"2026-09-04T12:30:02Z"}
            """
        }.joined(separator: ",")
        let state = try XCTUnwrap(
            SemanticJSON.parse(
                """
                {"characterId":"ref-shape","mood":{"kind":"neutral","intensity":0.0},
                 "activity":"idle","truth":{"state":"none"},"members":[\(members)],
                 "reducedMotion":false}
                """))
        XCTAssertNil(CharacterSemanticState.project(state), "超過成員上限就整份拒絕，不截斷")
    }

    // MARK: - 有界

    func testPendingIntentQueueAndDedupeRingAreBounded() {
        XCTAssertEqual(SessionClient.maxPendingIntents, 8)
        var ring = AIPDedupeRing()
        for index in 0...(AIPLimits.dedupeRing + 10) {
            _ = ring.note("m-\(index)")
        }
        XCTAssertEqual(ring.count, AIPLimits.dedupeRing)
        XCTAssertFalse(ring.has("m-0"), "最舊的必須被淘汰，環不會無界成長")
    }

    // MARK: - 換桌面／換 epoch（endpoint migration）

    /// §7 步驟 4 寫的是「epoch **不同** → 丟棄本地狀態、套用 snapshot」，不是「更大」。
    ///
    /// host 送 `session-reset` 時帶的是 **host 自己的** epoch：同一支 iPhone 被重新配對到
    /// 另一台桌面時，那個值可能比手機記得的小（全新 host 的 epoch 就是 1，
    /// `crates/interaction-runtime/src/character_session.rs`）。用 `>` 判斷會把那份權威快照
    /// 當成 rollback 丟掉，之後每一則狀態都被同樣理由丟掉、永遠不會恢復。
    func testASessionResetIsAppliedWheneverTheEpochDiffersEvenIfItWentBackwards() throws {
        for hostEpoch in [1, 2, 9] {
            var local = SessionSyncLocal()
            local.revision = 300
            local.epoch = 3
            local.state = try XCTUnwrap(fixture("state-snapshot.json")["payload"]?["state"])
            let payload = try XCTUnwrap(
                SemanticJSON.parse(
                    """
                    {"kind":"snapshot","reason":"session-reset","revision":1,"sequence":1,
                     "sessionEpoch":\(hostEpoch),
                     "state":{"characterId":"ref-shape","mood":{"kind":"neutral","intensity":0.0},
                              "activity":"idle","attention":{"kind":"none"},
                              "truth":{"state":"none"},"members":[],"reducedMotion":false}}
                    """))
            let message = try XCTUnwrap(SessionDecisions.stateMessage(payload: payload))
            guard case .applied(let revision, let epoch, _) = SessionDecisions.apply(
                message, to: local)
            else {
                return XCTFail("host 說它重建了 session（epoch \(hostEpoch) ≠ 本地 3），必須套用")
            }
            XCTAssertEqual(revision, 1)
            XCTAssertEqual(epoch, UInt64(hostEpoch))
        }
    }

    /// 被丟掉的 rollback **不是**終點：什麼都不做的話，之後每一則權威狀態都會用同樣理由
    /// 被丟掉，而畫面卻顯示「角色狀態已同步」（received ≠ applied）。
    @MainActor
    func testAnIgnoredRollbackAsksForAResumeInsteadOfClaimingSynced() throws {
        let transport = RecordingTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyFixtureSnapshot(client)
        XCTAssertEqual(client.syncStatus, .synced)
        XCTAssertEqual(transport.resumes.count, 0)

        // 沒有 `reason` 的舊快照（例如另一台桌面的權威狀態）：忽略是對的，
        // 但忽略之後必須去補齊，而且不得繼續宣稱「已同步」。
        try feed(client, stateEnvelope(revision: 7, sequence: 206, epoch: 3))
        XCTAssertEqual(transport.resumes.count, 1, "被忽略的權威狀態必須觸發補齊")
        XCTAssertNotEqual(client.syncStatus, .synced, "一則都沒套用就不能說已同步")
    }

    /// 重新配對到另一台桌面：本地對權威狀態的認知必須歸零，
    /// 否則新 host 的第一份快照（revision 從 1 起）會被當成 rollback。
    @MainActor
    func testRepairingToAnotherDesktopForgetsTheOldAuthoritativeState() throws {
        let transport = RecordingTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyFixtureSnapshot(client)
        XCTAssertEqual(client.advanced.revision, 204)

        client.pairingDidChange()
        XCTAssertEqual(client.advanced.revision, 0)
        XCTAssertNil(client.presentation)
        XCTAssertFalse(client.negotiated)

        // 全新桌面：epoch 1、revision 1，沒有 `reason`——歸零之後它就不再是 rollback。
        try negotiate(client)
        try feed(client, stateEnvelope(revision: 1, sequence: 1, epoch: 1))
        XCTAssertEqual(client.advanced.revision, 1)
        XCTAssertEqual(client.advanced.epoch, 1)
        XCTAssertEqual(client.syncStatus, .synced)
    }

    // MARK: - resume 預算（真的送出去才算一次嘗試）

    /// 兩件事一起釘住：
    /// 1. 第 3 次 resume 必須**真的送出去**——把計數擺在送出之前，使用者看到
    ///    「無法恢復，請重新連接」時實際只做過 2 次 round-trip。
    /// 2. §11 給使用者的指示就是「請重新連接」，所以重新連接必須真的重新給一輪機會；
    ///    失敗計數跨連線累積的話那句指示等於是假的。
    @MainActor
    func testEveryCountedResumeAttemptIsActuallySentAndReconnectingRestoresTheBudget() throws {
        let transport = RecordingTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyFixtureSnapshot(client)

        for attempt in 1...SessionSyncLocal.resumeFailureLimit {
            // 接不上的 patch（baseRevision 與本地對不起來）→ §7 要送 resume。
            try feed(
                client,
                statePatchEnvelope(
                    revision: UInt64(300 + attempt), baseRevision: UInt64(299 + attempt),
                    sequence: UInt64(300 + attempt)))
            XCTAssertEqual(
                transport.resumes.count, attempt,
                "第 \(attempt) 次嘗試必須真的送出一則 resume，而不是只加計數")
        }
        XCTAssertEqual(
            client.syncStatus, .unrecoverable,
            "連續 \(SessionSyncLocal.resumeFailureLimit) 次補不齊才誠實說「無法恢復」")

        client.connectionDidDisconnect()
        client.connectionDidConnect()
        XCTAssertEqual(
            transport.resumes.count, SessionSyncLocal.resumeFailureLimit + 1,
            "重新連接之後的第一次 resume 必須真的送出去")
        XCTAssertNotEqual(client.syncStatus, .unrecoverable, "新連線＝新的一輪嘗試")
    }

    /// 送不出去（有界佇列丟棄／編碼失敗）就不算一次嘗試，但也不得假裝還在同步。
    @MainActor
    func testAResumeThatWasNeverSentIsNotCountedAsAnAttempt() throws {
        let transport = RecordingTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyFixtureSnapshot(client)

        transport.accepts = false
        for _ in 1...(SessionSyncLocal.resumeFailureLimit + 2) {
            try feed(
                client, statePatchEnvelope(revision: 400, baseRevision: 399, sequence: 400))
        }
        XCTAssertEqual(transport.resumes.count, 0)
        XCTAssertNotEqual(
            client.syncStatus, .unrecoverable,
            "一次都沒送出去，就不能宣稱「試過三次」")
        XCTAssertNotEqual(client.syncStatus, .synced)
    }

    // MARK: - Reduced Motion 下的呈現（observed ＝ 真的演過）

    /// `docs/aip/iphone-companion.md` §4 的表格：Reduced Motion 開啟時
    /// `react-happily-to-touch` 是「不縮放，只換顏色」。
    /// 之前這個 intent 的**全部**效果都包在 `if !reduceMotion` 裡、沒有任何換色，
    /// 於是那次播放只是一段 `Task.sleep`，卻仍然回 `result{observed}`
    ///（`observed` 的定義是「呈現完成」——沒有呈現就不能這樣說）。
    func testReducedMotionStillHasSomethingToShowForReactHappilyToTouch() {
        let reduced = CharacterPlaybackEffect.plan(
            intent: .reactHappilyToTouch, intensity: 0.8, reduceMotion: true)
        XCTAssertEqual(reduced.scale, 1, "Reduced Motion 不得做位移或縮放")
        XCTAssertTrue(
            reduced.hasVisibleChange, "只剩一段 sleep 的播放不能回 observed")

        let resting = presentation(tone: .happy)
        XCTAssertNotEqual(
            CharacterView.components(presentation: resting, effect: reduced),
            CharacterView.components(presentation: resting, effect: CharacterPlaybackEffect()),
            "「只換顏色」那一格必須真的有顏色可換")

        // 沒開 Reduced Motion 時才有縮放，幅度隨 intensity（長度不變）。
        let full = CharacterPlaybackEffect.plan(
            intent: .reactHappilyToTouch, intensity: 0.8, reduceMotion: false)
        XCTAssertGreaterThan(full.scale, 1)
        XCTAssertLessThan(
            CharacterPlaybackEffect.plan(
                intent: .reactHappilyToTouch, intensity: 0.2, reduceMotion: false).scale,
            full.scale)

        // celebrate 兩種模式都是換色；settle／idle 是「回到靜止」。
        for reduceMotion in [true, false] {
            XCTAssertEqual(
                CharacterPlaybackEffect.plan(
                    intent: .celebrate, intensity: 1, reduceMotion: reduceMotion).highlight,
                .celebrate)
            XCTAssertEqual(
                CharacterPlaybackEffect.plan(
                    intent: .settle, intensity: 1, reduceMotion: reduceMotion),
                CharacterPlaybackEffect())
        }

        // 安全訊息只能加嚴：任何播放效果都不得蓋掉緊急停止的顏色。
        let emergency = presentation(tone: .emergency, isEmergency: true)
        for intent in BehaviorIntent.allCases {
            XCTAssertEqual(
                CharacterView.components(
                    presentation: emergency,
                    effect: CharacterPlaybackEffect.plan(
                        intent: intent, intensity: 1, reduceMotion: false)),
                CharacterView.components(for: .emergency),
                "\(intent.rawValue) 不得改寫緊急停止的呈現")
        }
    }

    // MARK: - 小工具

    private func presentation(tone: CharacterTone, isEmergency: Bool = false)
        -> CharacterPresentation
    {
        CharacterPresentation(
            headline: isEmergency ? CharacterPresentation.emergencyText : "待機", detail: nil,
            tone: tone, showsVerifiedCheck: false, isEmergency: isEmergency, fromSession: true)
    }

    /// 記錄每一則送出訊息的 mock transport。
    private final class RecordingTransport: SessionTransport {
        var isConnected = true
        var boundDeviceId: String? = "iphone-87b42264"
        /// false ＝ 模擬有界佇列丟棄／編碼失敗（`sendAip` 回 false，不假裝送出）。
        var accepts = true
        private(set) var sent: [AIPEnvelope] = []
        private(set) var observations: [String] = []

        @discardableResult
        func sendAip(_ envelope: AIPEnvelope) -> Bool {
            guard accepts else { return false }
            sent.append(envelope)
            return true
        }

        func sendObservation(receptor: String, facts: [String: JSONValue]) {
            observations.append(receptor)
        }

        var resumes: [AIPEnvelope] { sent.filter { $0.name == SessionNames.resume } }
    }

    /// 把 envelope 文字包成真正的 frame（`SessionClient` 讀的是 frame 原文）。
    @MainActor
    private func feed(_ client: SessionClient, _ envelopeText: String, now: Date = Date()) throws {
        client.handleFrame(
            try envelope(envelopeText),
            rawFrame: #"{"type":"aip","envelope":"# + envelopeText + "}", now: now)
    }

    @MainActor
    private func negotiate(_ client: SessionClient) throws {
        try feed(
            client,
            """
            {"specVersion":"aip/1.0","messageId":"aip-neg-\(UUID().uuidString.prefix(8))",
             "messageType":"capability","name":"character.session.capability",
             "source":{"kind":"session","id":"session.home"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"session.home","occurredAt":"2026-09-04T12:30:00.000Z",
             "payload":{"specVersion":"aip/1.0","newerMinor":false,"role":"remote-renderer",
                        "syncClass":"semantic",
                        "intents":{"react-happily-to-touch":"exact","celebrate":"exact",
                                   "settle":"exact","idle":"exact"},
                        "inputs":["character.interaction.touch"],"unsupportedInputs":[],
                        "limits":{"maxMessageBytes":65536,"maxPayloadBytes":32768,
                                  "maxIntentsPerMinute":60}}}
            """)
        XCTAssertTrue(client.negotiated, "協商失敗，後面的斷言就沒有意義")
    }

    /// 套用 Rust conformance fixture 的那份權威快照（revision 204、epoch 3）。
    @MainActor
    private func applyFixtureSnapshot(_ client: SessionClient) throws {
        let text = try XCTUnwrap(AIPFixtures.files["state-snapshot.json"])
        try feed(client, text)
        XCTAssertEqual(client.advanced.revision, 204, "快照必須被套用")
    }

    private func stateEnvelope(revision: UInt64, sequence: UInt64, epoch: UInt64) -> String {
        """
        {"specVersion":"aip/1.0","messageId":"aip-st-\(revision)-\(sequence)",
         "messageType":"state","name":"character.session.snapshot",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","occurredAt":"2026-09-04T12:30:04.000Z",
         "sequence":\(sequence),
         "payload":{"kind":"snapshot","revision":\(revision),"sessionEpoch":\(epoch),
                    "state":{"characterId":"ref-shape","mood":{"kind":"happy","intensity":0.5},
                             "activity":"idle","attention":{"kind":"none"},
                             "truth":{"state":"none"},"members":[],"reducedMotion":false}}}
        """
    }

    private func statePatchEnvelope(revision: UInt64, baseRevision: UInt64, sequence: UInt64)
        -> String
    {
        """
        {"specVersion":"aip/1.0","messageId":"aip-pt-\(revision)-\(sequence)",
         "messageType":"state","name":"character.session.patch",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","occurredAt":"2026-09-04T12:30:05.000Z",
         "sequence":\(sequence),"baseRevision":\(baseRevision),
         "payload":{"kind":"patch","revision":\(revision),"baseRevision":\(baseRevision),
                    "patch":{"activity":"working"}}}
        """
    }

    // MARK: - 小工具

    private func encode(_ envelope: AIPEnvelope) throws -> Data {
        switch envelope.encode() {
        case .success(let data): return data
        case .failure(let error): throw error
        }
    }

    private func semanticState(truth: String) throws -> CharacterSemanticState {
        try XCTUnwrap(
            CharacterSemanticState.project(
                try XCTUnwrap(
                    SemanticJSON.parse(
                        """
                        {"characterId":"ref-shape","mood":{"kind":"happy","intensity":0.45},
                         "activity":"idle","truth":{"state":"\(truth)"},
                         "members":[],"reducedMotion":false}
                        """))))
    }
}
