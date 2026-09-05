//
//  ConnectionManagerGateTests.swift
//  InteractionCompanionTests
//
//  背景／前景閘門的**行為**測試（不是純函式表——`LifecycleTests` 已經把決策表釘住了）。
//
//  為什麼需要這一層：`LifecycleDecision` 說「背景不重連、不送心跳、不送角色同步」，但真正
//  會不會送出去取決於 `ConnectionManager` 有沒有在那幾個接線點都問過它。**五個接線點，
//  每一個都獨立可達、也各自被下面的測試真的執行過**：
//    1. `sendStatusNow()`             — 直接送心跳的入口
//    2. `startStatusTimer()`          — 背景中意外復活的連線不得把心跳排程叫起來
//    3. `scheduleRetry()`             — 背景斷線只誠實記錄，不排重連（斷線路徑上唯一一道；
//                                       `handleConnectionLost` 以前有一道一模一樣的判斷，
//                                       兩處在同一次同步呼叫裡讀同一個階段，第二道不可達）
//    4. 重連 work 真的觸發的那一刻    — 排程時在前景、觸發時已在背景
//    5. `sendAip()`                   — AIP 出站（capability／resume／snapshot query／result）：
//                                       桌面把**任何一則**通過身分綁定的 inbound envelope
//                                       都當成存活證明，只擋 legacy `status` 等於只擋一半
//  少接一個就是「決策表寫得很好、程式沒照著做」，而畫面上照樣顯示已連線。
//
//  這裡用**可注入的 socket 與排程**（`SocketTransport`／`WorkScheduler`）：不開真的 wss、
//  不等真的 15 秒。握手、送出佇列、接收迴圈走的都是正式路徑，只有最外面那一層被換掉。
//  誠實範圍：**模擬器內的單元測試**，沒有真 daemon、更沒有真機。
//

import XCTest

@testable import InteractionCompanion

@MainActor
final class ConnectionManagerGateTests: XCTestCase {

    // MARK: - 五個接線點

    /// 1＋2：背景中不送 status 心跳，也不把心跳排程叫起來。
    func testBackgroundNeitherSendsNorSchedulesThePresenceHeartbeat() throws {
        let harness = try Harness.connected()
        try harness.socket().clear()

        harness.connection.lifecyclePhaseChanged(to: .background)
        XCTAssertEqual(try harness.socket().sentStatuses, 0, "背景中不得送 status")
        XCTAssertEqual(harness.scheduler.repeatingCount, 0, "背景中不得有心跳排程")
        XCTAssertEqual(harness.connection.localPresence, .background)

        // 背景中意外復活的連線（例如 iOS 短暫喚醒）也不得偷偷把心跳叫回來。
        harness.connection.sendStatusNow()
        XCTAssertEqual(try harness.socket().sentStatuses, 0, "背景中的直接呼叫也要被擋下來")
    }

    /// 3：背景斷線只誠實記錄，不排重連（`scheduleRetry` 是這條路上唯一一道閘門）。
    func testALostConnectionInTheBackgroundDoesNotScheduleAReconnect() throws {
        let harness = try Harness.connected()
        harness.connection.lifecyclePhaseChanged(to: .background)
        harness.scheduler.clear()

        try harness.socket()
            .fail(NSError(domain: NSURLErrorDomain, code: NSURLErrorNetworkConnectionLost))
        XCTAssertEqual(harness.scheduler.pendingCount, 0, "背景中不得排重連")
        guard case .failed(let reason) = harness.connection.phase else {
            return XCTFail("背景斷線之後應該是 .failed，實際是 \(harness.connection.phase)")
        }
        XCTAssertTrue(
            reason.contains("背景中暫停重連"), "要誠實說「回到前景再試」，不是假裝還在重試")
    }

    /// 4：排程時還在前景、真的觸發時已經進背景 → 不開新 socket。
    func testAReconnectThatFiresAfterEnteringTheBackgroundDoesNotOpenASocket() throws {
        let harness = try Harness.connected()
        // 前景斷線：照常排一次重連。
        try harness.socket()
            .fail(NSError(domain: NSURLErrorDomain, code: NSURLErrorNetworkConnectionLost))
        XCTAssertEqual(harness.scheduler.pendingCount, 1, "前景斷線要排重連")
        XCTAssertEqual(harness.opened.count, 1)

        // 觸發之前進了背景（沒有經過 `lifecyclePhaseChanged` 的取消路徑）。
        harness.connection.lifecyclePhaseForGating = { .background }
        harness.scheduler.fireAll()
        XCTAssertEqual(harness.opened.count, 1, "背景中觸發的重連不得開 socket")
    }

