//
//  ConnectionManager.swift
//  InteractionCompanion
//
//  URLSessionWebSocketTask + TLS 憑證指紋固定(trust-on-first-use)。
//  - wss:// 自簽憑證:以配對 payload 內的 SHA-256(cert DER) 指紋比對,不符即斷線。
//  - 配對握手(pair-request → pair-challenge → pair-response → paired)。
//  - 重連 backoff 1s → 15s;auth-fail 時停止自動重連並誠實告知,
//    只有使用者明確操作才清除配對。
//  - 斷線時自動停用高風險感測(mic / location / BLE gateway),重連後不自動恢復。
//  - 送出佇列有界(64 則);超出即丟棄並計數,不無界堆積。
//  - 冷啟動:已配對且使用者上次的意圖是「想要連線」時自動重連
//    (ColdStartConnectDecision);感測**不隨之恢復**——SensorCenter 每次啟動都是全關,
//    重連只重建 socket,不打開任何受器。
//  - 連續連線層失敗(timeout / host unreachable / connection refused)達門檻時,
//    ReconnectDiagnosis 建議「桌面位址可能已變更,請重新配對」;
//    TLS 指紋不符與 auth-fail 有各自的既有文案,絕不混為 IP 變更。
//
//  執行緒模型:所有狀態一律在主執行緒上讀寫(URLSession 回呼 funnel 到 main)。
//

import Foundation
import CryptoKit
import Security
import UIKit

// MARK: - 連線狀態

enum ConnectionPhase: Equatable {
    /// 尚未連線(可能已配對、可能未配對)
    case idle
    case connecting
    /// 配對握手進行中
    case pairing
    /// 已連上,等待 auth-ok
    case authenticating
    /// auth-ok / paired 之後
    case connected
    /// 等待重連
    case waitingRetry(inSeconds: Int)
    /// auth-fail:配對已被撤銷或過期。不自動清除配對、不自動重連。
    case revoked(reason: String)
    case failed(reason: String)

    var displayText: String {
        switch self {
        case .idle: return "未連線"
        case .connecting: return "連線中…"
        case .pairing: return "配對中…"
        case .authenticating: return "認證中…"
        case .connected: return "已連線"
        case .waitingRetry(let seconds): return "\(seconds) 秒後重試"
        case .revoked: return "配對已被撤銷或過期，請重新配對"
        case .failed(let reason): return "失敗:\(reason)"
        }
    }
}

// MARK: - 本地 presence

/// 本地對「桌面現在看不看得到這台手機」的誠實認知。
///
/// **不是**桌面那邊的權威 presence——那由桌面依 `status` 心跳決定。這裡只誠實記錄
/// 「我在背景、心跳已經停了」,避免 UI 在背景還顯示得像連線好好的。
enum LocalPresence: String, Equatable {
    case foreground
    case background

    var displayText: String {
        switch self {
        case .foreground:
            return "前景(status 心跳運作中)"
        case .background:
            return "背景(心跳已停;桌面最遲 45 秒後會把這台裝置標成離線)"
        }
    }
}

// MARK: - 失敗分類與重連診斷(純函式,可單獨測試)

/// 一次連線失敗的成因分類。分類決定 UI 該說什麼——**不同成因不得共用文案**。
enum ConnectionFailureKind: Equatable {
    /// 連線層級:逾時 / 找不到主機 / 主機不可達 / 連線被拒 / 網路不可用。
    /// 只有這一類累積到門檻,才可能推論「桌面位址已變更」。
    case connectivity
    /// TLS 憑證被拒:指紋不符(可能憑證輪替或中間人),或其他憑證錯誤。
    /// 這是安全事件,不是位址變更。
    case tlsMismatch
    /// 桌面端明確拒絕(auth-fail):配對被撤銷或過期。權威資訊,勝過任何猜測。
    case authRejected
    /// 其他(協定錯誤、送出失敗、我方主動取消…)。不足以推論任何事。
    case other

    /// 把底層錯誤分類。`pinningRejected` 是「本次連線的憑證指紋比對失敗」旗標——
    /// 指紋不符時 URLSession 只回報 `cancelled`,不靠這個旗標會被誤判成連線層失敗。
    static func classify(error: Error, pinningRejected: Bool) -> ConnectionFailureKind {
        if pinningRejected {
            return .tlsMismatch
        }
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            switch nsError.code {
            case NSURLErrorTimedOut,
                 NSURLErrorCannotFindHost,
                 NSURLErrorCannotConnectToHost,
                 NSURLErrorNetworkConnectionLost,
                 NSURLErrorNotConnectedToInternet,
                 NSURLErrorDNSLookupFailed,
                 NSURLErrorResourceUnavailable,
                 NSURLErrorInternationalRoamingOff,
                 NSURLErrorCallIsActive,
                 NSURLErrorDataNotAllowed:
                return .connectivity
            case NSURLErrorSecureConnectionFailed,
                 NSURLErrorServerCertificateHasBadDate,
                 NSURLErrorServerCertificateUntrusted,
                 NSURLErrorServerCertificateHasUnknownRoot,
                 NSURLErrorServerCertificateNotYetValid,
                 NSURLErrorClientCertificateRejected,
                 NSURLErrorClientCertificateRequired:
                return .tlsMismatch
            default:
                return .other
            }
        }
        if nsError.domain == NSPOSIXErrorDomain {
            switch Int32(nsError.code) {
            case ECONNREFUSED, EHOSTUNREACH, ENETUNREACH, ETIMEDOUT, ENETDOWN, EHOSTDOWN:
                return .connectivity
            default:
                return .other
            }
        }
        return .other
    }
}

