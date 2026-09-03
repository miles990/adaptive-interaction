//
//  Protocol.swift
//  InteractionCompanion
//
//  Wire protocol v1 — 與桌面端 Rust server 完全一致的訊息模型。
//  傳輸:TLS 上的 WebSocket,每個 text frame 一則 JSON 訊息。
//
//  誠實不變量(對應 repo CLAUDE.md):
//  - queued ≠ completed;acknowledged ≠ completed;completed ≠ verified。
//  - 結果未知一律回報 err / uncertain,不得謊稱成功。
//

import Foundation

// MARK: - 任意 JSON 值(facts / params / applied 等異質欄位)

enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "unsupported JSON value")
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .number(let value):
            // 整數值以整數輸出(count: 2 而非 2.0),與伺服器端 serde 行為一致
            if value.rounded() == value && abs(value) < 1e15 {
                try container.encode(Int64(value))
            } else {
                try container.encode(value)
            }
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }

    var stringValue: String? {
        if case .string(let value) = self { return value }
        return nil
    }

    var doubleValue: Double? {
        if case .number(let value) = self { return value }
        return nil
    }

    var intValue: Int? {
        if case .number(let value) = self, value.rounded() == value { return Int(value) }
        return nil
    }

    var boolValue: Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }
}

extension Dictionary where Key == String, Value == JSONValue {
    func string(_ key: String) -> String? { self[key]?.stringValue }
    func int(_ key: String) -> Int? { self[key]?.intValue }
    func double(_ key: String) -> Double? { self[key]?.doubleValue }
    func bool(_ key: String) -> Bool? { self[key]?.boolValue }
}

// MARK: - 十六進位工具

enum Hex {
    static func encode(_ data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined()
    }

    static func decode(_ string: String) -> Data? {
        let cleaned = string.lowercased()
        guard cleaned.count % 2 == 0 else { return nil }
        var data = Data(capacity: cleaned.count / 2)
        var index = cleaned.startIndex
        while index < cleaned.endIndex {
            let next = cleaned.index(index, offsetBy: 2)
            guard let byte = UInt8(cleaned[index..<next], radix: 16) else { return nil }
            data.append(byte)
            index = next
        }
        return data
    }
}

// MARK: - 時間戳(ISO8601)

enum WireTime {
    private static let formatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    static func nowISO8601(_ date: Date = Date()) -> String {
        formatter.string(from: date)
    }
}

// MARK: - 配對 payload(QR 掃描或手動貼上的 JSON 字串)
// {"v":1,"host":"192.168.x.x","port":18790,"fp":"<64-hex sha256 of cert DER>","code":"123456"}

struct PairingPayload: Codable, Equatable {
    let v: Int
    let host: String
    let port: Int
    /// 伺服器自簽憑證 DER 的 SHA-256,64 位十六進位(小寫正規化)
    let fp: String
    /// 配對碼:僅在配對握手期間存在於記憶體,絕不寫入 Keychain 或磁碟
    let code: String

    enum ParseError: LocalizedError, Equatable {
        case invalidJSON
        case unsupportedVersion(Int)
        case invalidFingerprint
        case invalidHostOrPort
        case missingCode

        var errorDescription: String? {
            switch self {
            case .invalidJSON:
                return "配對內容不是有效的 JSON(需含 v/host/port/fp/code)"
            case .unsupportedVersion(let version):
                return "不支援的配對版本 v\(version)(本 App 僅支援 v1)"
            case .invalidFingerprint:
                return "憑證指紋格式錯誤(需 64 位十六進位 SHA-256)"
            case .invalidHostOrPort:
                return "主機或連接埠無效"
            case .missingCode:
                return "缺少配對碼"
            }
        }
    }

