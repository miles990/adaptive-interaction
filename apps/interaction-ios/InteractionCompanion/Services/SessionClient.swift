//
//  SessionClient.swift
//  InteractionCompanion
//
//  AIP Character Session 的手機端（`docs/aip/character-session.md`、
//  `docs/aip/transport-bindings.md` §1）。角色是 `remote-renderer`：
//  只送語意事件、只收語意狀態與 Behavior Intent，**不擁有**任何共享狀態。
//
//  誠實不變量（對應 repo CLAUDE.md）：
//  - `observed` 只有在本地動畫**真的播完**之後才回；`accepted`／`applied` 都不是 `observed`。
//  - 不支援的 intent 回 `rejected{unsupported-capability}`，**絕不**回 `observed`。
//  - `verified` 永遠不由 App 產生（host 端 gate 也會擋，這裡先自律）。
//  - 未知 message type／未知 name／不合規訊息一律不執行，只誠實記錄。
//  - 權威狀態只由 host 決定：revision 對不上就要 resume，hash 對不上就丟掉本地狀態要 snapshot；
//    比本地舊的一律忽略（rollback 防護），除非 host 明說重建了 session。
//  - 所有集合有界：待播 intent ≤ 8、去重環 256、記錄 50 行、resume 連敗計數。
//  - 重連不重播互動事件與 intent（§8 離線政策：touch 靠 deadline 過期、intent 是 drop-if-offline）。
//
//  執行緒模型：本類別是 `@MainActor`；`ConnectionManager` 的接收迴圈已經把回呼
//  funnel 到主執行緒，所以呼叫端用 `MainActor.assumeIsolated` 同步進來——這樣
//  state patch 的處理順序才等於它們到達的順序（用 `Task` 排程無法保證這件事）。
//

import Foundation
import UIKit

// MARK: - Transport

/// SessionClient 需要的傳輸能力。由 `ConnectionManager` 實作。
protocol SessionTransport: AnyObject {
    /// 目前是否處於已認證的連線狀態。
    var isConnected: Bool { get }
    /// 配對綁定出來的裝置身分（`source` 必須宣稱同一個）。
    var boundDeviceId: String? { get }
    /// 送一則 AIP envelope；被有界佇列丟棄或編碼失敗時回 `false`（不假裝送出）。
    @discardableResult
    func sendAip(_ envelope: AIPEnvelope) -> Bool
    /// 舊路徑：wire protocol v1 的觀察訊息（尚未協商的桌面才會走）。
    func sendObservation(receptor: String, facts: [String: JSONValue])
}

// MARK: - 名稱常數（與 Rust `interaction_session::types` 一致）

enum SessionNames {
    static let sessionId = "session.home"
    static let capability = "character.session.capability"
    static let snapshot = "character.session.snapshot"
    static let patch = "character.session.patch"
    static let resume = "character.session.resume"
    static let result = "character.session.result"
    static let behaviorRequest = "character.behavior.request"
    static let behaviorCancel = "character.behavior.cancel"
    static let touch = "character.interaction.touch"
    static let dismiss = "character.interaction.dismiss"
    static let sessionReset = "session-reset"
}

// MARK: - 本機認知（純資料）

/// App 這一端對角色同步的認知。所有欄位都只由 `SessionDecisions` 的純函式改。
struct SessionSyncLocal: Equatable {
    var revision: UInt64 = 0
    var sequence: UInt64 = 0
    var epoch: UInt64 = 0
    var state: SemanticJSON?
    /// 連續幾次「送了 resume 卻還是沒對齊」。
    var resumeFailures: Int = 0
    var unrecoverable: Bool = false

    /// 連續失敗到這個數字就顯示「無法恢復，請重新連接」。
    static let resumeFailureLimit = 3
}

/// 一則 `state`（或 resume 回來的一項）的共同形狀。
struct SessionStateMessage: Equatable {
    enum Kind: Equatable {
        case snapshot(state: SemanticJSON)
        case patch(patch: SemanticJSON, baseRevision: UInt64)
    }

    var kind: Kind
    var revision: UInt64
    var sequence: UInt64?
    var epoch: UInt64?
    var hash: String?
    /// host 明說重建了 session：接收端必須丟棄本地狀態。
    var isSessionReset: Bool
}

