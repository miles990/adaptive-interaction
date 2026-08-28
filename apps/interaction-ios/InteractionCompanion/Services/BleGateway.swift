//
//  BleGateway.swift
//  InteractionCompanion
//
//  BLE 閘道:代替桌面端執行 scan / connect / GATT read-write-subscribe。
//
//  不變量:
//  - 預設 OFF。只有使用者明確開啟後才建立 CBCentralManager(避免提前觸發權限詢問)。
//  - 藍牙關閉 / 權限被拒 → 誠實回 err("bluetooth-off" / "bluetooth-denied"),不假裝掃到東西。
//  - 每個請求都有 TTL(scan ≤ 8s;connect / GATT 10s watchdog),逾時誠實回 err。
//  - 斷線或停用時,所有待處理請求以 err 收尾,不留無主 request。
//  - 掃描結果誠實:name 未知回 null;不編造裝置。
//
//  執行緒:CBCentralManager 使用 main queue,所有狀態在主執行緒讀寫。
//

import Foundation
import CoreBluetooth

final class BleGateway: NSObject, ObservableObject {
    @Published private(set) var enabled = false
    @Published private(set) var managerStateText = "未啟用"
    @Published private(set) var lastEvent: String?

    /// 回覆訊息外送(接到 ConnectionManager.send)
    var sendMessage: ((ClientMessage) -> Void)?
    /// 開關狀態變更 → 觸發 status 重送
    var onStatusChanged: (() -> Void)?

    private var central: CBCentralManager?
    private var knownPeripherals: [UUID: CBPeripheral] = [:]
    private var connectedPeripherals: [UUID: CBPeripheral] = [:]

    private struct ScanJob {
        let id: String
        var devices: [UUID: BleDeviceInfo] = [:]
        var timeoutTask: Task<Void, Never>?
    }
    private var scanJob: ScanJob?

    private struct ConnectJob {
        let id: String
        var timeoutTask: Task<Void, Never>?
    }
    private var connectJobs: [UUID: ConnectJob] = [:]

    private struct GattJob {
        let id: String
        let peripheralId: UUID
        let op: String  // read | write | subscribe
        let serviceUuid: CBUUID
        let charUuid: CBUUID
        let value: Data?
        var timeoutTask: Task<Void, Never>?
    }
    private var gattJobs: [GattJob] = []

    private struct SubscriptionKey: Hashable {
        let peripheralId: UUID
        let charUuid: CBUUID
    }
    /// 訂閱中的特徵 → 原始 subscribe 請求 id(通知沿用該 id 回 ble.value)
    private var subscriptions: [SubscriptionKey: String] = [:]

    private let gattTimeoutSeconds: TimeInterval = 10
    private let connectTimeoutSeconds: TimeInterval = 10

    // MARK: 開關

    func setEnabled(_ on: Bool) {
        if on {
            guard central == nil else {
                enabled = true
                onStatusChanged?()
                return
            }
            // 到這一刻才建立 manager → 系統此時才會詢問藍牙權限
            central = CBCentralManager(delegate: self, queue: .main)
            enabled = true
            managerStateText = "啟動中…"
            lastEvent = "BLE gateway 已啟用"
            onStatusChanged?()
        } else {
            disable(reason: "使用者關閉")
        }
    }

    /// 停用並以 err 收尾所有待處理請求。斷線時由 ConnectionManager 呼叫,
    /// 重連後「不」自動恢復——使用者必須重新手動開啟。
    func disable(reason: String) {
        guard enabled || central != nil else { return }
        if let job = scanJob {
            job.timeoutTask?.cancel()
            central?.stopScan()
            reply(.err(id: job.id, reason: "gateway-disabled"))
            scanJob = nil
        }
        for (_, job) in connectJobs {
            job.timeoutTask?.cancel()
            reply(.err(id: job.id, reason: "gateway-disabled"))
        }
        connectJobs.removeAll()
        for job in gattJobs {
            job.timeoutTask?.cancel()
            reply(.err(id: job.id, reason: "gateway-disabled"))
        }
        gattJobs.removeAll()
        // 訂閱者也誠實告知已失效
        for (_, subscriptionId) in subscriptions {
            reply(.err(id: subscriptionId, reason: "gateway-disabled"))
        }
        subscriptions.removeAll()
        for (_, peripheral) in connectedPeripherals {
            central?.cancelPeripheralConnection(peripheral)
        }
        connectedPeripherals.removeAll()
        knownPeripherals.removeAll()
        central = nil
        enabled = false
        managerStateText = "未啟用"
        lastEvent = "BLE gateway 已停用(\(reason))"
        onStatusChanged?()
    }