/// 一次失敗的紀錄。`at` 是單調時鐘秒數(不是牆鐘,不受校時影響)。
struct ConnectionFailure: Equatable {
    let kind: ConnectionFailureKind
    let at: TimeInterval
}

/// 重連診斷:純函式,不看網路、不猜測、也不代使用者做決定——只決定 UI 要不要
/// 主動提示「可能要重新配對」。
enum ReconnectDiagnosis: Equatable {
    /// 還在正常退避重連,不需要打擾使用者。
    case keepRetrying
    /// 建議使用者重新配對,並附上固定文案。
    case suggestRepair(reason: RepairReason)

    enum RepairReason: String, Equatable {
        /// 連續多次連不上舊位址:App 沒有 Bonjour 探索,host 釘在配對當下,
        /// 桌面換網路位址後只能重新配對。
        case hostAddressLikelyChanged

        /// UI 顯示的固定文案(唯一來源,避免各處各寫一句)。
        var message: String {
            switch self {
            case .hostAddressLikelyChanged:
                return "連不上桌面：可能是桌面的網路位址已變更。請在桌面重新產生配對碼並重新配對。"
            }
        }
    }

    /// 預設門檻:連續 4 次連線層失敗,或同一串失敗已持續 60 秒。
    static let defaultConsecutiveThreshold = 4
    static let defaultSustainedSeconds: TimeInterval = 60

    /// 只看**結尾那一串連續的 `.connectivity` 失敗**。
    /// 任何非 connectivity 的失敗(auth-fail / TLS 指紋不符 / 其他)都會打斷這串——
    /// 因為那些成因各有自己的、更精確的文案,不可被 IP 變更提示蓋掉。
    static func evaluate(failures: [ConnectionFailure],
                         consecutiveThreshold: Int = defaultConsecutiveThreshold,
                         sustainedSeconds: TimeInterval = defaultSustainedSeconds) -> ReconnectDiagnosis {
        var run: [ConnectionFailure] = []
        for failure in failures.reversed() {
            guard failure.kind == .connectivity else { break }
            run.append(failure)
        }
        guard let newest = run.first, let oldest = run.last else {
            return .keepRetrying
        }
        let reachedCount = run.count >= consecutiveThreshold
        let reachedDuration = run.count >= 2 && (newest.at - oldest.at) >= sustainedSeconds
        guard reachedCount || reachedDuration else {
            return .keepRetrying
        }
        return .suggestRepair(reason: .hostAddressLikelyChanged)
    }
}

/// 冷啟動要不要自動重連的純決策。
enum ColdStartConnectDecision {
    /// - Parameters:
    ///   - hasPairing: Keychain 內是否有配對資料。
    ///   - storedIntent: 使用者上次的連線意圖;`nil` 代表沒有紀錄
    ///     (例如從 v0.5.0 升級上來),此時視為「想要連線」。
    ///
    /// 這個決策**只影響 socket**。感測一律維持關閉:SensorCenter 每次啟動全關,
    /// 自動重連不會、也不可以打開任何受器。
    static func shouldAutoConnect(hasPairing: Bool, storedIntent: Bool?) -> Bool {
        guard hasPairing else { return false }
        return storedIntent ?? true
    }
}

// MARK: - TLS pinning 的 URLSession delegate

/// 只信任「憑證 DER 之 SHA-256 = 配對時記下的指紋」的伺服器。
/// 不看 CA、不看有效期以外的鏈——這就是 trust-on-first-use 的明確語意。
final class PinnedWebSocketDelegate: NSObject, URLSessionWebSocketDelegate {
    private let fingerprint: String
    var onOpen: (() -> Void)?
    var onClose: ((String) -> Void)?
    /// 指紋比對失敗。URLSession 之後只會回報 `cancelled`,沒有這個通知就無法
    /// 把「安全事件」與「連不到桌面」分開。
    var onFingerprintMismatch: (() -> Void)?

    init(fingerprint: String) {
        self.fingerprint = fingerprint.lowercased()
    }

