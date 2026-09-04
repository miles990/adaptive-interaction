//
//  ContentView.swift
//  InteractionCompanion
//
//  三個分頁:連線 / 感測 / 角色。
//  screen.flash 以全螢幕覆蓋層呈現(僅前景;由 ActuatorCenter 控制生命週期)。
//  角色同步用戶端(SessionClient)由 ConnectionManager 擁有,在這裡注入環境。
//

import SwiftUI

enum AppTab: Hashable {
    case pairing
    case sensors
    case character
}

struct ContentView: View {
    @EnvironmentObject private var connection: ConnectionManager
    @EnvironmentObject private var sensors: SensorCenter
    @EnvironmentObject private var actuators: ActuatorCenter
    @EnvironmentObject private var ble: BleGateway

    @State private var selectedTab: AppTab = Self.initialTab()
    #if DEBUG
    @State private var debugAutoConnectConsumed = false
    #endif

    var body: some View {
        ZStack {
            TabView(selection: $selectedTab) {
                PairingView()
                    .tabItem {
                        Label("連線", systemImage: "link")
                    }
                    .tag(AppTab.pairing)
                SensorsView()
                    .tabItem {
                        Label("感測", systemImage: "dot.radiowaves.left.and.right")
                    }
                    .tag(AppTab.sensors)
                CharacterView()
                    .tabItem {
                        Label("角色", systemImage: "face.smiling")
                    }
                    .tag(AppTab.character)
            }
            // 角色同步的用戶端由 ConnectionManager 擁有(auth-ok 之後才協商);
            // 在這裡注入,分頁裡的視圖才觀察得到它的變化。
            .environmentObject(connection.characterSession)
            .onAppear {
                #if DEBUG
                // `--auto-connect`:已配對且沒有新的配對 payload 時,等同按「連線」。
                if !debugAutoConnectConsumed, DebugLaunchOptions.autoConnect,
                   DebugLaunchOptions.pairingPayload == nil, connection.pairing != nil {
                    debugAutoConnectConsumed = true
                    connection.connectIfPaired()
                }
                #endif
            }

            // screen.flash 覆蓋層:不攔截觸控,時間到由 ActuatorCenter 移除
            if let flash = actuators.flash {
                flash.color
                    .ignoresSafeArea()
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
        }
        .animation(.easeInOut(duration: 0.12), value: actuators.flash)
    }

    /// 預設從「連線」分頁開始;DEBUG 可用 `--initial-tab` 指定
    /// (有 `--pairing-payload` 時一律停在「連線」,因為配對入口在那一頁)。
    private static func initialTab() -> AppTab {
        #if DEBUG
        guard DebugLaunchOptions.pairingPayload == nil else { return .pairing }
        switch DebugLaunchOptions.initialTab {
        case "sensors": return .sensors
        case "character": return .character
        default: return .pairing
        }
        #else
        return .pairing
        #endif
    }
}