    /// 解析並驗證配對 payload;`codeOverride` 供手動輸入配對碼使用。
    static func parse(_ text: String, codeOverride: String? = nil) -> Result<PairingPayload, ParseError> {
        struct RawPayload: Decodable {
            let v: Int
            let host: String
            let port: Int
            let fp: String
            let code: String?
        }
        guard let data = text.data(using: .utf8),
              let raw = try? JSONDecoder().decode(RawPayload.self, from: data) else {
            return .failure(.invalidJSON)
        }
        guard raw.v == 1 else { return .failure(.unsupportedVersion(raw.v)) }
        let fingerprint = raw.fp.lowercased()
        guard fingerprint.count == 64, fingerprint.allSatisfy({ $0.isHexDigit }) else {
            return .failure(.invalidFingerprint)
        }
        guard !raw.host.isEmpty, (1...65535).contains(raw.port) else {
            return .failure(.invalidHostOrPort)
        }
        let override = codeOverride?.trimmingCharacters(in: .whitespacesAndNewlines)
        let code = (override?.isEmpty == false ? override : raw.code) ?? ""
        guard !code.isEmpty else { return .failure(.missingCode) }
        return .success(PairingPayload(v: raw.v, host: raw.host, port: raw.port, fp: fingerprint, code: code))
    }
}

// MARK: - status 訊息子結構

/// 感測開關快照。鍵名與 wire protocol 完全一致:
/// {"motion":bool,"battery":bool,"micLevel":bool,"location":bool,"bleGateway":bool}
struct SensorFlags: Codable, Equatable {
    var motion = false
    var battery = false
    var micLevel = false
    var location = false
    var bleGateway = false

    var anyActive: Bool { motion || battery || micLevel || location || bleGateway }
}

enum PermissionState: String, Codable, Equatable {
    case granted
    case denied
    case notDetermined

    /// UI 顯示用(誠實呈現,不美化)
    var displayText: String {
        switch self {
        case .granted: return "已授權"
        case .denied: return "已拒絕"
        case .notDetermined: return "未詢問"
        }
    }
}

/// {"microphone":"granted|denied|notDetermined","location":"...","bluetooth":"..."}
struct PermissionStates: Codable, Equatable {
    var microphone: PermissionState = .notDetermined
    var location: PermissionState = .notDetermined
    var bluetooth: PermissionState = .notDetermined
}

/// ble.result 內的裝置項:{"id":"<peripheral uuid>","name":"...","rssi":-60}
/// name 未知時誠實送 null,不編造名稱。
struct BleDeviceInfo: Equatable {
    let id: String
    let name: String?
    let rssi: Int
}

// MARK: - App → Server 訊息

enum ProtocolError: LocalizedError {
    case encodingFailed
    case decodingFailed(String)

    var errorDescription: String? {
        switch self {
        case .encodingFailed:
            return "訊息編碼失敗"
        case .decodingFailed(let detail):
            return "訊息解碼失敗:\(detail)"
        }
    }
}

enum ClientMessage {
    /// {"type":"pair-request","deviceName":"...","model":"<utsname machine>"}
    case pairRequest(deviceName: String, model: String)
    /// {"type":"pair-response","hmac":"<hex HMAC-SHA256(key: pairing code utf8, msg: nonce utf8)>"}
    case pairResponse(hmac: String)
    /// {"type":"auth","deviceId":"...","token":"..."}
    case auth(deviceId: String, token: String)
    /// {"type":"status","sensors":{...},"permissions":{...}} — 每次變更 + 每 30 秒
    case status(sensors: SensorFlags, permissions: PermissionStates)
    /// {"type":"observation","receptor":"...","facts":{...}}
    /// 依協議,僅 iphone.motion 帶 "at"(ISO8601);其他 receptor 不帶。
    case observation(receptor: String, facts: [String: JSONValue], at: String?)
    /// {"type":"ack","id":"...","applied":{...}}
    case ack(id: String, applied: [String: JSONValue])
    /// {"type":"ack","stopAll":true,"sensors":bool} — 回應 stop-all。
    /// `sensors` 回音請求裡的旗標:桌面端才能誠實區分
    /// 「只停動器」與「連感測一起停」,不必猜。
    case ackStopAll(sensors: Bool)
    /// {"type":"err","id":"...","reason":"..."}
    case err(id: String, reason: String)
    /// {"type":"ble.result","id":"...","devices":[...]}
    case bleResult(id: String, devices: [BleDeviceInfo])
    /// {"type":"ble.value","id":"...","charUuid":"...","valueHex":"..."}
    case bleValue(id: String, charUuid: String, valueHex: String)

