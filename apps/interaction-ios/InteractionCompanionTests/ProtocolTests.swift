//
//  ProtocolTests.swift
//  InteractionCompanionTests
//
//  Wire protocol v1 編碼/解碼測試。
//  這些案例已於交付時在 macOS 上實際執行通過(含 HMAC 與 openssl 交叉驗證,
//  見 apps/interaction-ios/README.md);在 Xcode 中請加入 Unit Test target 執行。
//

import XCTest
import CryptoKit
@testable import InteractionCompanion

final class ProtocolTests: XCTestCase {
    private func jsonObject(_ text: String) throws -> [String: Any] {
        let data = try XCTUnwrap(text.data(using: .utf8))
        return try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    func testPairRequestShape() throws {
        let text = try ClientMessage.pairRequest(deviceName: "小明的 iPhone",
                                                 model: "iPhone15,3").encodeToJSONString()
        let object = try jsonObject(text)
        XCTAssertEqual(object["type"] as? String, "pair-request")
        XCTAssertEqual(object["deviceName"] as? String, "小明的 iPhone")
        XCTAssertEqual(object["model"] as? String, "iPhone15,3")
        XCTAssertEqual(object.count, 3)
    }

    func testStatusShapeHasExactSensorAndPermissionKeys() throws {
        var flags = SensorFlags()
        flags.micLevel = true
        flags.bleGateway = true
        var permissions = PermissionStates()
        permissions.microphone = .granted
        permissions.bluetooth = .denied
        let text = try ClientMessage.status(sensors: flags,
                                            permissions: permissions).encodeToJSONString()
        let object = try jsonObject(text)
        let sensors = try XCTUnwrap(object["sensors"] as? [String: Any])
        let perms = try XCTUnwrap(object["permissions"] as? [String: Any])
        XCTAssertEqual(sensors["motion"] as? Bool, false)
        XCTAssertEqual(sensors["battery"] as? Bool, false)
        XCTAssertEqual(sensors["micLevel"] as? Bool, true)
        XCTAssertEqual(sensors["location"] as? Bool, false)
        XCTAssertEqual(sensors["bleGateway"] as? Bool, true)
        XCTAssertEqual(sensors.count, 5)
        XCTAssertEqual(perms["microphone"] as? String, "granted")
        XCTAssertEqual(perms["location"] as? String, "notDetermined")
        XCTAssertEqual(perms["bluetooth"] as? String, "denied")
        XCTAssertEqual(perms.count, 3)
    }

    func testObservationMotionCarriesAtButBatteryDoesNot() throws {
        let motion = try ClientMessage.observation(
            receptor: "iphone.motion",
            facts: ["event": .string("lifted")],
            at: "2026-08-28T10:00:00.000Z").encodeToJSONString()
        let motionObject = try jsonObject(motion)
        XCTAssertEqual(motionObject["at"] as? String, "2026-08-28T10:00:00.000Z")

        let battery = try ClientMessage.observation(
            receptor: "iphone.battery",
            facts: ["level": .number(0.87), "charging": .bool(true), "foreground": .bool(true)],
            at: nil).encodeToJSONString()
        let batteryObject = try jsonObject(battery)
        XCTAssertNil(batteryObject["at"])
        let facts = try XCTUnwrap(batteryObject["facts"] as? [String: Any])
        XCTAssertEqual(facts["level"] as? Double, 0.87)
    }

    func testUnknownBatteryLevelIsNullNotFabricated() throws {
        let text = try ClientMessage.observation(
            receptor: "iphone.battery",
            facts: ["level": .null, "charging": .bool(false), "foreground": .bool(true)],
            at: nil).encodeToJSONString()
        let facts = try XCTUnwrap(try jsonObject(text)["facts"] as? [String: Any])
        XCTAssertTrue(facts["level"] is NSNull)
    }

    func testAckStopAllShape() throws {
        let text = try ClientMessage.ackStopAll.encodeToJSONString()
        let object = try jsonObject(text)
        XCTAssertEqual(object["type"] as? String, "ack")
        XCTAssertEqual(object["stopAll"] as? Bool, true)
        XCTAssertNil(object["id"])
        XCTAssertEqual(object.count, 2)
    }

    func testAppliedIntegersSerializeWithoutDecimalPoint() throws {
        let text = try ClientMessage.ack(
            id: "req-1",
            applied: ["style": .string("purr"), "count": .number(2)]).encodeToJSONString()
        XCTAssertTrue(text.contains("\"count\":2"))
        XCTAssertFalse(text.contains("2.0"))
    }

    func testBleResultUnknownNameIsNull() throws {
        let devices = [
            BleDeviceInfo(id: "9A2B7C1D-0000-0000-0000-000000000001", name: "MiBand", rssi: -60),
            BleDeviceInfo(id: "9A2B7C1D-0000-0000-0000-000000000002", name: nil, rssi: -80),
        ]
        let text = try ClientMessage.bleResult(id: "scan-1", devices: devices).encodeToJSONString()
        let list = try XCTUnwrap(try jsonObject(text)["devices"] as? [[String: Any]])
        XCTAssertEqual(list[0]["name"] as? String, "MiBand")
        XCTAssertEqual(list[0]["rssi"] as? Int, -60)
        XCTAssertTrue(list[1]["name"] is NSNull)
    }

    func testServerMessageDecoding() throws {
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"pair-challenge","nonce":"deadbeef"}"#),
                       .pairChallenge(nonce: "deadbeef"))
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"paired","deviceId":"iphone-a1b2c3d4","deviceToken":"ff00"}"#),
                       .paired(deviceId: "iphone-a1b2c3d4", deviceToken: "ff00"))
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"auth-ok"}"#), .authOk)
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"auth-fail","reason":"revoked"}"#),
                       .authFail(reason: "revoked"))
        // 舊桌面端不帶 sensors → 只停動器(不擅自關掉使用者的感測)。
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"stop-all"}"#), .stopAll(sensors: false))
        // 緊急停止:連感測一起停。
        XCTAssertEqual(
            try ServerMessage.decode(#"{"type":"stop-all","sensors":true}"#),
            .stopAll(sensors: true))
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"ble.scan","id":"s1","serviceUuid":null,"durationMs":5000}"#),
                       .bleScan(id: "s1", serviceUuid: nil, durationMs: 5000))
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"future-thing","x":1}"#),
                       .unknown(type: "future-thing"))

        let act = try ServerMessage.decode(#"{"type":"act","id":"a1","name":"haptic.pulse","params":{"style":"heartbeat","count":3}}"#)
        guard case .act(let id, let name, let params) = act else {
            XCTFail("應解碼為 act")
            return
        }
        XCTAssertEqual(id, "a1")
        XCTAssertEqual(name, "haptic.pulse")
        XCTAssertEqual(params.string("style"), "heartbeat")
        XCTAssertEqual(params.int("count"), 3)

        let gatt = try ServerMessage.decode(#"{"type":"ble.gatt","id":"g1","peripheralId":"9A2B7C1D-0000-0000-0000-000000000001","op":"write","serviceUuid":"180D","charUuid":"2A39","valueHex":"01"}"#)
        XCTAssertEqual(gatt, .bleGatt(id: "g1",
                                      peripheralId: "9A2B7C1D-0000-0000-0000-000000000001",
                                      op: "write", serviceUuid: "180D",
                                      charUuid: "2A39", valueHex: "01"))
    }

    func testPairingPayloadValidation() {
        let fp = String(repeating: "ab", count: 32)
        let good = PairingPayload.parse(
            "{\"v\":1,\"host\":\"192.168.1.20\",\"port\":18790,\"fp\":\"\(fp)\",\"code\":\"123456\"}")
        guard case .success(let payload) = good else {
            XCTFail("合法 payload 應解析成功")
            return
        }
        XCTAssertEqual(payload.host, "192.168.1.20")
        XCTAssertEqual(payload.port, 18790)
        XCTAssertEqual(payload.code, "123456")

        guard case .failure(.unsupportedVersion(2)) = PairingPayload.parse(
            #"{"v":2,"host":"h","port":1,"fp":"00","code":"1"}"#) else {
            XCTFail("v2 應被拒絕")
            return
        }
        guard case .failure(.invalidFingerprint) = PairingPayload.parse(
            #"{"v":1,"host":"h","port":1,"fp":"zz","code":"1"}"#) else {
            XCTFail("壞指紋應被拒絕")
            return
        }
    }

    func testPairingHmacMatchesOpensslReference() {
        // openssl 參考值:printf 'deadbeef' | openssl dgst -sha256 -hmac "123456"
        let key = SymmetricKey(data: Data("123456".utf8))
        let mac = HMAC<SHA256>.authenticationCode(for: Data("deadbeef".utf8), using: key)
        XCTAssertEqual(Hex.encode(Data(mac)),
                       "1c630de93f4c9c6d68d4ba3bb18607cb01fab690363da303ffe054d04a02b6f8")
    }

    func testHexRoundTrip() {
        XCTAssertEqual(Hex.encode(Data([0xDE, 0xAD, 0xBE, 0xEF])), "deadbeef")
        XCTAssertEqual(Hex.decode("DEADBEEF"), Data([0xDE, 0xAD, 0xBE, 0xEF]))
        XCTAssertNil(Hex.decode("abc"))
        XCTAssertNil(Hex.decode("zz"))
    }
}