    func urlSession(_ session: URLSession,
                    didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust,
              let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = chain.first else {
            DispatchQueue.main.async { [weak self] in self?.onFingerprintMismatch?() }
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        let der = SecCertificateCopyData(leaf) as Data
        let hash = Hex.encode(Data(SHA256.hash(data: der)))
        if hash == fingerprint {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            // 指紋不符:可能是憑證輪替或中間人。一律拒絕,由 UI 誠實顯示。
            DispatchQueue.main.async { [weak self] in self?.onFingerprintMismatch?() }
            completionHandler(.cancelAuthenticationChallenge, nil)
        }
    }

    func urlSession(_ session: URLSession,
                    webSocketTask: URLSessionWebSocketTask,
                    didOpenWithProtocol proto: String?) {
        DispatchQueue.main.async { [weak self] in
            self?.onOpen?()
        }
    }

    func urlSession(_ session: URLSession,
                    webSocketTask: URLSessionWebSocketTask,
                    didCloseWith closeCode: URLSessionWebSocketTask.CloseCode,
                    reason: Data?) {
        let text = reason.flatMap { String(data: $0, encoding: .utf8) }
            ?? "close code \(closeCode.rawValue)"
        DispatchQueue.main.async { [weak self] in
            self?.onClose?(text)
        }
    }
}

// MARK: - ConnectionManager

final class ConnectionManager: NSObject, ObservableObject {
    @Published private(set) var phase: ConnectionPhase = .idle
    @Published private(set) var pairing: StoredPairing?
    /// 有界佇列丟棄的訊息數(誠實計數,UI 可見)
    @Published private(set) var droppedFrames = 0
    @Published private(set) var lastError: String?
    /// 目前的 App 生命週期階段(由 `InteractionCompanionApp` 的 `scenePhase` 注入)。
    @Published private(set) var lifecyclePhase: AppLifecyclePhase = .active
    /// 本地 presence:背景時誠實標成 background,不假裝連線還活著。
    @Published private(set) var localPresence: LocalPresence = .foreground
    /// 重連診斷:連續連線層失敗達門檻時,UI 應主動提示「桌面位址可能已變更」。
    /// 只有 `ReconnectDiagnosis.evaluate` 能決定這個值。
    @Published private(set) var reconnectDiagnosis: ReconnectDiagnosis = .keepRetrying
    /// 診斷記錄(最多保留 100 行,只在本機)
    @Published private(set) var log: [String] = []

    /// AIP Character Session 的手機端(`docs/aip/character-session.md`)。
    /// 由本類別擁有:auth-ok 之後才協商,斷線即重置協商狀態。
    /// 所有存取都經 `withSession`——接收迴圈已把回呼 funnel 到主執行緒,
    /// 同步處理才能保證 state 的套用順序等於到達順序。
    let characterSession = SessionClient()
    private var characterSessionAttached = false

    // MARK: 外部接線(由 AppModel 設定)

    /// status 訊息內容的提供者
    var statusProvider: (() -> (SensorFlags, PermissionStates))?
    /// act 處理者:回傳 ack 或 err
    var actHandler: ((_ id: String, _ name: String, _ params: [String: JSONValue]) async -> ClientMessage)?
    /// stop-all 處理者(完成後由本類別送出
    /// {"type":"ack","stopAll":true,"sensors":<回音>})。
    /// `sensors == true` 時必須連感測一起停,不只是停動器;
    /// `reason` 只決定 UI 顯示哪一句停用說明,不改變停的範圍。
    var stopAllHandler: ((_ sensors: Bool, _ reason: StopAllReason) async -> Void)?
    /// ble.scan / ble.connect / ble.gatt 轉交 BleGateway
    var bleHandler: ((ServerMessage) -> Void)?
    /// 斷線通知:接線方必須在此停用 mic / location / BLE gateway(不自動恢復)
    var onDisconnected: (() -> Void)?

    // MARK: 私有狀態

    private let store: PairingStorage
    private let defaults: UserDefaults
    /// 目前這條 socket（`nil` ＝ 沒有連線）。正式路徑是 `URLSessionSocket`。
    private var socket: SocketTransport?
    /// 每開一條 socket ＋1：AIP 決策表規則 0 用它丟掉舊連線的遲到訊息
    /// （舊連線送出的 `session-reset` 宣告的 epoch 一定與本地不同，這是唯一防線）。
    private var connectionGeneration: UInt64 = 0

    private var outbox: [String] = []
    private let outboxLimit = 64
    private var sending = false

    private var statusWork: ScheduledWork?
    private var retryWork: ScheduledWork?
    private var backoffSeconds: Double = 1
    private let backoffMaxSeconds: Double = 15
    /// 使用者是否希望保持連線(立即中斷後為 false,停止自動重連)
    private var userWantsConnection = false
    /// 配對進行中的 payload(含配對碼;完成或失敗後立即丟棄)
    private var pendingPairing: PairingPayload?
    /// 最近的失敗紀錄(有界 32 筆),餵給 ReconnectDiagnosis
    private var failureHistory: [ConnectionFailure] = []
    private let failureHistoryLimit = 32
    /// 本次連線的憑證指紋比對是否失敗(每次 openSocket 重置)
    private var pinningRejected = false
    /// 上次離開前景的單調時鐘秒數;`nil` ＝ 這次回到前景之前不曾進過背景。
    private var leftForegroundAt: TimeInterval?
    /// 單調時鐘(測試可替換;systemUptime 不受校時影響)
    var monotonicNow: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
    /// 牆鐘(測試可替換)。AIP 的 `occurredAt`／`expiresAt`／resume 寬限窗用的是這一個。
    var wallClockNow: () -> Date = { Date() }
    /// 怎麼開一條 socket(測試可替換;正式路徑是 URLSession + TLS 指紋固定)。
    var socketFactory: SocketFactory = { url, fingerprint, events in
        URLSessionSocket(url: url, fingerprint: fingerprint, events: events)
    }
    /// 心跳與重連的排程(測試可替換;正式路徑是 main runloop 上的 Timer)。
    var scheduler: WorkScheduler = RunLoopScheduler()
    /// 生命週期閘門讀到的階段。**唯一的 owner 仍是 `lifecyclePhaseChanged`**——
    /// 這裡只是把「讀」獨立成一個可注入的點,讓重連/心跳的閘門不必真的開一條 socket 才測得到。
    var lifecyclePhaseForGating: () -> AppLifecyclePhase = { .active }

