//
//  LifecycleTests.swift
//  InteractionCompanionTests
//
//  M4 §5.4 iOS 生命週期：前景 presence、heartbeat／legacy status 耦合、
//  AIP heartbeat 處理。
//
//  這裡只測**純決策與狀態機**，不開任何 socket、不進背景：
//  - `LifecycleDecision.on(phase:socketAlive:sinceForeground:)` 的表格化決策
//  - `LifecycleDecision.shouldReconnectImmediately(...)` 的重連閘門
//  - `PresenceHeartbeatPolicy` 的常數不變量（心跳間隔必須小於 presence 逾時的一半）
//  - `SessionClient.foregroundDidResume()`（§7 resume，不重播事件）
//  - `SessionClient` 對 AIP `heartbeat` 的回應（誠實記錄 ＋ 以 legacy status 回覆）
//
//  **誠實範圍**：模擬器上驗證的是狀態機與時序決策。真機的「背景 socket 到底會不會活著」
//  只有 v0.5 那一筆觀察（README「背景／前景」段落），本檔案不宣稱任何真機結論。
//

import SwiftUI
import XCTest

@testable import InteractionCompanion

final class LifecycleTests: XCTestCase {

    // MARK: - 1. 生命週期決策（表格）

    /// 一列＝一個場景。欄位順序與 `LifecycleDecision` 的欄位一致，方便對照閱讀。
    private struct Row {
        let name: String
        let phase: AppLifecyclePhase
        let socketAlive: Bool
        let sinceForeground: TimeInterval?
        let expected: LifecycleDecision
    }

    func testLifecycleDecisionTable() {
        let rows: [Row] = [
            Row(
                name: "進背景：停掉 status 心跳，什麼都不送（沒有 Background Mode，不假裝連線活著）",
                phase: .background, socketAlive: true, sinceForeground: nil,
                expected: LifecycleDecision(
                    sendStatusNow: false, restartTimer: false, stopTimer: true,
                    resumeSession: false, reconnect: false)),
            Row(
                name: "進背景（socket 已死）：一樣只停 timer，不重連（背景不重連）",
                phase: .background, socketAlive: false, sinceForeground: 120,
                expected: LifecycleDecision(
                    sendStatusNow: false, restartTimer: false, stopTimer: true,
                    resumeSession: false, reconnect: false)),
            Row(
                name: "inactive 是過渡狀態（通知中心／App 切換器）：不拆任何東西、也不送",
                phase: .inactive, socketAlive: true, sinceForeground: nil,
                expected: LifecycleDecision()),
            Row(
                name: "冷啟動後第一次 active（不曾進背景）：立刻送 status、起 timer，不必 resume",
                phase: .active, socketAlive: true, sinceForeground: nil,
                expected: LifecycleDecision(
                    sendStatusNow: true, restartTimer: true, stopTimer: false,
                    resumeSession: false, reconnect: false)),
            Row(
                name: "背景一瞬間（< 1 秒，系統翻頁）：送 status＋起 timer，但不值得一次 resume round-trip",
                phase: .active, socketAlive: true, sinceForeground: 0.4,
                expected: LifecycleDecision(
                    sendStatusNow: true, restartTimer: true, stopTimer: false,
                    resumeSession: false, reconnect: false)),
            Row(
                name: "真的進過背景（≥ 1 秒）且 socket 還活著：立刻送 status、起 timer、走一次 resume",
                phase: .active, socketAlive: true, sinceForeground: 1.0,
                expected: LifecycleDecision(
                    sendStatusNow: true, restartTimer: true, stopTimer: false,
                    resumeSession: true, reconnect: false)),
            Row(
                name: "背景很久（超過 presence 逾時）且 socket 還活著：一樣是 status＋timer＋resume",
                phase: .active, socketAlive: true, sinceForeground: 600,
                expected: LifecycleDecision(
                    sendStatusNow: true, restartTimer: true, stopTimer: false,
                    resumeSession: true, reconnect: false)),
            Row(
                name: "回前景但 socket 已死：不假裝送得出 status，改走重連",
                phase: .active, socketAlive: false, sinceForeground: 600,
                expected: LifecycleDecision(
                    sendStatusNow: false, restartTimer: false, stopTimer: false,
                    resumeSession: false, reconnect: true)),
            Row(
                name: "socket 已死且不曾進背景：也是重連（不 resume、不送 status）",
                phase: .active, socketAlive: false, sinceForeground: nil,
                expected: LifecycleDecision(
                    sendStatusNow: false, restartTimer: false, stopTimer: false,
                    resumeSession: false, reconnect: true)),
        ]

        for row in rows {
            let decision = LifecycleDecision.on(
                phase: row.phase, socketAlive: row.socketAlive,
                sinceForeground: row.sinceForeground)
            XCTAssertEqual(decision, row.expected, row.name)
        }
    }