    // MARK: Server 訊息入口

    func handleServerMessage(_ message: ServerMessage) {
        switch message {
        case .bleScan(let id, let serviceUuid, let durationMs):
            handleScan(id: id, serviceUuid: serviceUuid, durationMs: durationMs)
        case .bleConnect(let id, let peripheralId):
            handleConnect(id: id, peripheralId: peripheralId)
        case .bleGatt(let id, let peripheralId, let op, let serviceUuid, let charUuid, let valueHex):
            handleGatt(id: id, peripheralId: peripheralId, op: op,
                       serviceUuid: serviceUuid, charUuid: charUuid, valueHex: valueHex)
        default:
            break
        }
    }

    /// 回傳 nil 表示可用;否則為 err reason(誠實描述不可用原因)。
    private func readinessError() -> String? {
        guard enabled, let central else { return "ble-gateway-disabled" }
        switch CBManager.authorization {
        case .denied, .restricted:
            return "bluetooth-denied"
        case .notDetermined, .allowedAlways:
            break
        @unknown default:
            break
        }
        switch central.state {
        case .poweredOn:
            return nil
        case .poweredOff:
            return "bluetooth-off"
        case .unauthorized:
            return "bluetooth-denied"
        case .unsupported:
            return "bluetooth-unsupported"
        case .resetting, .unknown:
            return "bluetooth-not-ready"
        @unknown default:
            return "bluetooth-not-ready"
        }
    }

    // MARK: ble.scan