    /// UserDefaults 內的「使用者上次是否想要連線」。冷啟動據此決定要不要自動重連。
    private static let autoConnectIntentKey = "ai.adaptive-interaction.companion.autoConnectIntent"

    init(store: PairingStorage, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        super.init()
        lifecyclePhaseForGating = { [weak self] in self?.lifecyclePhase ?? .active }
        pairing = store.load()
    }

    // MARK: 冷啟動意圖

    /// `nil` = 沒有紀錄(例如從 v0.5.0 升級上來)。
    private var storedAutoConnectIntent: Bool? {
        get { defaults.object(forKey: Self.autoConnectIntentKey) as? Bool }
        set {
            if let newValue {
                defaults.set(newValue, forKey: Self.autoConnectIntentKey)
            } else {
                defaults.removeObject(forKey: Self.autoConnectIntentKey)
            }
        }
    }

    /// 冷啟動是否該自動重連(純決策的即時取值)。
    var shouldAutoConnectOnLaunch: Bool {
        ColdStartConnectDecision.shouldAutoConnect(hasPairing: pairing != nil,
                                                  storedIntent: storedAutoConnectIntent)
    }

    /// App 啟動時呼叫:已配對且使用者上次想連線就自動重連(沿用 1s→15s 退避)。
    /// **不會**恢復任何感測——SensorCenter 啟動時全關,這裡也不碰它。
    /// 已在連線/配對流程中(例如 DEBUG `--auto-connect` 先跑了)就不重複開 socket。
    func connectOnLaunchIfDesired() {
        guard case .idle = phase else { return }
        guard shouldAutoConnectOnLaunch else { return }
        logLine("冷啟動:已配對且上次為連線狀態,自動重連(感測維持關閉,不自動恢復)")
        connectIfPaired()
    }

    // MARK: 公開操作(一律在主執行緒呼叫)

    /// 已配對時建立連線並認證。
    func connectIfPaired() {
        guard let stored = pairing else {
            phase = .failed(reason: "尚未配對")
            return
        }
        userWantsConnection = true
        storedAutoConnectIntent = true
        cancelRetry()
        openSocket(host: stored.host, port: stored.port, fingerprint: stored.fingerprint)
    }

    /// 以配對 payload 開始首次配對(或位址變更後的重新配對)。
    func startPairing(with payload: PairingPayload) {
        userWantsConnection = true
        storedAutoConnectIntent = true
        cancelRetry()
        // 換了配對目標:舊位址的失敗紀錄不再有參考價值。
        resetFailureHistory()
        // 新的桌面是另一個 character session(epoch 可能比手機記得的還小):
        // 本地對權威狀態的認知必須歸零,否則對方的第一份快照會被當成 rollback。
        withSession { $0.pairingDidChange() }
        pendingPairing = payload
        openSocket(host: payload.host, port: payload.port, fingerprint: payload.fp)
    }

    /// 立即中斷:停止連線與自動重連。高風險感測由 onDisconnected 接線停用。
    func disconnectByUser() {
        userWantsConnection = false
        storedAutoConnectIntent = false
        pendingPairing = nil
        cancelRetry()
        resetFailureHistory()
        teardownSocket(notify: true)
        phase = .idle
        logLine("使用者中斷連線")
    }

    /// 解除配對:唯一會清除 Keychain 的路徑,必須來自使用者明確操作。
    func unpairByUser() {
        disconnectByUser()
        storedAutoConnectIntent = nil
        do {
            try store.clear()
        } catch {
            lastError = error.localizedDescription
            logLine("清除配對失敗:\(error.localizedDescription)")
        }
        pairing = nil
        withSession { $0.pairingDidChange() }
        logLine("使用者解除配對,Keychain 已清除")
    }

    /// 送出訊息。未連線(或超出有界佇列)時丟棄並計數——不無界堆積、不假裝已送達。
    func send(_ message: ClientMessage) {
        let allowed: Bool
        switch phase {
        case .connected:
            allowed = true
        case .pairing, .authenticating, .connecting:
            allowed = message.isHandshake
        default:
            allowed = false
        }
        guard allowed, socket != nil else {
            droppedFrames += 1
            return
        }
        do {
            enqueue(try message.encodeToJSONString())
        } catch {
            lastError = error.localizedDescription
            logLine("編碼失敗:\(error.localizedDescription)")
        }
    }

    // MARK: AIP Character Session 橋接