    private var jsonObject: [String: JSONValue] {
        switch self {
        case .pairRequest(let deviceName, let model):
            return [
                "type": .string("pair-request"),
                "deviceName": .string(deviceName),
                "model": .string(model),
            ]
        case .pairResponse(let hmac):
            return [
                "type": .string("pair-response"),
                "hmac": .string(hmac),
            ]
        case .auth(let deviceId, let token):
            return [
                "type": .string("auth"),
                "deviceId": .string(deviceId),
                "token": .string(token),
            ]
        case .status(let sensors, let permissions):
            return [
                "type": .string("status"),
                "sensors": .object([
                    "motion": .bool(sensors.motion),
                    "battery": .bool(sensors.battery),
                    "micLevel": .bool(sensors.micLevel),
                    "location": .bool(sensors.location),
                    "bleGateway": .bool(sensors.bleGateway),
                ]),
                "permissions": .object([
                    "microphone": .string(permissions.microphone.rawValue),
                    "location": .string(permissions.location.rawValue),
                    "bluetooth": .string(permissions.bluetooth.rawValue),
                ]),
            ]
        case .observation(let receptor, let facts, let at):
            var object: [String: JSONValue] = [
                "type": .string("observation"),
                "receptor": .string(receptor),
                "facts": .object(facts),
            ]
            if let at {
                object["at"] = .string(at)
            }
            return object
        case .ack(let id, let applied):
            return [
                "type": .string("ack"),
                "id": .string(id),
                "applied": .object(applied),
            ]
        case .ackStopAll(let sensors):
            return [
                "type": .string("ack"),
                "stopAll": .bool(true),
                "sensors": .bool(sensors),
            ]
        case .err(let id, let reason):
            return [
                "type": .string("err"),
                "id": .string(id),
                "reason": .string(reason),
            ]
        case .bleResult(let id, let devices):
            return [
                "type": .string("ble.result"),
                "id": .string(id),
                "devices": .array(devices.map { device in
                    JSONValue.object([
                        "id": .string(device.id),
                        "name": device.name.map { JSONValue.string($0) } ?? .null,
                        "rssi": .number(Double(device.rssi)),
                    ])
                }),
            ]
        case .bleValue(let id, let charUuid, let valueHex):
            return [
                "type": .string("ble.value"),
                "id": .string(id),
                "charUuid": .string(charUuid),
                "valueHex": .string(valueHex),
            ]
        }
    }

    /// 編碼為單一 WebSocket text frame 的 JSON 字串。
    func encodeToJSONString() throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let data = try encoder.encode(JSONValue.object(jsonObject))
        guard let text = String(data: data, encoding: .utf8) else {
            throw ProtocolError.encodingFailed
        }
        return text
    }

    /// 是否屬於配對/認證握手訊息(未進入 connected 狀態也允許送出)
    var isHandshake: Bool {
        switch self {
        case .pairRequest, .pairResponse, .auth:
            return true
        default:
            return false
        }
    }
}

// MARK: - Server → App 訊息

/// `stop-all` 的原因。**只影響 UI 誠實顯示的停用說明,不影響停的範圍**——
/// 兩種原因停的東西完全一樣(動器,加上 `sensors:true` 時的感測),
/// 差別只在使用者看到「由桌面停止全部感測」還是「因桌面緊急停止而停用」。
///
/// 誠實預設:欄位缺席(舊桌面端)或值無法辨識時一律當成 `.emergency`——
/// 寧可把使用者的動作說成緊急停止(較嚴格、較顯眼),
/// 也不要把真正的緊急停止淡化成一般停止。
enum StopAllReason: String, Equatable {
    /// 使用者在桌面按了「停止所有感測」。
    case user
    /// 桌面緊急停止(emergency stop)。
    case emergency

    /// 由 wire 字串建構;`nil` 或未知值 → `.emergency`。
    init(wire: String?) {
        switch wire {
        case StopAllReason.user.rawValue: self = .user
        default: self = .emergency
        }
    }
}

