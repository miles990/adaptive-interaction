//
//  PairingView.swift
//  InteractionCompanion
//
//  連線分頁:
//  - 未配對:QR 掃描(VisionKit DataScannerViewController)或手動貼上 JSON + 配對碼。
//  - 已配對:顯示對方主機、憑證指紋、連線狀態,以及大顆「立即中斷」按鈕。
//  - auth-fail:誠實顯示「配對已被撤銷或過期,請重新配對」;
//    只有使用者按「解除配對」才清除 Keychain。
//  - 連續連線層失敗達門檻(ReconnectDiagnosis):顯示「桌面位址可能已變更」的固定文案
//    與「重新配對」捷徑(直接展開掃描/貼上)。TLS 指紋不符與 auth-fail 各有既有文案,
//    不會被這句蓋掉。
//

import SwiftUI
import AVFoundation
import VisionKit

struct PairingView: View {
    @EnvironmentObject private var connection: ConnectionManager

    @State private var showScanner = false
    @State private var scannerUnavailableNote: String?
    @State private var manualJSON = ""
    @State private var manualCode = ""
    @State private var parseErrorText: String?
    @State private var showUnpairConfirm = false
    /// 使用者按過「重新配對」捷徑:即使已配對也展開掃描/貼上區塊。
    @State private var showRepairInput = false
    #if DEBUG
    /// DEBUG 啟動參數只套用一次(onAppear 會因切換分頁重複觸發)。
    @State private var debugLaunchPayloadConsumed = false
    #endif

    var body: some View {
        NavigationStack {
            Form {
                statusSection
                if connection.pairing == nil || showRepairInput {
                    pairingInputSection
                }
                if connection.pairing != nil {
                    pairedInfoSection
                    disconnectSection
                }
                diagnosticsSection
            }
            .onAppear {
                #if DEBUG
                applyDebugLaunchPayloadIfAny()
                #endif
            }
            .onChange(of: connection.phase) { _, newPhase in
                // 連上了就收起重新配對區塊(不再需要)。
                if case .connected = newPhase {
                    showRepairInput = false
                }
            }
            .navigationTitle("連線")
            .sheet(isPresented: $showScanner) {
                QRScannerSheet { text in
                    showScanner = false
                    applyPayloadText(text)
                }
            }
            .confirmationDialog("確定解除配對?",
                                isPresented: $showUnpairConfirm,
                                titleVisibility: .visible) {
                Button("解除配對並清除本機金鑰", role: .destructive) {
                    connection.unpairByUser()
                }
                Button("取消", role: .cancel) {}
            } message: {
                Text("將刪除 Keychain 中的裝置憑證,之後需重新掃描 QR 配對。")
            }
        }
    }

    // MARK: 狀態

