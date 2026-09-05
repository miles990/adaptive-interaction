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
//  - 權威狀態只由 host 決定：收到一則 `state` 之後要做什麼，逐條照 **AIP 1.0 接收端決策表**
//    （`docs/aip/character-session.md` §7.2；表本身在 `SessionReceive.swift`，權威實作是 Rust
//    `interaction_session::receive`）。
//
//  與權威實作的差異：**零**（v0.7.0 起）。三處曾經各走各的規則，現在三端讀同一張表：
//  1. snapshot 的 epoch 與本地不同又沒有 `session-reset` 宣告 → 以前直接套用並靜默改寫本地
//     epoch，現在 `realign(epoch-changed)`（規則 5）。
//  2. patch 以前完全不看 epoch，只靠 `baseRevision` 恰巧不符去擋 → 現在 `realign`（規則 11）。
//  3. 落後的 snapshot 以前一律送 resume → 現在忽略（規則 7）；真的倒退過的 host 會說
//     `recovery`（規則 6），那才套用。
//  兩個上限（`maxResumePatches` 512、`maxRealignAttempts` 3）由 codegen 從 golden schema 帶進
//  `AIPLimits`，**不得**在這一端手寫。
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
    /// 立刻送一則 legacy `status`：這是本版**唯一**維持 presence 的心跳。
    func sendStatusNow()
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
    /// `payload.reason`：host 明說它重建了 session（Rust `REASON_SESSION_RESET`）。
    static let sessionReset = "session-reset"
    /// `payload.reason`：host 明說它從較舊的快照還原（Rust `REASON_RECOVERY`）。
    /// 成員自己宣稱超前不算證據，只有 host 說了才允許 revision 往回走。
    static let recovery = "recovery"
}

// MARK: - 本機認知（純資料）

/// App 這一端對角色同步的認知。所有欄位都只由 `SessionDecisions` 的純函式改。
struct SessionSyncLocal: Equatable {
    var revision: UInt64 = 0
    var sequence: UInt64 = 0
    var epoch: UInt64 = 0
    var state: SemanticJSON?
    /// 本地那份 state 的 canonical hash（**本地自己算的**，不是照抄 payload）。
    var stateHash: String?
    /// 這份權威狀態屬於哪一個 session（`nil` ＝ 還不知道，就不宣稱不符）。
    var sessionId: String?
    /// 現行連線／請求世代：帶著別的世代的訊息就是舊連線的遲到品（決策表規則 0）。
    var connectionGeneration: UInt64 = 0
    /// 有界 realign 預算（連續幾次未能 apply）。
    var budget = SessionRealignBudget()

    /// 連續幾次「送了 resume 卻還是沒對齊」。
    var resumeFailures: Int { budget.attempts }
    /// 已達上限：狀態是**未知**，照實說，不再自動重試。
    var unrecoverable: Bool { budget.isUnrecoverable }

    /// 連續失敗到這個數字就顯示「無法恢復，請重新連接」。
    /// 權威值在 `interaction_aip::limits::MAX_REALIGN_ATTEMPTS`，由 codegen 帶進 `AIPLimits`
    /// ——**不得**在這一端手寫成 3。
    static let resumeFailureLimit = AIPLimits.maxRealignAttempts

    /// 決策表看得到的本地摘要（沒有 state 本身）。
    var view: SessionReceiverView {
        SessionReceiverView(
            hasState: state != nil, sessionId: sessionId, epoch: epoch, revision: revision,
            stateHash: stateHash, connectionGeneration: connectionGeneration)
    }

    /// 一則**真的送出去**的 resume 最多等桌面回覆多久；超過這段時間，回前景才允許再問一次。
    ///
    /// 為什麼要有這個數字：回前景會觸發 resume，快速鎖螢幕／解鎖可以在桌面回覆第一則之前
    /// 連續觸發好幾次。沒有這道閘門的話，`noteResumeAttempt` 會把「還沒收到回覆」一次次記成
    /// 失敗，累到 `resumeFailureLimit` 就宣稱「無法恢復」——但桌面一次都沒有拒絕過，
    /// 那是在沒有失敗證據時宣稱失敗。**有界**：等超過這段時間就再問一次，不會永遠卡在等回覆。
    static let resumeResponseGraceSeconds: TimeInterval = 10
}

