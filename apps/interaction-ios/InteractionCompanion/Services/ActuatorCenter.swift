//
//  ActuatorCenter.swift
//  InteractionCompanion
//
//  動器中心:haptic.pulse / notify.show / tts.speak / screen.flash /
//  torch.set / character.present / stop-all。
//
//  誠實不變量:
//  - 每個 act 一律回 ack(附 applied 實況)或 err(附原因);不假裝成功。
//  - applied 用詞誠實:scheduled ≠ 已顯示;started ≠ 已完成。
//  - haptic 間隔 < 500ms → err "rate-limited"。
//  - screen.flash 僅前景;背景 → err "background"。
//  - 無手電筒硬體 → err "no-torch"。
//  - stop-all 立即停止 haptics / tts / torch / flash。
//

import Foundation
import SwiftUI
import CoreHaptics
import AVFoundation
import UserNotifications
import UIKit

// MARK: - 角色狀態(character.present)

enum CharacterPresentState: String, CaseIterable {
    case idle
    case working
    case waiting
    case verifiedSuccess = "verified-success"
    case failed
    case unknown
    case emergency
}

/// 角色顯示狀態。綠色勾號只允許在 verifiedSuccess 出現(CharacterView 強制)。
@MainActor
final class CharacterState: ObservableObject {
    @Published var state: CharacterPresentState = .idle
}

// MARK: - 畫面閃光要求

struct FlashRequest: Equatable {
    let color: Color
    let durationMs: Int
    let startedAt: Date
}

// MARK: - 前景通知呈現

/// 讓本機通知在 App 前景時也以橫幅顯示(否則會靜默)。
final class ForegroundNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification,
                                withCompletionHandler completionHandler:
                                @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound])
    }
}

// MARK: - ActuatorCenter

@MainActor
final class ActuatorCenter: NSObject, ObservableObject {
    @Published var flash: FlashRequest?
    @Published private(set) var torchOn = false
    /// 動器活動記錄(最多 50 行,只在本機;UI 誠實顯示實際發生的事)
    @Published private(set) var actionLog: [String] = []

    let characterState: CharacterState
    /// 前景判斷(接 SensorCenter.isForeground)
    var isForeground: (() -> Bool)?
    /// stop-all 帶 sensors:true(桌面緊急停止)時要停用的感測。
    /// 由 AppModel 接線到 SensorCenter.stopAllSensors(reason:) 與
    /// BleGateway.disable(reason:)——緊急停止不能只停動器,
    /// 手機的麥克風也是這個系統的感測器。重連後不自動恢復。
    var stopSensorsOnEmergency: ((String) -> Void)?

    private var hapticEngine: CHHapticEngine?
    private var hapticEngineStarted = false
    private let impactLight = UIImpactFeedbackGenerator(style: .light)
    private let impactMedium = UIImpactFeedbackGenerator(style: .medium)
    private let impactHeavy = UIImpactFeedbackGenerator(style: .heavy)
    private let synthesizer = AVSpeechSynthesizer()
    private let notificationDelegate = ForegroundNotificationDelegate()

    private var lastHapticAt = Date.distantPast
    private let hapticMinInterval: TimeInterval = 0.5
    private var hapticPlaybackTask: Task<Void, Never>?
    private var torchOffTask: Task<Void, Never>?
    private var flashClearTask: Task<Void, Never>?

    init(characterState: CharacterState) {
        self.characterState = characterState
        super.init()
        UNUserNotificationCenter.current().delegate = notificationDelegate
    }

    // MARK: act 進入點

    func handleAct(id: String, name: String, params: [String: JSONValue]) async -> ClientMessage {
        switch name {
        case "haptic.pulse":
            return handleHapticPulse(id: id, params: params)
        case "notify.show":
            return await handleNotifyShow(id: id, params: params)
        case "tts.speak":
            return handleTtsSpeak(id: id, params: params)
        case "screen.flash":
            return handleScreenFlash(id: id, params: params)
        case "torch.set":
            return handleTorchSet(id: id, params: params)
        case "character.present":
            return handleCharacterPresent(id: id, params: params)
        default:
            return .err(id: id, reason: "unknown-act:\(name)")
        }
    }