    /// 在主執行緒上同步操作 SessionClient。
    ///
    /// 呼叫點全部來自已經 funnel 到主執行緒的路徑(`receiveNext` 的
    /// `DispatchQueue.main.async`、握手回呼、使用者操作),所以
    /// `assumeIsolated` 是**成立的事實**而不是繞過檢查;真的在別的執行緒被呼叫
    /// 時會立刻中止,而不是靜默地弄壞狀態。
    private func withSession(_ body: @MainActor (SessionClient) -> Void) {
        MainActor.assumeIsolated {
            if !characterSessionAttached {
                characterSession.transport = self
                characterSessionAttached = true
            }
            body(characterSession)
        }
    }

    /// 立即送出一次 status(感測/權限變更時由接線方呼叫)。
    func sendStatusNow() {
        guard LifecycleDecision.shouldSendPresenceHeartbeat(phase: lifecyclePhaseForGating()) else {
            // 背景送出的心跳會讓桌面把這支手機標成 online,與本機顯示的「背景」互相矛盾。
            return
        }
        guard case .connected = phase, let provider = statusProvider else { return }
        let (sensors, permissions) = provider()
        send(.status(sensors: sensors, permissions: permissions))
    }

    // MARK: 生命週期(前景/背景)

    /// `scenePhase` 變化。決策由 `LifecycleDecision`(純函式)負責,這裡只執行。
    ///
    /// 為什麼要管:presence 完全靠 legacy `status` 心跳維持(`PresenceHeartbeatPolicy`),
    /// 而本 App **沒有 Background Mode**——背景時心跳送不出去也不該假裝送得出去。
    /// 回前景時則要把「假離線」的窗口縮到最小:立刻補一則 status,不等下一次 timer。
    func lifecyclePhaseChanged(to newPhase: AppLifecyclePhase) {
        guard newPhase != lifecyclePhase else { return }
        let sinceForeground = leftForegroundAt.map { max(0, monotonicNow() - $0) }
        let decision = LifecycleDecision.on(phase: newPhase,
                                            socketAlive: isConnected,
                                            sinceForeground: sinceForeground)
        lifecyclePhase = newPhase

        switch newPhase {
        case .background:
            leftForegroundAt = monotonicNow()
            localPresence = .background
            logLine("進入背景:停止 status 心跳。本 App 沒有 Background Mode,"
                + "不宣稱背景仍然保持連線")
            if case .waitingRetry = phase {
                // 排程中的重連留著只會在背景裡偷偷開一條 socket,而畫面上的「n 秒後重試」
                // 也不再是真的。停掉並誠實改成「回到前景再試」——`.failed` 正是
                // `shouldReconnectImmediately` 會立刻重連的狀態。
                cancelRetry()
                phase = .failed(reason: "背景中暫停重連,回到前景再試")
                logLine("進入背景:暫停等待中的重連,回到前景再試")
            }
        case .active:
            localPresence = .foreground
        case .inactive:
            break
        }

        if decision.stopTimer {
            stopStatusTimer()
        }
        if decision.sendStatusNow {
            // 不等 timer:縮小「桌面以為手機離線」的窗口。
            sendStatusNow()
            logLine("回到前景:立即送出一則 status 心跳(不等下一次 "
                + "\(Int(PresenceHeartbeatPolicy.statusIntervalSeconds)) 秒)")
        }
        if decision.restartTimer {
            startStatusTimer()
        }
        if decision.resumeSession {
            // socket 還活著:只 reconcile 角色狀態,不重播任何事件(AIP §8)。
            withSession { $0.foregroundDidResume(now: self.wallClockNow()) }
        }
        if decision.reconnect,
           LifecycleDecision.shouldReconnectImmediately(phase: phase,
                                                        userWantsConnection: userWantsConnection,
                                                        hasPairing: pairing != nil),
           let stored = pairing {
            // 回前景是新的一輪嘗試:第一次不等退避(使用者正看著畫面)。
            cancelRetry()
            backoffSeconds = 1
            logLine("回到前景:連線已不在,立即重連(不等退避)")
            openSocket(host: stored.host, port: stored.port, fingerprint: stored.fingerprint)
        }

        if newPhase == .active {
            // 這次背景往返已經處理完;之後的 .inactive 閃動不該再算成一次背景。
            leftForegroundAt = nil
        }
    }

    // MARK: 建立/拆除 socket

    private func openSocket(host: String, port: Int, fingerprint: String) {
        teardownSocket(notify: false)

        // IPv6 位址需加中括號
        let hostPart = host.contains(":") && !host.hasPrefix("[") ? "[\(host)]" : host
        guard let url = URL(string: "wss://\(hostPart):\(port)") else {
            phase = .failed(reason: "無效的主機位址")
            return
        }

        phase = .connecting
        pinningRejected = false
        // 新的一條連線 ＝ 新的世代:上一條連線的遲到訊息從這一刻起一律丟掉。
        connectionGeneration &+= 1
        logLine("連線 \(hostPart):\(port)")

        var events = SocketEvents()
        events.onOpen = { [weak self] in self?.handleSocketOpen() }
        events.onFingerprintMismatch = { [weak self] in
            guard let self else { return }
            self.pinningRejected = true
            self.logLine("憑證指紋不符,已拒絕連線(不是位址變更)")
        }
        events.onClose = { [weak self] reason in
            guard let self else { return }
            self.handleConnectionLost(reason: "連線關閉:\(reason)",
                                      kind: self.pinningRejected ? .tlsMismatch : .other)
        }

        let newSocket = socketFactory(url, fingerprint, events)
        socket = newSocket
        newSocket.resume()
        receiveNext()
    }

