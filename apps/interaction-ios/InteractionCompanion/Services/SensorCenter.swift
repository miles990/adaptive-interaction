//
//  SensorCenter.swift
//  InteractionCompanion
//
//  感測中心:動作 / 電池 / 麥克風音量 / 位置(權限與狀態)。
//
//  不變量(對應 repo CLAUDE.md「感測不靜默」「預設關閉」):
//  - 每個感測預設 OFF;啟用需「使用者明確切換」且「系統權限已授予」兩者同時成立。
//  - 權限被拒 → status 誠實回報 denied,切換自動彈回 OFF,絕不假裝在感測。
//  - 麥克風只送音量 level(0.0–1.0,最多 2 次/秒),絕不送原始音訊。
//  - 斷線時 disableHighRiskSensors() 停用 mic / location;重連後不自動恢復。
//  - 位置(location)在 wire protocol v1 僅有 status 旗標與權限回報,
//    沒有定義 location observation——本 App 誠實地不送任何座標。
//

import Foundation
import AVFoundation
import UIKit
import CoreLocation
import CoreBluetooth

final class SensorCenter: NSObject, ObservableObject {
    // MARK: 開關(全部預設 OFF)

    @Published private(set) var motionEnabled = false
    @Published private(set) var batteryEnabled = false
    @Published private(set) var micLevelEnabled = false
    @Published private(set) var locationEnabled = false

    // MARK: 權限與能力(誠實呈現)

    @Published private(set) var micPermission: PermissionState = .notDetermined
    @Published private(set) var locationPermission: PermissionState = .notDetermined
    @Published private(set) var bluetoothPermission: PermissionState = .notDetermined
    /// 本裝置是否支援 deviceMotion(不可假設所有 iPhone 相同)
    let motionAvailable: Bool

    // MARK: 可見狀態

    @Published private(set) var isForeground = true
    @Published private(set) var lastMicLevel: Double?
    @Published private(set) var lastMotionEvent: String?
    /// 最近一次自動停用的原因(例如斷線),UI 誠實顯示
    @Published private(set) var lastAutoDisableNote: String?

    // MARK: 接線

    /// 觀察事件外送(接到 ConnectionManager.send)
    var onObservation: ((ClientMessage) -> Void)?
    /// 感測/權限有變 → 觸發 status 重送
    var onStatusChanged: (() -> Void)?
    /// BLE gateway 開關由 BleGateway 持有,這裡只匯總進 status
    var bleGatewayEnabledProvider: (() -> Bool)?

    // MARK: 私有

    private let motionService = MotionService()
    private var audioEngine: AVAudioEngine?
    private var lastMicSentAt = Date.distantPast
    /// 音量上報頻率上限:每 0.5 秒一次(協議:最多 2 次/秒)
    private let micMinInterval: TimeInterval = 0.5
    private let locationManager = CLLocationManager()
    /// 使用者「剛才」主動要求開啟位置、正在等系統授權結果。
    /// 沒有這個旗標,系統在其他時機回報 granted 也絕不自動開啟(預設 OFF 不變量)。
    private var pendingLocationEnableRequest = false
    private var batteryObservers: [NSObjectProtocol] = []

    override init() {
        motionAvailable = motionService.isAvailable
        super.init()
        locationManager.delegate = self
        refreshPermissions()
    }