    /// socket 已死時**絕不**宣稱送出 status——那是假的存活證明。
    func testADeadSocketNeverSendsStatusOrResumes() {
        for since in [nil, 0.1, 45, 3_600] as [TimeInterval?] {
            let decision = LifecycleDecision.on(
                phase: .active, socketAlive: false, sinceForeground: since)
            XCTAssertFalse(decision.sendStatusNow)
            XCTAssertFalse(decision.resumeSession)
            XCTAssertTrue(decision.reconnect)
        }
    }

    /// 背景不做任何網路動作（沒有 Background Mode，做了也只是假裝）。
    func testBackgroundNeverSendsReconnectsOrResumes() {
        for alive in [true, false] {
            let decision = LifecycleDecision.on(
                phase: .background, socketAlive: alive, sinceForeground: 30)
            XCTAssertEqual(
                decision,
                LifecycleDecision(
                    sendStatusNow: false, restartTimer: false, stopTimer: true,
                    resumeSession: false, reconnect: false))
        }
    }

    // MARK: - 2. 立即重連的閘門

    func testForegroundReconnectOnlyHappensWhenTheUserStillWantsTheConnection() {
        // 等退避中：回前景時**不等退避**，立刻重連。
        XCTAssertTrue(
            LifecycleDecision.shouldReconnectImmediately(
                phase: .waitingRetry(inSeconds: 8), userWantsConnection: true, hasPairing: true))
        // 失敗狀態：一樣立刻重連。
        XCTAssertTrue(
            LifecycleDecision.shouldReconnectImmediately(
                phase: .failed(reason: "逾時"), userWantsConnection: true, hasPairing: true))
        // 使用者按過「立即中斷」：不得偷偷復活。
        XCTAssertFalse(
            LifecycleDecision.shouldReconnectImmediately(
                phase: .idle, userWantsConnection: false, hasPairing: true))
        // 配對被撤銷：撤銷有自己的文案與流程，不重連。
        XCTAssertFalse(
            LifecycleDecision.shouldReconnectImmediately(
                phase: .revoked(reason: "已撤銷"), userWantsConnection: true, hasPairing: true))
        // 沒有配對：沒有可以連的對象。
        XCTAssertFalse(
            LifecycleDecision.shouldReconnectImmediately(
                phase: .failed(reason: "逾時"), userWantsConnection: true, hasPairing: false))
        // 已經連上／正在連：不重來。
        for phase: ConnectionPhase in [.connected, .connecting, .authenticating, .pairing] {
            XCTAssertFalse(
                LifecycleDecision.shouldReconnectImmediately(
                    phase: phase, userWantsConnection: true, hasPairing: true),
                "\(phase) 不應該再開一條 socket")
        }
    }

    // MARK: - 3. heartbeat／legacy status 的耦合常數

