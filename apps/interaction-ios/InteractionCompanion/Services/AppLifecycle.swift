//
//  AppLifecycle.swift
//  InteractionCompanion
//
//  App 生命週期（前景／背景）與 presence 心跳的**純決策**。
//  沒有 I/O、沒有時鐘、不依賴 SwiftUI：`ConnectionManager` 只負責照決策執行，
//  決策本身由 `LifecycleTests` 表格化驗證。
//
//  誠實前提（不得偷偷放寬）：
//  - 本 App **沒有申請任何 Background Mode**（`Info.plist` 的 `UIBackgroundModes` 整段註解掉）。
//    進背景後 socket 交給 iOS，可能被暫停或收回；因此背景一律「停心跳、標成 background」，
//    **不假裝連線還活著**，也不在背景重連。
//  - presence 完全靠 legacy `status` 心跳維持，見 `PresenceHeartbeatPolicy`。
//

import Foundation

// MARK: - 生命週期階段

/// SwiftUI `ScenePhase` 的服務層鏡射：核心邏輯不依賴 SwiftUI，才測得動。
enum AppLifecyclePhase: String, Equatable {
    /// 前景且可互動。
    case active
    /// 過渡（通知中心下拉、App 切換器預覽、來電橫幅）：**還沒**進背景，不拆任何東西。
    case inactive
    /// 背景：iOS 隨時可能暫停行程並收回 socket。
    case background
}

extension AppLifecyclePhase {
    /// SwiftUI 未來若在 `ScenePhase` 新增 case，`@unknown default` 一律映射到這裡。
    ///
    /// 保守方向只有一個：**不是前景就當背景**——寧可停心跳並誠實標成背景，
    /// 也不要假裝連線還活著（`InteractionCompanionApp.ScenePhase.appLifecyclePhase`
    /// 是唯一的使用者，映射的意圖由 `LifecycleTests` 釘住）。
    static let unknownScenePhaseFallback: AppLifecyclePhase = .background
}

// MARK: - presence 心跳政策（唯一常數來源）

/// 手機在桌面 Character Session 裡的 presence 是怎麼維持的。
///
/// **耦合明示**：本版**不送** AIP `heartbeat`（`docs/aip/transport-bindings.md` §1.4 明載
/// 「App 端仍然建議之後補上每 15 秒一則 AIP heartbeat……目前**尚未**實作」）。
/// presence 完全靠 **wire protocol v1 的 `status` 訊息**維持：桌面 `mobile.rs` 收到 `status`
/// 就對已協商的手機呼叫 `Runtime::character_session_touch_presence`，`lastSeenAt` 前進。
/// 換句話說——**status 心跳停掉，presence 就會過期**，這兩件事是同一條命脈，不是兩個獨立功能。
enum PresenceHeartbeatPolicy {
    /// legacy `status` 心跳的間隔（秒）。同一個 timer 也送 WebSocket ping（殭屍連線 watchdog）。
    ///
    /// 必須 < `presenceTimeoutSeconds / 2`，否則漏送**一次**就會被桌面標成 offline
    /// （30 秒 × 2 = 60 秒 > 45 秒逾時 ＝ 零容錯，這是 v0.5／v0.6.0 的實況）。
    /// 電量取捨：心跳只在**前景**跑（`LifecycleDecision` 進背景就停 timer），
    /// 前景時螢幕本來就亮著、radio 也醒著，多一則小 JSON ＋ 一次 ws ping 的代價遠小於
    /// 「使用者正看著角色、桌面卻以為手機離線」的假離線。
    static let statusIntervalSeconds: TimeInterval = 15

    /// 桌面端的 presence 逾時（秒）：Rust `SessionConfig::presence_timeout_ms` 預設 45 000
    /// （`crates/interaction-session/src/types.rs`、`docs/aip/device-profile.md` §4）。
    /// **這是跨端契約**，改這個數字等於改桌面行為，兩邊要一起改。
    static let presenceTimeoutSeconds: TimeInterval = 45

    /// 連續漏送幾次心跳仍然不會被標成 offline。
    ///
    /// 漏 k 次代表相鄰兩次成功心跳相距 `(k + 1) × 間隔`；要嚴格小於逾時才安全。
    static var missedBeatsTolerated: Int {
        var tolerated = 0
        while TimeInterval(tolerated + 2) * statusIntervalSeconds < presenceTimeoutSeconds {
            tolerated += 1
        }
        return tolerated
    }

    /// 背景短於這個秒數就回前景（系統翻頁、截圖、權限對話框）時，不值得一次 resume round-trip：
    /// 這麼短的時間行程不會被暫停，socket 上的 frame 也不會遺失。
    static let minimumBackgroundForResume: TimeInterval = 1