    // MARK: haptic.pulse

    private func handleHapticPulse(id: String, params: [String: JSONValue]) -> ClientMessage {
        let validStyles = ["light", "medium", "heavy", "purr", "heartbeat"]
        guard let style = params.string("style"), validStyles.contains(style) else {
            return .err(id: id, reason: "bad-params:style")
        }
        let count = params.int("count") ?? 1
        guard (1...5).contains(count) else {
            return .err(id: id, reason: "bad-params:count")
        }
        let now = Date()
        guard now.timeIntervalSince(lastHapticAt) >= hapticMinInterval else {
            return .err(id: id, reason: "rate-limited")
        }
        lastHapticAt = now

        let engineUsed: String
        switch style {
        case "purr", "heartbeat":
            if let engine = preparedHapticEngine(),
               playPattern(style: style, count: count, on: engine) {
                engineUsed = "coreHaptics"
            } else {
                // 誠實降級:CoreHaptics 不可用時以 UIImpact 近似,並在 applied 註明
                playImpactFallback(style: style, count: count)
                engineUsed = "uiImpactFallback"
            }
        default:
            playImpactPulses(style: style, count: count)
            engineUsed = "uiImpact"
        }
        logAction("haptic.pulse \(style) x\(count)(\(engineUsed))")
        return .ack(id: id, applied: [
            "style": .string(style),
            "count": .number(Double(count)),
            "engine": .string(engineUsed),
        ])
    }

    private func preparedHapticEngine() -> CHHapticEngine? {
        guard CHHapticEngine.capabilitiesForHardware().supportsHaptics else { return nil }
        if let engine = hapticEngine, hapticEngineStarted {
            return engine
        }
        do {
            let engine = try CHHapticEngine()
            engine.stoppedHandler = { [weak self] _ in
                Task { @MainActor in self?.hapticEngineStarted = false }
            }
            engine.resetHandler = { [weak self] in
                Task { @MainActor in self?.hapticEngineStarted = false }
            }
            try engine.start()
            hapticEngine = engine
            hapticEngineStarted = true
            return engine
        } catch {
            hapticEngine = nil
            hapticEngineStarted = false
            return nil
        }
    }