    /// presence 完全靠 legacy `status` 心跳維持（`docs/aip/transport-bindings.md` §1.4）。
    /// 心跳間隔必須小於 presence 逾時的**一半**，才容得下一次整包漏送。
    func testStatusIntervalLeavesRoomForAMissedBeat() {
        XCTAssertLessThan(
            PresenceHeartbeatPolicy.statusIntervalSeconds,
            PresenceHeartbeatPolicy.presenceTimeoutSeconds / 2,
            "心跳間隔 ≥ presence 逾時的一半時，漏送一次就會被桌面標成 offline")
        XCTAssertGreaterThanOrEqual(
            PresenceHeartbeatPolicy.missedBeatsTolerated, 1,
            "至少要容得下一次漏送")
    }

    /// 常數要對得上桌面端：`SessionConfig::presence_timeout_ms` 預設 45 000
    /// （`docs/aip/device-profile.md` §4）。改這裡就等於改跨端契約。
    func testPresenceTimeoutMatchesTheHostConstant() {
        XCTAssertEqual(PresenceHeartbeatPolicy.presenceTimeoutSeconds, 45)
        XCTAssertEqual(PresenceHeartbeatPolicy.statusIntervalSeconds, 15)
        XCTAssertEqual(PresenceHeartbeatPolicy.missedBeatsTolerated, 1)
    }

    // MARK: - 4. 回前景的 resume（§7）

    @MainActor
    func testForegroundResumeAsksTheHostToRealignWithRevisionSequenceAndEpoch() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applySnapshot(client, revision: 204, sequence: 87, epoch: 3)
        transport.reset()

        client.foregroundDidResume()