enum ServerMessage: Equatable {
    case pairChallenge(nonce: String)
    case paired(deviceId: String, deviceToken: String)
    case pairFail(reason: String)
    case authOk
    case authFail(reason: String)
    /// {"type":"act","id":"...","name":"...","params":{...}}
    case act(id: String, name: String, params: [String: JSONValue])
    /// {"type":"stop-all","sensors":bool,"reason":"user"|"emergency"} —
    /// `sensors:true` 時連感測一起停(麥克風 / 位置 / BLE 閘道),不只是停動器。
    /// 舊桌面端不帶 `sensors` → 預設 false(只停動器),不擅自關掉使用者的感測。
    /// `reason` 只影響 UI 誠實顯示的停用說明,不影響停的範圍;
    /// 缺席或無法辨識時一律當成 `.emergency`(向後相容,且取較嚴格的那一邊)。
    case stopAll(sensors: Bool, reason: StopAllReason)
    /// {"type":"ble.scan","id":"...","serviceUuid":"<uuid|null>","durationMs":≤8000}
    case bleScan(id: String, serviceUuid: String?, durationMs: Int)
    /// {"type":"ble.connect","id","peripheralId"}
    case bleConnect(id: String, peripheralId: String)
    /// {"type":"ble.gatt","id","peripheralId","op":"read|write|subscribe","serviceUuid","charUuid","valueHex"?}
    case bleGatt(id: String, peripheralId: String, op: String,
                 serviceUuid: String, charUuid: String, valueHex: String?)
    /// 未知型別:保留原字串供記錄,不假裝已處理
    case unknown(type: String)

    static func decode(_ text: String) throws -> ServerMessage {
        struct TypeProbe: Decodable { let type: String }
        struct PairChallengeBody: Decodable { let nonce: String }
        struct PairedBody: Decodable { let deviceId: String; let deviceToken: String }
        struct FailBody: Decodable { let reason: String? }
        struct ActBody: Decodable {
            let id: String
            let name: String
            let params: [String: JSONValue]?
        }
        struct BleScanBody: Decodable {
            let id: String
            let serviceUuid: String?
            let durationMs: Int?
        }
        struct StopAllBody: Decodable { let sensors: Bool?; let reason: String? }
        struct BleConnectBody: Decodable { let id: String; let peripheralId: String }
        struct BleGattBody: Decodable {
            let id: String
            let peripheralId: String
            let op: String
            let serviceUuid: String
            let charUuid: String
            let valueHex: String?
        }

        guard let data = text.data(using: .utf8) else {
            throw ProtocolError.decodingFailed("非 UTF-8 內容")
        }
        let decoder = JSONDecoder()
        let probe: TypeProbe
        do {
            probe = try decoder.decode(TypeProbe.self, from: data)
        } catch {
            throw ProtocolError.decodingFailed("缺少 type 欄位")
        }
        do {
            switch probe.type {
            case "pair-challenge":
                let body = try decoder.decode(PairChallengeBody.self, from: data)
                return .pairChallenge(nonce: body.nonce)
            case "paired":
                let body = try decoder.decode(PairedBody.self, from: data)
                return .paired(deviceId: body.deviceId, deviceToken: body.deviceToken)
            case "pair-fail":
                let body = try decoder.decode(FailBody.self, from: data)
                return .pairFail(reason: body.reason ?? "未知原因")
            case "auth-ok":
                return .authOk
            case "auth-fail":
                let body = try decoder.decode(FailBody.self, from: data)
                return .authFail(reason: body.reason ?? "未知原因")
            case "act":
                let body = try decoder.decode(ActBody.self, from: data)
                return .act(id: body.id, name: body.name, params: body.params ?? [:])
            case "stop-all":
                let body = try? decoder.decode(StopAllBody.self, from: data)
                return .stopAll(sensors: body?.sensors ?? false,
                                reason: StopAllReason(wire: body?.reason))
            case "ble.scan":
                let body = try decoder.decode(BleScanBody.self, from: data)
                return .bleScan(id: body.id, serviceUuid: body.serviceUuid,
                                durationMs: body.durationMs ?? 0)
            case "ble.connect":
                let body = try decoder.decode(BleConnectBody.self, from: data)
                return .bleConnect(id: body.id, peripheralId: body.peripheralId)
            case "ble.gatt":
                let body = try decoder.decode(BleGattBody.self, from: data)
                return .bleGatt(id: body.id, peripheralId: body.peripheralId, op: body.op,
                                serviceUuid: body.serviceUuid, charUuid: body.charUuid,
                                valueHex: body.valueHex)
            default:
                return .unknown(type: probe.type)
            }
        } catch let error as ProtocolError {
            throw error
        } catch {
            throw ProtocolError.decodingFailed("type=\(probe.type) 欄位不完整:\(error.localizedDescription)")
        }
    }
}