    private var statusSection: some View {
        Section("狀態") {
            HStack {
                Circle()
                    .fill(statusColor)
                    .frame(width: 12, height: 12)
                Text(connection.phase.displayText)
                    .font(.headline)
            }
            if case .revoked = connection.phase {
                Text("配對已被撤銷或過期，請重新配對。已儲存的配對資料仍保留,只有你按「解除配對」才會清除。")
                    .font(.footnote)
                    .foregroundStyle(.red)
            } else if case .suggestRepair(let reason) = connection.reconnectDiagnosis {
                // 只在「連續連線層失敗」時出現。TLS 指紋不符 / auth-fail 走各自文案,
                // 不會落到這裡(ReconnectDiagnosis 已把它們排除)。
                Text(reason.message)
                    .font(.footnote)
                    .foregroundStyle(.orange)
                Button {
                    showRepairInput = true
                } label: {
                    Label("重新配對", systemImage: "arrow.triangle.2.circlepath")
                }
            }
            if let error = connection.lastError {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var statusColor: Color {
        switch connection.phase {
        case .connected: return .green
        case .connecting, .pairing, .authenticating, .waitingRetry: return .orange
        case .revoked, .failed: return .red
        case .idle: return .gray
        }
    }

    // MARK: 未配對:輸入

    private var pairingInputSection: some View {
        Section(showRepairInput ? "重新配對" : "配對") {
            if showRepairInput {
                Text("在桌面重新產生配對碼(控制中心 → 手機),再掃描或貼上新的配對 JSON。配對成功會覆寫本機的舊位址。")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Button {
                startScanner()
            } label: {
                Label("掃描桌面端 QR Code", systemImage: "qrcode.viewfinder")
            }
            if let note = scannerUnavailableNote {
                Text(note)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("或手動貼上配對 JSON")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                TextEditor(text: $manualJSON)
                    .font(.system(.footnote, design: .monospaced))
                    .frame(minHeight: 88)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                TextField("配對碼(JSON 內含 code 時可留空)", text: $manualCode)
                    .keyboardType(.numberPad)
                Button("開始配對") {
                    applyPayloadText(manualJSON)
                }
                .disabled(manualJSON.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            if let parseError = parseErrorText {
                Text(parseError)
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
            if showRepairInput {
                Button("取消重新配對") {
                    showRepairInput = false
                    parseErrorText = nil
                }
            }
        }
    }

    private func startScanner() {
        guard DataScannerViewController.isSupported else {
            scannerUnavailableNote = "此裝置不支援相機文字/條碼掃描,請改用手動貼上。"
            return
        }
        AVCaptureDevice.requestAccess(for: .video) { granted in
            DispatchQueue.main.async {
                if granted && DataScannerViewController.isAvailable {
                    scannerUnavailableNote = nil
                    showScanner = true
                } else {
                    scannerUnavailableNote = "相機不可用或未授權,請改用手動貼上。"
                }
            }
        }
    }

    private func applyPayloadText(_ text: String) {
        switch PairingPayload.parse(text, codeOverride: manualCode) {
        case .success(let payload):
            parseErrorText = nil
            connection.startPairing(with: payload)
        case .failure(let error):
            parseErrorText = error.errorDescription ?? "配對內容無效"
        }
    }

    #if DEBUG
    /// DEBUG 限定的自動化配對入口(release 不編入):
    /// 啟動參數 `--pairing-payload <json>` 或環境變數 `INTERACT_PAIRING_PAYLOAD`
    /// 存在時,等同使用者把該 JSON 貼進「手動貼上」欄位並按「開始配對」——
    /// 走的是同一條 `applyPayloadText` 路徑,不複製任何配對邏輯。
    /// 用途:模擬器/CI 驗收(`xcrun simctl launch booted <bundle> --pairing-payload '<json>'`)。
    private func applyDebugLaunchPayloadIfAny() {
        guard !debugLaunchPayloadConsumed,
              let text = DebugLaunchOptions.pairingPayload else { return }
        debugLaunchPayloadConsumed = true
        manualJSON = text
        applyPayloadText(text)
    }
    #endif

    // MARK: 已配對:資訊

    private var pairedInfoSection: some View {
        Section("已配對的電腦") {
            if let pairing = connection.pairing {
                LabeledContent("主機", value: "\(pairing.host):\(pairing.port)")
                LabeledContent("裝置 ID", value: pairing.deviceId)
                VStack(alignment: .leading, spacing: 4) {
                    Text("憑證指紋(SHA-256)")
                    Text(Self.groupedFingerprint(pairing.fingerprint))
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                Text("連線只信任此指紋的自簽憑證;指紋不符會直接拒絕(防中間人)。")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    static func groupedFingerprint(_ fingerprint: String) -> String {
        var groups: [String] = []
        var remaining = Substring(fingerprint)
        while !remaining.isEmpty {
            let chunk = remaining.prefix(8)
            groups.append(String(chunk))
            remaining = remaining.dropFirst(chunk.count)
        }
        return groups.joined(separator: " ")
    }

    // MARK: 已配對:連線控制

    private var disconnectSection: some View {
        Section {
            if case .connected = connection.phase {
                bigDisconnectButton
            } else {
                Button {
                    connection.connectIfPaired()
                } label: {
                    Label("連線", systemImage: "bolt.horizontal")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                bigDisconnectButton
            }
            Button("解除配對…", role: .destructive) {
                showUnpairConfirm = true
            }
        } footer: {
            Text("「立即中斷」會停止連線與自動重連,並自動停用麥克風、位置與 BLE 閘道;重新連線後這些高風險感測不會自動恢復,需要你重新開啟。")
        }
    }

    private var bigDisconnectButton: some View {
        Button(role: .destructive) {
            connection.disconnectByUser()
        } label: {
            Text("立即中斷")
                .font(.title2.bold())
                .frame(maxWidth: .infinity, minHeight: 56)
        }
        .buttonStyle(.borderedProminent)
        .tint(.red)
    }

    // MARK: 診斷

    private var diagnosticsSection: some View {
        Section("診斷") {
            LabeledContent("丟棄的訊息", value: "\(connection.droppedFrames)")
            DisclosureGroup("連線記錄(僅本機)") {
                if connection.log.isEmpty {
                    Text("尚無記錄")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(Array(connection.log.suffix(20).enumerated()), id: \.offset) { _, line in
                        Text(line)
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}

// MARK: - QR 掃描(VisionKit DataScannerViewController 包裝)

struct QRScannerSheet: View {
    let onScan: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            QRScannerRepresentable(onScan: onScan)
                .ignoresSafeArea()
                .navigationTitle("掃描配對 QR")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("取消") { dismiss() }
                    }
                }
        }
    }
}

struct QRScannerRepresentable: UIViewControllerRepresentable {
    let onScan: (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        return scanner
    }

    func updateUIViewController(_ controller: DataScannerViewController, context: Context) {
        // 啟動失敗(相機被其他程式占用等)不吞掉,交由 UI 顯示手動備援
        do {
            try controller.startScanning()
        } catch {
            context.coordinator.reportStartFailure()
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onScan: onScan)
    }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        private let onScan: (String) -> Void
        private var delivered = false

        init(onScan: @escaping (String) -> Void) {
            self.onScan = onScan
        }

        func reportStartFailure() {
            // 誠實:無法啟動掃描時不假裝在掃;sheet 由使用者關閉後改用手動貼上
        }

        func dataScanner(_ dataScanner: DataScannerViewController,
                         didAdd addedItems: [RecognizedItem],
                         allItems: [RecognizedItem]) {
            guard !delivered else { return }
            for item in addedItems {
                if case .barcode(let barcode) = item,
                   let value = barcode.payloadStringValue {
                    delivered = true
                    onScan(value)
                    return
                }
            }
        }
    }
}