    private func handleScan(id: String, serviceUuid: String?, durationMs: Int) {
        if let reason = readinessError() {
            reply(.err(id: id, reason: reason))
            return
        }
        guard scanJob == nil else {
            reply(.err(id: id, reason: "scan-busy"))
            return
        }
        guard (1...8000).contains(durationMs) else {
            reply(.err(id: id, reason: "bad-params:durationMs"))
            return
        }
        var services: [CBUUID]?
        if let serviceUuid {
            guard let uuid = Self.makeCBUUID(serviceUuid) else {
                reply(.err(id: id, reason: "bad-uuid:serviceUuid"))
                return
            }
            services = [uuid]
        }

        var job = ScanJob(id: id)
        job.timeoutTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(durationMs) * 1_000_000)
            guard !Task.isCancelled else { return }
            self?.finishScan()
        }
        scanJob = job
        lastEvent = "掃描中(\(durationMs)ms)"
        central?.scanForPeripherals(withServices: services, options: nil)
    }

    private func finishScan() {
        guard let job = scanJob else { return }
        central?.stopScan()
        scanJob = nil
        // 依 RSSI 由強到弱排序;結果可能為空——空就是空,誠實回報
        let devices = job.devices.values.sorted { $0.rssi > $1.rssi }
        lastEvent = "掃描完成:\(devices.count) 個裝置"
        reply(.bleResult(id: job.id, devices: devices))
    }

    // MARK: ble.connect

    private func handleConnect(id: String, peripheralId: String) {
        if let reason = readinessError() {
            reply(.err(id: id, reason: reason))
            return
        }
        guard let uuid = UUID(uuidString: peripheralId) else {
            reply(.err(id: id, reason: "bad-peripheral-id"))
            return
        }
        if connectedPeripherals[uuid] != nil {
            reply(.ack(id: id, applied: ["connected": .bool(true), "alreadyConnected": .bool(true)]))
            return
        }
        guard connectJobs[uuid] == nil else {
            reply(.err(id: id, reason: "connect-busy"))
            return
        }
        var peripheral = knownPeripherals[uuid]
        if peripheral == nil {
            peripheral = central?.retrievePeripherals(withIdentifiers: [uuid]).first
            if let found = peripheral {
                knownPeripherals[uuid] = found
            }
        }
        guard let target = peripheral else {
            reply(.err(id: id, reason: "not-found"))
            return
        }
        var job = ConnectJob(id: id)
        job.timeoutTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(self?.connectTimeoutSeconds ?? 10) * 1_000_000_000)
            guard !Task.isCancelled else { return }
            guard let self, let pending = self.connectJobs.removeValue(forKey: uuid) else { return }
            self.central?.cancelPeripheralConnection(target)
            self.reply(.err(id: pending.id, reason: "connect-timeout"))
        }
        connectJobs[uuid] = job
        central?.connect(target, options: nil)
    }

    // MARK: ble.gatt

    private func handleGatt(id: String, peripheralId: String, op: String,
                            serviceUuid: String, charUuid: String, valueHex: String?) {
        if let reason = readinessError() {
            reply(.err(id: id, reason: reason))
            return
        }
        guard ["read", "write", "subscribe"].contains(op) else {
            reply(.err(id: id, reason: "bad-op"))
            return
        }
        guard let uuid = UUID(uuidString: peripheralId) else {
            reply(.err(id: id, reason: "bad-peripheral-id"))
            return
        }
        guard let peripheral = connectedPeripherals[uuid] else {
            reply(.err(id: id, reason: "not-connected"))
            return
        }
        guard let service = Self.makeCBUUID(serviceUuid) else {
            reply(.err(id: id, reason: "bad-uuid:serviceUuid"))
            return
        }
        guard let characteristic = Self.makeCBUUID(charUuid) else {
            reply(.err(id: id, reason: "bad-uuid:charUuid"))
            return
        }
        var value: Data?
        if op == "write" {
            guard let hexText = valueHex, let decoded = Hex.decode(hexText) else {
                reply(.err(id: id, reason: "bad-value-hex"))
                return
            }
            value = decoded
        }

        var job = GattJob(id: id, peripheralId: uuid, op: op,
                          serviceUuid: service, charUuid: characteristic, value: value)
        job.timeoutTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(self?.gattTimeoutSeconds ?? 10) * 1_000_000_000)
            guard !Task.isCancelled else { return }
            guard let self else { return }
            if let index = self.gattJobs.firstIndex(where: { $0.id == id }) {
                let expired = self.gattJobs.remove(at: index)
                self.reply(.err(id: expired.id, reason: "gatt-timeout"))
            }
        }
        gattJobs.append(job)

        // 已有已探索的目標特徵就直接執行,否則走探索鏈
        if let existing = findCharacteristic(on: peripheral, service: service, characteristic: characteristic) {
            execute(job: job, on: peripheral, characteristic: existing)
        } else {
            peripheral.discoverServices([service])
        }
    }

    private func findCharacteristic(on peripheral: CBPeripheral,
                                    service serviceUuid: CBUUID,
                                    characteristic charUuid: CBUUID) -> CBCharacteristic? {
        guard let services = peripheral.services else { return nil }
        for service in services where service.uuid == serviceUuid {
            for characteristic in service.characteristics ?? [] where characteristic.uuid == charUuid {
                return characteristic
            }
        }
        return nil
    }

    private func execute(job: GattJob, on peripheral: CBPeripheral,
                         characteristic: CBCharacteristic) {
        switch job.op {
        case "read":
            peripheral.readValue(for: characteristic)
        case "write":
            guard let value = job.value else {
                completeGattJob(id: job.id) { self.reply(.err(id: job.id, reason: "bad-value-hex")) }
                return
            }
            peripheral.writeValue(value, for: characteristic, type: .withResponse)
        case "subscribe":
            peripheral.setNotifyValue(true, for: characteristic)
        default:
            completeGattJob(id: job.id) { self.reply(.err(id: job.id, reason: "bad-op")) }
        }
    }

    /// 取出並結束一個 GATT job(取消 watchdog),再執行回覆。
    private func completeGattJob(id: String, thenReply replyBlock: () -> Void) {
        if let index = gattJobs.firstIndex(where: { $0.id == id }) {
            let job = gattJobs.remove(at: index)
            job.timeoutTask?.cancel()
        }
        replyBlock()
    }

    private func pendingGattJobs(for peripheralId: UUID) -> [GattJob] {
        gattJobs.filter { $0.peripheralId == peripheralId }
    }

    // MARK: 工具

    /// CBUUID 前置驗證:CBUUID(string:) 對非法輸入會丟 ObjC 例外,必須先擋。
    static func makeCBUUID(_ text: String) -> CBUUID? {
        let isHexOnly = text.allSatisfy { $0.isHexDigit }
        if (text.count == 4 || text.count == 8) && isHexOnly {
            return CBUUID(string: text)
        }
        if text.count == 36, UUID(uuidString: text) != nil {
            return CBUUID(string: text)
        }
        return nil
    }

    private func reply(_ message: ClientMessage) {
        sendMessage?(message)
    }
}