    deinit {
        for observer in batteryObservers {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    // MARK: status 快照

    func snapshotFlags() -> SensorFlags {
        SensorFlags(
            motion: motionEnabled,
            battery: batteryEnabled,
            micLevel: micLevelEnabled,
            location: locationEnabled,
            bleGateway: bleGatewayEnabledProvider?() ?? false
        )
    }

    func snapshotPermissions() -> PermissionStates {
        refreshPermissions()
        return PermissionStates(
            microphone: micPermission,
            location: locationPermission,
            bluetooth: bluetoothPermission
        )
    }

    private func refreshPermissions() {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: micPermission = .granted
        case .denied: micPermission = .denied
        case .undetermined: micPermission = .notDetermined
        @unknown default: micPermission = .notDetermined
        }
        locationPermission = Self.mapLocation(locationManager.authorizationStatus)
        switch CBManager.authorization {
        case .allowedAlways: bluetoothPermission = .granted
        case .denied, .restricted: bluetoothPermission = .denied
        case .notDetermined: bluetoothPermission = .notDetermined
        @unknown default: bluetoothPermission = .notDetermined
        }
    }

    private static func mapLocation(_ status: CLAuthorizationStatus) -> PermissionState {
        switch status {
        case .authorizedWhenInUse, .authorizedAlways: return .granted
        case .denied, .restricted: return .denied
        case .notDetermined: return .notDetermined
        @unknown default: return .notDetermined
        }
    }

    private func statusChanged() {
        onStatusChanged?()
    }

    // MARK: 前景/背景(App scenePhase 餵入)

    func setForeground(_ foreground: Bool) {
        guard isForeground != foreground else { return }
        isForeground = foreground
        if batteryEnabled {
            sendBatteryObservation()  // foreground 是 battery facts 的一部分
        }
    }

    // MARK: 動作感測

    func setMotionEnabled(_ enabled: Bool) {
        if enabled {
            guard motionAvailable else {
                // 誠實:硬體不支援就維持 OFF,不模擬
                motionEnabled = false
                lastAutoDisableNote = "此裝置不支援動作感測"
                statusChanged()
                return
            }
            motionService.onEvent = { [weak self] kind in
                self?.emitMotionEvent(kind)
            }
            motionService.start()
            motionEnabled = true
        } else {
            motionService.stop()
            motionEnabled = false
        }
        statusChanged()
    }

    private func emitMotionEvent(_ kind: MotionEventKind) {
        lastMotionEvent = kind.rawValue
        // 僅語意事件 + ISO8601 時間戳;絕無原始軌跡
        onObservation?(.observation(
            receptor: "iphone.motion",
            facts: ["event": .string(kind.rawValue)],
            at: WireTime.nowISO8601()))
    }

    // MARK: 電池

    func setBatteryEnabled(_ enabled: Bool) {
        batteryEnabled = enabled
        UIDevice.current.isBatteryMonitoringEnabled = enabled
        if enabled {
            installBatteryObservers()
            sendBatteryObservation()
        } else {
            removeBatteryObservers()
        }
        statusChanged()
    }

    private func installBatteryObservers() {
        removeBatteryObservers()
        let center = NotificationCenter.default
        let names: [Notification.Name] = [
            UIDevice.batteryLevelDidChangeNotification,
            UIDevice.batteryStateDidChangeNotification,
        ]
        for name in names {
            let observer = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                self?.sendBatteryObservation()
            }
            batteryObservers.append(observer)
        }
    }

    private func removeBatteryObservers() {
        for observer in batteryObservers {
            NotificationCenter.default.removeObserver(observer)
        }
        batteryObservers.removeAll()
    }

    private func sendBatteryObservation() {
        guard batteryEnabled else { return }
        let device = UIDevice.current
        let rawLevel = device.batteryLevel
        // batteryLevel < 0 表示未知(例如模擬器)——誠實送 null,不編造數字
        let level: JSONValue = rawLevel >= 0
            ? .number((Double(rawLevel) * 100).rounded() / 100)
            : .null
        let charging = device.batteryState == .charging || device.batteryState == .full
        onObservation?(.observation(
            receptor: "iphone.battery",
            facts: [
                "level": level,
                "charging": .bool(charging),
                "foreground": .bool(isForeground),
            ],
            at: nil))
    }

    // MARK: 麥克風音量(高風險:斷線自動停用)

    func setMicLevelEnabled(_ enabled: Bool) {
        if !enabled {
            stopMic()
            micLevelEnabled = false
            statusChanged()
            return
        }
        switch AVAudioApplication.shared.recordPermission {
        case .granted:
            startMic()
        case .denied:
            micPermission = .denied
            micLevelEnabled = false
            statusChanged()  // 誠實回報 denied,不假裝在感測
        case .undetermined:
            Task { @MainActor [weak self] in
                let granted = await AVAudioApplication.requestRecordPermission()
                guard let self else { return }
                if granted {
                    self.startMic()
                } else {
                    self.micPermission = .denied
                    self.micLevelEnabled = false
                    self.statusChanged()
                }
            }
        @unknown default:
            micLevelEnabled = false
            statusChanged()
        }
    }

    private func startMic() {
        guard audioEngine == nil else {
            micLevelEnabled = true
            statusChanged()
            return
        }
        let audioSession = AVAudioSession.sharedInstance()
        do {
            try audioSession.setCategory(.playAndRecord, mode: .measurement,
                                         options: [.mixWithOthers, .defaultToSpeaker])
            try audioSession.setActive(true)
            let engine = AVAudioEngine()
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            input.installTap(onBus: 0, bufferSize: 2048, format: format) { [weak self] buffer, _ in
                // 只在此回呼內計算 RMS;buffer 不保留、不外送
                self?.processMicBuffer(buffer)
            }
            try engine.start()
            audioEngine = engine
            micLevelEnabled = true
            micPermission = .granted
            lastAutoDisableNote = nil
            statusChanged()
        } catch {
            audioEngine = nil
            micLevelEnabled = false
            lastAutoDisableNote = "麥克風啟動失敗:\(error.localizedDescription)"
            statusChanged()
        }
    }

