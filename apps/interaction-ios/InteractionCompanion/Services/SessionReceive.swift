//
//  SessionReceive.swift
//  InteractionCompanion
//
//  AIP 1.0 **接收端決策表**（純函式；`docs/aip/character-session.md` §7.2）。
//
//  權威實作是 Rust 的 `crates/interaction-session/src/receive.rs::decide_receive`，
//  跨語言 fixture 是 `crates/interaction-aip/tests/fixtures/manifest.json` 的
//  `receiveDecisions` 段（45 個具名案例，由 codegen 內嵌成 `AIPFixtures.manifest`）。
//  **同一則訊息，Rust／TypeScript／Swift 必須得到同一個決策**——這張表就是拿來對答案的。
//
//  為什麼要一張表：同一則訊息，桌面回 realign、iPhone 直接套用、Rust 靜默改寫本地 epoch，
//  三個畫面都寫著「已同步」，分歧卻沒有任何一端看得見。決策順序本身就是安全邊界：
//  先判連線世代、再判身分、再判格式，最後才輪到 revision。
//
//  這張表**不**做的事：不碰 I/O、不看時鐘、不持有狀態、不自己算 hash
//  （`computedHash` 由呼叫端算好；`nil` 代表「這個呼叫端沒有核對」，不是「核對過了」）。
//

import Foundation

// MARK: - 輸入

/// `state` 訊息的兩種形狀。
enum SessionStateKind: String, Equatable {
    case snapshot
    case patch

    /// wire 上的 `payload.kind`；認不得的字串不是這張表能處理的訊息。
    static func parse(_ raw: String?) -> SessionStateKind? {
        guard let raw else { return nil }
        return SessionStateKind(rawValue: raw)
    }
}

/// 本地那份權威狀態副本的**摘要**（沒有 state 本身：這張表只看得到中繼資料）。
struct SessionReceiverView: Equatable {
    /// 本地有沒有一份可用的狀態（false ＝ 還沒 bootstrap）。
    var hasState: Bool = false
    /// 本地記得的 session id（`nil` ＝ 不知道，就不宣稱不符）。
    var sessionId: String?
    var epoch: UInt64 = 0
    var revision: UInt64 = 0
    /// 本地那份 state 的 canonical hash（本地自己算的；`nil` ＝ 沒算過）。
    var stateHash: String?
    /// 現行連線／請求世代。訊息帶著別的世代就是舊連線的遲到品。
    var connectionGeneration: UInt64 = 0
}

/// 一則**已經通過 typed boundary** 的 `state` 訊息，攤平成決策需要的欄位。
struct SessionIncomingState: Equatable {
    var kind: SessionStateKind
    /// envelope 的 `sessionId`。
    var sessionId: String?
    /// `payload.sessionEpoch`。
    var epoch: UInt64 = 0
    /// `payload.revision`。
    var revision: UInt64 = 0
    /// `baseRevision`（patch 必填）。
    var baseRevision: UInt64?
    /// `payload.reason`（`session-reset`／`recovery`；未知值視同沒有 reason）。
    var reason: String?
    /// `payload.hash`（snapshot 必填）。
    var hash: String?
    /// 呼叫端**自己算出來**的 hash：snapshot ＝對收到的 `state`；patch ＝merge 之後的結果。
    /// `nil` ＝ 這個呼叫端沒有核對（不代表核對過了）。
    var computedHash: String?
    /// snapshot 的 payload 真的帶了 `state`。
    var statePresent: Bool = false
    /// 這則訊息是在哪個連線／請求世代上收到的。
    var arrivedOnGeneration: UInt64 = 0
    /// 它是不是我們自己要來的權威回覆（resume／snapshot response），而不是推播。
    var viaAuthoritativeReply: Bool = false
}

// MARK: - 決策

/// realign 的原因（穩定字串；三端與 fixture 共用）。
enum SessionRealignReason: String, Equatable {
    /// 本地沒有狀態，補丁沒有東西可以套上去。
    case noLocal = "no-local"
    /// epoch 不同，兩份狀態沒有可比的順序。
    case epochChanged = "epoch-changed"
    /// `baseRevision` 接不上本地 revision。
    case baseMismatch = "base-mismatch"
    /// 套用後算出來的 hash 與 host 宣告的不同。
    case hashMismatch = "hash-mismatch"
    /// resume 回覆的 patch 數量超過 `AIPLimits.maxResumePatches`（**不**靜默截斷）。
    case resumeTooLong = "resume-too-long"
}