        XCTAssertEqual(transport.resumes.count, 1, "回前景應該只送一則 resume")
        guard case .object(let payload)? = transport.resumes.first?.payload else {
            return XCTFail("resume 的 payload 應該是物件")
        }
        XCTAssertEqual(payload["lastRevision"]?.doubleValue, 204)
        XCTAssertEqual(payload["lastSequence"]?.doubleValue, 87)
        XCTAssertEqual(payload["sessionEpoch"]?.doubleValue, 3)
    }

    /// §8：重連／回前景**只 reconcile 狀態**，不重播任何互動事件或 intent。
    @MainActor
    func testForegroundResumeNeverReplaysInteractionEvents() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applySnapshot(client, revision: 204, sequence: 87, epoch: 3)
        client.touch(kind: "tap")
        transport.reset()

        client.foregroundDidResume()

        XCTAssertTrue(
            transport.sent.allSatisfy { $0.name == SessionNames.resume },
            "回前景只能送 resume，不得重播 touch／dismiss")
    }

    /// 還沒協商（例如剛斷線重連、capability 還沒回來）：沒有 session 可以對齊，不硬送。
    @MainActor
    func testForegroundResumeSendsNothingBeforeNegotiation() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport

        client.foregroundDidResume()

        XCTAssertTrue(transport.sent.isEmpty, "尚未協商時不得送 resume")
        XCTAssertFalse(client.log.isEmpty, "但必須留下一行誠實說明")
    }

    /// 已協商但還沒有任何權威狀態：沒有 lastRevision 可以宣稱，等桌面送快照。
    @MainActor
    func testForegroundResumeSendsNothingWithoutAuthoritativeState() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        transport.reset()

        client.foregroundDidResume()

        XCTAssertTrue(transport.resumes.isEmpty, "本地沒有權威狀態時不送 resume")
    }

    /// 未連線：不假裝送得出去。
    @MainActor
    func testForegroundResumeSendsNothingWhenOffline() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applySnapshot(client, revision: 204, sequence: 87, epoch: 3)
        transport.reset()
        transport.isConnected = false
        let logBefore = client.log.count

        client.foregroundDidResume()

        XCTAssertTrue(transport.sent.isEmpty)
        XCTAssertFalse(
            client.log.isEmpty,
            "未連線時也必須留下一行誠實說明，比照 testForegroundResumeSendsNothingBeforeNegotiation")
        XCTAssertGreaterThan(
            client.log.count, logBefore,
            "「未連線」是三個送不出去的分支之一，不得靜默吞掉——文件承諾三個分支對稱")
        XCTAssertEqual(client.syncStatus, .offline, "未連線就誠實顯示未連線，不停在舊狀態")
    }

    /// F-006：快速鎖螢幕／解鎖。桌面還沒回覆第一則 resume 之前，後續的回前景不得重送，
    /// 更不得把「還沒收到回覆」累積成「無法恢復」——桌面一次都沒有拒絕過，
    /// 宣稱失敗就是在沒有失敗證據時說失敗（誠實階梯）。
    @MainActor
    func testRapidForegroundCyclesSendOneResumeAndNeverFakeUnrecoverable() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applySnapshot(client, revision: 204, sequence: 87, epoch: 3)
        transport.reset()

        let base = Date(timeIntervalSince1970: 2_000)
        for step in 0..<5 {
            client.foregroundDidResume(now: base.addingTimeInterval(Double(step)))
        }

        XCTAssertEqual(
            transport.resumes.count, 1,
            "第一則 resume 還在等桌面回覆，重送不會帶來新資訊")
        XCTAssertNotEqual(
            client.syncStatus, .unrecoverable,
            "桌面一次都沒有回過拒絕或失敗，不得宣稱「無法恢復」")
        XCTAssertEqual(client.syncStatus, .resuming, "還沒對齊就誠實顯示「同步尚未完成」")

        // 桌面終於回覆「已經對齊，沒有東西要補」：計數歸零、狀態才變成已同步。
        let causationId = try XCTUnwrap(transport.resumes.first?.messageId)
        try feed(
            client, resumeResponse(causationId: causationId),
            now: base.addingTimeInterval(6))
        XCTAssertEqual(client.syncStatus, .synced)
    }

    /// 有界：等超過寬限窗之後的回前景**要**再問一次，不能永遠卡在「等回覆」。
    @MainActor
    func testAForegroundResumeIsRetriedOnceTheGraceWindowHasPassed() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applySnapshot(client, revision: 204, sequence: 87, epoch: 3)
        transport.reset()

        let base = Date(timeIntervalSince1970: 3_000)
        client.foregroundDidResume(now: base)
        client.foregroundDidResume(
            now: base.addingTimeInterval(SessionSyncLocal.resumeResponseGraceSeconds))

        XCTAssertEqual(transport.resumes.count, 2, "超過寬限窗就得再問一次，否則永遠卡住")
    }

    /// 純決策：在飛的 resume 要不要重送。
    func testAnOutstandingResumeIsNotResentUntilTheGraceWindowPasses() {
        let sentAt = Date(timeIntervalSince1970: 1_000)
        XCTAssertTrue(
            SessionDecisions.shouldResendResumeOnForeground(inFlightSince: nil, now: sentAt),
            "沒有 resume 在飛：本來就該送")
        XCTAssertFalse(
            SessionDecisions.shouldResendResumeOnForeground(
                inFlightSince: sentAt, now: sentAt.addingTimeInterval(1)),
            "還在寬限窗內：同一個問題不問第二次")
        XCTAssertTrue(
            SessionDecisions.shouldResendResumeOnForeground(
                inFlightSince: sentAt,
                now: sentAt.addingTimeInterval(SessionSyncLocal.resumeResponseGraceSeconds)),
            "有界：超過寬限窗才允許再問一次")
        XCTAssertTrue(
            SessionDecisions.shouldResendResumeOnForeground(
                inFlightSince: sentAt, now: sentAt.addingTimeInterval(-30)),
            "時鐘往回跳時寧可再問一次，也不要卡死")
    }

    // MARK: - 6. 背景閘門（重連與心跳）

    /// F-007：「不在背景重連」不能只發生在 scenePhase 變化那一刻——
    /// 斷線重試與 status 心跳這兩條既有路徑也要看生命週期。
    func testBackgroundNeverSchedulesAReconnectOrSendsAHeartbeat() {
        XCTAssertFalse(
            LifecycleDecision.shouldScheduleReconnect(phase: .background),
            "背景沒有 Background Mode：排重連只是在被系統暫停前多開一條 socket")
        XCTAssertFalse(
            LifecycleDecision.shouldSendPresenceHeartbeat(phase: .background),
            "背景送出的 status 會讓桌面把手機標成 online，與本機顯示的「背景」互相矛盾")
        for phase: AppLifecyclePhase in [.active, .inactive] {
            XCTAssertTrue(
                LifecycleDecision.shouldScheduleReconnect(phase: phase), "\(phase) 應該照常重連")
            XCTAssertTrue(
                LifecycleDecision.shouldSendPresenceHeartbeat(phase: phase),
                "\(phase) 還在前景，心跳照送")
        }
    }

    // MARK: - 7. ScenePhase 對應

    /// F-008：`@unknown default` 的 fallback 必須真的是「背景」，與註解一致。
    func testUnknownScenePhaseFallsBackToBackground() {
        XCTAssertEqual(
            AppLifecyclePhase.unknownScenePhaseFallback, .background,
            "未知階段一律當成不是前景：寧可停心跳並誠實標成背景")
        XCTAssertEqual(
            LifecycleDecision.on(
                phase: .unknownScenePhaseFallback, socketAlive: true, sinceForeground: nil),
            LifecycleDecision(stopTimer: true))
        XCTAssertFalse(
            LifecycleDecision.shouldSendPresenceHeartbeat(phase: .unknownScenePhaseFallback))
        XCTAssertFalse(
            LifecycleDecision.shouldScheduleReconnect(phase: .unknownScenePhaseFallback))
    }

    func testScenePhaseMapsOntoTheServiceLayerPhase() {
        XCTAssertEqual(ScenePhase.active.appLifecyclePhase, .active)
        XCTAssertEqual(ScenePhase.inactive.appLifecyclePhase, .inactive)
        XCTAssertEqual(ScenePhase.background.appLifecyclePhase, .background)
    }

    // MARK: - 5. AIP heartbeat frame

    /// 本版**不送** AIP heartbeat（`docs/aip/transport-bindings.md` §1.4 記載為尚未實作）；
    /// 收到時不再靜默吞掉：記一行進階診斷，並以 legacy `status`（真正維持 presence 的那條路）回應。
    @MainActor
    func testAipHeartbeatIsNotedAndAnsweredWithALegacyStatus() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        transport.reset()

        try feed(client, heartbeatEnvelope(id: "aip-hb-1"), now: Date(timeIntervalSince1970: 100))

        XCTAssertEqual(transport.statusSends, 1, "收到 AIP heartbeat 要以一則 legacy status 回應")
        XCTAssertEqual(client.advanced.heartbeatsReceived, 1)
        XCTAssertTrue(
            client.log.contains { $0.contains("AIP heartbeat") },
            "進階診斷必須看得到「收到 AIP heartbeat」：\(client.log)")
    }

    /// heartbeat 不是「忽略的同步訊息」——被處理過的 frame 不得算進去（誠實計數）。
    @MainActor
    func testAipHeartbeatIsNotCountedAsAnIgnoredFrame() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        let before = client.advanced.framesIgnored

        try feed(client, heartbeatEnvelope(id: "aip-hb-2"), now: Date(timeIntervalSince1970: 100))

        XCTAssertEqual(client.advanced.framesIgnored, before)
    }

    /// 對方猛送 heartbeat 時不得把有界佇列灌爆：回覆有最短間隔。
    @MainActor
    func testHeartbeatRepliesAreThrottled() throws {
        let transport = LifecycleTransport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        transport.reset()

        let base = Date(timeIntervalSince1970: 1_000)
        try feed(client, heartbeatEnvelope(id: "aip-hb-3"), now: base)
        try feed(client, heartbeatEnvelope(id: "aip-hb-4"), now: base.addingTimeInterval(1))
        try feed(client, heartbeatEnvelope(id: "aip-hb-5"), now: base.addingTimeInterval(2))

        XCTAssertEqual(transport.statusSends, 1, "節流窗內只回一次")
        XCTAssertEqual(client.advanced.heartbeatsReceived, 3, "但收到幾則就誠實記幾則")

        try feed(
            client, heartbeatEnvelope(id: "aip-hb-6"),
            now: base.addingTimeInterval(PresenceHeartbeatPolicy.heartbeatReplyMinIntervalSeconds))
        XCTAssertEqual(transport.statusSends, 2, "超過節流窗就再回一次")
    }

    /// 時鐘往回跳（使用者改時間）不得讓回覆永遠卡住。
    func testHeartbeatReplyDecisionSurvivesABackwardClock() {
        let now = Date(timeIntervalSince1970: 1_000)
        XCTAssertTrue(SessionDecisions.shouldAnswerHeartbeat(lastAnswerAt: nil, now: now))
        XCTAssertFalse(
            SessionDecisions.shouldAnswerHeartbeat(
                lastAnswerAt: now.addingTimeInterval(-1), now: now))
        XCTAssertTrue(
            SessionDecisions.shouldAnswerHeartbeat(
                lastAnswerAt: now.addingTimeInterval(60), now: now),
            "時鐘往回跳時寧可回一則，也不要永遠不回")
    }

    /// 其它仍然忽略的型別：維持不執行，但**各自**留一行誠實說明（不再靜默）。
    @MainActor
    func testEveryOtherIgnoredMessageTypeLeavesItsOwnNote() throws {
        let cases: [(String, String)] = [
            (
                "event",
                """
                {"specVersion":"aip/1.0","messageId":"aip-ev-1","messageType":"event",
                 "name":"character.session.presence",
                 "source":{"kind":"session","id":"session.home"},
                 "sessionId":"session.home","occurredAt":"2026-09-05T12:30:00.000Z",
                 "payload":{}}
                """
            ),
            (
                "query",
                """
                {"specVersion":"aip/1.0","messageId":"aip-qy-1","messageType":"query",
                 "name":"character.session.diagnostics",
                 "source":{"kind":"session","id":"session.home"},
                 "target":{"kind":"device","id":"iphone-87b42264"},
                 "sessionId":"session.home","occurredAt":"2026-09-05T12:30:00.000Z",
                 "payload":{}}
                """
            ),
            (
                "cancel",
                """
                {"specVersion":"aip/1.0","messageId":"aip-cx-1","messageType":"cancel",
                 "name":"character.session.cancel",
                 "source":{"kind":"session","id":"session.home"},
                 "sessionId":"session.home","causationId":"aip-1-1",
                 "occurredAt":"2026-09-05T12:30:00.000Z","payload":{}}
                """
            ),
            (
                "approval-request",
                """
                {"specVersion":"aip/1.0","messageId":"aip-ap-1","messageType":"approval-request",
                 "name":"character.session.approval",
                 "source":{"kind":"runtime","id":"runtime"},
                 "target":{"kind":"human","id":"desktop"},
                 "sessionId":"session.home","correlationId":"flow-9",
                 "occurredAt":"2026-09-05T12:30:00.000Z","expiresAt":"2036-09-05T12:30:00.000Z",
                 "payload":{}}
                """
            ),
            (
                "approval-result",
                """
                {"specVersion":"aip/1.0","messageId":"aip-ar-1","messageType":"approval-result",
                 "name":"character.session.approval",
                 "source":{"kind":"runtime","id":"runtime"},
                 "sessionId":"session.home","causationId":"aip-ap-1",
                 "occurredAt":"2026-09-05T12:30:00.000Z","payload":{}}
                """
            ),
        ]

        for (label, text) in cases {
            let transport = LifecycleTransport()
            let client = SessionClient()
            client.transport = transport
            try negotiate(client)
            let ignoredBefore = client.advanced.framesIgnored
            let logBefore = client.log.count

            try feed(client, text)

            XCTAssertEqual(
                client.advanced.framesIgnored, ignoredBefore + 1, "\(label) 應該仍然算一則忽略")
            XCTAssertGreaterThan(client.log.count, logBefore, "\(label) 必須留下自己的一行說明")
            XCTAssertTrue(transport.sent.isEmpty, "\(label) 不得觸發任何送出")
        }
    }

    // MARK: - 工具

    /// 記錄送出內容的 mock transport（不開任何 socket）。
    private final class LifecycleTransport: SessionTransport {
        var isConnected = true
        var boundDeviceId: String? = "iphone-87b42264"
        private(set) var sent: [AIPEnvelope] = []
        private(set) var observations: [String] = []
        private(set) var statusSends = 0

        @discardableResult
        func sendAip(_ envelope: AIPEnvelope) -> Bool {
            sent.append(envelope)
            return true
        }

        func sendObservation(receptor: String, facts: [String: JSONValue]) {
            observations.append(receptor)
        }

        func sendStatusNow() {
            statusSends += 1
        }

        var resumes: [AIPEnvelope] { sent.filter { $0.name == SessionNames.resume } }

        func reset() {
            sent.removeAll()
            observations.removeAll()
            statusSends = 0
        }
    }

    private func envelope(_ text: String) throws -> AIPEnvelope {
        switch AIPCheck.evaluate(Data(text.utf8)) {
        case .success(let envelope):
            return envelope
        case .failure(let error):
            XCTFail("信封應該通過驗證，卻回 \(error.code.rawValue)")
            throw error
        }
    }

    @MainActor
    private func feed(_ client: SessionClient, _ envelopeText: String, now: Date = Date()) throws {
        client.handleFrame(
            try envelope(envelopeText),
            rawFrame: #"{"type":"aip","envelope":"# + envelopeText + "}", now: now)
    }

    /// 桌面對 resume 的回覆：`kind:"patches"` 且沒有東西要補 ＝ 已經對齊。
    private func resumeResponse(causationId: String) -> String {
        """
        {"specVersion":"aip/1.0","messageId":"aip-resume-reply-1","messageType":"response",
         "name":"character.session.resume",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","causationId":"\(causationId)",
         "occurredAt":"2026-09-05T12:30:05.000Z",
         "payload":{"kind":"patches","patches":[]}}
        """
    }

    private func heartbeatEnvelope(id: String) -> String {
        """
        {"specVersion":"aip/1.0","messageId":"\(id)","messageType":"heartbeat",
         "name":"character.session.heartbeat",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","occurredAt":"2026-09-05T12:30:00.000Z",
         "payload":{}}
        """
    }

    @MainActor
    private func negotiate(_ client: SessionClient) throws {
        try feed(
            client,
            """
            {"specVersion":"aip/1.0","messageId":"aip-neg-1",
             "messageType":"capability","name":"character.session.capability",
             "source":{"kind":"session","id":"session.home"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"session.home","occurredAt":"2026-09-05T12:30:00.000Z",
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

    /// 這個檔案用的權威狀態。AIP 1.0 的 snapshot **必帶 hash**，所以下面的 `hash` 一律
    /// 對這份文字現算，不寫死（寫死的話改一個欄位就會變成「hash 對不上」的假失敗）。
    private static let stateText = """
        {"characterId":"ref-shape","mood":{"kind":"happy","intensity":0.5},
         "activity":"idle","attention":{"kind":"none"},
         "truth":{"state":"none"},"members":[],"reducedMotion":false}
        """

    @MainActor
    private func applySnapshot(
        _ client: SessionClient, revision: UInt64, sequence: UInt64, epoch: UInt64
    ) throws {
        let hash = try XCTUnwrap(SemanticJSON.parse(Self.stateText)?.canonicalSHA256)
        try feed(
            client,
            """
            {"specVersion":"aip/1.0","messageId":"aip-st-\(revision)",
             "messageType":"state","name":"character.session.snapshot",
             "source":{"kind":"session","id":"session.home"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"session.home","occurredAt":"2026-09-05T12:30:04.000Z",
             "sequence":\(sequence),
             "payload":{"kind":"snapshot","revision":\(revision),"sessionEpoch":\(epoch),
                        "hash":"\(hash)","state":\(Self.stateText)}}
            """)
        XCTAssertEqual(client.advanced.revision, revision, "快照必須被套用")
    }
}