/// 套用一則 state 之後該做什麼。
enum SessionStateOutcome: Equatable {
    /// 可以套用（`state` 是套用後的完整權威狀態）。
    case applied(revision: UInt64, epoch: UInt64, state: SemanticJSON)
    /// 比本地舊：忽略（rollback 防護）。
    case ignoredRollback
    /// 已經套用過同一個版本：忽略。
    case ignoredAlreadyApplied
    /// 接不上：要送 resume。
    case needsResume
    /// 套用後對不上 host 的 hash：丟掉本地狀態、要一份完整快照。
    case needsSnapshot
    /// 不是合法的 state 訊息：不執行。
    case invalid
}

/// 收到一則 Behavior Intent 之後該做什麼。
enum SessionIntentOutcome: Equatable {
    case play(PlayingIntent)
    /// 同一則訊息又來一次：不重播。
    case duplicate
    /// 已過 deadline：不播，回 `expired`。
    case expired
    /// 本版不支援這個 intent：回 `rejected{unsupported-capability}`，**不是** `observed`。
    case unsupported(String)
    /// 還沒協商就收到 intent：不播（正常情況不會發生）。
    case notNegotiated
    case invalid
}

// MARK: - 純決策（沒有 I/O、沒有時鐘；全部可單獨測試）

enum SessionDecisions {
    /// 本 App 的能力宣告。內容固定（golden test 釘住），只有 reduced motion 隨系統設定變。
    static func capabilityAnnouncement(reducedMotion: Bool) -> AIPCapabilityAnnouncement {
        AIPCapabilityAnnouncement(
            features: [
                // haptic 由受 governor 管的 `haptic.pulse` 動器負責，
                // 角色 intent **不得**自己震動 → 這裡誠實宣告 false。
                "haptic": .bool(false),
                "reducedMotion": .bool(reducedMotion),
            ],
            inputs: [SessionNames.touch, SessionNames.dismiss],
            intents: BehaviorIntent.allCases.map(\.rawValue),
            limits: AIPCapabilityLimits(maxMessageBytes: AIPLimits.maxMessageBytes),
            profiles: ["character-session"],
            role: .remoteRenderer,
            specVersions: [AIPConstants.specVersion],
            syncClasses: [.semantic])
    }

    /// 任一 `Encodable` → `JSONValue`（envelope payload 用）。失敗回 `nil`，不塞假資料。
    static func payloadValue<T: Encodable>(_ value: T) -> JSONValue? {
        guard let data = try? JSONEncoder().encode(value),
            let json = try? JSONDecoder().decode(JSONValue.self, from: data)
        else { return nil }
        return json
    }

    /// 共同的信封骨架。`source` 只是宣稱，host 會拿配對身分比對。
    static func envelope(
        type: AIPMessageType,
        name: String,
        deviceId: String,
        sessionId: String,
        messageId: String,
        now: Date,
        payload: JSONValue
    ) -> AIPEnvelope {
        AIPEnvelope(
            specVersion: AIPConstants.specVersion,
            messageId: messageId,
            messageType: type,
            name: name,
            source: AIPParty(kind: .device, id: deviceId),
            occurredAt: WireTime.nowISO8601(now),
            payload: payload,
            sessionId: sessionId)
    }

    static func capabilityEnvelope(
        deviceId: String, sessionId: String, messageId: String, now: Date, reducedMotion: Bool
    ) -> AIPEnvelope? {
        guard let payload = payloadValue(capabilityAnnouncement(reducedMotion: reducedMotion))
        else { return nil }
        return envelope(
            type: .capability, name: SessionNames.capability, deviceId: deviceId,
            sessionId: sessionId, messageId: messageId, now: now, payload: payload)
    }

    /// 互動事件（§7：`character.interaction.*` 的 `expiresAt` 必填，建議 5 秒）。
    static func interactionEnvelope(
        name: String,
        payload: [String: JSONValue],
        deviceId: String,
        sessionId: String,
        messageId: String,
        now: Date,
        ttlMs: Int = AIPLimits.defaultInteractionTtlMs
    ) -> AIPEnvelope {
        var result = envelope(
            type: .event, name: name, deviceId: deviceId, sessionId: sessionId,
            messageId: messageId, now: now, payload: .object(payload))
        result.target = AIPParty(kind: .session, id: sessionId)
        result.expiresAt = WireTime.nowISO8601(now.addingTimeInterval(Double(ttlMs) / 1000))
        return result
    }