/// 一則 `state` 訊息對本地副本的意義。
enum SessionReceiveDecision: Equatable {
    /// 舊連線／舊請求世代的遲到品：丟掉並計數（**先於**一切 epoch 判斷）。
    case ignoreStaleConnection
    /// 別的 session 的狀態：不套用、不 realign（realign 只會再要一次別人的 session）。
    case rejectIdentity
    /// 不是一則能用的 state 訊息（snapshot 缺 hash／state；patch 缺 baseRevision）。
    case rejectInvalid
    /// host 明說 session 被重建：丟掉本地狀態，採用新的 epoch／revision。
    case reset
    /// 套用。
    case apply
    /// 接不上：不套用，改要求重新對齊（送 `character.session.resume`）。
    case realign(reason: SessionRealignReason)
    /// 同一個 session 真的倒退過（host 從較舊快照還原）：套用並退回 host 的 revision。
    case recover
    /// 落後：忽略（rollback 防護）。
    case ignoreStale
    /// 重播：已經套用過，什麼都不做。
    case alreadyApplied

    /// 穩定字串（fixture 與診斷共用）。
    var wireName: String {
        switch self {
        case .ignoreStaleConnection: return "ignore-stale-connection"
        case .rejectIdentity: return "reject-identity"
        case .rejectInvalid: return "reject-invalid"
        case .reset: return "reset"
        case .apply: return "apply"
        case .realign: return "realign"
        case .recover: return "recover"
        case .ignoreStale: return "ignore-stale"
        case .alreadyApplied: return "already-applied"
        }
    }

    /// 這個決策會讓本地採用 incoming 的狀態嗎？
    var adoptsState: Bool {
        switch self {
        case .apply, .reset, .recover: return true
        default: return false
        }
    }

    /// 這個決策需要呼叫端再發一次請求嗎？
    var realignReason: SessionRealignReason? {
        if case .realign(let reason) = self { return reason }
        return nil
    }

    /// resume 回覆逐則處理時，這一則是不是「良性的舊項」（跳過但不中止整批）。
    var isBenignSkip: Bool {
        switch self {
        case .alreadyApplied, .ignoreStale: return true
        default: return false
        }
    }
}

// MARK: - 有界 realign 預算

/// 連續 `AIPLimits.maxRealignAttempts` 次未能 apply 就是 unrecoverable。
///
/// realign 的效果是「再要一次權威讀取」；host 一直給對不上的東西時，沒有上限就是一個
/// 打不完的請求迴圈。達上限要照實說「狀態未知」——不是繼續轉圈圈，也不是假裝同步。
struct SessionRealignBudget: Equatable {
    private(set) var attempts: Int = 0

    init() {}

    /// 記下一次決策的結果（純函式：回傳新的預算，不改自己）。**App 的收訊路徑
    /// （`SessionClient.consume`）走的就是這一個**，不另外手寫一份記帳。
    ///
    /// `viaAuthoritativeReply` ＝ 這一則是**權威回覆**卻被 typed boundary 或身分／格式檢查
    /// 擋下：對方回答了、但答案沒用，算一次失敗。推播上的垃圾不算——它不是我們要來的答案。
    ///
    /// `realignRequestSent` ＝ realign 決定要發的那一則請求**真的送出去了嗎**。誠實階梯：
    /// 宣稱的嘗試次數必須等於真的做過的 round-trip，送出佇列滿／未連線時只是「還沒問成」，
    /// 不是「問了沒用」。預設 `true` ＝ 決策表本身的語意（跨語言 fixture 對的是這一版：
    /// 表看不到傳輸層），呼叫端知道送出結果時就把真相傳進來。
    func observing(
        _ decision: SessionReceiveDecision,
        viaAuthoritativeReply: Bool,
        realignRequestSent: Bool = true
    ) -> SessionRealignBudget {
        switch decision {
        case .apply, .reset, .recover:
            return SessionRealignBudget()
        case .realign:
            return realignRequestSent ? counting() : self
        case .rejectInvalid where viaAuthoritativeReply:
            return counting()
        default:
            return self
        }
    }

    /// 呼叫端自己記一次失敗（例如：真的送出去的 resume 還沒被回答就又對不齊）。
    func counting() -> SessionRealignBudget {
        var next = self
        // 有界：不得溢位（連續失敗的次數只需要與上限比較）。
        next.attempts = min(next.attempts &+ 1, Int.max - 1)
        return next
    }

    /// 已經到上限：狀態是**未知**，畫面照實說，不再自動重試。
    var isUnrecoverable: Bool { attempts >= AIPLimits.maxRealignAttempts }
}

// MARK: - resume 批次

/// 一則 `character.session.resume` 回覆逐則處理過後的結果。
struct SessionResumeBatch: Equatable {
    /// 逐則決策（中止之後不再有）。
    var decisions: [SessionReceiveDecision] = []
    /// 處理完之後的本地摘要。
    var view: SessionReceiverView
    /// 中止在第幾則（`nil` ＝ 整批走完）。
    var stoppedAt: Int?
    /// 中止的那一則決策。
    var halted: SessionReceiveDecision?
    /// 真的套用了幾則。
    var applied: Int = 0
    /// 良性跳過幾則（已套用／落後）。
    var skipped: Int = 0

