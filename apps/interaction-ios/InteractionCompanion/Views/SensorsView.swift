//
//  SensorsView.swift
//  InteractionCompanion
//
//  感測分頁:
//  - 每個感測一列:開關 + 權限狀態(誠實顯示 granted/denied/notDetermined)。
//  - 不可用的感測顯示「不可用」並停用開關(不可假設所有 iPhone 硬體相同)。
//  - 任一感測啟用時顯示「感測中」橫幅 + 立即停止按鈕(感測不靜默)。
//

import SwiftUI

struct SensorsView: View {
    @EnvironmentObject private var sensors: SensorCenter
    @EnvironmentObject private var ble: BleGateway
    @EnvironmentObject private var connection: ConnectionManager

    var body: some View {
        NavigationStack {
            Form {
                if activeSensorNames.isEmpty == false {
                    activeBanner
                }
                sensorRows
                bleSection
                notesSection
            }
            .navigationTitle("感測")
        }
    }

    // MARK: 感測中橫幅

    private var activeSensorNames: [String] {
        var names: [String] = []
        if sensors.motionEnabled { names.append("動作") }
        if sensors.batteryEnabled { names.append("電池") }
        if sensors.micLevelEnabled { names.append("麥克風音量") }
        if sensors.locationEnabled { names.append("位置") }
        if ble.enabled { names.append("BLE 閘道") }
        return names
    }

    private var activeBanner: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Label("感測中:\(activeSensorNames.joined(separator: "、"))",
                      systemImage: "dot.radiowaves.left.and.right")
                    .font(.headline)
                    .foregroundStyle(.orange)
                Button(role: .destructive) {
                    sensors.stopAllSensors()
                    ble.disable(reason: "使用者手動停止全部感測")
                } label: {
                    Text("立即停止全部感測")
                        .frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.borderedProminent)
                .tint(.red)
            }
            .padding(.vertical, 4)
        }
    }

    // MARK: 感測列

    private var sensorRows: some View {
        Section("感測項目(全部預設關閉)") {
            // 動作
            VStack(alignment: .leading, spacing: 4) {
                Toggle(isOn: Binding(
                    get: { sensors.motionEnabled },
                    set: { sensors.setMotionEnabled($0) })) {
                    Label("動作(拿起/搖晃/放下/旋轉)", systemImage: "iphone.gen3.radiowaves.left.and.right")
                }
                .disabled(!sensors.motionAvailable)
                if !sensors.motionAvailable {
                    unavailableText("此裝置不支援 deviceMotion")
                } else {
                    footnote("只送語意事件,原始軌跡僅存在記憶體 3 秒滑動視窗,不落盤、不外送。")
                    if let last = sensors.lastMotionEvent {
                        footnote("最近事件:\(last)")
                    }
                }
            }

            // 電池
            VStack(alignment: .leading, spacing: 4) {
                Toggle(isOn: Binding(
                    get: { sensors.batteryEnabled },
                    set: { sensors.setBatteryEnabled($0) })) {
                    Label("電池(電量/充電/前景)", systemImage: "battery.75percent")
                }
                footnote("電量未知時(如模擬器)誠實回報 null,不編數字。")
            }

            // 麥克風音量
            VStack(alignment: .leading, spacing: 4) {
                Toggle(isOn: Binding(
                    get: { sensors.micLevelEnabled },
                    set: { sensors.setMicLevelEnabled($0) })) {
                    Label("麥克風音量", systemImage: "mic")
                }
                permissionLine("麥克風權限", sensors.micPermission)
                footnote("只送 0.0–1.0 音量值(最多每秒 2 次),絕不傳原始音訊。")
                if let level = sensors.lastMicLevel {
                    ProgressView(value: level)
                        .tint(.orange)
                }
            }

            // 位置
            VStack(alignment: .leading, spacing: 4) {
                Toggle(isOn: Binding(
                    get: { sensors.locationEnabled },
                    set: { sensors.setLocationEnabled($0) })) {
                    Label("位置(僅權限/狀態回報)", systemImage: "location")
                }
                permissionLine("位置權限", sensors.locationPermission)
                footnote("Wire protocol v1 未定義位置觀察,本 App 不會送出任何座標;此開關只影響 status 旗標與權限回報。")
            }
        }
    }

    // MARK: BLE 閘道

    private var bleSection: some View {
        Section("BLE 閘道") {
            VStack(alignment: .leading, spacing: 4) {
                Toggle(isOn: Binding(
                    get: { ble.enabled },
                    set: { ble.setEnabled($0) })) {
                    Label("BLE 閘道(代桌面端掃描/連線)", systemImage: "antenna.radiowaves.left.and.right")
                }
                permissionLine("藍牙權限", sensors.bluetoothPermission)
                footnote("狀態:\(ble.managerStateText)")
                if let event = ble.lastEvent {
                    footnote(event)
                }
                footnote("藍牙關閉或權限被拒時,對桌面端誠實回報錯誤,不假裝掃描。")
            }
        }
    }

    // MARK: 說明

    private var notesSection: some View {
        Section {
            if let note = sensors.lastAutoDisableNote {
                Label(note, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
            }
            footnote("與桌面端連線中斷時,麥克風、位置與 BLE 閘道會自動停用;重新連線後不會自動恢復,需要你重新開啟(高風險能力不自動恢復)。")
            if case .connected = connection.phase {
                footnote("感測狀態每 30 秒與每次變更時回報給桌面端。")
            } else {
                footnote("目前未連線:感測值不會外送(觀察訊息會被丟棄並計數)。")
            }
        }
    }

    // MARK: 小工具

    private func permissionLine(_ title: String, _ state: PermissionState) -> some View {
        HStack(spacing: 6) {
            Text("\(title):")
            Text(state.displayText)
                .foregroundStyle(state == .granted ? Color.green
                                 : state == .denied ? Color.red : Color.secondary)
        }
        .font(.footnote)
    }

    private func footnote(_ text: String) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.secondary)
    }

    private func unavailableText(_ reason: String) -> some View {
        Label("不可用:\(reason)", systemImage: "xmark.circle")
            .font(.footnote)
            .foregroundStyle(.secondary)
    }
}