/// 一則 `state`（或 resume 回來的一項）的共同形狀。
///
/// `state`／`baseRevision` 是**選填**：缺了不是「讀不出來」，而是決策表規則 2 的
/// `reject-invalid`（AIP 1.0 的 snapshot 必帶 hash 與 state，patch 必帶 baseRevision）。
struct SessionStateMessage: Equatable {
    enum Kind: Equatable {
        case snapshot(state: SemanticJSON?)
        case patch(patch: SemanticJSON, baseRevision: UInt64?)
    }

    var kind: Kind
    var revision: UInt64
    var sequence: UInt64?
    var epoch: UInt64
    var hash: String?
    /// `payload.reason`（`session-reset`／`recovery`；未知值視同沒有 reason）。
    var reason: String?
    /// envelope 的 `sessionId`（`nil` ＝ 沒宣稱）。
    var sessionId: String?
}

extension SessionStateMessage.Kind {
    /// 決策表看得到的種類。
    var tableKind: SessionStateKind {
        switch self {
        case .snapshot: return .snapshot
        case .patch: return .patch
        }
    }

    /// snapshot 的 payload 真的帶了 `state`（規則 2）。
    var statePresent: Bool {
        if case .snapshot(let state) = self { return state != nil }
        return false
    }

    /// patch 宣告接在哪一個 revision 後面（規則 2／13）。
    var baseRevision: UInt64? {
        if case .patch(_, let base) = self { return base }
        return nil
    }
}

/// 一則 state 走完決策表之後的結果：決策本身，加上「採用的話本地會變成什麼」。
struct SessionStateOutcome: Equatable {
    var decision: SessionReceiveDecision
    /// 採用時套用後的**完整**權威狀態；不採用時 `nil`。
    var state: SemanticJSON?
    /// 採用後的本地摘要（不採用時就是原值）。
    var view: SessionReceiverView

    var revision: UInt64 { view.revision }
    var epoch: UInt64 { view.epoch }
    var hash: String? { view.stateHash }
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
    ///
    /// `sessionEpoch` 是**必填**：沒有 epoch 就沒有可比的順序，猜一個本地值只會讓
    /// 「換了 session」看起來像「同一個 session 的新版本」（TypeScript 端同樣要求）。
    static func stateMessage(
        payload: SemanticJSON,
        baseRevision: UInt64? = nil,
        sequence: UInt64? = nil,
        sessionId: String? = nil,
        assumedKind: SessionStateKind? = nil
    ) -> SessionStateMessage? {
        guard let revision = payload["revision"]?.uintValue,
            let epoch = payload["sessionEpoch"]?.uintValue
        else { return nil }
        let common = { (kind: SessionStateMessage.Kind) in
            SessionStateMessage(
                kind: kind, revision: revision,
                sequence: payload["sequence"]?.uintValue ?? sequence, epoch: epoch,
                hash: payload["hash"]?.stringValue, reason: payload["reason"]?.stringValue,
                sessionId: payload["sessionId"]?.stringValue ?? sessionId)
        }
        switch SessionStateKind.parse(payload["kind"]?.stringValue) ?? assumedKind {
        case .snapshot:
            // 缺 `state` 不是「讀不出來」：決策表規則 2 會把它記成 reject-invalid。
            return common(.snapshot(state: payload["state"]))
        case .patch:
            // 缺 `patch` 本體才是真的讀不出來（沒有東西可以套）；缺 `baseRevision` 是規則 2。
            guard let patch = payload["patch"] else { return nil }
            return common(
                .patch(
                    patch: patch,
                    baseRevision: payload["baseRevision"]?.uintValue ?? baseRevision))
        case nil:
            return nil
        }
    }

    /// resume 回覆裡的一則補丁（`docs/aip/transport-bindings.md` §1.3 的 `patches[]`）。
    ///
    /// 形狀是**攤平**的（`{sequence, baseRevision, revision, patch, hash, sessionEpoch}`）：
    /// 沒有 envelope 外殼，也**沒有 `kind`**——規則與 `state{kind:"patch"}` 完全一樣。
    /// 認不得它的話，每一次帶補丁的 resume 都會被記成「讀不出來」，對齊永遠補不完。
    static func resumePatchMessage(_ item: SemanticJSON, sessionId: String? = nil)
        -> SessionStateMessage?
    {
        stateMessage(payload: item, sessionId: sessionId, assumedKind: .patch)
    }