    /// 整批的結論：中止就是中止的那一則，否則是最後一則決策（空批＝已對齊）。
    var outcome: SessionReceiveDecision {
        halted ?? decisions.last ?? .alreadyApplied
    }
}

// MARK: - 表本身

extension SessionDecisions {

    /// 兩個**都知道**的 hash 不同才算不符。有一邊不知道就是「沒得核對」，
    /// 誠實地不核對（不假裝核對過，也不把沒核對升級成錯誤）。
    static func hashesDisagree(_ lhs: String?, _ rhs: String?) -> Bool {
        guard let lhs, let rhs else { return false }
        return lhs != rhs
    }

    /// 接收端決策表（`docs/aip/character-session.md` §7.2，逐條照抄；第一個命中即決定）。
    static func decideReceive(view: SessionReceiverView, incoming: SessionIncomingState)
        -> SessionReceiveDecision
    {
        // 0. 舊連線／舊請求世代的遲到品。這是「上一條連線送出的 reset 現在才到」的唯一防線：
        //    它宣告的 epoch 一定與本地不同，任何 epoch 判斷都會被它騙過去。
        if incoming.arrivedOnGeneration != view.connectionGeneration {
            return .ignoreStaleConnection
        }
        // 1. 身分：別的 session 的狀態不是「比較舊」，是**不相干**——不套用也不 realign。
        //    只在本地**知道**自己的 sessionId 時才比對：本地有狀態但身分未知（例如由不帶
        //    `sessionId` 的 resume snapshot payload bootstrap 出來的那一份）不算不符。
        //    把「未知」當成不符是 fail-closed 的地雷：rejectIdentity 不 realign，之後每一則
        //    帶 sessionId 的訊息都會被擋掉，那台裝置永遠凍在舊狀態且沒有任何出路。
        //    未知的身分由 `advance` 在套用時記下 incoming 的 sessionId 補齊，下一則就有得比。
        if view.hasState, let known = view.sessionId, let claimed = incoming.sessionId,
            known != claimed
        {
            return .rejectIdentity
        }
        switch incoming.kind {
        case .snapshot: return decideSnapshot(view: view, incoming: incoming)
        case .patch: return decidePatch(view: view, incoming: incoming)
        }
    }

    private static func decideSnapshot(
        view: SessionReceiverView, incoming: SessionIncomingState
    ) -> SessionReceiveDecision {
        // 2. AIP 1.0 的 snapshot 必帶 hash 與 state；沒有 legacy profile。
        guard incoming.hash != nil, incoming.statePresent else { return .rejectInvalid }
        // 套用之前一律核對：算出來的與宣告的不同就不採用（reset／bootstrap 也一樣）。
        let unverified = hashesDisagree(incoming.hash, incoming.computedHash)
        func adopt(_ decision: SessionReceiveDecision) -> SessionReceiveDecision {
            unverified ? .realign(reason: .hashMismatch) : decision
        }
        // 3. host 明說重建了 session。epoch 相同的 `session-reset` **不算**（§7 第 4 步是
        //    「epoch 不同」）：host 重灌後 epoch 可能比本地記得的小，所以是「不同」不是「更大」。
        if incoming.reason == SessionNames.sessionReset,
            !view.hasState || incoming.epoch != view.epoch
        {
            return adopt(.reset)
        }
        // 4. bootstrap：本地什麼都沒有，第一份權威狀態直接收下。
        if !view.hasState { return adopt(.apply) }
        // 5. epoch 不同又沒有重建宣告：兩份狀態沒有可比的順序，不猜。
        //    host 對 epoch 不同的 resume 一律回 `session-reset` snapshot，所以一次就收斂。
        if incoming.epoch != view.epoch { return .realign(reason: .epochChanged) }
        // 6. 同一個 session 真的倒退過：host 明說 `recovery` 才採納
        //    （成員自己宣稱超前不算證據）。
        if incoming.reason == SessionNames.recovery, incoming.revision < view.revision {
            return adopt(.recover)
        }
        if incoming.revision < view.revision {
            // 7. 落後：忽略。權威回覆也一樣——「最新的回覆」不代表同一個 incarnation
            //    的回退是合法的；真的倒退過的 host 會說 `recovery`。
            return .ignoreStale
        }
        if incoming.revision == view.revision {
            // 8. 重播：什麼都不做。除非它宣告的 hash 與本地算出來的不同——那就是同一個
            //    revision 有兩份不同的狀態，只能重新對齊。
            return hashesDisagree(incoming.hash, view.stateHash)
                ? .realign(reason: .hashMismatch) : .alreadyApplied
        }
        // 9. 較新：核對過就套用。
        return adopt(.apply)
    }