    private func teardownSocket(notify: Bool) {
        statusWork?.cancel()
        statusWork = nil
        outbox.removeAll()
        sending = false
        socket?.cancel()
        socket = nil
        if notify {
            // 不變量:斷線即停用高風險感測(mic/location/BLE gateway),不自動恢復
            onDisconnected?()
            // 角色同步:成員身分留在桌面那邊等逾時,但本地必須重新協商才能再送事件。
            withSession { $0.connectionDidDisconnect() }
        }
    }

    // MARK: 握手

    private func handleSocketOpen() {
        if let payload = pendingPairing {
            phase = .pairing
            send(.pairRequest(deviceName: UIDevice.current.name,
                              model: Self.deviceModelIdentifier()))
            _ = payload  // code 於 pair-challenge 時使用
        } else if let stored = pairing {
            phase = .authenticating
            send(.auth(deviceId: stored.deviceId, token: stored.deviceToken))
        } else {
            phase = .failed(reason: "無配對資料")
            teardownSocket(notify: true)
        }
    }

    // MARK: 接收迴圈

    private func receiveNext() {
        guard let currentSocket = socket else { return }
        let generation = connectionGeneration
        currentSocket.receive { [weak self] result in
            self?.onMain {
                guard let self else { return }
                switch result {
                case .success(let frame):
                    switch frame {
                    case .text(let text):
                        self.handleIncoming(text, generation: generation)
                    case .nonText(let description):
                        // 協議規定 text frame;其餘誠實記錄後忽略。
                        self.logLine("收到\(description),已忽略")
                    }
                    self.receiveNext()
                case .failure(let error):
                    self.handleConnectionLost(error: error)
                }
            }
        }
    }

    /// 把 socket 回呼 funnel 到主執行緒。**已經在主執行緒時同步執行**:
    /// state patch 的套用順序必須等於它們到達的順序,多排一次隊就不保證了
    ///(也讓閘門測試不必等 runloop)。
    private func onMain(_ body: @escaping () -> Void) {
        if Thread.isMainThread {
            body()
        } else {
            DispatchQueue.main.async(execute: body)
        }
    }

    private func handleIncoming(_ text: String, generation: UInt64) {
        // 舊連線的遲到訊息:socket 已經換過一條,這一則不代表現在的事實(**先於**一切)。
        guard generation == connectionGeneration else {
            logLine("忽略一則上一條連線的遲到訊息(世代 \(generation),現在是 \(connectionGeneration))")
            return
        }
        let message: ServerMessage
        do {
            message = try ServerMessage.decode(text)
        } catch {
            logLine("無法解碼訊息:\(error.localizedDescription)")
            return
        }

        switch message {
        case .pairChallenge(let nonce):
            guard let payload = pendingPairing else {
                logLine("收到 pair-challenge 但非配對狀態,忽略")
                return
            }
            // HMAC-SHA256(key: 配對碼 UTF-8, msg: nonce UTF-8)
            send(.pairResponse(hmac: Self.hmacHex(code: payload.code, nonce: nonce)))

        case .paired(let deviceId, let deviceToken):
            guard let payload = pendingPairing else {
                logLine("收到 paired 但非配對狀態,忽略")
                return
            }
            let stored = StoredPairing(deviceId: deviceId, deviceToken: deviceToken,
                                       host: payload.host, port: payload.port,
                                       fingerprint: payload.fp)
            pendingPairing = nil  // 配對碼用畢即棄
            do {
                try store.save(stored)
                pairing = stored
                enterConnected()
                logLine("配對完成:\(deviceId)")
            } catch {
                phase = .failed(reason: "配對成功但無法寫入 Keychain:\(error.localizedDescription)")
                teardownSocket(notify: true)
            }

        case .pairFail(let reason):
            pendingPairing = nil
            userWantsConnection = false
            storedAutoConnectIntent = false
            phase = .failed(reason: "配對失敗:\(reason)")
            teardownSocket(notify: true)

        case .authOk:
            enterConnected()
            logLine("認證成功")

        case .authFail(let reason):
            // 誠實告知;保留 Keychain,只有使用者明確操作才清除。
            // 桌面端明確拒絕是權威資訊:記為 authRejected,絕不讓「位址可能變更」
            // 的提示蓋掉撤銷文案;冷啟動也不再自動重連(避免對已撤銷的配對狂敲)。
            userWantsConnection = false
            storedAutoConnectIntent = false
            recordFailure(.authRejected)
            phase = .revoked(reason: reason)
            teardownSocket(notify: true)
            logLine("認證失敗:\(reason)")

        case .act(let id, let name, let params):
            dispatchAct(id: id, name: name, params: params)

        case .stopAll(let sensors, let reason):
            let handler = stopAllHandler
            // 整個 Task 綁在 MainActor:handler 完成後直接在主執行緒回覆,
            // 不需再 MainActor.run 內捕捉 weak var self(Swift 6 會視為錯誤)。
            Task { @MainActor [weak self] in
                await handler?(sensors, reason)
                // ack 回音 sensors:桌面端不必猜手機到底停了什麼。
                self?.send(.ackStopAll(sensors: sensors))
                let scope = sensors ? "動器與感測全部停止" : "只停動器"
                let cause = reason == .user ? "使用者停止全部感測" : "緊急停止"
                self?.logLine("stop-all(\(cause))已執行並回覆:\(scope)")
            }

        case .bleScan(let id, _, _), .bleConnect(let id, _), .bleGatt(let id, _, _, _, _, _):
            if let bleHandler {
                bleHandler(message)
            } else {
                send(.err(id: id, reason: "ble-gateway-disabled"))
            }

        case .aip(let envelope):
            // 原始 frame 文字要一起帶進去:角色狀態的 hash 必須對「桌面寫出來的文字」取,
            // 重新編碼過的 JSON 會讓數字字面走樣、hash 就對不上(見 CharacterSemantic.swift)。
            withSession {
                $0.handleFrame(envelope, rawFrame: text, arrivedOnGeneration: generation)
            }

        case .unknown(let type):
            logLine("未知訊息型別 \(type),已忽略(不假裝處理)")
        }
    }