    /// 收到桌面 AIP `heartbeat` 後，最短多久才再用一則 legacy `status` 回應一次。
    /// 對方猛送 heartbeat 時不得把有界送出佇列（64 則）灌爆。
    static let heartbeatReplyMinIntervalSeconds: TimeInterval = 5
}

// MARK: - 生命週期決策

/// 一次 scenePhase 變化要做的事。**只描述動作，不描述理由**——理由寫在下面的規則註解裡。
struct LifecycleDecision: Equatable {
    /// 立刻送一則 legacy `status`（不等下一次 timer）：把「假離線」的窗口縮到最小。
    var sendStatusNow = false
    /// （重新）啟動 status 心跳 timer。
    var restartTimer = false
    /// 停掉 status 心跳 timer（背景不送、也送不出去）。
    var stopTimer = false
    /// 讓 `SessionClient` 走一次 §7 的 resume（lastRevision／lastSequence／epoch）。
    var resumeSession = false
    /// socket 已經不在了：走重連（呼叫端會跳過第一次退避等待）。
    var reconnect = false

    /// - Parameters:
    ///   - phase: 新的生命週期階段。
    ///   - socketAlive: 目前是否真的處於已認證的連線（`ConnectionPhase.connected`）。
    ///   - sinceForeground: 距離「上次離開前景」有幾秒；`nil` ＝ 這次啟用之前**不曾**進過背景
    ///     （冷啟動，或只是 `.inactive` 閃一下）。
    static func on(
        phase: AppLifecyclePhase, socketAlive: Bool, sinceForeground: TimeInterval?
    ) -> LifecycleDecision {
        switch phase {
        case .background:
            // 沒有 Background Mode：停心跳，什麼都不送、也不重連。
            // 呼叫端另外把本地 presence 標成 background 並記下離開前景的時間。
            return LifecycleDecision(stopTimer: true)

        case .inactive:
            // 過渡狀態，隨時可能直接回 active。不拆 timer、不送、不重連。
            return LifecycleDecision()

        case .active:
            guard socketAlive else {
                // socket 已死：送不出 status 就不假裝送得出去，也沒有 session 可以對齊。
                return LifecycleDecision(reconnect: true)
            }
            let awayLongEnough =
                (sinceForeground ?? 0) >= PresenceHeartbeatPolicy.minimumBackgroundForResume
            return LifecycleDecision(
                sendStatusNow: true,
                restartTimer: true,
                // 真的進過背景才 reconcile：行程可能被暫停過，收到的 state 未必連續。
                resumeSession: awayLongEnough)
        }
    }

    /// 斷線之後可不可以排一次重連。
    ///
    /// 「不在背景重連」不能只發生在 scenePhase 變化那一刻：真正驅動重連的是
    /// `handleConnectionLost → scheduleRetry → openSocket`，socket 在剛進背景、
    /// 行程還沒被系統暫停的那幾秒窗口內回報錯誤時照樣會走這條路。本 App 沒有
    /// Background Mode，背景排重連只是「在被暫停前多開一條 socket」，而且就算連上
    /// 也送不出心跳——所以背景只記錄失敗，等 `lifecyclePhaseChanged(.active)` 自己的
    /// 重連分支處理。
    static func shouldScheduleReconnect(phase: AppLifecyclePhase) -> Bool {
        phase != .background
    }

    /// 現在可不可以送 presence 心跳（legacy `status`）。
    ///
    /// 背景中意外復活的連線若把 `status` 送出去，桌面會呼叫
    /// `character_session_touch_presence` 把這支手機標成 online，與本機 UI 這時顯示的
    /// 「背景（心跳已停）」直接矛盾。兩邊只能有一個事實。
    static func shouldSendPresenceHeartbeat(phase: AppLifecyclePhase) -> Bool {
        phase != .background
    }

    /// 回前景時可不可以「立刻」重連（跳過退避的那一次等待）。
    ///
    /// 只在使用者**仍然想連線**、手上有配對、而且目前既沒連上也沒在連的時候才算：
    /// - 按過「立即中斷」→ `userWantsConnection == false`，不得偷偷復活；
    /// - 配對被撤銷（`.revoked`）→ 有自己的文案與流程，不重連；
    /// - 已連線／連線中／配對中 → 不再開第二條 socket。
    static func shouldReconnectImmediately(
        phase: ConnectionPhase, userWantsConnection: Bool, hasPairing: Bool
    ) -> Bool {
        guard userWantsConnection, hasPairing else { return false }
        switch phase {
        case .waitingRetry, .failed, .idle:
            return true
        case .connected, .connecting, .authenticating, .pairing, .revoked:
            return false
        }
    }
}
