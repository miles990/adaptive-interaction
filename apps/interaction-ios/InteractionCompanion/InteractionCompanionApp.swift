//
//  InteractionCompanionApp.swift
//  InteractionCompanion
//
//  App 進入點與元件接線。
//  scenePhase →
//   - SensorCenter.setForeground(電池觀察含 foreground 欄位;screen.flash 僅前景可用)
//   - ConnectionManager.lifecyclePhaseChanged(前景 presence 心跳、回前景 resume/重連)
//

import SwiftUI

/// 全部服務的組裝與接線。閉包一律弱捕捉,避免
/// connection ↔ sensors / ble 之間的引用循環。
@MainActor
final class AppModel: ObservableObject {
    let connection: ConnectionManager
    let sensors: SensorCenter
    let actuators: ActuatorCenter
    let characterState: CharacterState
    let ble: BleGateway

    /// 冷啟動自動重連只做一次(WindowGroup 的 .task 可能因場景重建再跑)。
    private var launchReconnectAttempted = false

    init() {
        let store = PairingStore()
        characterState = CharacterState()
        actuators = ActuatorCenter(characterState: characterState)
        sensors = SensorCenter()
        ble = BleGateway()
        connection = ConnectionManager(store: store)

        // status 內容:感測旗標 + 權限(BLE gateway 旗標由 BleGateway 提供)
        sensors.bleGatewayEnabledProvider = { [weak ble] in
            ble?.enabled ?? false
        }
        connection.statusProvider = { [weak sensors] in
            guard let sensors else {
                return (SensorFlags(), PermissionStates())
            }
            return (sensors.snapshotFlags(), sensors.snapshotPermissions())
        }

        // 感測 → 連線
        sensors.onObservation = { [weak connection] message in
            connection?.send(message)
        }
        sensors.onStatusChanged = { [weak connection] in
            connection?.sendStatusNow()
        }
        ble.sendMessage = { [weak connection] message in
            connection?.send(message)
        }
        ble.onStatusChanged = { [weak connection] in
            connection?.sendStatusNow()
        }

        // 連線 → 動器 / BLE
        connection.actHandler = { [weak actuators] id, name, params in
            guard let actuators else {
                return .err(id: id, reason: "unavailable")
            }
            return await actuators.handleAct(id: id, name: name, params: params)
        }
        connection.stopAllHandler = { [weak actuators] sensors, reason in
            await actuators?.stopAll(sensors: sensors, reason: reason)
        }
        // 桌面 stop-all { sensors: true } → 動器與感測一起停;
        // 重連後不自動恢復,使用者必須重新開啟。
        // note 已由 ActuatorCenter 依 reason(使用者停止全部感測 / 緊急停止)決定,
        // 這裡只負責把同一句誠實說明帶到感測與 BLE 閘道兩邊的 UI。
        actuators.stopSensorsOnStopAll = { [weak sensors, weak ble] note in
            sensors?.stopAllSensors(reason: note)
            ble?.disable(reason: note)
        }
        connection.bleHandler = { [weak ble] message in
            ble?.handleServerMessage(message)
        }

        // 動器的前景判斷
        actuators.isForeground = { [weak sensors] in
            sensors?.isForeground ?? false
        }

        // 不變量:斷線 → 自動停用高風險感測(mic / location / BLE gateway),
        // 重連後不自動恢復,使用者必須重新開啟。
        connection.onDisconnected = { [weak sensors, weak ble] in
            sensors?.disableHighRiskSensors(reason: "連線中斷,已自動停用麥克風與位置")
            ble?.disable(reason: "連線中斷")
        }
    }

    /// 冷啟動(含系統終止 App 後重新啟動)自動重連。
    ///
    /// 只在「Keychain 有配對」且「使用者上次的意圖是想要連線」時才連
    /// (按過「立即中斷」、配對被撤銷、解除配對之後都不會自動連)。
    /// 沿用 ConnectionManager 既有的 1s→15s 退避,不另開重試邏輯。
    ///
    /// **不變量**:這裡只重建 socket,不碰任何感測。SensorCenter / BleGateway 每次
    /// 啟動都是全關,自動重連後麥克風、位置、BLE 閘道、電池、動作一律維持關閉,
    /// 必須由使用者在「感測」頁重新開啟。
    func startupReconnectIfDesired() {
        guard !launchReconnectAttempted else { return }
        launchReconnectAttempted = true
        #if DEBUG
        // 有新的配對 payload 時由 PairingView 走配對流程,不搶先開舊位址的 socket。
        guard DebugLaunchOptions.pairingPayload == nil else { return }
        #endif
        connection.connectOnLaunchIfDesired()
    }
}

#if DEBUG
/// DEBUG 限定的啟動選項(模擬器 / CI 自動化驗收;release 不編入)。
/// 每個選項都只是「替使用者做一個他本來就能在 UI 上做的動作」,
/// 不繞過任何配對、權限或政策檢查。
enum DebugLaunchOptions {
    /// `--pairing-payload <json>` 或 `INTERACT_PAIRING_PAYLOAD`:
    /// 等同貼上配對 JSON 並按「開始配對」。
    static var pairingPayload: String? {
        value(argument: "--pairing-payload", environment: "INTERACT_PAIRING_PAYLOAD")
    }

    /// `--initial-tab pairing|sensors|character` 或 `INTERACT_INITIAL_TAB`:
    /// 啟動時直接切到該分頁(方便截圖)。
    static var initialTab: String? {
        value(argument: "--initial-tab", environment: "INTERACT_INITIAL_TAB")
    }

    /// `--auto-connect` 或 `INTERACT_AUTO_CONNECT=1`:
    /// 已配對時等同按「連線」(走 Keychain token 的 auth → auth-ok 路徑)。
    static var autoConnect: Bool {
        CommandLine.arguments.contains("--auto-connect")
            || ProcessInfo.processInfo.environment["INTERACT_AUTO_CONNECT"] == "1"
    }

    private static func value(argument: String, environment: String) -> String? {
        let arguments = CommandLine.arguments
        if let index = arguments.firstIndex(of: argument),
           arguments.indices.contains(index + 1) {
            let text = arguments[index + 1].trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : text
        }
        if let text = ProcessInfo.processInfo.environment[environment]?
            .trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty {
            return text
        }
        return nil
    }
}
#endif

/// SwiftUI 的 `ScenePhase` → 服務層的 `AppLifecyclePhase`。
///
/// 服務層刻意不依賴 SwiftUI(才測得動),轉換只做這一次、只做在這裡。
/// 未來 SwiftUI 若新增 case,`@unknown default` 一律當成「不是前景」——
/// 寧可停心跳並誠實標成背景,也不要假裝連線還活著。
extension ScenePhase {
    var appLifecyclePhase: AppLifecyclePhase {
        switch self {
        case .active: return .active
        case .inactive: return .inactive
        case .background: return .background
        @unknown default: return .inactive
        }
    }
}

@main
struct InteractionCompanionApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model.connection)
                .environmentObject(model.sensors)
                .environmentObject(model.actuators)
                .environmentObject(model.characterState)
                .environmentObject(model.ble)
                .onChange(of: scenePhase) { _, newPhase in
                    model.sensors.setForeground(newPhase == .active)
                    // presence 靠 status 心跳維持,而心跳只在前景跑:
                    // 連線層也必須知道生命週期(見 ConnectionManager.lifecyclePhaseChanged)。
                    model.connection.lifecyclePhaseChanged(to: newPhase.appLifecyclePhase)
                }
                // 冷啟動自動重連(只重建連線,感測不隨之恢復)。
                .task {
                    model.startupReconnectIfDesired()
                }
        }
    }
}