    private func dispatchAct(id: String, name: String, params: [String: JSONValue]) {
        guard let handler = actHandler else {
            send(.err(id: id, reason: "unsupported"))
            return
        }
        Task { @MainActor [weak self] in
            let reply = await handler(id, name, params)
            self?.send(reply)
        }
    }

    // MARK: 已連線狀態

    private func enterConnected() {
        phase = .connected
        backoffSeconds = 1
        lastError = nil
        // 連上了:先前的失敗紀錄與「位址可能變更」提示都失去意義。
        resetFailureHistory()
        // 連上不代表感測恢復:感測只由使用者在「感測」頁明確開啟。
        sendStatusNow()
        startStatusTimer()
        // 角色同步:auth-ok 之後才(重新)協商。舊桌面收到 aip frame 只會忽略,
        // 所以這裡不需要先問「桌面支不支援」——沒有協商結果就一直是「尚未提供角色同步」。
        // 重連**不重播**任何互動事件或 intent,只 reconcile 狀態(AIP §8)。
        withSession { [connectionGeneration] in
            $0.connectionDidConnect(now: self.wallClockNow(), generation: connectionGeneration)
        }
    }

    /// 每 `PresenceHeartbeatPolicy.statusIntervalSeconds` 秒:送 status + WebSocket ping
    /// (watchdog,偵測殭屍連線)。
    ///
    /// **這則 `status` 就是 presence 心跳**:桌面 `mobile.rs` 收到後對已協商的手機呼叫
    /// `character_session_touch_presence`。停掉這個 timer ＝ 放棄 presence,所以背景時
    /// 只停 timer 並誠實標成 background,不假裝還活著(見 `lifecyclePhaseChanged`)。
    private func startStatusTimer() {
        statusWork?.cancel()
        statusWork = nil
        guard LifecycleDecision.shouldSendPresenceHeartbeat(phase: lifecyclePhaseForGating()) else {
            // 背景中意外復活的連線不得把心跳排程重新叫起來(見 `lifecyclePhaseChanged`)。
            return
        }
        statusWork = scheduler.schedule(
            after: PresenceHeartbeatPolicy.statusIntervalSeconds, repeats: true
        ) { [weak self] in
            guard let self else { return }
            self.sendStatusNow()
            self.socket?.ping { error in
                guard let error else { return }
                self.onMain {
                    self.handleConnectionLost(error: error, prefix: "ping 失敗:")
                }
            }
        }
    }

    private func stopStatusTimer() {
        statusWork?.cancel()
        statusWork = nil
    }

    // MARK: 斷線與重連

    /// 依底層錯誤自動分類後處理斷線。
    private func handleConnectionLost(error: Error, prefix: String = "") {
        let kind = ConnectionFailureKind.classify(error: error, pinningRejected: pinningRejected)
        handleConnectionLost(reason: prefix + error.localizedDescription, kind: kind)
    }

    private func handleConnectionLost(reason: String, kind: ConnectionFailureKind) {
        guard socket != nil else { return }  // 已拆除,避免重複處理
        logLine("連線中斷:\(reason)")
        lastError = reason
        recordFailure(kind)
        teardownSocket(notify: true)

        if case .revoked = phase {
            return  // 已撤銷:不重連
        }
        guard userWantsConnection, pairing != nil, pendingPairing == nil else {
            if pendingPairing != nil {
                pendingPairing = nil
                phase = .failed(reason: "配對連線中斷:\(reason)")
            } else if userWantsConnection {
                phase = .failed(reason: reason)
            } else {
                phase = .idle
            }
            return
        }
        guard LifecycleDecision.shouldScheduleReconnect(phase: lifecyclePhaseForGating()) else {
            // 背景:只誠實記錄失敗,不排重連。回到前景由 `lifecyclePhaseChanged(.active)`
            // 的重連分支處理(`.failed` 會走「立刻重連、不等退避」那一支)。
            // 底層原因已經記在 `lastError` 與診斷記錄裡;這裡只給使用者看得懂的下一步。
            phase = .failed(reason: "背景中暫停重連,回到前景再試")
            logLine("背景中連線中斷:不在背景重連,回到前景再試")
            return
        }
        scheduleRetry()
    }

