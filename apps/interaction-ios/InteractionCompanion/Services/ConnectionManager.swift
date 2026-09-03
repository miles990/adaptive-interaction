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
    /// 重連診斷:連續連線層失敗達門檻時,UI 應主動提示「桌面位址可能已變更」。
    /// 只有 `ReconnectDiagnosis.evaluate` 能決定這個值。
    @Published private(set) var reconnectDiagnosis: ReconnectDiagnosis = .keepRetrying
    /// 診斷記錄(最多保留 100 行,只在本機)
    @Published private(set) var log: [String] = []

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

    private let store: PairingStore
    private let defaults: UserDefaults
    private var session: URLSession?
    private var wsDelegate: PinnedWebSocketDelegate?
    private var task: URLSessionWebSocketTask?

    private var outbox: [String] = []
    private let outboxLimit = 64
    private var sending = false

    private var statusTimer: Timer?
    private var retryTimer: Timer?
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
    /// 單調時鐘(測試可替換;systemUptime 不受校時影響)
    var monotonicNow: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }

    /// UserDefaults 內的「使用者上次是否想要連線」。冷啟動據此決定要不要自動重連。
    private static let autoConnectIntentKey = "ai.adaptive-interaction.companion.autoConnectIntent"

    init(store: PairingStore, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        super.init()
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
        guard allowed, task != nil else {
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

    /// 立即送出一次 status(感測/權限變更時由接線方呼叫)。
    func sendStatusNow() {
        guard case .connected = phase, let provider = statusProvider else { return }
        let (sensors, permissions) = provider()
        send(.status(sensors: sensors, permissions: permissions))
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
        logLine("連線 \(hostPart):\(port)")

        let delegate = PinnedWebSocketDelegate(fingerprint: fingerprint)
        delegate.onOpen = { [weak self] in self?.handleSocketOpen() }
        delegate.onFingerprintMismatch = { [weak self] in
            guard let self else { return }
            self.pinningRejected = true
            self.logLine("憑證指紋不符,已拒絕連線(不是位址變更)")
        }
        delegate.onClose = { [weak self] reason in
            guard let self else { return }
            self.handleConnectionLost(reason: "連線關閉:\(reason)",
                                      kind: self.pinningRejected ? .tlsMismatch : .other)
        }
        wsDelegate = delegate

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 10
        configuration.waitsForConnectivity = false
        let newSession = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
        session = newSession

        let newTask = newSession.webSocketTask(with: url)
        task = newTask
        newTask.resume()
        receiveNext()
    }

    private func teardownSocket(notify: Bool) {
        statusTimer?.invalidate()
        statusTimer = nil
        outbox.removeAll()
        sending = false
        if let existing = task {
            existing.cancel(with: .normalClosure, reason: nil)
        }
        task = nil
        session?.invalidateAndCancel()
        session = nil
        wsDelegate = nil
        if notify {
            // 不變量:斷線即停用高風險感測(mic/location/BLE gateway),不自動恢復
            onDisconnected?()
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
        guard let currentTask = task else { return }
        currentTask.receive { [weak self] result in
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let frame):
                    switch frame {
                    case .string(let text):
                        self.handleIncoming(text)
                    case .data:
                        // 協議規定 text frame;binary 誠實記錄後忽略
                        self.logLine("收到非預期的 binary frame,已忽略")
                    @unknown default:
                        self.logLine("收到未知 frame 型別,已忽略")
                    }
                    self.receiveNext()
                case .failure(let error):
                    self.handleConnectionLost(error: error)
                }
            }
        }
    }

    private func handleIncoming(_ text: String) {
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
    }

    /// 每 30 秒:送 status + WebSocket ping(watchdog,偵測殭屍連線)。
    private func startStatusTimer() {
        statusTimer?.invalidate()
        let timer = Timer(timeInterval: 30, repeats: true) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self else { return }
                self.sendStatusNow()
                self.task?.sendPing { error in
                    if let error {
                        DispatchQueue.main.async {
                            self.handleConnectionLost(error: error, prefix: "ping 失敗:")
                        }
                    }
                }
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        statusTimer = timer
    }

    // MARK: 斷線與重連

    /// 依底層錯誤自動分類後處理斷線。
    private func handleConnectionLost(error: Error, prefix: String = "") {
        let kind = ConnectionFailureKind.classify(error: error, pinningRejected: pinningRejected)
        handleConnectionLost(reason: prefix + error.localizedDescription, kind: kind)
    }

    private func handleConnectionLost(reason: String, kind: ConnectionFailureKind) {
        guard task != nil else { return }  // 已拆除,避免重複處理
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
        scheduleRetry()
    }

    private func scheduleRetry() {
        cancelRetry()
        let delay = backoffSeconds
        backoffSeconds = min(backoffSeconds * 2, backoffMaxSeconds)
        phase = .waitingRetry(inSeconds: Int(delay.rounded()))
        let timer = Timer(timeInterval: delay, repeats: false) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self, self.userWantsConnection, let stored = self.pairing else { return }
                self.openSocket(host: stored.host, port: stored.port,
                                fingerprint: stored.fingerprint)
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        retryTimer = timer
    }

    private func cancelRetry() {
        retryTimer?.invalidate()
        retryTimer = nil
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
        guard !sending, let currentTask = task, !outbox.isEmpty else { return }
        sending = true
        let text = outbox.removeFirst()
        currentTask.send(.string(text)) { [weak self] error in
            DispatchQueue.main.async {
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