// MARK: - CBCentralManagerDelegate

extension BleGateway: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        switch central.state {
        case .poweredOn:
            managerStateText = "藍牙可用"
        case .poweredOff:
            managerStateText = "藍牙已關閉"
        case .unauthorized:
            managerStateText = "藍牙權限被拒"
        case .unsupported:
            managerStateText = "此裝置不支援 BLE"
        case .resetting:
            managerStateText = "藍牙重置中"
        case .unknown:
            managerStateText = "藍牙狀態未知"
        @unknown default:
            managerStateText = "藍牙狀態未知"
        }
        // 藍牙變不可用 → 進行中的工作誠實失敗,不掛著等
        if central.state != .poweredOn {
            failAllInFlight(reason: central.state == .unauthorized ? "bluetooth-denied" : "bluetooth-off")
        }
        onStatusChanged?()
    }

    private func failAllInFlight(reason: String) {
        if let job = scanJob {
            job.timeoutTask?.cancel()
            reply(.err(id: job.id, reason: reason))
            scanJob = nil
        }
        for (_, job) in connectJobs {
            job.timeoutTask?.cancel()
            reply(.err(id: job.id, reason: reason))
        }
        connectJobs.removeAll()
        for job in gattJobs {
            job.timeoutTask?.cancel()
            reply(.err(id: job.id, reason: reason))
        }
        gattJobs.removeAll()
        for (_, subscriptionId) in subscriptions {
            reply(.err(id: subscriptionId, reason: reason))
        }
        subscriptions.removeAll()
        connectedPeripherals.removeAll()
    }

    func centralManager(_ central: CBCentralManager,
                        didDiscover peripheral: CBPeripheral,
                        advertisementData: [String: Any],
                        rssi RSSI: NSNumber) {
        knownPeripherals[peripheral.identifier] = peripheral
        guard scanJob != nil else { return }
        scanJob?.devices[peripheral.identifier] = BleDeviceInfo(
            id: peripheral.identifier.uuidString,
            name: peripheral.name,  // 未知就是 nil → wire 上為 null,不編造
            rssi: RSSI.intValue)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        peripheral.delegate = self
        connectedPeripherals[peripheral.identifier] = peripheral
        if let job = connectJobs.removeValue(forKey: peripheral.identifier) {
            job.timeoutTask?.cancel()
            reply(.ack(id: job.id, applied: ["connected": .bool(true)]))
        }
        lastEvent = "已連上 \(peripheral.name ?? peripheral.identifier.uuidString)"
    }

    func centralManager(_ central: CBCentralManager,
                        didFailToConnect peripheral: CBPeripheral,
                        error: Error?) {
        if let job = connectJobs.removeValue(forKey: peripheral.identifier) {
            job.timeoutTask?.cancel()
            let detail = error?.localizedDescription ?? "unknown"
            reply(.err(id: job.id, reason: "connect-failed:\(detail)"))
        }
    }

    func centralManager(_ central: CBCentralManager,
                        didDisconnectPeripheral peripheral: CBPeripheral,
                        error: Error?) {
        let peripheralId = peripheral.identifier
        connectedPeripherals.removeValue(forKey: peripheralId)
        // 該裝置的待處理 GATT job 誠實失敗
        for job in pendingGattJobs(for: peripheralId) {
            completeGattJob(id: job.id) {
                reply(.err(id: job.id, reason: "disconnected"))
            }
        }
        // 該裝置的訂閱全部失效,以 err 告知(沿用訂閱 id)
        let deadKeys = subscriptions.keys.filter { $0.peripheralId == peripheralId }
        for key in deadKeys {
            if let subscriptionId = subscriptions.removeValue(forKey: key) {
                reply(.err(id: subscriptionId, reason: "disconnected"))
            }
        }
        lastEvent = "裝置已斷線 \(peripheralId.uuidString)"
    }
}

// MARK: - CBPeripheralDelegate