    static func touchEnvelope(
        kind: String, deviceId: String, sessionId: String, messageId: String, now: Date
    ) -> AIPEnvelope {
        interactionEnvelope(
            name: SessionNames.touch, payload: ["kind": .string(kind)], deviceId: deviceId,
            sessionId: sessionId, messageId: messageId, now: now)
    }

    static func dismissEnvelope(
        deviceId: String, sessionId: String, messageId: String, now: Date
    ) -> AIPEnvelope {
        interactionEnvelope(
            name: SessionNames.dismiss, payload: [:], deviceId: deviceId, sessionId: sessionId,
            messageId: messageId, now: now)
    }

    /// §7 步驟 3：重連後對齊。`sessionEpoch` 是 host 讀的鍵名。
    static func resumeEnvelope(
        local: SessionSyncLocal, deviceId: String, sessionId: String, messageId: String, now: Date
    ) -> AIPEnvelope {
        var result = envelope(
            type: .query, name: SessionNames.resume, deviceId: deviceId, sessionId: sessionId,
            messageId: messageId, now: now,
            payload: .object([
                "lastRevision": .number(Double(local.revision)),
                "lastSequence": .number(Double(local.sequence)),
                "sessionEpoch": .number(Double(local.epoch)),
            ]))
        result.target = AIPParty(kind: .session, id: sessionId)
        return result
    }

    static func snapshotQueryEnvelope(
        deviceId: String, sessionId: String, messageId: String, now: Date
    ) -> AIPEnvelope {
        var result = envelope(
            type: .query, name: SessionNames.snapshot, deviceId: deviceId, sessionId: sessionId,
            messageId: messageId, now: now, payload: .object([:]))
        result.target = AIPParty(kind: .session, id: sessionId)
        return result
    }

    /// 對 host 訊息的處理結果。`verified` 永遠不會出現在這裡（App 沒有人類驗證路徑）。
    static func resultEnvelope(
        causationId: String,
        status: AIPOutcome,
        code: AIPErrorCode?,
        deviceId: String,
        sessionId: String,
        messageId: String,
        now: Date
    ) -> AIPEnvelope? {
        guard status != .verified else { return nil }  // 自律：App 不得宣告 verified
        var payload: [String: JSONValue] = ["status": .string(status.rawValue)]
        if let code {
            payload["code"] = .string(code.rawValue)
        }
        var result = envelope(
            type: .result, name: SessionNames.result, deviceId: deviceId, sessionId: sessionId,
            messageId: messageId, now: now, payload: .object(payload))
        result.causationId = causationId
        return result
    }

    // MARK: state

    /// 把一則 `state` envelope 的 payload（逐字保留）讀成 `SessionStateMessage`。
    static func stateMessage(
        payload: SemanticJSON, baseRevision: UInt64? = nil, sequence: UInt64? = nil
    ) -> SessionStateMessage? {
        guard let revision = payload["revision"]?.uintValue else { return nil }
        let hash = payload["hash"]?.stringValue
        let epoch = payload["sessionEpoch"]?.uintValue
        let seq = payload["sequence"]?.uintValue ?? sequence
        let isReset = payload["reason"]?.stringValue == SessionNames.sessionReset
        switch payload["kind"]?.stringValue {
        case "snapshot":
            guard let state = payload["state"] else { return nil }
            return SessionStateMessage(
                kind: .snapshot(state: state), revision: revision, sequence: seq, epoch: epoch,
                hash: hash, isSessionReset: isReset)
        case "patch":
            guard let patch = payload["patch"],
                let base = payload["baseRevision"]?.uintValue ?? baseRevision
            else { return nil }
            return SessionStateMessage(
                kind: .patch(patch: patch, baseRevision: base), revision: revision, sequence: seq,
                epoch: epoch, hash: hash, isSessionReset: isReset)
        default:
            return nil
        }
    }

