//
//  SocketTransport.swift
//  InteractionCompanion
//
//  `ConnectionManager` 與外界的兩個接縫：**socket** 與**排程**。
//
//  為什麼要這兩個協定：背景／前景閘門（`LifecycleDecision`）散落在五個接線點——
//  `sendStatusNow`／`startStatusTimer`／`handleConnectionLost`／`scheduleRetry`／重連 work
//  真的觸發的那一刻。少接一個，決策表寫得再好也沒用，而畫面照樣顯示已連線。
//  以前要驗這五個點得開一條真的 wss 並等真的 15 秒，等於沒辦法驗；把最外面這一層抽出來
//  之後，握手、有界送出佇列、接收迴圈、重連退避全都走**正式路徑**，只有 URLSession 與
//  Timer 被換成測試替身（`ConnectionManagerGateTests`）。
//
//  這裡**沒有**任何政策：TLS 指紋比對仍在 `PinnedWebSocketDelegate`，
//  重連與心跳的決策仍在 `ConnectionManager`／`LifecycleDecision`。
//

import Foundation

// MARK: - socket

/// 收到的一個 frame。協議規定 text；其他一律誠實記錄後忽略（不猜內容）。
enum SocketFrame: Equatable {
    case text(String)
    case nonText(description: String)
}

/// 一條 socket 的事件。TLS 指紋比對失敗要單獨通知——URLSession 之後只會回報
/// `cancelled`，沒有這個事件就無法把「安全事件」與「連不到桌面」分開。
struct SocketEvents {
    var onOpen: () -> Void = {}
    var onClose: (String) -> Void = { _ in }
    var onFingerprintMismatch: () -> Void = {}
}

/// `ConnectionManager` 用得到的 socket 能力（完成回呼可能在任何執行緒）。
protocol SocketTransport: AnyObject {
    func resume()
    func cancel()
    func send(_ text: String, completion: @escaping (Error?) -> Void)
    func receive(_ completion: @escaping (Result<SocketFrame, Error>) -> Void)
    /// 殭屍連線 watchdog。
    func ping(_ completion: @escaping (Error?) -> Void)
}

/// 依 URL 與憑證指紋開一條 socket。
typealias SocketFactory = (URL, String, SocketEvents) -> SocketTransport

/// 正式路徑：`URLSessionWebSocketTask` ＋ TLS 指紋固定。
///
/// 這個類別只是把「一條 socket 的三件事」（session／delegate／task）綁在一起，
/// 讓 `ConnectionManager` 不必自己管三個欄位的生命週期。
final class URLSessionSocket: SocketTransport {
    private let session: URLSession
    private let task: URLSessionWebSocketTask
    /// URLSession 會強引用 delegate 到 invalidate 為止；這裡留一份是為了生命週期清楚。
    private let delegate: PinnedWebSocketDelegate

    init(url: URL, fingerprint: String, events: SocketEvents) {
        let delegate = PinnedWebSocketDelegate(fingerprint: fingerprint)
        delegate.onOpen = events.onOpen
        delegate.onClose = events.onClose
        delegate.onFingerprintMismatch = events.onFingerprintMismatch
        self.delegate = delegate

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 10
        configuration.waitsForConnectivity = false
        session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
        task = session.webSocketTask(with: url)
    }

    func resume() { task.resume() }

    func cancel() {
        task.cancel(with: .normalClosure, reason: nil)
        session.invalidateAndCancel()
    }

    func send(_ text: String, completion: @escaping (Error?) -> Void) {
        task.send(.string(text)) { completion($0) }
    }

    func receive(_ completion: @escaping (Result<SocketFrame, Error>) -> Void) {
        task.receive { result in
            switch result {
            case .success(let message):
                switch message {
                case .string(let text):
                    completion(.success(.text(text)))
                case .data:
                    completion(.success(.nonText(description: "非預期的 binary frame")))
                @unknown default:
                    completion(.success(.nonText(description: "未知 frame 型別")))
                }
            case .failure(let error):
                completion(.failure(error))
            }
        }
    }

    func ping(_ completion: @escaping (Error?) -> Void) {
        task.sendPing { completion($0) }
    }
}

// MARK: - 排程

/// 一則排好的工作。取消之後不得再觸發。
protocol ScheduledWork: AnyObject {
    func cancel()
}

/// 延後／週期性執行（可注入；正式路徑是 main runloop 上的 `Timer`）。
protocol WorkScheduler {
    /// - Returns: 可取消的把手。**一定**在主執行緒上執行 `body`。
    func schedule(after seconds: TimeInterval, repeats: Bool, _ body: @escaping () -> Void)
        -> ScheduledWork
}

/// 正式路徑：`RunLoop.main` 上的 `Timer`（`.common` mode，捲動時也照跑）。
struct RunLoopScheduler: WorkScheduler {
    private final class TimerWork: ScheduledWork {
        private let timer: Timer
        init(_ timer: Timer) { self.timer = timer }
        func cancel() { timer.invalidate() }
    }

    func schedule(after seconds: TimeInterval, repeats: Bool, _ body: @escaping () -> Void)
        -> ScheduledWork
    {
        // Timer 加在 main runloop 上，觸發時本來就在主執行緒：不再多跳一次
        //（多跳一次只會讓「送出」與「進背景」之間多一個看不見的視窗）。
        let timer = Timer(timeInterval: seconds, repeats: repeats) { _ in body() }
        RunLoop.main.add(timer, forMode: .common)
        return TimerWork(timer)
    }
}