    /// purr:綿密的低強度 transient 連發;heartbeat:兩拍一組。
    private func playPattern(style: String, count: Int, on engine: CHHapticEngine) -> Bool {
        var events: [CHHapticEvent] = []
        switch style {
        case "purr":
            for repeatIndex in 0..<count {
                let base = Double(repeatIndex) * 0.8
                for step in 0..<10 {
                    events.append(CHHapticEvent(
                        eventType: .hapticTransient,
                        parameters: [
                            CHHapticEventParameter(parameterID: .hapticIntensity, value: 0.35),
                            CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.15),
                        ],
                        relativeTime: base + Double(step) * 0.07))
                }
            }
        case "heartbeat":
            for repeatIndex in 0..<count {
                let base = Double(repeatIndex) * 0.9
                events.append(CHHapticEvent(
                    eventType: .hapticTransient,
                    parameters: [
                        CHHapticEventParameter(parameterID: .hapticIntensity, value: 0.9),
                        CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.5),
                    ],
                    relativeTime: base))
                events.append(CHHapticEvent(
                    eventType: .hapticTransient,
                    parameters: [
                        CHHapticEventParameter(parameterID: .hapticIntensity, value: 0.55),
                        CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.3),
                    ],
                    relativeTime: base + 0.25))
            }
        default:
            return false
        }
        do {
            let pattern = try CHHapticPattern(events: events, parameters: [])
            let player = try engine.makePlayer(with: pattern)
            try player.start(atTime: CHHapticTimeImmediate)
            return true
        } catch {
            return false
        }
    }

    private func playImpactPulses(style: String, count: Int) {
        let generator: UIImpactFeedbackGenerator
        switch style {
        case "light": generator = impactLight
        case "heavy": generator = impactHeavy
        default: generator = impactMedium
        }
        generator.prepare()
        hapticPlaybackTask?.cancel()
        hapticPlaybackTask = Task { @MainActor [weak self] in
            for index in 0..<count {
                guard !Task.isCancelled, self != nil else { return }
                generator.impactOccurred()
                if index < count - 1 {
                    try? await Task.sleep(nanoseconds: 120_000_000)
                }
            }
        }
    }

    private func playImpactFallback(style: String, count: Int) {
        let pulses = style == "purr" ? count * 4 : count * 2
        let interval: UInt64 = style == "purr" ? 90_000_000 : 250_000_000
        impactLight.prepare()
        hapticPlaybackTask?.cancel()
        hapticPlaybackTask = Task { @MainActor [weak self] in
            for index in 0..<pulses {
                guard !Task.isCancelled, self != nil else { return }
                self?.impactLight.impactOccurred(intensity: style == "purr" ? 0.4 : 0.8)
                if index < pulses - 1 {
                    try? await Task.sleep(nanoseconds: interval)
                }
            }
        }
    }

    // MARK: notify.show

    private func handleNotifyShow(id: String, params: [String: JSONValue]) async -> ClientMessage {
        guard let title = params.string("title"), let body = params.string("body") else {
            return .err(id: id, reason: "bad-params:title/body")
        }
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            let granted = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
            guard granted else {
                return .err(id: id, reason: "notification-permission-denied")
            }
        case .denied:
            // 誠實:權限被拒就是失敗,不靜默吞掉
            return .err(id: id, reason: "notification-permission-denied")
        default:
            break
        }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(identifier: "interact-\(id)",
                                            content: content, trigger: nil)
        do {
            try await center.add(request)
            logAction("notify.show「\(title)」已排入系統通知")
            // scheduled ≠ 已顯示:實際呈現由 iOS 決定,誠實回報 scheduled
            return .ack(id: id, applied: [
                "scheduled": .bool(true),
                "title": .string(title),
            ])
        } catch {
            return .err(id: id, reason: "notification-failed:\(error.localizedDescription)")
        }
    }

    // MARK: tts.speak

    private func handleTtsSpeak(id: String, params: [String: JSONValue]) -> ClientMessage {
        guard let text = params.string("text"), !text.isEmpty else {
            return .err(id: id, reason: "bad-params:text")
        }
        guard text.count <= 200 else {
            return .err(id: id, reason: "text-too-long")
        }
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = AVSpeechSynthesisVoice(language: "zh-TW")
        utterance.rate = AVSpeechUtteranceDefaultSpeechRate
        synthesizer.speak(utterance)
        logAction("tts.speak 開始朗讀(\(text.count) 字)")
        // started ≠ 已唸完:誠實回報 started
        return .ack(id: id, applied: [
            "started": .bool(true),
            "chars": .number(Double(text.count)),
        ])
    }

    // MARK: screen.flash

    private func handleScreenFlash(id: String, params: [String: JSONValue]) -> ClientMessage {
        guard isForeground?() == true else {
            return .err(id: id, reason: "background")
        }
        guard let hexText = params.string("color"), let color = Self.parseHexColor(hexText) else {
            return .err(id: id, reason: "bad-params:color")
        }
        guard let durationMs = params.int("durationMs"), (1...1500).contains(durationMs) else {
            return .err(id: id, reason: "bad-params:durationMs")
        }
        flashClearTask?.cancel()
        flash = FlashRequest(color: color, durationMs: durationMs, startedAt: Date())
        flashClearTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(durationMs) * 1_000_000)
            guard !Task.isCancelled else { return }
            self?.flash = nil
        }
        logAction("screen.flash \(hexText) \(durationMs)ms")
        return .ack(id: id, applied: [
            "color": .string(hexText),
            "durationMs": .number(Double(durationMs)),
        ])
    }

    static func parseHexColor(_ hex: String) -> Color? {
        var cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if cleaned.hasPrefix("#") {
            cleaned.removeFirst()
        }
        guard cleaned.count == 6, let value = UInt32(cleaned, radix: 16) else { return nil }
        let red = Double((value >> 16) & 0xFF) / 255.0
        let green = Double((value >> 8) & 0xFF) / 255.0
        let blue = Double(value & 0xFF) / 255.0
        return Color(red: red, green: green, blue: blue)
    }

    // MARK: torch.set

    private func handleTorchSet(id: String, params: [String: JSONValue]) -> ClientMessage {
        guard let on = params.bool("on") else {
            return .err(id: id, reason: "bad-params:on")
        }
        guard let device = AVCaptureDevice.default(for: .video), device.hasTorch else {
            // 誠實:沒有手電筒硬體(部分 iPad / 模擬器)
            return .err(id: id, reason: "no-torch")
        }
        if on {
            guard let durationMs = params.int("durationMs"), (1...5000).contains(durationMs) else {
                return .err(id: id, reason: "bad-params:durationMs")
            }
            do {
                try device.lockForConfiguration()
                try device.setTorchModeOn(level: 1.0)
                device.unlockForConfiguration()
            } catch {
                return .err(id: id, reason: "torch-failed:\(error.localizedDescription)")
            }
            torchOn = true
            torchOffTask?.cancel()
            torchOffTask = Task { @MainActor [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(durationMs) * 1_000_000)
                guard !Task.isCancelled else { return }
                self?.setTorchHardwareOff()
            }
            logAction("torch.set on \(durationMs)ms(到時自動關)")
            return .ack(id: id, applied: [
                "on": .bool(true),
                "durationMs": .number(Double(durationMs)),
            ])
        } else {
            torchOffTask?.cancel()
            torchOffTask = nil
            do {
                try device.lockForConfiguration()
                device.torchMode = .off
                device.unlockForConfiguration()
            } catch {
                return .err(id: id, reason: "torch-failed:\(error.localizedDescription)")
            }
            torchOn = false
            logAction("torch.set off")
            return .ack(id: id, applied: ["on": .bool(false)])
        }
    }

    private func setTorchHardwareOff() {
        guard let device = AVCaptureDevice.default(for: .video), device.hasTorch else {
            torchOn = false
            return
        }
        do {
            try device.lockForConfiguration()
            device.torchMode = .off
            device.unlockForConfiguration()
            torchOn = false
        } catch {
            // 關不掉也要誠實記錄,不假裝已關
            logAction("torch 自動關閉失敗:\(error.localizedDescription)")
        }
    }

    // MARK: character.present

    private func handleCharacterPresent(id: String, params: [String: JSONValue]) -> ClientMessage {
        guard let raw = params.string("state"),
              let state = CharacterPresentState(rawValue: raw) else {
            return .err(id: id, reason: "bad-state")
        }
        characterState.state = state
        logAction("character.present → \(raw)")
        return .ack(id: id, applied: ["state": .string(raw)])
    }

    // MARK: stop-all

    /// 立即停止 haptics / tts / torch / flash。`sensors == true`(桌面
    /// 緊急停止)時連感測一起停:麥克風 / 位置 / BLE 閘道。
    /// 呼叫端(ConnectionManager)於本方法完成後回覆
    /// {"type":"ack","stopAll":true}。
    func stopAll(sensors: Bool = false) async {
        hapticPlaybackTask?.cancel()
        hapticPlaybackTask = nil
        if let engine = hapticEngine, hapticEngineStarted {
            // 停止失敗也照樣把旗標清掉:下次 haptic 會重新 start。
            try? await engine.stop()
            hapticEngineStarted = false
        }
        synthesizer.stopSpeaking(at: .immediate)
        torchOffTask?.cancel()
        torchOffTask = nil
        setTorchHardwareOff()
        flashClearTask?.cancel()
        flashClearTask = nil
        flash = nil
        logAction("stop-all:haptics/tts/torch/flash 已全部停止")
        if sensors {
            // 誠實:UI 會顯示「因桌面緊急停止而停用」,且不自動恢復。
            stopSensorsOnEmergency?("因桌面緊急停止而停用(麥克風/位置/BLE 閘道)")
            logAction("stop-all(sensors):已停用麥克風/位置/BLE 閘道")
        }
    }

    // MARK: 記錄

    private func logAction(_ text: String) {
        actionLog.append("[\(WireTime.nowISO8601())] \(text)")
        if actionLog.count > 50 {
            actionLog.removeFirst(actionLog.count - 50)
        }
    }
}