extension BleGateway: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        let peripheralId = peripheral.identifier
        if error != nil {
            for job in pendingGattJobs(for: peripheralId) {
                completeGattJob(id: job.id) {
                    reply(.err(id: job.id, reason: "service-discovery-failed"))
                }
            }
            return
        }
        for job in pendingGattJobs(for: peripheralId) {
            if let service = peripheral.services?.first(where: { $0.uuid == job.serviceUuid }) {
                peripheral.discoverCharacteristics([job.charUuid], for: service)
            } else {
                completeGattJob(id: job.id) {
                    reply(.err(id: job.id, reason: "service-not-found"))
                }
            }
        }
    }

    func peripheral(_ peripheral: CBPeripheral,
                    didDiscoverCharacteristicsFor service: CBService,
                    error: Error?) {
        let peripheralId = peripheral.identifier
        let jobs = pendingGattJobs(for: peripheralId).filter { $0.serviceUuid == service.uuid }
        if error != nil {
            for job in jobs {
                completeGattJob(id: job.id) {
                    reply(.err(id: job.id, reason: "characteristic-discovery-failed"))
                }
            }
            return
        }
        for job in jobs {
            if let characteristic = service.characteristics?.first(where: { $0.uuid == job.charUuid }) {
                execute(job: job, on: peripheral, characteristic: characteristic)
            } else {
                completeGattJob(id: job.id) {
                    reply(.err(id: job.id, reason: "characteristic-not-found"))
                }
            }
        }
    }

    func peripheral(_ peripheral: CBPeripheral,
                    didUpdateValueFor characteristic: CBCharacteristic,
                    error: Error?) {
        let peripheralId = peripheral.identifier
        let charText = characteristic.uuid.uuidString

        // 1) 待處理的 read job 優先吃掉這筆值
        if let index = gattJobs.firstIndex(where: {
            $0.peripheralId == peripheralId && $0.charUuid == characteristic.uuid && $0.op == "read"
        }) {
            let job = gattJobs.remove(at: index)
            job.timeoutTask?.cancel()
            if error != nil {
                reply(.err(id: job.id, reason: "read-failed"))
            } else {
                let hexText = Hex.encode(characteristic.value ?? Data())
                reply(.bleValue(id: job.id, charUuid: charText, valueHex: hexText))
            }
            return
        }
        // 2) 訂閱通知:沿用 subscribe 請求 id
        let key = SubscriptionKey(peripheralId: peripheralId, charUuid: characteristic.uuid)
        if let subscriptionId = subscriptions[key] {
            guard error == nil else { return }  // 通知讀取錯誤:不送假值
            let hexText = Hex.encode(characteristic.value ?? Data())
            reply(.bleValue(id: subscriptionId, charUuid: charText, valueHex: hexText))
        }
    }

    func peripheral(_ peripheral: CBPeripheral,
                    didWriteValueFor characteristic: CBCharacteristic,
                    error: Error?) {
        let peripheralId = peripheral.identifier
        guard let index = gattJobs.firstIndex(where: {
            $0.peripheralId == peripheralId && $0.charUuid == characteristic.uuid && $0.op == "write"
        }) else { return }
        let job = gattJobs.remove(at: index)
        job.timeoutTask?.cancel()
        if let error {
            reply(.err(id: job.id, reason: "write-failed:\(error.localizedDescription)"))
        } else {
            // .withResponse 已回應 → 確實寫入
            reply(.ack(id: job.id, applied: ["written": .bool(true)]))
        }
    }

    func peripheral(_ peripheral: CBPeripheral,
                    didUpdateNotificationStateFor characteristic: CBCharacteristic,
                    error: Error?) {
        let peripheralId = peripheral.identifier
        guard let index = gattJobs.firstIndex(where: {
            $0.peripheralId == peripheralId && $0.charUuid == characteristic.uuid && $0.op == "subscribe"
        }) else { return }
        let job = gattJobs.remove(at: index)
        job.timeoutTask?.cancel()
        if error != nil || !characteristic.isNotifying {
            reply(.err(id: job.id, reason: "subscribe-failed"))
        } else {
            let key = SubscriptionKey(peripheralId: peripheralId, charUuid: characteristic.uuid)
            subscriptions[key] = job.id
            reply(.ack(id: job.id, applied: ["subscribed": .bool(true)]))
        }
    }
}