    /// resume 回覆的補丁數超過上限（**不**靜默截斷成「我以為我追上了」）。
    static func resumeExceedsBound(_ count: Int) -> Bool { count > AIPLimits.maxResumePatches }

    /// AIP 1.0 接收端決策表（`docs/aip/character-session.md` §7.2）＋「採用的話會變成什麼」。
    ///
    /// 決策本身在 `SessionReceive.swift`（三端共用、由 fixture 對答案）；這裡只負責把
    /// wire 上的一則訊息攤平成表看得懂的欄位，並**自己算 hash**
    ///（snapshot ＝對收到的 state；patch ＝merge 之後的結果）。
    static func apply(
        _ message: SessionStateMessage,
        to local: SessionSyncLocal,
        arrivedOnGeneration: UInt64? = nil,
        viaAuthoritativeReply: Bool = false
    ) -> SessionStateOutcome {
        let candidate: SemanticJSON?
        switch message.kind {
        case .snapshot(let state):
            candidate = state
        case .patch(let patch, _):
            // 沒有本地副本就沒有東西可以套上去（規則 10 會處理）。
            candidate = local.state.map { SemanticJSON.mergePatch($0, patch) }
        }
        let incoming = SessionIncomingState(
            kind: message.kind.tableKind,
            sessionId: message.sessionId,
            epoch: message.epoch,
            revision: message.revision,
            baseRevision: message.kind.baseRevision,
            reason: message.reason,
            hash: message.hash,
            computedHash: candidate?.canonicalSHA256,
            statePresent: message.kind.statePresent,
            arrivedOnGeneration: arrivedOnGeneration ?? local.connectionGeneration,
            viaAuthoritativeReply: viaAuthoritativeReply)
        let view = local.view
        let decision = decideReceive(view: view, incoming: incoming)
        return SessionStateOutcome(
            decision: decision,
            state: decision.adoptsState ? candidate : nil,
            view: advance(view: view, incoming: incoming, decision: decision))
    }

    /// 送出一次 resume 就記一次「還沒對齊」；連續達上限即視為無法恢復。
    static func noteResumeAttempt(_ local: inout SessionSyncLocal) {
        local.budget = local.budget.counting()
    }

    /// 回到前景時要不要**再**送一則 resume。
    ///
    /// `inFlightSince` ＝ 上一則真的送出去、還在等桌面回覆的 resume 的時間（`nil` ＝ 沒有）。
    /// 還在等回覆時重送只是把同一個問題問第二次，不會帶來新資訊，卻會被記成一次失敗。
    static func shouldResendResumeOnForeground(inFlightSince: Date?, now: Date) -> Bool {
        guard let inFlightSince else { return true }
        let waited = now.timeIntervalSince(inFlightSince)
        // 時鐘往回跳（使用者改時間、NTP 校正）時寧可再問一次，也不要永遠卡在「等回覆」。
        guard waited >= 0 else { return true }
        return waited >= SessionSyncLocal.resumeResponseGraceSeconds
    }

    /// 只要成功套用過一次狀態，先前的失敗就不再有意義。
    static func noteSyncSucceeded(_ local: inout SessionSyncLocal) {
        local.budget = SessionRealignBudget()
    }

    /// 新的一條連線：重新給一輪補齊的機會，並記下新的連線世代。
    /// 只清失敗計數，**不**清 revision／state——重連是 reconcile，不是重來（§7）。
    ///
    /// 世代是決策表規則 0 的依據：上一條連線送出的訊息現在才到，它宣告的 epoch 一定與
    /// 本地不同，任何 epoch 判斷都會被它騙過去，世代檢查是唯一防線。
    static func noteNewConnection(_ local: inout SessionSyncLocal, generation: UInt64? = nil) {
        local.budget = SessionRealignBudget()
        if let generation { local.connectionGeneration = generation }
    }