    /// 回前景：立刻補一則 status（不等下一次 15 秒），並走一次 §7 的 resume。
    func testForegroundSendsStatusImmediatelyAndRealignsTheSession() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        try harness.socket().clear()

        harness.connection.lifecyclePhaseChanged(to: .background)
        harness.now += 30
        harness.connection.lifecyclePhaseChanged(to: .active)

        XCTAssertEqual(try harness.socket().sentStatuses, 1, "回前景要立刻補一則 status")
        XCTAssertEqual(try harness.socket().sentResumes, 1, "回前景要對齊角色狀態")
        XCTAssertEqual(harness.scheduler.repeatingCount, 1, "心跳排程要回來")
        XCTAssertEqual(harness.connection.localPresence, .foreground)
    }

    /// 防重入：上一則 resume 還在等桌面回覆時（10 秒寬限窗內）再回前景不重送——
    /// 重送不會帶來新資訊，卻會被記成一次失敗，連續三次就在桌面根本沒拒絕過任何事情的
    /// 情況下宣稱「無法恢復」。**有界**：超過寬限窗就再問一次。
    func testRapidForegroundCyclesSendOneResumeUntilTheGraceWindowPasses() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        try harness.socket().clear()

        for _ in 0..<3 {
            harness.connection.lifecyclePhaseChanged(to: .background)
            harness.now += 2
            harness.connection.lifecyclePhaseChanged(to: .active)
        }
        XCTAssertEqual(
            try harness.socket().sentResumes, 1,
            "寬限窗內的多次回前景只問一次（不得把「還沒回覆」記成失敗）")
        XCTAssertNotEqual(harness.connection.characterSession.syncStatus, .unrecoverable)

        // 超過寬限窗：可以再問一次（不會永遠卡在等回覆）。
        harness.connection.lifecyclePhaseChanged(to: .background)
        harness.now += SessionSyncLocal.resumeResponseGraceSeconds + 1
        harness.connection.lifecyclePhaseChanged(to: .active)
        XCTAssertEqual(try harness.socket().sentResumes, 2, "超過寬限窗要再問一次")
    }


    // MARK: - AIP 出站也在同一道閘門下

    /// 5：剛進背景、行程還沒被 iOS 暫停的那個窗口裡，socket 還活著、inbound 照常送達。
    /// 桌面把**任何一則**通過身分綁定的 inbound envelope 當成存活證明
    ///（`crates/interaction-session/src/session.rs` gate 4.1 → `note_alive` → `Presence::Online`），
    /// 所以背景送出的 resume／snapshot query 會讓桌面顯示這支手機 online，
    /// 與本機這時顯示的「背景（心跳已停）」互相矛盾——presence 心跳擋住了、AIP 沒擋住，
    /// 等於同一件事只擋了一半。
    func testBackgroundDoesNotLetAnAipFrameProveTheDeviceIsAlive() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        try harness.socket().clear()

        harness.connection.lifecyclePhaseChanged(to: .background)
        XCTAssertEqual(
            harness.connection.phase, .connected, "剛進背景時 socket 還活著（正是要擋的那個窗口）")
        let droppedBefore = harness.connection.droppedFrames

        // 桌面推播一則對不上的權威狀態：決策表會說 realign（送 snapshot query／resume）。
        try harness.socket()
            .deliver(Harness.frame(Harness.snapshotEnvelope(revision: 11, hash: Harness.wrongHash)))
        try harness.socket().deliver(Harness.frame(Harness.snapshotEnvelope(revision: 12, epoch: 6)))

        XCTAssertEqual(try harness.socket().sentSnapshotQueries, 0, "背景中不得送 snapshot query")
        XCTAssertEqual(try harness.socket().sentResumes, 0, "背景中不得送 resume")
        XCTAssertGreaterThan(
            harness.connection.droppedFrames, droppedBefore, "被擋下來的訊息要誠實計數，不得靜默")
    }

    /// 對照組：同一則 frame 在前景照樣要求對齊——閘門擋的是背景，不是把功能關掉。
    func testTheSameStateFrameInTheForegroundStillAsksForRealignment() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        try harness.socket().clear()

        try harness.socket()
            .deliver(Harness.frame(Harness.snapshotEnvelope(revision: 11, hash: Harness.wrongHash)))
        XCTAssertEqual(try harness.socket().sentSnapshotQueries, 1, "前景中 hash 對不上要問一份快照")
    }

    /// 背景中才收到 `auth-ok`：capability 被閘門擋下來，沒有人會再送第二次——
    /// 回前景是唯一的補送時機，不補就永遠停在「尚未協商」。
    func testAnAuthOkThatLandedInTheBackgroundIsRenegotiatedOnForeground() throws {
        let harness = Harness()
        harness.connection.connectIfPaired()
        let socket = try harness.socket()
        socket.events?.onOpen()
        harness.connection.lifecyclePhaseChanged(to: .background)
        socket.deliver(#"{"type":"auth-ok"}"#)
        XCTAssertEqual(harness.connection.phase, .connected)
        XCTAssertEqual(socket.sentCapabilities, 0, "背景中不得送能力宣告")
        XCTAssertFalse(harness.connection.characterSession.negotiated)

        harness.now += 30
        harness.connection.lifecyclePhaseChanged(to: .active)
        XCTAssertEqual(socket.sentCapabilities, 1, "回前景要補送一次能力宣告")
    }

    /// `SessionClient` 讀的時鐘必須就是 `ConnectionManager` 注入的那一個。
    ///
    /// 兩個時鐘並存時，由 inbound frame 觸發的 resume 會用真實牆鐘寫下「還在等回覆」，
    /// 而回前景的判斷用的是注入的時鐘：`shouldResendResumeOnForeground` 會算出負值
    ///（時鐘往回跳）而重問一次，10 秒寬限窗的保證就形同虛設。
    func testTheSessionReadsTheSameInjectedClockAsTheLifecycleGates() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        try harness.socket().clear()

        // 由 inbound frame（不是生命週期）觸發一次 resume：它寫下的時間也必須是注入的時鐘。
        try harness.socket().deliver(Harness.frame(Harness.snapshotEnvelope(revision: 11, epoch: 6)))
        XCTAssertEqual(try harness.socket().sentResumes, 1, "epoch 不同要向桌面要求對齊")

        harness.connection.lifecyclePhaseChanged(to: .background)
        harness.now += 2
        harness.connection.lifecyclePhaseChanged(to: .active)
        XCTAssertEqual(
            try harness.socket().sentResumes, 1,
            "同一個時鐘：寬限窗（\(Int(SessionSyncLocal.resumeResponseGraceSeconds)) 秒）內不重送")
    }

    /// 重連閘門只有一道。同一次同步呼叫裡問第二次不會有不同答案：多出來的那一道永遠
    /// 不可達，只會讓「接線點」的枚舉虛胖成一件沒有任何測試能證明的事。
    func testTheReconnectGateIsAskedExactlyOncePerDisconnect() throws {
        let harness = try Harness.connected()
        harness.gateReads = 0

        try harness.socket()
            .fail(NSError(domain: NSURLErrorDomain, code: NSURLErrorNetworkConnectionLost))

        XCTAssertEqual(harness.scheduler.pendingCount, 1, "前景斷線要排重連")
        XCTAssertEqual(
            harness.gateReads, 1, "斷線路徑上的重連閘門只有一道；問第二次代表有不可達的重複判斷")
    }

    // MARK: - 注入點本身

    /// 可注入的 socket 真的被用上了：握手走的是同一條路（不是測試專用旁路）。
    func testTheInjectedSocketCarriesTheRealHandshake() throws {
        let harness = try Harness.connected()
        XCTAssertEqual(harness.opened.count, 1)
        XCTAssertEqual(harness.opened.first?.url.absoluteString, "wss://127.0.0.1:18790")
        XCTAssertEqual(harness.opened.first?.fingerprint, Harness.fingerprint)
        XCTAssertTrue(
            try harness.socket().sent.contains { $0.contains("\"type\":\"auth\"") },
            "auth 必須真的從 socket 送出去")
        XCTAssertEqual(harness.connection.phase, .connected)
    }

    /// 每開一條 socket 就換一個世代：上一條連線的遲到訊息不得再被套用（決策表規則 0）。
    func testEachSocketGetsItsOwnGenerationAndLateFramesAreDropped() throws {
        let harness = try Harness.connected()
        try harness.negotiateAndSnapshot()
        let stale = try harness.socket()

        // 重新連線（使用者按「連線」／位址變更後重連）：新 socket、新世代。
        // 舊 socket 早就被 `teardownSocket` 取消了，但它先前收下的那一則 frame 仍可能
        // 在這之後才被交上來——這正是規則 0 要擋的情況。
        harness.connection.connectIfPaired()
        XCTAssertEqual(harness.opened.count, 2, "重新連線要開一條新的 socket")
        let fresh = try harness.socket()
        fresh.events?.onOpen()
        fresh.deliver(#"{"type":"auth-ok"}"#)
        XCTAssertEqual(harness.connection.phase, .connected)

        // 舊 socket 的遲到訊息（宣告另一個 epoch 的權威狀態）現在才到。
        // 第一道防線就在 `ConnectionManager`：世代不符的整則訊息連解碼結果都不採用
        //（`SessionClient` 自己那一道由 `ReceiveDecisionConformanceTests` 涵蓋）。
        stale.deliver(Harness.frame(Harness.snapshotEnvelope(revision: 1, epoch: 99)))
        XCTAssertEqual(
            harness.connection.characterSession.advanced.revision, 10, "舊世代的狀態不得套用")
        XCTAssertEqual(harness.connection.characterSession.advanced.epoch, 5)
        XCTAssertTrue(
            harness.connection.log.contains { $0.contains("上一條連線的遲到訊息") },
            "被丟掉的遲到訊息要留下一行誠實說明")
    }

    // MARK: - 測試替身

    /// 一台已經配對、可注入 socket 與排程的 `ConnectionManager`。
    @MainActor
    private final class Harness {
        static let fingerprint = String(repeating: "ab", count: 32)

        let connection: ConnectionManager
        let scheduler = ManualScheduler()
        var opened: [(url: URL, fingerprint: String, socket: FakeSocket)] = []
        /// 測試自己的時鐘（單調時鐘與牆鐘一起走，省得兩邊各記一份）。
        var now: TimeInterval = 1000
        /// 生命週期閘門被問過幾次（重複而不可達的那一道會讓這個數字虛胖）。
        var gateReads = 0

        init() {
            let store = InMemoryPairingStore(
                StoredPairing(
                    deviceId: "iphone-87b42264", deviceToken: "token", host: "127.0.0.1",
                    port: 18790, fingerprint: Harness.fingerprint))
            let defaults = UserDefaults(suiteName: "gate-tests-\(UUID().uuidString)")!
            connection = ConnectionManager(store: store, defaults: defaults)
            connection.scheduler = scheduler
            connection.socketFactory = { [weak self] url, fingerprint, events in
                let socket = FakeSocket()
                socket.events = events
                self?.opened.append((url, fingerprint, socket))
                return socket
            }
            connection.lifecyclePhaseForGating = { [weak self] in
                self?.gateReads += 1
                return self?.connection.lifecyclePhase ?? .active
            }
            connection.monotonicNow = { [weak self] in self?.now ?? 0 }
            connection.wallClockNow = { [weak self] in
                Date(timeIntervalSince1970: self?.now ?? 0)
            }
            connection.statusProvider = { (SensorFlags(), PermissionStates()) }
        }

        func socket() throws -> FakeSocket {
            try XCTUnwrap(opened.last?.socket, "還沒有任何 socket 被建立")
        }

        /// 已經 auth-ok 的 harness。
        static func connected() throws -> Harness {
            let harness = Harness()
            harness.connection.connectIfPaired()
            let socket = try harness.socket()
            socket.events?.onOpen()
            socket.deliver(#"{"type":"auth-ok"}"#)
            XCTAssertEqual(harness.connection.phase, .connected, "auth-ok 之後應該是已連線")
            return harness
        }

        /// 協商 ＋ 一份權威快照：resume 要有東西可以對齊。
        func negotiateAndSnapshot() throws {
            let socket = try socket()
            socket.deliver(Self.frame(Self.capabilityEnvelope))
            XCTAssertTrue(connection.characterSession.negotiated, "協商必須成立")
            socket.deliver(Self.frame(Self.snapshotEnvelope()))
            XCTAssertEqual(connection.characterSession.advanced.revision, 10, "快照必須套用")
        }

        static func frame(_ envelope: String) -> String {
            #"{"type":"aip","envelope":"# + envelope + "}"
        }

        static let capabilityEnvelope = """
            {"specVersion":"aip/1.0","messageId":"aip-neg-1","messageType":"capability",
             "name":"character.session.capability",
             "source":{"kind":"session","id":"session.home"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"session.home","occurredAt":"2026-09-05T12:30:00.000Z",
             "payload":{"specVersion":"aip/1.0","newerMinor":false,"role":"remote-renderer",
                        "syncClass":"semantic","intents":{},"inputs":[],"unsupportedInputs":[],
                        "limits":{"maxMessageBytes":65536,"maxPayloadBytes":32768,
                                  "maxIntentsPerMinute":60}}}
            """

        static let stateText = """
            {"characterId":"ref-shape","mood":{"kind":"happy","intensity":0.5},
             "activity":"idle","attention":{"kind":"none"},
             "truth":{"state":"none"},"members":[],"reducedMotion":false}
            """

        /// 一個格式正確、但一定對不上任何 state 的 hash（用來觸發 `hash-mismatch`）。
        static let wrongHash = String(repeating: "0", count: 64)

        static func snapshotEnvelope(
            revision: UInt64 = 10, epoch: UInt64 = 5, hash overrideHash: String? = nil
        ) -> String {
            let hash = overrideHash ?? SemanticJSON.parse(stateText)?.canonicalSHA256 ?? "?"
            return """
                {"specVersion":"aip/1.0","messageId":"aip-st-\(epoch)-\(revision)",
                 "messageType":"state","name":"character.session.snapshot",
                 "source":{"kind":"session","id":"session.home"},
                 "target":{"kind":"device","id":"iphone-87b42264"},
                 "sessionId":"session.home","occurredAt":"2026-09-05T12:30:04.000Z",
                 "sequence":\(revision),
                 "payload":{"kind":"snapshot","revision":\(revision),"sessionEpoch":\(epoch),
                            "hash":"\(hash)","state":\(stateText)}}
                """
        }
    }

    /// 記憶體版配對儲存（單元測試不碰 Keychain）。
    private final class InMemoryPairingStore: PairingStorage {
        private var stored: StoredPairing?
        init(_ stored: StoredPairing?) { self.stored = stored }
        func save(_ pairing: StoredPairing) throws { stored = pairing }
        func load() -> StoredPairing? { stored }
        func clear() throws { stored = nil }
    }

    /// 假 socket：記下送出的每一行，讓測試自己決定何時「收到」訊息或斷線。
    private final class FakeSocket: SocketTransport {
        var events: SocketEvents?
        private(set) var sent: [String] = []
        private var pendingReceive: ((Result<SocketFrame, Error>) -> Void)?
        private(set) var cancelled = false

        func resume() {}
        func cancel() { cancelled = true }

        func send(_ text: String, completion: @escaping (Error?) -> Void) {
            sent.append(text)
            completion(nil)
        }

        func receive(_ completion: @escaping (Result<SocketFrame, Error>) -> Void) {
            pendingReceive = completion
        }

        func ping(_ completion: @escaping (Error?) -> Void) { completion(nil) }

        /// 桌面送來一則訊息。
        func deliver(_ text: String) {
            let completion = pendingReceive
            pendingReceive = nil
            completion?(.success(.text(text)))
        }

        /// 連線掛了。
        func fail(_ error: Error) {
            let completion = pendingReceive
            pendingReceive = nil
            completion?(.failure(error))
        }

        func clear() { sent.removeAll() }
        var sentStatuses: Int { sent.filter { $0.contains("\"type\":\"status\"") }.count }
        var sentResumes: Int { sent.filter { $0.contains("character.session.resume") }.count }
        var sentSnapshotQueries: Int {
            sent.filter {
                $0.contains("character.session.snapshot") && $0.contains("\"messageType\":\"query\"")
            }.count
        }
        var sentCapabilities: Int {
            sent.filter { $0.contains("character.session.capability") }.count
        }
    }

    /// 手動排程：不等真的秒數，由測試決定何時觸發。
    private final class ManualScheduler: WorkScheduler {
        private final class Work: ScheduledWork {
            let repeats: Bool
            let body: () -> Void
            var cancelled = false
            init(repeats: Bool, body: @escaping () -> Void) {
                self.repeats = repeats
                self.body = body
            }
            func cancel() { cancelled = true }
        }

        private var works: [Work] = []

        func schedule(after seconds: TimeInterval, repeats: Bool, _ body: @escaping () -> Void)
            -> ScheduledWork
        {
            let work = Work(repeats: repeats, body: body)
            works.append(work)
            return work
        }

        /// 還在等待中（沒被取消）的一次性排程數。
        var pendingCount: Int { works.filter { !$0.cancelled && !$0.repeats }.count }
        /// 還活著的重複排程（＝心跳）數。
        var repeatingCount: Int { works.filter { !$0.cancelled && $0.repeats }.count }

        func fireAll() {
            for work in works where !work.cancelled {
                work.body()
            }
        }

        func clear() { works.removeAll() }
    }
}