    /// AIP §6 的完整接收規則：rollback 防護、`session-reset` 例外、patch 續接、hash 核對。
    static func apply(_ message: SessionStateMessage, to local: SessionSyncLocal)
        -> SessionStateOutcome
    {
        switch message.kind {
        case .snapshot(let state):
            let epoch = message.epoch ?? local.epoch
            let reset = message.isSessionReset && epoch > local.epoch
            if !reset {
                if message.revision < local.revision { return .ignoredRollback }
                if message.revision == local.revision { return .ignoredAlreadyApplied }
            }
            if let hash = message.hash, state.canonicalSHA256 != hash {
                // 快照自己就對不上自己的 hash：不執行、也不再要一次同一份（會無限迴圈）。
                return .invalid
            }
            return .applied(revision: message.revision, epoch: epoch, state: state)
        case .patch(let patch, let baseRevision):
            if message.revision < local.revision { return .ignoredRollback }
            if message.revision == local.revision { return .ignoredAlreadyApplied }
            guard let base = local.state, baseRevision == local.revision else { return .needsResume }
            let merged = SemanticJSON.mergePatch(base, patch)
            if let hash = message.hash, merged.canonicalSHA256 != hash {
                return .needsSnapshot
            }
            return .applied(
                revision: message.revision, epoch: message.epoch ?? local.epoch, state: merged)
        }
    }

    /// 送出一次 resume 就記一次「還沒對齊」；連續達上限即視為無法恢復。
    static func noteResumeAttempt(_ local: inout SessionSyncLocal) {
        local.resumeFailures += 1
        if local.resumeFailures >= SessionSyncLocal.resumeFailureLimit {
            local.unrecoverable = true
        }
    }

    /// 只要成功套用過一次狀態，先前的失敗就不再有意義。
    static func noteSyncSucceeded(_ local: inout SessionSyncLocal) {
        local.resumeFailures = 0
        local.unrecoverable = false
    }

    // MARK: intent

    /// §5：不支援的 intent 只能降級並回 `rejected`，不得謊稱播過。
    static func intentOutcome(
        envelope: AIPEnvelope,
        now: Date,
        negotiated: Bool,
        alreadySeen: Bool
    ) -> SessionIntentOutcome {
        guard negotiated else { return .notNegotiated }
        guard let name = envelope.payload?.objectValue?["intent"]?.stringValue, !name.isEmpty else {
            return .invalid
        }
        if envelope.isExpired(now: now) { return .expired }
        if alreadySeen { return .duplicate }
        guard let intent = BehaviorIntent(rawValue: name) else { return .unsupported(name) }
        let body = envelope.payload?.objectValue ?? [:]
        return .play(
            PlayingIntent(
                messageId: envelope.messageId,
                intent: intent,
                intensity: min(max(body["intensity"]?.doubleValue ?? 0.5, 0), 1),
                interruptible: body["interruptible"]?.boolValue ?? true))
    }

    // MARK: 同步狀態文案

    static func syncStatus(
        local: SessionSyncLocal,
        connected: Bool,
        negotiated: Bool,
        hasUnsupportedIntents: Bool,
        resuming: Bool
    ) -> SessionSyncStatus {
        guard connected else { return .offline }
        if local.unrecoverable { return .unrecoverable }
        guard negotiated else { return .notNegotiated }
        if resuming { return .resuming }
        if hasUnsupportedIntents { return .partialCapabilities }
        return .synced
    }
}

// MARK: - JSONValue 小工具（envelope payload 走訪）

extension JSONValue {
    fileprivate var objectValue: [String: JSONValue]? {
        if case .object(let map) = self { return map }
        return nil
    }
}

// MARK: - 進階診斷（只在「連線」頁的診斷折疊區顯示）

/// 一般模式**不得**顯示這些數字（`docs/aip/character-session.md` §11）。
struct SessionAdvancedInfo: Equatable {
    var revision: UInt64 = 0
    var sequence: UInt64 = 0
    var epoch: UInt64 = 0
    var appliedStates = 0
    var resumesSent = 0
    var intentsPlayed = 0
    var intentsRejected = 0
    var intentsDropped = 0
    var framesIgnored = 0
}

// MARK: - SessionClient

/// 手機端的 Character Session 成員。
@MainActor
final class SessionClient: ObservableObject {
    /// 待播 intent 的上限（有界；滿了淘汰最舊並誠實計數）。
    /// `nonisolated`：這是純常數，測試與診斷不必為了讀它跳到主執行緒。
    nonisolated static let maxPendingIntents = 8