    /// 換了配對目標（另一台桌面）：本地對權威狀態的一切認知都失效，全部歸零。
    /// 留著舊的 revision／epoch 會讓新 host 的第一份快照被當成 rollback。
    ///
    /// **連線世代不歸零**：它描述的是「現在這條 socket」，不是權威狀態的一部分；
    /// 歸零會讓正在用的這條連線送來的東西被當成舊世代的遲到品。
    static func forgetAuthoritativeState(_ local: inout SessionSyncLocal) {
        let generation = local.connectionGeneration
        local = SessionSyncLocal()
        local.connectionGeneration = generation
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

extension SessionDecisions {
    /// 收到 host AIP `heartbeat` 時要不要回一則 legacy `status`（節流，避免灌爆有界佇列）。
    ///
    /// 時鐘往回跳（使用者改時間、NTP 校正）時**寧可多回一則**：漏回的代價是被誤判離線，
    /// 多回一則的代價只是一則小 frame。
    static func shouldAnswerHeartbeat(
        lastAnswerAt: Date?,
        now: Date,
        minInterval: TimeInterval = PresenceHeartbeatPolicy.heartbeatReplyMinIntervalSeconds
    ) -> Bool {
        guard let lastAnswerAt else { return true }
        let elapsed = now.timeIntervalSince(lastAnswerAt)
        return elapsed < 0 || elapsed >= minInterval
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
    /// 舊連線世代的遲到品（決策表規則 0）。與 `framesIgnored` 分開記：這不是「訊息有問題」，
    /// 而是「這條連線已經不是那條了」。
    var staleConnectionFrames = 0
    /// 收到桌面 AIP `heartbeat` 的次數（收到幾則就記幾則，與有沒有回覆無關）。
    var heartbeatsReceived = 0
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
    /// 上一則**真的送出去**、還在等桌面回覆的 resume／snapshot 查詢的時間；`nil` ＝ 沒有在飛。
    ///
    /// 與 `resuming` 是兩件不同的事實，不能互相取代：`resuming` 說的是「本地還沒對齊」
    /// （連送都送不出去時也成立），這裡說的是「線上真的有一則在等回覆」。兩者一起由
    /// `markResumeSettled()` 收尾，避免兩個地方各自記一份。
    private var resumeInFlightSince: Date?
    private var messageCounter: UInt64 = 0
    private var sessionId = SessionNames.sessionId
    /// 上一次因為 host heartbeat 而回送 legacy status 的時間（節流用）。
    private var lastHeartbeatAnswerAt: Date?

    /// 這個 client 讀的牆鐘。`ConnectionManager` 接上 transport 時把**自己那一個**注入進來
    ///（`withSession`），所以 frame 驅動的路徑（`handleFrame` 一路到 `resumeInFlightSince`）
    /// 與生命週期驅動的路徑（回前景 resume）讀的是同一個時鐘。
    ///
    /// 為什麼要在意：兩個時鐘並存時，inbound frame 用真實牆鐘寫下「還在等回覆」、回前景卻用
    /// 注入的時鐘去比，`shouldResendResumeOnForeground` 會算出負值而走「時鐘往回跳就重問」
    /// 那一支，10 秒寬限窗的保證就形同虛設。
    var wallClockNow: () -> Date = { Date() }

    nonisolated init() {}

    // MARK: 連線生命週期

    /// auth-ok 之後：（重新）協商；已經有本地狀態時再要求對齊。
    ///
    /// **不重播**任何互動事件或 intent（§8）——重連只 reconcile 狀態。
    ///
    /// - Parameter generation: 這條連線的世代（`ConnectionManager` 每開一條 socket 就 +1）。
    ///   `nil` ＝ 沿用目前的世代（單元測試不開 socket 時用）。
    func connectionDidConnect(now: Date? = nil, generation: UInt64? = nil) {
        let now = now ?? wallClockNow()
        SessionDecisions.noteNewConnection(&local, generation: generation)
        guard let deviceId = transport?.boundDeviceId else { return }
        negotiated = false
        unsupportedIntents = []
        nowPlaying = nil
        queue.removeAll()
        markResumeSettled()
        // 新連線 ＝ 新的一輪嘗試（計數已經在最上面清掉了）。§11 給使用者的指示就是
        //「請重新連接」；如果重新連接之後計數還留著，第一次 resume 就會被吞掉並繼續顯示
        //「無法恢復」，那句指示等於是假的（上一條連線的失敗不能算在這一條頭上）。
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
        if local.revision > 0, sendResume(now: now) {
            SessionDecisions.noteResumeAttempt(&local)
        }
        refreshStatus()
    }

    /// 換配對目標（首次配對、位址變更後重新配對、使用者解除配對）。
    ///
    /// 新的桌面是另一個 session：它的 epoch／revision 與這支手機記得的沒有可比性
    /// （全新 host 的 epoch 就是 1），留著舊認知只會讓對方的權威快照被當成 rollback。
    func pairingDidChange() {
        SessionDecisions.forgetAuthoritativeState(&local)
        presentation = nil
        negotiated = false
        unsupportedIntents = []
        nowPlaying = nil
        queue.removeAll()
        markResumeSettled()
        advanced = SessionAdvancedInfo()
        refreshStatus()
    }

    /// 回到前景（socket 仍在）：只 reconcile 狀態。
    ///
    /// §8 離線政策：重連／回前景**不重播**任何互動事件或 intent——背景期間使用者的觸碰
    /// 早就過了 `expiresAt`，補送只會讓桌面收到一則遲到的假事實。這裡只做一件事：
    /// 用 §7 的 resume 問桌面「我停在 revision／sequence／epoch，之後發生了什麼」。
    ///
    /// 送不出去的情況（未連線、尚未協商、本地還沒有權威狀態）一律誠實留一行說明，
    /// **不**假裝已經對齊。
    func foregroundDidResume(now: Date? = nil) {
        let now = now ?? wallClockNow()
        guard let transport, transport.isConnected else {
            // 三個送不出去的分支對稱：這一支最常見（使用者回前景時 socket 剛好斷了），
            // 靜默 return 會讓畫面停在斷線前的舊狀態、看起來像已經對齊。
            note("回到前景：未連線，角色狀態暫時無法對齊")
            refreshStatus()
            return
        }
        guard negotiated else {
            // 連線流程本身會重送 capability；這裡不搶跑，也不假裝已經是成員。
            note("回到前景：角色同步尚未協商，等待桌面回覆能力協商結果")
            refreshStatus()
            return
        }
        guard local.revision > 0 else {
            note("回到前景：本地還沒有權威角色狀態，等待桌面送出快照")
            refreshStatus()
            return
        }
        guard
            SessionDecisions.shouldResendResumeOnForeground(
                inFlightSince: resumeInFlightSince, now: now)
        else {
            // 桌面還沒回覆上一則：重送不會帶來新資訊，也不得把「還沒回覆」記成一次失敗。
            note("回到前景：上一則對齊要求還在等桌面回覆，先不重送")
            refreshStatus()
            return
        }
        note("回到前景：向桌面要求對齊角色狀態（只對齊狀態，不重播任何事件）")
        if sendResume(now: now) {
            SessionDecisions.noteResumeAttempt(&local)
        }
        refreshStatus()
    }

    /// 斷線：成員身分留在 host 那邊等逾時，但本地必須重新協商才能再送事件。
    func connectionDidDisconnect() {
        negotiated = false
        nowPlaying = nil
        queue.removeAll()
        markResumeSettled()
        refreshStatus()
    }

    // MARK: 收訊

    /// 處理一則 `{"type":"aip","envelope":…}`。
    ///
    /// `rawFrame` 是原始 frame 文字：state 的 hash 必須對 **host 寫出來的文字**取，
    /// 重新編碼過的 JSON 會讓 `0.0` 變成 `0`、hash 就對不上（見 `CharacterSemantic.swift`）。
    /// - Parameter arrivedOnGeneration: 這則 frame 是在哪一條連線上收到的
    ///   （`nil` ＝ 就是現在這條）。決策表規則 0：世代不符的遲到品**先於一切**被丟掉——
    ///   上一條連線送出的 `session-reset` 宣告的 epoch 一定與本地不同，任何 epoch 判斷
    ///   都會被它騙過去。
    func handleFrame(
        _ envelope: AIPEnvelope,
        rawFrame: String,
        arrivedOnGeneration: UInt64? = nil,
        now: Date? = nil
    ) {
        let now = now ?? wallClockNow()
        if let generation = arrivedOnGeneration, generation != local.connectionGeneration {
            advanced.staleConnectionFrames += 1
            note("忽略一則上一條連線送來的角色同步訊息（世代 \(generation)，現在是 \(local.connectionGeneration)）")
            return
        }
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
        // 身分：已經有一份權威狀態之後，**別的 session 不得改寫我們宣稱的身分**——
        // 改寫了的話，決策表規則 1 就再也擋不住對方的狀態（我們會以為那是自己人）。
        if let id = envelope.sessionId, !id.isEmpty, local.sessionId == nil || local.sessionId == id
        {
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
        case .heartbeat:
            handleHostHeartbeat(now: now)
        case .event:
            // host→device 的 event 在本 profile 沒有語意（角色狀態一律走 `state`）。
            advanced.framesIgnored += 1
            note("忽略一則桌面送來的 event（本版只從 state 取角色狀態）")
        case .query:
            // 手機是 remote-renderer，不擁有任何共享狀態，回答不了 query。
            advanced.framesIgnored += 1
            note("忽略一則桌面送來的 query（手機不持有可被查詢的狀態）")
        case .cancel:
            // 角色動作的取消走 `command{character.behavior.cancel}`，不走 cancel 型別。
            advanced.framesIgnored += 1
            note("忽略一則 cancel 訊息（角色動作的取消走 character.behavior.cancel）")
        case .approvalRequest:
            // 人類決定只在桌面的可信介面上做；手機不是 approval surface，不代答。
            advanced.framesIgnored += 1
            note("忽略一則 approval-request（手機不是人類決定的介面，不代為回答）")
        case .approvalResult:
            advanced.framesIgnored += 1
            note("忽略一則 approval-result（手機不參與人類決定流程）")
        case .unknown:
            advanced.framesIgnored += 1
            note("忽略一則本版不認得的角色同步訊息")
        }
        refreshStatus()
    }

    // MARK: 送訊（使用者動作）

    /// 角色頁的點擊／長按。回傳要顯示給使用者的一行誠實說明。
    @discardableResult
    func touch(kind: String, now: Date? = nil) -> String {
        let now = now ?? wallClockNow()
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
    func dismiss(now: Date? = nil) {
        let now = now ?? wallClockNow()
        guard negotiated, let transport, transport.isConnected,
            let deviceId = transport.boundDeviceId
        else { return }
        let envelope = SessionDecisions.dismissEnvelope(
            deviceId: deviceId, sessionId: sessionId, messageId: nextMessageId("dismiss"), now: now)
        _ = transport.sendAip(envelope)
    }

    /// 本地動畫**真的播完**之後才回 `observed`（誠實階梯的底線）。
    func intentDidFinishPlaying(messageId: String, now: Date? = nil) {
        let now = now ?? wallClockNow()
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
                payload: payload, baseRevision: envelope.baseRevision,
                sequence: envelope.sequence, sessionId: envelope.sessionId)
        else {
            advanced.framesIgnored += 1
            note("忽略一則讀不出來的角色狀態訊息")
            return
        }
        // 推播（不是我們要來的權威回覆）：被擋下來不算一次對齊失敗。
        consume(message, now: now, viaAuthoritativeReply: false)
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
        markResumeSettled()
        switch payload["kind"]?.stringValue {
        case "patches":
            let items = payload["patches"]?.arrayValue ?? []
            if items.isEmpty {
                // 已經對齊了（sequence 落後不是狀態錯誤）：沒有東西要補。
                SessionDecisions.noteSyncSucceeded(&local)
                return
            }
            // 批次規則（上限／良性跳過／第一個帶 effect 的決策中止整批）只有一份實作，
            // 就是跨語言 fixture 對答案的那一份：`SessionDecisions.runResumeBatch`。
            var unreadable = false
            let batch = SessionDecisions.runResumeBatch(view: local.view, count: items.count) {
                index, _ in
                // 決策用的是**本地那份真的副本**（`consume` 已經把它推進了），不是驅動器
                // 傳進來的 view：補丁的 hash 只有在前一則真的 merge 之後才算得出來，
                // 所以整批的決策不可能先算完再套用。
                guard
                    let message = SessionDecisions.resumePatchMessage(
                        items[index], sessionId: envelope.sessionId)
                else {
                    unreadable = true
                    return (.rejectInvalid, local.view)
                }
                return (consume(message, now: now, viaAuthoritativeReply: true), local.view)
            }
            if unreadable {
                failedToSync()
                return
            }
            if batch.halted == .realign(reason: .resumeTooLong) {
                // 超過上限**不**靜默截斷成「我以為我追上了」：整批不處理，改要一份完整快照。
                // 權威 host 在補丁塞不下時本來就會改回 snapshot，所以這是縱深防禦。
                advanced.framesIgnored += 1
                note(
                    "桌面回來的角色補丁超過上限（\(items.count) > \(AIPLimits.maxResumePatches)），"
                        + "整批不套用，改要求完整快照")
                if sendSnapshotQuery(now: now) { SessionDecisions.noteResumeAttempt(&local) }
            }
        case "snapshot":
            guard
                let message = SessionDecisions.stateMessage(
                    payload: payload, sessionId: envelope.sessionId)
            else {
                failedToSync()
                return
            }
            _ = consume(message, now: now, viaAuthoritativeReply: true)
        default:
            failedToSync()
        }
    }

    /// 走一遍決策表並執行結論；回傳**這一則的決策**。
    ///
    /// 「這一批還要不要繼續」不在這裡判斷：批次規則由 `SessionDecisions.runResumeBatch`
    /// 讀這個回傳值決定（良性的舊項跳過不中止，第一個帶 effect 的決策中止整批），
    /// 這一端不再重寫一份。
    @discardableResult
    private func consume(
        _ message: SessionStateMessage, now: Date, viaAuthoritativeReply: Bool
    ) -> SessionReceiveDecision {
        let outcome = SessionDecisions.apply(
            message, to: local, viaAuthoritativeReply: viaAuthoritativeReply)
        /// realign 決定要發的那一則請求**真的送出去了嗎**（誠實階梯：沒送出去不算一次嘗試）。
        var realignRequestSent = false
        switch outcome.decision {
        case .apply, .reset, .recover:
            adopt(outcome, message: message)
            if outcome.decision == .reset {
                note("桌面重建了角色 session（epoch \(outcome.epoch)），已改用它的權威狀態")
            }
            if outcome.decision == .recover {
                note("桌面回報它從較舊的快照還原（revision \(outcome.revision)），已跟著退回")
            }

        case .alreadyApplied:
            // 同一版又來一次：本來就已經套用過，還在同步軌道上。
            advanced.framesIgnored += 1

        case .ignoreStale:
            // 落後的權威狀態：忽略就是對的（rollback 防護）。真的倒退過的 host 會說
            // `recovery`（規則 6）；沒說就不是證據，也不必為它再問一次。
            advanced.framesIgnored += 1

        case .ignoreStaleConnection:
            advanced.staleConnectionFrames += 1

        case .rejectIdentity:
            // 別的 session 的狀態不是「比較舊」，是**不相干**：不套用，也**不**重新對齊
            //（realign 只會再要一次別人的 session）。
            advanced.framesIgnored += 1
            note("忽略一則別的角色 session 的狀態（身分不符，不會拿它來對齊）")

        case .rejectInvalid:
            // 對方回答了、但答案沒用：算一次對齊失敗（推播上的垃圾不算）——這條規則
            // 由 `observing` 統一記帳，不在這裡自己加一次。
            advanced.framesIgnored += 1
            note("忽略一則不完整的角色狀態訊息（snapshot 必須帶 hash 與 state）")

        case .realign(let reason):
            note("角色狀態接不上（\(reason.rawValue)），向桌面要求重新對齊")
            if reason == .hashMismatch {
                // 內容對不起來時，補丁鏈已經沒有意義：直接要一份完整快照才收斂得了。
                // 決策仍然是 realign（三端一致），只是這一端選了比較有效的那一種請求。
                realignRequestSent = sendSnapshotQuery(now: now)
            } else {
                realignRequestSent = sendResume(now: now)
            }
        }
        // 有界 realign 預算也走同一張表：出貨路徑與 fixture 對的是同一份記帳規則
        //（`apply`／`reset`／`recover` 歸零、權威回覆的 `reject-invalid` 記一次、
        // realign 只在請求真的送出去時記一次）。
        local.budget = local.budget.observing(
            outcome.decision, viaAuthoritativeReply: viaAuthoritativeReply,
            realignRequestSent: realignRequestSent)
        return outcome.decision
    }

    /// 採用一份權威狀態（apply／reset／recover 共用）。
    private func adopt(_ outcome: SessionStateOutcome, message: SessionStateMessage) {
        guard let state = outcome.state else { return }
        local.revision = outcome.revision
        local.epoch = outcome.epoch
        local.state = state
        local.stateHash = outcome.hash
        local.sessionId = outcome.view.sessionId
        switch outcome.decision {
        case .reset, .recover:
            // host 重建／還原之後 sequence 也重新算：留著舊的較大值會把之後每一則的
            // sequence 都判成「舊的」，診斷數字就永遠停在上一個 incarnation。
            local.sequence = message.sequence ?? 0
        default:
            if let sequence = message.sequence, sequence > local.sequence {
                local.sequence = sequence
            }
        }
        markResumeSettled()
        // 失敗計數的歸零由 `consume` 的 `observing(.apply/.reset/.recover)` 負責，
        // 這裡不再另外記一次（同一件事只有一個地方記帳）。
        presentation = CharacterSemanticState.project(state)
        if presentation == nil {
            note("收到的角色狀態不符合本版認得的形狀，未套用到畫面")
        }
        advanced.appliedStates += 1
        advanced.revision = local.revision
        advanced.sequence = local.sequence
        advanced.epoch = local.epoch
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

    /// 桌面送來 AIP `heartbeat`。
    ///
    /// 本版**不送** AIP heartbeat（`docs/aip/transport-bindings.md` §1.4 記為尚未實作，
    /// 能力宣告裡也沒有宣稱），而 AIP §2.1 對 heartbeat 的回應是「選填 heartbeat」——
    /// 所以不回 AIP heartbeat 是合規的。真正維持這支手機 presence 的是 wire protocol v1 的
    /// `status`（桌面收到就 `character_session_touch_presence`），因此這裡回的是**真的有效的
    /// 那一則**：立刻送一次 legacy `status`，並在進階診斷留下一行誠實說明。
    private func handleHostHeartbeat(now: Date) {
        advanced.heartbeatsReceived += 1
        note("收到 AIP heartbeat，本版以 wire protocol v1 的 status 心跳回應（尚未實作送出 AIP heartbeat）")
        guard let transport, transport.isConnected else { return }
        guard
            SessionDecisions.shouldAnswerHeartbeat(lastAnswerAt: lastHeartbeatAnswerAt, now: now)
        else {
            return
        }
        lastHeartbeatAnswerAt = now
        transport.sendStatusNow()
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

    /// 這一輪補齊結束（成功套用、收到回覆、或連線本身換了一條）：兩個欄位一起收。
    private func markResumeSettled() {
        resuming = false
        resumeInFlightSince = nil
    }

    /// 送一次 §7 的 resume；回傳**真的送出去了嗎**。
    ///
    /// **順序很重要**：先判斷「已經放棄了嗎」，真的送出去之後才由呼叫端記一次嘗試。
    /// 反過來寫的話，第 3 次會只增加計數卻根本沒送出，使用者看到「無法恢復」時
    /// 實際只做過 2 次 round-trip（誠實階梯：宣稱的嘗試次數必須等於真的做過的次數）。
    private func sendResume(now: Date) -> Bool {
        // 走到這裡就代表本地與 host 對不齊：在真的補齊之前都不得宣稱「已同步」，
        // 送不出去也一樣（誠實階梯：received ≠ applied）。
        resuming = true
        guard let transport, transport.isConnected, let deviceId = transport.boundDeviceId else {
            return false
        }
        guard !local.unrecoverable else {
            note("連續無法補齊角色狀態，需要重新連接")
            return false
        }
        let envelope = SessionDecisions.resumeEnvelope(
            local: local, deviceId: deviceId, sessionId: sessionId,
            messageId: nextMessageId("resume"), now: now)
        guard transport.sendAip(envelope) else {
            // 有界佇列丟掉、編碼失敗、或背景中被生命週期閘門擋下：沒送出去就不算一次嘗試
            //（也不能算成失敗）。
            note("角色狀態的補齊要求沒有送出，稍後再試")
            return false
        }
        advanced.resumesSent += 1
        resumeInFlightSince = now
        return true
    }

    /// 送一次 snapshot query；回傳**真的送出去了嗎**（記帳規則同 `sendResume`）。
    private func sendSnapshotQuery(now: Date) -> Bool {
        resuming = true
        guard let transport, transport.isConnected, let deviceId = transport.boundDeviceId else {
            return false
        }
        guard !local.unrecoverable else {
            note("連續無法補齊角色狀態，需要重新連接")
            return false
        }
        let envelope = SessionDecisions.snapshotQueryEnvelope(
            deviceId: deviceId, sessionId: sessionId, messageId: nextMessageId("snap"), now: now)
        guard transport.sendAip(envelope) else {
            note("角色狀態的補齊要求沒有送出，稍後再試")
            return false
        }
        resumeInFlightSince = now
        return true
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
