//
//  AIPConformanceTests.swift
//  InteractionCompanionTests
//
//  AIP 1.0 跨語言 conformance（Swift 端）。
//
//  讀的是 Rust crate 底下**同一份** fixture index（crates/interaction-aip/tests/fixtures/manifest.json），
//  經 scripts/aip-codegen.mjs 內嵌成 AIPFixtures。Rust、TypeScript、Swift 三個實作對同一組訊息
//  必須得到同一個結論；iPhone 端寬鬆一分，桌面的確定性檢查就漏一分。
//
//  契約：docs/aip/README.md §14；跑法：docs/aip/conformance.md。
//

import XCTest

@testable import InteractionCompanion

final class AIPConformanceTests: XCTestCase {

    // MARK: - 索引

    private func manifest() throws -> [String: Any] {
        let data = Data(AIPFixtures.manifest.utf8)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw XCTSkip("manifest.json is not an object")
        }
        return object
    }

    private func section(_ key: String) throws -> [[String: Any]] {
        guard let entries = try manifest()[key] as? [[String: Any]] else {
            XCTFail("manifest.json is missing the `\(key)` section")
            return []
        }
        return entries
    }

    private func fixture(_ name: String) throws -> Data {
        guard let text = AIPFixtures.files[name] else {
            XCTFail("missing embedded fixture \(name)")
            return Data()
        }
        return Data(text.utf8)
    }

    /// 依 manifest 的 `expect`／`code` 斷言結論一致，並確保錯誤訊息不回顯輸入。
    @discardableResult
    private func assertExpectation(
        _ id: String, _ entry: [String: Any], _ data: Data
    ) -> AIPEnvelope? {
        let outcome = AIPCheck.evaluate(data)
        let expect = entry["expect"] as? String
        switch (expect, outcome) {
        case ("ok", .success(let envelope)):
            return envelope
        case ("ok", .failure(let error)):
            XCTFail("fixture \(id) should be accepted but failed: \(error.code.rawValue)")
            return nil
        case ("error", .success):
            XCTFail("fixture \(id) should be rejected but passed validation")
            return nil
        case ("error", .failure(let error)):
            XCTAssertEqual(
                error.code.rawValue, entry["code"] as? String,
                "fixture \(id) produced the wrong ErrorCode")
            for token in (entry["mustNotEcho"] as? [String]) ?? [] {
                XCTAssertFalse(
                    error.message.contains(token),
                    "fixture \(id): error message echoes caller input")
            }
            for leak in ["/Users", "/private", "/home", ".json", ".swift", "\\", "://"] {
                XCTAssertFalse(
                    error.message.contains(leak),
                    "fixture \(id): error message leaks a path-like fragment")
            }
            XCTAssertLessThanOrEqual(error.message.count, 200)
            return nil
        default:
            XCTFail("fixture \(id) has an unknown expect value")
            return nil
        }
    }

    // MARK: - Envelope fixtures

    func testReadsTheSameIndexTheRustAndTypeScriptSuitesRead() throws {
        let index = try manifest()
        XCTAssertEqual(index["specVersion"] as? String, "aip/1.0")
        XCTAssertGreaterThanOrEqual(try section("envelopes").count, 20)
    }

    func testEnvelopeFixturesAgreeWithTheIndex() throws {
        var ids = Set<String>()
        for entry in try section("envelopes") {
            let id = entry["id"] as? String ?? "<missing id>"
            XCTAssertTrue(ids.insert(id).inserted, "duplicate fixture id \(id)")
            let file = entry["file"] as? String ?? ""
            assertExpectation(id, entry, try fixture(file))
        }
    }

    func testEveryMessageTypeHasAtLeastOneAcceptedFixture() throws {
        var covered = Set<String>()
        for entry in try section("envelopes") where (entry["expect"] as? String) == "ok" {
            let file = entry["file"] as? String ?? ""
            let object =
                try JSONSerialization.jsonObject(with: try fixture(file)) as? [String: Any] ?? [:]
            if let type = object["messageType"] as? String { covered.insert(type) }
        }
        for type in AIPConstants.messageTypes {
            XCTAssertTrue(covered.contains(type), "no accepted fixture covers \(type)")
        }
    }

    /// 超大訊息與壞 JSON 在測試內生成，不存大檔。
    func testGeneratedFixturesCoverTheOversizedAndMalformedCases() throws {
        for entry in try section("generated") {
            let id = entry["id"] as? String ?? "<missing id>"
            let data: Data
            if let raw = entry["raw"] as? String {
                data = Data(raw.utf8)
            } else {
                let base = entry["base"] as? String ?? ""
                let chars = entry["inflatePayloadChars"] as? Int ?? 0
                var object =
                    try JSONSerialization.jsonObject(with: try fixture(base)) as? [String: Any]
                    ?? [:]
                var payload = object["payload"] as? [String: Any] ?? [:]
                payload["blob"] = String(repeating: "x", count: chars)
                object["payload"] = payload
                data = try JSONSerialization.data(withJSONObject: object)
            }
            assertExpectation(id, entry, data)
        }
    }

    func testAcceptedFixturesRoundTripWithoutLosingUnknownFields() throws {
        for entry in try section("envelopes") where (entry["expect"] as? String) == "ok" {
            let id = entry["id"] as? String ?? "<missing id>"
            let file = entry["file"] as? String ?? ""
            let data = try fixture(file)
            guard case .success(let parsed) = AIPEnvelope.parse(data) else {
                XCTFail("fixture \(id) does not parse")
                continue
            }
            guard case .success(let encoded) = parsed.encode() else {
                XCTFail("fixture \(id) does not re-encode")
                continue
            }
            guard case .success(let reparsed) = AIPEnvelope.parse(encoded) else {
                XCTFail("fixture \(id) does not re-parse")
                continue
            }
            XCTAssertEqual(parsed, reparsed, "fixture \(id) is not round-trip stable")

            if entry["roundTrip"] as? Bool == true {
                XCTAssertFalse(
                    parsed.extra.isEmpty,
                    "fixture \(id) is flagged roundTrip but carries no unknown top-level field")
                let original =
                    try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
                for key in parsed.extra.keys {
                    XCTAssertNotNil(
                        original[key], "fixture \(id): invented an unknown field \(key)")
                    XCTAssertEqual(
                        parsed.extra[key], reparsed.extra[key],
                        "fixture \(id): unknown field \(key) changed across round-trip")
                }
            }
        }
    }

    // MARK: - 決策表

    func testIdentityDecisionTable() throws {
        for entry in try section("identity") {
            let id = entry["id"] as? String ?? "<missing id>"
            guard let bound = party(entry["bound"]), let claimed = party(entry["claimed"]) else {
                XCTFail("identity \(id) has a malformed party")
                continue
            }
            let decision = aipBindIdentity(bound: bound, claimed: claimed)
            XCTAssertEqual(
                decision.isAccept, (entry["expect"] as? String) == "accept",
                "identity \(id): a mismatched claim must be rejected, never normalised")
        }
    }

    func testOfflinePolicyTable() throws {
        for entry in try section("offlinePolicy") {
            let name = entry["name"] as? String ?? ""
            let grant = entry["hasConsentGrant"] as? Bool ?? false
            XCTAssertEqual(
                aipOfflinePolicy(name, hasConsentGrant: grant).rawValue,
                entry["expect"] as? String,
                "offline policy for \(name)")
        }
    }

    func testOutcomeMigrationTable() throws {
        for entry in try section("outcomeTransitions") {
            guard let from = AIPOutcome(rawValue: entry["from"] as? String ?? ""),
                let to = AIPOutcome(rawValue: entry["to"] as? String ?? "")
            else {
                XCTFail("outcome transition names an unknown Outcome")
                continue
            }
            XCTAssertEqual(
                from.canTransition(to: to), entry["allowed"] as? Bool,
                "transition \(from.rawValue) -> \(to.rawValue) disagrees with the index")
        }
        for entry in try section("outcomeProfiles") {
            guard let status = AIPOutcome(rawValue: entry["status"] as? String ?? "") else {
                XCTFail("outcome profile names an unknown Outcome")
                continue
            }
            XCTAssertEqual(
                status.isAllowed(forProfile: entry["profile"] as? String ?? ""),
                entry["allowed"] as? Bool,
                "outcome \(status.rawValue) in profile")
        }
        XCTAssertTrue(AIPOutcome.verified.isRuntimeOnly, "verified must stay runtime-only")
        XCTAssertFalse(AIPOutcome.observed.canTransition(to: .verified))
        XCTAssertFalse(AIPOutcome.acknowledged.canTransition(to: .verified))
    }

    func testNameScopeTable() throws {
        for entry in try section("nameScope") {
            let name = entry["name"] as? String ?? ""
            XCTAssertEqual(
                AIPName.isRuntimeOnly(name), entry["runtimeOnly"] as? Bool, "name scope for \(name)")
        }
    }

    func testErrorCodesInTheIndexAreAllKnown() throws {
        let known = Set(AIPConstants.errorCodes)
        for key in ["envelopes", "generated", "negotiations"] {
            for entry in try section(key) {
                if let code = entry["code"] as? String {
                    XCTAssertTrue(known.contains(code), "unknown ErrorCode `\(code)` in \(key)")
                }
            }
        }
    }

    // MARK: - 邊界

    func testDeadlineTreatsTheDeadlineItselfAsExpired() throws {
        guard case .success(let envelope) = AIPEnvelope.parse(try fixture("event-touch.json")) else {
            return XCTFail("event-touch.json does not parse")
        }
        guard let deadline = AIPTime.parse(envelope.expiresAt ?? "") else {
            return XCTFail("event-touch.json has no parseable deadline")
        }
        XCTAssertFalse(envelope.isExpired(now: deadline.addingTimeInterval(-1)))
        XCTAssertTrue(envelope.isExpired(now: deadline))
        XCTAssertTrue(envelope.isExpired(now: deadline.addingTimeInterval(60)))
    }

    func testDedupeRingIsBoundedAndEvictsTheOldest() {
        var ring = AIPDedupeRing(cap: 2)
        XCTAssertTrue(ring.note("a"))
        XCTAssertFalse(ring.note("a"))
        XCTAssertTrue(ring.note("b"))
        XCTAssertTrue(ring.note("c"))
        XCTAssertFalse(ring.has("a"), "oldest evicted")
        XCTAssertEqual(ring.count, 2)

        var big = AIPDedupeRing(cap: 1_000_000)
        for index in 0..<(AIPLimits.dedupeRing + 50) { _ = big.note("msg_\(index)") }
        XCTAssertEqual(big.count, AIPLimits.dedupeRing, "the ring never grows past the limit")
    }

    func testUnknownEnumValuesDoNotCrashAndAreNeverTreatedAsKnown() {
        let type = AIPMessageType(rawValue: "teleport")
        XCTAssertFalse(type.isKnown)
        XCTAssertEqual(type.rawValue, "teleport")
        let kind = AIPPartyKind(rawValue: "space-probe")
        XCTAssertFalse(kind.isKnown)
        let code = AIPErrorCode(rawValue: "brand-new-code")
        XCTAssertFalse(code.isKnown)
    }

    func testVersionNegotiationRefusesADifferentMajorAndAcceptsANewerMinor() {
        guard case .success(let newer) = AIPVersion.negotiate("aip/1.3") else {
            return XCTFail("a newer minor must still negotiate")
        }
        XCTAssertEqual(newer.specVersion, "aip/1.0")
        XCTAssertTrue(newer.newerMinor)
        guard case .failure(let error) = AIPVersion.negotiate("aip/2.0") else {
            return XCTFail("a different major must be refused")
        }
        XCTAssertEqual(error.code, .unsupportedVersion)
        guard case .failure(let syntax) = AIPVersion.negotiate("1.0") else {
            return XCTFail("a malformed version must be refused")
        }
        XCTAssertEqual(syntax.code, .schemaInvalid)
    }

    // MARK: - 數值與字元計數的跨語言一致性

    /// AIP §4.1／§11：`validate()` 的契約是「拒絕、不執行、**不崩潰**」。
    /// `payload.revision` 是對方可控的資料；超出 Int 範圍的數字以前會讓整個 App 進程 trap。
    func testAStateEnvelopeWithAnOutOfRangeRevisionIsRejectedInsteadOfTrapping() {
        for revision in ["1e30", "-1", "1.5", "1e400"] {
            let text = """
                {"specVersion":"aip/1.0","messageId":"aip-1-1","messageType":"state",
                 "name":"character.session.snapshot",
                 "source":{"kind":"runtime","id":"runtime"},
                 "sessionId":"session.home","sequence":1,
                 "occurredAt":"2026-09-04T12:30:00.000Z",
                 "payload":{"kind":"snapshot","revision":\(revision),"state":{}}}
                """
            switch AIPCheck.evaluate(Data(text.utf8)) {
            case .success:
                XCTFail("revision \(revision) 不是 u64，必須被拒絕")
            case .failure(let failure):
                XCTAssertEqual(
                    failure.code, .schemaInvalid, "revision \(revision) 應該回 schema-invalid")
            }
        }

        // 2^63 仍在 u64 內：Rust 的 `Value::as_u64` 收，Swift 也必須收（不能因為超過
        // Int.max 就拒絕，那是另一種分歧）。
        let big = """
            {"specVersion":"aip/1.0","messageId":"aip-1-2","messageType":"state",
             "name":"character.session.snapshot",
             "source":{"kind":"runtime","id":"runtime"},
             "sessionId":"session.home","sequence":1,
             "occurredAt":"2026-09-04T12:30:00.000Z",
             "payload":{"kind":"snapshot","revision":9223372036854775808,"state":{}}}
            """
        if case .failure(let failure) = AIPCheck.evaluate(Data(big.utf8)) {
            XCTFail("2^63 是合法的 u64 revision，卻被拒絕（\(failure.code.rawValue)）")
        }
    }

    /// §11 的 `MAX_ID_CHARS`／`MAX_STRING_CHARS` 在 Rust 數 `chars()`（scalar）、
    /// TS 數 code point；Swift 的 `String.count` 數的是 grapheme cluster，
    /// 同一則訊息會得到相反的結論（`docs/aip/conformance.md` §1 宣稱三邊一致）。
    func testIdentifierAndStringLimitsAreCountedInUnicodeScalarsLikeRustAndTypeScript() {
        // 128 個「e + 結合尖音符」＝128 grapheme cluster、256 scalar：Rust／TS 只因長度就拒絕
        //（結合符號既不是控制字元也不是空白，所以只有字元計數這一關會擋）。
        let oversizedId = String(repeating: "e\u{0301}", count: AIPLimits.maxIdChars)
        XCTAssertEqual(oversizedId.count, AIPLimits.maxIdChars)
        XCTAssertGreaterThan(oversizedId.unicodeScalars.count, AIPLimits.maxIdChars)
        let idText = """
            {"specVersion":"aip/1.0","messageId":"\(oversizedId)","messageType":"heartbeat",
             "name":"character.session.presence",
             "source":{"kind":"runtime","id":"runtime"},
             "occurredAt":"2026-09-04T12:30:00.000Z","payload":{}}
            """
        switch AIPCheck.evaluate(Data(idText.utf8)) {
        case .success:
            XCTFail("messageId 超過 128 個 scalar，必須與 Rust／TS 一樣拒絕")
        case .failure(let failure):
            XCTAssertEqual(failure.code, .schemaInvalid)
        }

        // 2000 個「e + 結合尖音符」＝2000 grapheme cluster、4000 scalar：同樣必須拒絕。
        let oversizedString = String(repeating: "e\u{0301}", count: 2_000)
        XCTAssertEqual(oversizedString.count, AIPLimits.maxStringChars)
        XCTAssertGreaterThan(oversizedString.unicodeScalars.count, AIPLimits.maxStringChars)
        XCTAssertEqual(
            AIPEnvelope.checkPayload(.object(["note": .string(oversizedString)]))?.code,
            .schemaInvalid,
            "payload 字串長度也必須以 scalar 計")

        // 錯誤訊息的 200 字截斷同樣以 scalar 計（Rust `chars().take(200)`）。
        let failure = AIPFailure(.schemaInvalid, String(repeating: "e\u{0301}", count: 300))
        XCTAssertEqual(failure.message.unicodeScalars.count, 200)
    }

    /// AIP §1：未知的頂層選填欄位必須 round-trip 不遺失。
    /// `JSONValue` 只有 Double 一種數字，超過 2^53 的整數會被改值、≥1e15 還會被寫成指數形式。
    func testLargeIntegersInUnknownTopLevelFieldsSurviveTheRoundTripByteForByte() {
        let text = """
            {"specVersion":"aip/1.0","messageId":"aip-1-1","messageType":"heartbeat",
             "name":"character.session.presence",
             "source":{"kind":"runtime","id":"runtime"},
             "occurredAt":"2026-09-04T12:30:00.000Z",
             "traceId":9007199254740993,"big":1000000000000000001,"payload":{}}
            """
        guard case .success(let parsed) = AIPEnvelope.parse(Data(text.utf8)) else {
            return XCTFail("信封應該可以解析")
        }
        guard case .success(let encoded) = parsed.encode() else {
            return XCTFail("信封應該可以重新編碼")
        }
        let out = String(decoding: encoded, as: UTF8.self)
        XCTAssertTrue(
            out.contains("9007199254740993"),
            "2^53 以上的整數不得被改值（實際輸出：\(out)）")
        XCTAssertTrue(
            out.contains("1000000000000000001"),
            "大整數不得被寫成指數形式（實際輸出：\(out)）")
    }

    // MARK: - helpers

    private func party(_ value: Any?) -> AIPParty? {
        guard let object = value as? [String: Any],
            let kind = object["kind"] as? String,
            let id = object["id"] as? String
        else { return nil }
        return AIPParty(kind: AIPPartyKind(rawValue: kind), id: id)
    }
}