    // MARK: 對外可觀察的狀態

    @Published private(set) var syncStatus: SessionSyncStatus = .offline
    /// 權威語意狀態的投影；還沒收到快照時是 `nil`（此時角色頁退回舊路徑）。
    @Published private(set) var presentation: CharacterSemanticState?
    @Published private(set) var negotiated = false
    /// 協商後被標成不支援的 intent 名稱（有就顯示「部分能力目前不可用」）。
    @Published private(set) var unsupportedIntents: [String] = []
    /// 目前要在本地播的 intent；播完由 View 呼叫 `intentDidFinishPlaying`。
    @Published private(set) var nowPlaying: PlayingIntent?
    @Published private(set) var advanced = SessionAdvancedInfo()
    /// 診斷記錄（最多 50 行，只在本機）。
    @Published private(set) var log: [String] = []

    // MARK: 私有狀態

    weak var transport: SessionTransport?

    private var local = SessionSyncLocal()
    private var dedupe = AIPDedupeRing()
    private var queue: [PlayingIntent] = []
    private var resuming = false
    private var messageCounter: UInt64 = 0
    private var sessionId = SessionNames.sessionId

    nonisolated init() {}

    // MARK: 連線生命週期

    /// auth-ok 之後：（重新）協商；已經有本地狀態時再要求對齊。
    ///
    /// **不重播**任何互動事件或 intent（§8）——重連只 reconcile 狀態。
    func connectionDidConnect(now: Date = Date()) {
        guard let deviceId = transport?.boundDeviceId else { return }
        negotiated = false
        unsupportedIntents = []
        nowPlaying = nil
        queue.removeAll()
        resuming = false
        guard
            let envelope = SessionDecisions.capabilityEnvelope(
                deviceId: deviceId, sessionId: sessionId, messageId: nextMessageId("cap"),
                now: now, reducedMotion: UIAccessibility.isReduceMotionEnabled)
        else {
            note("無法產生能力宣告，角色同步暫時無法使用")
            refreshStatus()
            return
        }
        _ = transport?.sendAip(envelope)
        if local.revision > 0 {
            sendResume(now: now)
        }
        refreshStatus()
    }

    /// 斷線：成員身分留在 host 那邊等逾時，但本地必須重新協商才能再送事件。
    func connectionDidDisconnect() {
        negotiated = false
        nowPlaying = nil
        queue.removeAll()
        resuming = false
        refreshStatus()
    }

    // MARK: 收訊

    /// 處理一則 `{"type":"aip","envelope":…}`。
    ///
    /// `rawFrame` 是原始 frame 文字：state 的 hash 必須對 **host 寫出來的文字**取，
    /// 重新編碼過的 JSON 會讓 `0.0` 變成 `0`、hash 就對不上（見 `CharacterSemantic.swift`）。
    func handleFrame(_ envelope: AIPEnvelope, rawFrame: String, now: Date = Date()) {
        if let failure = envelope.validate() {
            advanced.framesIgnored += 1
            note("忽略一則不合規的角色同步訊息（\(failure.code.rawValue)）")
            return
        }
        // 宣稱不是身分：只有 host（runtime／session）能對我們送這些訊息。
        guard envelope.source.kind == .runtime || envelope.source.kind == .session else {
            advanced.framesIgnored += 1
            note("忽略一則來源不對的角色同步訊息")
            return
        }
        // 點對點訊息只收給自己的那些。
        if let target = envelope.target, target.kind == .device,
            target.id != transport?.boundDeviceId
        {
            advanced.framesIgnored += 1
            return
        }
        if let id = envelope.sessionId, !id.isEmpty {
            sessionId = id
        }

        switch envelope.messageType {
        case .capability:
            handleNegotiated(envelope)
        case .state:
            handleState(envelope, rawFrame: rawFrame, now: now)
        case .response:
            handleResponse(envelope, rawFrame: rawFrame, now: now)
        case .command:
            handleCommand(envelope, now: now)
        case .result:
            handleHostResult(envelope, now: now)
        case .error:
            handleHostError(envelope)
        case .event, .query, .cancel, .approvalRequest, .approvalResult, .heartbeat:
            advanced.framesIgnored += 1
        case .unknown:
            advanced.framesIgnored += 1
            note("忽略一則本版不認得的角色同步訊息")
        }
        refreshStatus()
    }