    private static func decidePatch(view: SessionReceiverView, incoming: SessionIncomingState)
        -> SessionReceiveDecision
    {
        // 2（patch 版）：typed boundary 已經擋掉缺 baseRevision 的 patch，這裡是第二道。
        guard let baseRevision = incoming.baseRevision else { return .rejectInvalid }
        // 10. 補丁不是完整狀態：沒有本地副本就沒有東西可以套上去。
        if !view.hasState { return .realign(reason: .noLocal) }
        // 11. epoch 不同 → realign（以前 patch 分支完全不看 epoch，
        //     只靠 `baseRevision` 恰巧不符去擋）。
        if incoming.epoch != view.epoch { return .realign(reason: .epochChanged) }
        // 12. 落後／重播。
        if incoming.revision < view.revision { return .ignoreStale }
        if incoming.revision == view.revision { return .alreadyApplied }
        // 13. 接不上前一個 revision。
        if baseRevision != view.revision { return .realign(reason: .baseMismatch) }
        // 14. merge 之後的 hash 與 host 宣告的不同。
        if hashesDisagree(incoming.hash, incoming.computedHash) {
            return .realign(reason: .hashMismatch)
        }
        // 15. 其餘：套用。
        return .apply
    }

    /// 決策套用之後，本地那份摘要會變成什麼（純函式；不採用狀態的決策原樣回傳）。
    ///
    /// 套用時**記下 incoming 的 sessionId**：本地身分未知（規則 1 因此不比對）的那一份，
    /// 收下第一則帶 sessionId 的權威狀態之後就有身分可比，之後別的 session 立刻被規則 1 擋下。
    static func advance(
        view: SessionReceiverView,
        incoming: SessionIncomingState,
        decision: SessionReceiveDecision
    ) -> SessionReceiverView {
        guard decision.adoptsState else { return view }
        return SessionReceiverView(
            hasState: true,
            sessionId: incoming.sessionId ?? view.sessionId,
            epoch: incoming.epoch,
            revision: incoming.revision,
            // 本地重算的優先；沒算過就只能記下 host 宣告的那一個。
            stateHash: incoming.computedHash ?? incoming.hash,
            connectionGeneration: view.connectionGeneration)
    }

    /// resume 回覆的逐則規則：
    ///
    /// - 數量超過 `AIPLimits.maxResumePatches` → 整批不處理，直接 realign
    ///   （**不**靜默截斷成「我以為我追上了」）。
    /// - `already-applied`／`ignore-stale` 是良性的舊項（host 回放的範圍本來就可能與本地
    ///   重疊）：跳過，**不**中止。
    /// - 第一個帶 effect 的 realign／reject 就中止：後面的補丁都建立在沒套用的那一則之上。
    static func decideResumeBatch(view: SessionReceiverView, items: [SessionIncomingState])
        -> SessionResumeBatch
    {
        runResumeBatch(view: view, count: items.count) { index, view in
            let decision = decideReceive(view: view, incoming: items[index])
            return (decision, advance(view: view, incoming: items[index], decision: decision))
        }
    }

    /// 上面那些規則的**唯一一份實作**。
    ///
    /// 為什麼要多這一層：App 不能先把整批決策算完再套用——補丁的 `computedHash` 只有在
    /// 前一則真的 merge 進本地狀態之後才算得出來。所以逐則的「怎麼決定」由呼叫端提供
    ///（純函式版查表；`SessionClient.handleResponse` 版真的套用並回報決策），而**中止／
    /// 跳過／上限**這三條批次規則只有這裡有一份：兩邊漂移不了，fixture 對到的就是出貨路徑。
    ///
    /// - Parameter step: 第 `index` 則在 `view` 之下的決策，以及採用之後的新 view。
    static func runResumeBatch(
        view: SessionReceiverView,
        count: Int,
        step: (Int, SessionReceiverView) -> (SessionReceiveDecision, SessionReceiverView)
    ) -> SessionResumeBatch {
        // 上限的判斷式只有一個（`resumeExceedsBound`，與 `AIPLimits` 同一個數字）：
        // 這裡用它，那個 helper 才不會變成只有測試看得到的第二份規則。
        guard !resumeExceedsBound(count) else {
            return SessionResumeBatch(
                decisions: [], view: view, stoppedAt: 0,
                halted: .realign(reason: .resumeTooLong), applied: 0, skipped: 0)
        }
        var batch = SessionResumeBatch(view: view, stoppedAt: nil, halted: nil)
        for index in 0..<count {
            let (decision, next) = step(index, batch.view)
            batch.decisions.append(decision)
            if decision.adoptsState {
                batch.view = next
                batch.applied += 1
            } else if decision.isBenignSkip {
                batch.skipped += 1
            } else {
                batch.stoppedAt = index
                batch.halted = decision
                return batch
            }
        }
        return batch
    }
}