    private func stopMic() {
        guard let engine = audioEngine else { return }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        audioEngine = nil
        lastMicLevel = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func processMicBuffer(_ buffer: AVAudioPCMBuffer) {
        guard let channelData = buffer.floatChannelData else { return }
        let frameCount = Int(buffer.frameLength)
        guard frameCount > 0 else { return }
        let samples = channelData[0]
        var sumSquares: Float = 0
        for index in 0..<frameCount {
            let sample = samples[index]
            sumSquares += sample * sample
        }
        let rms = sqrt(sumSquares / Float(frameCount))
        // dBFS 映射到 0.0–1.0(-60 dB → 0,0 dB → 1)
        let db = 20 * log10(max(Double(rms), 1e-9))
        let level = min(1.0, max(0.0, (db + 60) / 60))

        DispatchQueue.main.async { [weak self] in
            guard let self, self.micLevelEnabled else { return }
            let now = Date()
            guard now.timeIntervalSince(self.lastMicSentAt) >= self.micMinInterval else { return }
            self.lastMicSentAt = now
            let rounded = (level * 1000).rounded() / 1000
            self.lastMicLevel = rounded
            self.onObservation?(.observation(
                receptor: "iphone.mic-level",
                facts: ["level": .number(rounded)],
                at: nil))
        }
    }

    // MARK: 位置(v1 僅 status 旗標;高風險:斷線自動停用)

    func setLocationEnabled(_ enabled: Bool) {
        if !enabled {
            locationEnabled = false
            statusChanged()
            return
        }
        switch locationManager.authorizationStatus {
        case .authorizedWhenInUse, .authorizedAlways:
            locationEnabled = true
            statusChanged()
        case .notDetermined:
            // 授權結果經 delegate 回來後再真正開啟
            pendingLocationEnableRequest = true
            locationManager.requestWhenInUseAuthorization()
        case .denied, .restricted:
            locationPermission = .denied
            locationEnabled = false
            statusChanged()
        @unknown default:
            locationEnabled = false
            statusChanged()
        }
    }

    // MARK: 高風險感測停用(斷線時由 ConnectionManager 觸發)

    /// 停用 mic 與 location。BLE gateway 由 BleGateway.disable() 另行處理。
    /// 重連後「不」自動恢復——使用者必須重新手動開啟。
    func disableHighRiskSensors(reason: String) {
        var changed = false
        if micLevelEnabled {
            stopMic()
            micLevelEnabled = false
            changed = true
        }
        if locationEnabled {
            locationEnabled = false
            changed = true
        }
        if changed {
            lastAutoDisableNote = reason
            statusChanged()
        }
    }

    /// 「立即停止全部感測」按鈕:全部關閉(含低風險)。
    func stopAllSensors() {
        stopAllSensors(reason: "使用者手動停止全部感測")
    }

    /// 全部關閉(含低風險),並誠實記下原因給 UI 顯示。
    /// 桌面緊急停止(`stop-all { sensors: true }`)走這條:
    /// 重連後「不」自動恢復,使用者必須重新手動開啟。
    func stopAllSensors(reason: String) {
        setMotionEnabled(false)
        setBatteryEnabled(false)
        if micLevelEnabled {
            stopMic()
            micLevelEnabled = false
        }
        locationEnabled = false
        lastAutoDisableNote = reason
        statusChanged()
    }
}

// MARK: - CLLocationManagerDelegate

extension SensorCenter: CLLocationManagerDelegate {
    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let newState = Self.mapLocation(manager.authorizationStatus)
            self.locationPermission = newState
            if self.pendingLocationEnableRequest {
                // 只有使用者剛才主動要求開啟時,授權通過才真正開啟(預設 OFF 不變量)
                self.pendingLocationEnableRequest = false
                self.locationEnabled = (newState == .granted)
            } else if newState != .granted, self.locationEnabled {
                // 授權被收回 → 誠實地立刻關閉
                self.locationEnabled = false
            }
            self.statusChanged()
        }
    }
}