    // MARK: 送訊（使用者動作）

    /// 角色頁的點擊／長按。回傳要顯示給使用者的一行誠實說明。
    @discardableResult
    func touch(kind: String, now: Date = Date()) -> String {
        guard let transport, transport.isConnected else {
            return "未連線，觸控事件未送出（已丟棄）"
        }
        guard negotiated, let deviceId = transport.boundDeviceId else {
            // 舊桌面：走既有的 wire protocol v1 觀察路徑（不重複送，避免同一次觸碰算兩次）。
            transport.sendObservation(receptor: "iphone.touch", facts: ["kind": .string(kind)])
            return "已送出觸控事件：\(kind)"
        }
        let envelope = SessionDecisions.touchEnvelope(
            kind: kind, deviceId: deviceId, sessionId: sessionId,
            messageId: nextMessageId("touch"), now: now)
        guard transport.sendAip(envelope) else {
            return "觸控事件未送出（已丟棄）"
        }
        return "已送出觸控事件：\(kind)"
    }

    /// 使用者離開角色頁＝不再看著角色（§4 `character.interaction.dismiss`）。
    /// 只在已協商時送；舊路徑沒有對應訊息，不硬造。
    func dismiss(now: Date = Date()) {
        guard negotiated, let transport, transport.isConnected,
            let deviceId = transport.boundDeviceId
        else { return }
        let envelope = SessionDecisions.dismissEnvelope(
            deviceId: deviceId, sessionId: sessionId, messageId: nextMessageId("dismiss"), now: now)
        _ = transport.sendAip(envelope)
    }

    /// 本地動畫**真的播完**之後才回 `observed`（誠實階梯的底線）。
    func intentDidFinishPlaying(messageId: String, now: Date = Date()) {
        guard let playing = nowPlaying, playing.messageId == messageId else { return }
        sendResult(causationId: messageId, status: .observed, code: nil, now: now)
        advanced.intentsPlayed += 1
        nowPlaying = queue.isEmpty ? nil : queue.removeFirst()
    }

    // MARK: - 內部：各種訊息

    private func handleNegotiated(_ envelope: AIPEnvelope) {
        guard let payload = envelope.payload,
            let data = try? JSONEncoder().encode(payload),
            let negotiatedCapabilities = try? JSONDecoder().decode(
                AIPNegotiatedCapabilities.self, from: data)
        else {
            note("收到看不懂的能力協商結果，角色同步維持關閉")
            return
        }
        negotiated = true
        unsupportedIntents =
            negotiatedCapabilities.intents
            .filter { $0.value == .unsupported }
            .keys.sorted()
        if negotiatedCapabilities.newerMinor {
            note("桌面的角色同步版本較新，本 App 只用雙方都支援的部分")
        }
        note("角色同步已協商（不支援的動作 \(unsupportedIntents.count) 個）")
    }

    private func handleState(_ envelope: AIPEnvelope, rawFrame: String, now: Date) {
        guard let payload = payloadTokens(rawFrame),
            let message = SessionDecisions.stateMessage(
                payload: payload, baseRevision: envelope.baseRevision, sequence: envelope.sequence)
        else {
            advanced.framesIgnored += 1
            note("忽略一則讀不出來的角色狀態訊息")
            return
        }
        consume(message, now: now)
    }

    private func handleResponse(_ envelope: AIPEnvelope, rawFrame: String, now: Date) {
        guard envelope.name == SessionNames.resume || envelope.name == SessionNames.snapshot else {
            advanced.framesIgnored += 1
            return
        }
        guard let payload = payloadTokens(rawFrame) else {
            advanced.framesIgnored += 1
            return
        }
        resuming = false
        switch payload["kind"]?.stringValue {
        case "patches":
            let items = payload["patches"]?.arrayValue ?? []
            if items.isEmpty {
                // 已經對齊了（sequence 落後不是狀態錯誤）：沒有東西要補。
                SessionDecisions.noteSyncSucceeded(&local)
                return
            }
            for item in items {
                guard
                    let message = SessionDecisions.stateMessage(
                        payload: item, baseRevision: item["baseRevision"]?.uintValue)
                else {
                    failedToSync()
                    return
                }
                if !consume(message, now: now) { return }
            }
        case "snapshot":
            guard let message = SessionDecisions.stateMessage(payload: payload) else {
                failedToSync()
                return
            }
            _ = consume(message, now: now)
        default:
            failedToSync()
        }
    }

