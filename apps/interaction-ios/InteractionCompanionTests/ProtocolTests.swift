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

    func testAckStopAllEchoesSensorsFlag() throws {
        let withSensors = try jsonObject(
            try ClientMessage.ackStopAll(sensors: true).encodeToJSONString())
        XCTAssertEqual(withSensors["type"] as? String, "ack")
        XCTAssertEqual(withSensors["stopAll"] as? Bool, true)
        XCTAssertEqual(withSensors["sensors"] as? Bool, true)
        XCTAssertNil(withSensors["id"])
        XCTAssertEqual(withSensors.count, 3)

        // 只停動器時 sensors 必須是 false,不能省略也不能謊報 true。
        let actuatorsOnly = try jsonObject(
            try ClientMessage.ackStopAll(sensors: false).encodeToJSONString())
        XCTAssertEqual(actuatorsOnly["stopAll"] as? Bool, true)
        XCTAssertEqual(actuatorsOnly["sensors"] as? Bool, false)
        XCTAssertEqual(actuatorsOnly.count, 3)
    }

    /// `reason` 缺席(舊桌面端)或無法辨識時,一律當成 emergency——
    /// 寧可把一般停止說成緊急停止,也不要把緊急停止淡化成一般停止。
    func testStopAllReasonDefaultsToEmergency() throws {
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"stop-all","sensors":true}"#),
                       .stopAll(sensors: true, reason: .emergency))
        XCTAssertEqual(
            try ServerMessage.decode(#"{"type":"stop-all","sensors":true,"reason":"emergency"}"#),
            .stopAll(sensors: true, reason: .emergency))
        // 未知值不得被當成 user。
        XCTAssertEqual(
            try ServerMessage.decode(#"{"type":"stop-all","sensors":true,"reason":"whatever"}"#),
            .stopAll(sensors: true, reason: .emergency))
        XCTAssertEqual(StopAllReason(wire: nil), .emergency)
        XCTAssertEqual(StopAllReason(wire: "USER"), .emergency)
    }

    /// 使用者在桌面按「停止所有感測」→ reason=user,App 才能顯示
    /// 「由桌面停止全部感測」而不是「因桌面緊急停止而停用」。
    func testStopAllUserReasonIsDecoded() throws {
        XCTAssertEqual(
            try ServerMessage.decode(#"{"type":"stop-all","sensors":true,"reason":"user"}"#),
            .stopAll(sensors: true, reason: .user))
        XCTAssertEqual(StopAllReason(wire: "user"), .user)
    }

    /// Belt-and-braces:桌面緊急停止時另外送的 `character.present emergency`
    /// 可能送不到(runtime 誠實記成 outcome=unknown)。
    /// `stop-all{sensors:true,reason:emergency}` 本身就是緊急停止的事實,
    /// 所以 App 收到它就要把角色狀態切成 emergency。
    @MainActor
    func testEmergencyStopAllSetsTheCharacterStateEvenIfCharacterPresentIsLost() async {
        let state = CharacterState()
        let center = ActuatorCenter(characterState: state)
        var note: String?
        center.stopSensorsOnStopAll = { note = $0 }

        await center.stopAll(sensors: true, reason: .emergency)

        XCTAssertEqual(state.state, .emergency)
        XCTAssertEqual(note, "因桌面緊急停止而停用(麥克風/位置/BLE 閘道)")
    }

    /// 使用者按的「停止所有感測」不是緊急停止:角色狀態不得被改成 emergency
    /// (把一般停止顯示成緊急停止同樣是說謊)。
    @MainActor
    func testUserStopAllDoesNotFakeAnEmergencyCharacterState() async {
        let state = CharacterState()
        state.state = .working
        let center = ActuatorCenter(characterState: state)
        var note: String?
        center.stopSensorsOnStopAll = { note = $0 }

        await center.stopAll(sensors: true, reason: .user)

        XCTAssertEqual(state.state, .working)
        XCTAssertEqual(note, "由桌面停止全部感測(麥克風/位置/BLE 閘道)")
    }

    /// 只停動器(sensors:false)時什麼感測都沒停,角色狀態也不該動。
    @MainActor
    func testActuatorOnlyStopAllTouchesNeitherSensorsNorCharacterState() async {
        let state = CharacterState()
        state.state = .idle
        let center = ActuatorCenter(characterState: state)
        var called = false
        center.stopSensorsOnStopAll = { _ in called = true }

        await center.stopAll(sensors: false, reason: .emergency)

        XCTAssertFalse(called, "sensors:false 不得擅自關掉使用者的感測")
        XCTAssertEqual(state.state, .idle)
    }

    /// 解除只由 runtime 決定:桌面送 `character.present idle` 之後才回到 idle,
    /// App 自己不會恢復(手機不能自稱緊急停止結束了)。
    @MainActor
    func testOnlyTheRuntimeClearsTheEmergencyCharacterState() async {
        let state = CharacterState()
        let center = ActuatorCenter(characterState: state)
        await center.stopAll(sensors: true, reason: .emergency)
        XCTAssertEqual(state.state, .emergency)

        // 再收一次 stop-all 不會自行恢復。
        await center.stopAll(sensors: true, reason: .emergency)
        XCTAssertEqual(state.state, .emergency)

        let reply = await center.handleAct(
            id: "p1", name: "character.present", params: ["state": .string("idle")])
        guard case .ack = reply else {
            return XCTFail("character.present idle 應該被接受")
        }
        XCTAssertEqual(state.state, .idle)
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
        // 舊桌面端不帶 sensors → 只停動器(不擅自關掉使用者的感測);
        // 也不帶 reason → 保守地當成 emergency。
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"stop-all"}"#),
                       .stopAll(sensors: false, reason: .emergency))
        // 連感測一起停。
        XCTAssertEqual(
            try ServerMessage.decode(#"{"type":"stop-all","sensors":true}"#),
            .stopAll(sensors: true, reason: .emergency))
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

    // MARK: - AIP frame(v0.6.0 Character Session)

    /// `{"type":"aip","envelope":…}` 的編碼:信封逐字輸出,外殼只有 type 與 envelope 兩個鍵。
    func testAipFrameEncodesTheEnvelopeVerbatim() throws {
        let envelope = SessionDecisions.touchEnvelope(
            kind: "tap", deviceId: "iphone-87b42264", sessionId: "session.home",
            messageId: "ios-touch-1", now: Date())
        let text = try ClientMessage.aip(envelope).encodeToJSONString()
        let object = try jsonObject(text)
        XCTAssertEqual(object["type"] as? String, "aip")
        XCTAssertEqual(object.count, 2)
        let body = try XCTUnwrap(object["envelope"] as? [String: Any])
        XCTAssertEqual(body["messageType"] as? String, "event")
        XCTAssertEqual(body["name"] as? String, "character.interaction.touch")
        XCTAssertEqual(body["sessionId"] as? String, "session.home")
        XCTAssertNotNil(body["expiresAt"])
        let source = try XCTUnwrap(body["source"] as? [String: Any])
        XCTAssertEqual(source["kind"] as? String, "device")
        XCTAssertEqual(source["id"] as? String, "iphone-87b42264")
    }

    /// 解碼後仍是同一個信封(round-trip 不遺失,含未知的頂層選填欄位)。
    func testAipFrameDecodesBackToTheSameEnvelopeAndKeepsUnknownFields() throws {
        let frame = """
            {"type":"aip","envelope":{"specVersion":"aip/1.0","messageId":"aip-1-1",\
            "messageType":"state","name":"character.session.patch",\
            "source":{"kind":"runtime","id":"runtime"},"sessionId":"session.home",\
            "occurredAt":"2026-09-04T12:30:03Z","sequence":206,"baseRevision":204,\
            "futureField":{"keep":true},\
            "payload":{"kind":"patch","revision":205,"hash":"abc","patch":{"activity":"reacting"}}}}
            """
        guard case .aip(let envelope) = try ServerMessage.decode(frame) else {
            return XCTFail("type=aip 必須解碼成 .aip")
        }
        XCTAssertEqual(envelope.messageType, .state)
        XCTAssertEqual(envelope.name, "character.session.patch")
        XCTAssertEqual(envelope.sequence, 206)
        XCTAssertEqual(envelope.baseRevision, 204)
        XCTAssertEqual(envelope.extra["futureField"], .object(["keep": .bool(true)]))
        XCTAssertNil(envelope.validate())
    }

    /// 壞掉的 aip frame 誠實丟錯,而且錯誤訊息不回顯輸入內容(AIP §5)。
    func testABrokenAipFrameFailsWithoutEchoingTheInput() {
        let frame = #"{"type":"aip","envelope":{"messageId":"secret-token-value"}}"#
        XCTAssertThrowsError(try ServerMessage.decode(frame)) { error in
            let text = (error as? ProtocolError)?.errorDescription ?? "\(error)"
            XCTAssertFalse(text.contains("secret-token-value"), "錯誤訊息不得回顯輸入")
        }
    }

    /// 舊 App 的既有行為不變:未知 type 仍然是 `.unknown`,不假裝處理。
    func testUnknownServerMessageTypeIsStillUnknown() throws {
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"aip-v2","x":1}"#),
                       .unknown(type: "aip-v2"))
        XCTAssertEqual(try ServerMessage.decode(#"{"type":"character.future"}"#),
                       .unknown(type: "character.future"))
    }
}