    private func scheduleRetry() {
        cancelRetry()
        guard LifecycleDecision.shouldScheduleReconnect(phase: lifecyclePhaseForGating()) else {
            logLine("背景中:不排重連,回到前景再試")
            return
        }
        let delay = backoffSeconds
        backoffSeconds = min(backoffSeconds * 2, backoffMaxSeconds)
        phase = .waitingRetry(inSeconds: Int(delay.rounded()))
        retryWork = scheduler.schedule(after: delay, repeats: false) { [weak self] in
            guard let self, self.userWantsConnection, let stored = self.pairing else { return }
            guard LifecycleDecision.shouldScheduleReconnect(
                    phase: self.lifecyclePhaseForGating()) else {
                // 排程時還在前景、觸發時已經進背景:不開新 socket。
                self.phase = .failed(reason: "背景中暫停重連,回到前景再試")
                self.logLine("背景中:不重連,回到前景再試")
                return
            }
            self.openSocket(host: stored.host, port: stored.port,
                            fingerprint: stored.fingerprint)
        }
    }

    private func cancelRetry() {
        retryWork?.cancel()
        retryWork = nil
    }

    // MARK: 失敗紀錄與診斷

    /// 記一次失敗並重算診斷。診斷值只由 `ReconnectDiagnosis.evaluate` 決定。
    private func recordFailure(_ kind: ConnectionFailureKind) {
        failureHistory.append(ConnectionFailure(kind: kind, at: monotonicNow()))
        if failureHistory.count > failureHistoryLimit {
            failureHistory.removeFirst(failureHistory.count - failureHistoryLimit)
        }
        let updated = ReconnectDiagnosis.evaluate(failures: failureHistory)
        if updated != reconnectDiagnosis {
            reconnectDiagnosis = updated
            if case .suggestRepair(let reason) = updated {
                logLine("重連診斷:\(reason.message)")
            }
        }
    }

    private func resetFailureHistory() {
        failureHistory.removeAll()
        reconnectDiagnosis = .keepRetrying
    }

    // MARK: 有界送出佇列(完成回呼串行化,保序)

    private func enqueue(_ text: String) {
        guard outbox.count < outboxLimit else {
            droppedFrames += 1
            return
        }
        outbox.append(text)
        pump()
    }

    private func pump() {
        guard !sending, let currentSocket = socket, !outbox.isEmpty else { return }
        sending = true
        let text = outbox.removeFirst()
        currentSocket.send(text) { [weak self] error in
            self?.onMain {
                guard let self else { return }
                self.sending = false
                if let error {
                    self.handleConnectionLost(error: error, prefix: "送出失敗:")
                } else {
                    self.pump()
                }
            }
        }
    }

    // MARK: 工具

    private static func hmacHex(code: String, nonce: String) -> String {
        let key = SymmetricKey(data: Data(code.utf8))
        let mac = HMAC<SHA256>.authenticationCode(for: Data(nonce.utf8), using: key)
        return Hex.encode(Data(mac))
    }

    /// utsname.machine(例如 "iPhone15,3")
    static func deviceModelIdentifier() -> String {
        var info = utsname()
        uname(&info)
        var identifier = ""
        let mirror = Mirror(reflecting: info.machine)
        for child in mirror.children {
            if let value = child.value as? Int8, value != 0 {
                identifier.append(Character(UnicodeScalar(UInt8(bitPattern: value))))
            }
        }
        return identifier.isEmpty ? "unknown" : identifier
    }

    private func logLine(_ text: String) {
        let stamp = WireTime.nowISO8601()
        log.append("[\(stamp)] \(text)")
        if log.count > 100 {
            log.removeFirst(log.count - 100)
        }
    }
}

// MARK: - SessionTransport

/// AIP Character Session 用得到的傳輸能力。全部沿用既有的有界佇列與連線狀態判斷——
/// 角色同步**不會**多開一條旁路,也不會在未連線時假裝送出。
extension ConnectionManager: SessionTransport {
    var isConnected: Bool {
        if case .connected = phase { return true }
        return false
    }

    /// 配對綁定出來的身分。AIP 的 `source` 只能宣稱這一個 id(不符會被桌面拒絕)。
    var boundDeviceId: String? { pairing?.deviceId }

    @discardableResult
    func sendAip(_ envelope: AIPEnvelope) -> Bool {
        guard isConnected, socket != nil else {
            droppedFrames += 1
            return false
        }
        do {
            enqueue(try ClientMessage.aip(envelope).encodeToJSONString())
            return true
        } catch {
            // 編碼失敗(例如超過 AIP 大小上限):誠實計數,不假裝已送出。
            droppedFrames += 1
            logLine("角色同步訊息編碼失敗,未送出")
            return false
        }
    }

    func sendObservation(receptor: String, facts: [String: JSONValue]) {
        send(.observation(receptor: receptor, facts: facts, at: nil))
    }
}