    /// 套用一則 state；回傳「是否還在同步軌道上」（false 表示已經改走 resume／snapshot）。
    @discardableResult
    private func consume(_ message: SessionStateMessage, now: Date) -> Bool {
        switch SessionDecisions.apply(message, to: local) {
        case .applied(let revision, let epoch, let state):
            local.revision = revision
            local.epoch = epoch
            local.state = state
            if let sequence = message.sequence, sequence > local.sequence {
                local.sequence = sequence
            }
            resuming = false
            SessionDecisions.noteSyncSucceeded(&local)
            presentation = CharacterSemanticState.project(state)
            if presentation == nil {
                note("收到的角色狀態不符合本版認得的形狀，未套用到畫面")
            }
            advanced.appliedStates += 1
            advanced.revision = revision
            advanced.sequence = local.sequence
            advanced.epoch = epoch
            return true
        case .ignoredRollback, .ignoredAlreadyApplied:
            advanced.framesIgnored += 1
            return true
        case .needsResume:
            sendResume(now: now)
            return false
        case .needsSnapshot:
            // 對不上 host 的內容：丟掉本地狀態，重新要一份完整的。
            local.state = nil
            local.revision = 0
            sendSnapshotQuery(now: now)
            return false
        case .invalid:
            advanced.framesIgnored += 1
            note("忽略一則內容對不起來的角色狀態訊息")
            return false
        }
    }

    private func handleCommand(_ envelope: AIPEnvelope, now: Date) {
        switch envelope.name {
        case SessionNames.behaviorRequest:
            let firstTime = dedupe.note(envelope.messageId)
            let outcome = SessionDecisions.intentOutcome(
                envelope: envelope, now: now, negotiated: negotiated, alreadySeen: !firstTime)
            switch outcome {
            case .play(let intent):
                enqueue(intent)
            case .duplicate:
                // 重複的 command 永不重執行（§7）；不回第二次結果。
                advanced.framesIgnored += 1
            case .expired:
                sendResult(
                    causationId: envelope.messageId, status: .expired, code: nil, now: now)
            case .unsupported(let name):
                advanced.intentsRejected += 1
                sendResult(
                    causationId: envelope.messageId, status: .rejected,
                    code: .unsupportedCapability, now: now)
                note("桌面請求的角色動作「\(name)」本版不支援，已誠實回報未執行")
            case .notNegotiated:
                advanced.framesIgnored += 1
            case .invalid:
                sendResult(
                    causationId: envelope.messageId, status: .rejected, code: .schemaInvalid,
                    now: now)
            }
        case SessionNames.behaviorCancel:
            cancelIntents(matching: envelope)
            sendResult(
                causationId: envelope.messageId, status: .cancelConfirmed, code: nil, now: now)
        default:
            // 未知 name：不猜、不執行。
            sendResult(
                causationId: envelope.messageId, status: .rejected, code: .unknownName, now: now)
        }
    }

    private func handleHostResult(_ envelope: AIPEnvelope, now: Date) {
        let body = envelope.payload?.objectValue ?? [:]
        let status = body["status"]?.stringValue ?? ""
        let code = body["code"]?.stringValue
        if status == AIPOutcome.rejected.rawValue, code == AIPErrorCode.notAMember.rawValue {
            // 逾時被清出成員：必須重新協商才能再送事件。
            negotiated = false
            note("桌面已把這台裝置移出角色同步，正在重新加入")
            connectionDidConnect(now: now)
            return
        }
        if status == AIPOutcome.rejected.rawValue || status == AIPOutcome.expired.rawValue {
            note("桌面未採用剛才送出的角色事件（\(code ?? status)）")
        }
    }

    private func handleHostError(_ envelope: AIPEnvelope) {
        let code = envelope.payload?.objectValue?["code"]?.stringValue ?? ""
        switch AIPErrorCode(rawValue: code) {
        case .sessionDisabled, .unsupportedCapability:
            negotiated = false
            note("這台桌面目前沒有開啟角色同步")
        default:
            advanced.framesIgnored += 1
            note("桌面回報角色同步錯誤（\(code)）")
        }
    }

    // MARK: - 內部：intent 佇列

    private func enqueue(_ intent: PlayingIntent) {
        guard let playing = nowPlaying else {
            nowPlaying = intent
            return
        }
        if playing.interruptible {
            // 可中斷：新的蓋掉舊的。舊的沒播完 → 不回 observed（誠實）。
            nowPlaying = intent
            return
        }
        if queue.count >= Self.maxPendingIntents {
            queue.removeFirst()
            advanced.intentsDropped += 1
            note("角色動作太多，已捨棄最舊的一個（不會補播）")
        }
        queue.append(intent)
    }

    private func cancelIntents(matching envelope: AIPEnvelope) {
        let name = envelope.payload?.objectValue?["intent"]?.stringValue
        queue.removeAll { name == nil || $0.intent.rawValue == name }
        if let playing = nowPlaying, name == nil || playing.intent.rawValue == name {
            nowPlaying = queue.isEmpty ? nil : queue.removeFirst()
        }
    }

    // MARK: - 內部：送訊

    private func sendResume(now: Date) {
        guard let transport, transport.isConnected, let deviceId = transport.boundDeviceId else {
            return
        }
        SessionDecisions.noteResumeAttempt(&local)
        advanced.resumesSent += 1
        resuming = !local.unrecoverable
        guard !local.unrecoverable else {
            note("連續無法補齊角色狀態，需要重新連接")
            return
        }
        let envelope = SessionDecisions.resumeEnvelope(
            local: local, deviceId: deviceId, sessionId: sessionId,
            messageId: nextMessageId("resume"), now: now)
        _ = transport.sendAip(envelope)
    }

    private func sendSnapshotQuery(now: Date) {
        guard let transport, transport.isConnected, let deviceId = transport.boundDeviceId else {
            return
        }
        SessionDecisions.noteResumeAttempt(&local)
        resuming = !local.unrecoverable
        guard !local.unrecoverable else {
            note("連續無法補齊角色狀態，需要重新連接")
            return
        }
        let envelope = SessionDecisions.snapshotQueryEnvelope(
            deviceId: deviceId, sessionId: sessionId, messageId: nextMessageId("snap"), now: now)
        _ = transport.sendAip(envelope)
    }

    private func failedToSync() {
        SessionDecisions.noteResumeAttempt(&local)
        note("桌面回來的角色狀態讀不出來，稍後再試")
    }

    private func sendResult(
        causationId: String, status: AIPOutcome, code: AIPErrorCode?, now: Date
    ) {
        guard let transport, transport.isConnected, let deviceId = transport.boundDeviceId else {
            return
        }
        guard
            let envelope = SessionDecisions.resultEnvelope(
                causationId: causationId, status: status, code: code, deviceId: deviceId,
                sessionId: sessionId, messageId: nextMessageId("res"), now: now)
        else { return }
        _ = transport.sendAip(envelope)
    }

    // MARK: - 內部：小工具

    /// 從原始 frame 文字取出 `envelope.payload`，數字字面逐字保留。
    private func payloadTokens(_ rawFrame: String) -> SemanticJSON? {
        SemanticJSON.parse(rawFrame)?["envelope"]?["payload"]
    }

    private func refreshStatus() {
        syncStatus = SessionDecisions.syncStatus(
            local: local,
            connected: transport?.isConnected ?? false,
            negotiated: negotiated,
            hasUnsupportedIntents: !unsupportedIntents.isEmpty,
            resuming: resuming)
        advanced.revision = local.revision
        advanced.sequence = local.sequence
        advanced.epoch = local.epoch
    }

    private func nextMessageId(_ prefix: String) -> String {
        messageCounter += 1
        return "ios-\(prefix)-\(messageCounter)-\(UUID().uuidString.prefix(8).lowercased())"
    }

    private func note(_ text: String) {
        log.append("[\(WireTime.nowISO8601())] \(text)")
        if log.count > 50 {
            log.removeFirst(log.count - 50)
        }
    }
}
