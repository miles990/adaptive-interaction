//
//  AIPEnvelope.swift
//  InteractionCompanion
//
//  AIP 1.0 的 Swift 驗證邏輯（手寫）。型別在 AIPGenerated.swift（由 scripts/aip-codegen.mjs
//  從 schemas/aip-1.0.schema.json 產生，不要手改）。
//
//  這裡的每一條規則都必須與 Rust 權威實作（crates/interaction-aip）一致：檢查順序、錯誤碼、
//  上限、profile 必填欄位。一致性由 AIPConformanceTests 對同一份 fixture index 釘住。
//
//  誠實不變量（對應 repo CLAUDE.md 與 docs/aip/README.md）：
//  - received ≠ accepted ≠ acknowledged ≠ applied ≠ observed ≠ claimed-completed ≠ verified。
//  - `verified` 只有 Runtime 的人類驗證路徑能產生；App 永遠不得自己宣告。
//  - 未知 message type／未知 name 不執行；未知選填欄位保留並忽略。
//  - 所有集合有界：去重環滿了淘汰最舊，不會無界成長。
//

import Foundation

// MARK: - 失敗

/// AIP 層的處理失敗。`message` ≤ 200 字、不回顯輸入、不含路徑（AIP §5／§12）。
struct AIPFailure: Error, Equatable {
    let code: AIPErrorCode
    let message: String
    let retryable: Bool

    init(_ code: AIPErrorCode, _ message: String) {
        self.code = code
        // 截斷以 Unicode scalar 計，與 Rust 的 `chars().take(200)`、TS 的 `[...message]` 一致。
        self.message = String(String.UnicodeScalarView(message.unicodeScalars.prefix(200)))
        self.retryable = code == .rateLimited || code == .`internal`
    }
}

// MARK: - JSONValue 走訪工具

extension JSONValue {
    fileprivate var aipObject: [String: JSONValue]? {
        if case .object(let value) = self { return value }
        return nil
    }

    fileprivate var aipArray: [JSONValue]? {
        if case .array(let value) = self { return value }
        return nil
    }
}

// MARK: - 時間

enum AIPTime {
    private static let withFraction: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let plain: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    /// RFC 3339。桌面端（chrono）也只接受這個形狀，兩邊必須一樣嚴。
    static func parse(_ value: String) -> Date? {
        withFraction.date(from: value) ?? plain.date(from: value)
    }
}

// MARK: - name 語法與範圍

enum AIPName {
    /// `^[a-z][a-z0-9]*(\.[a-z][a-z0-9-]*)+$`，≤ MAX_NAME_CHARS。
    static func isValid(_ name: String) -> Bool {
        // 長度以 Unicode scalar 計（Rust `chars().count()`／TS `[...name].length`）。
        guard !name.isEmpty, name.unicodeScalars.count <= AIPLimits.maxNameChars else {
            return false
        }
        let segments = name.split(separator: ".", omittingEmptySubsequences: false)
        guard segments.count >= 2 else { return false }
        for (index, segment) in segments.enumerated() {
            guard let first = segment.first, first.isLowercaseAsciiLetter else { return false }
            let tail = segment.dropFirst()
            let ok = tail.allSatisfy { character in
                character.isLowercaseAsciiLetter || character.isAsciiDigit
                    || (index > 0 && character == "-")
            }
            if !ok { return false }
        }
        return true
    }

    /// 只有 Runtime 可以送的 name 前綴；device／renderer 送來一律 `scope-denied`。
    static func isRuntimeOnly(_ name: String) -> Bool {
        name.hasPrefix("task.") || name.hasPrefix("runtime.")
    }
}

extension Character {
    fileprivate var isLowercaseAsciiLetter: Bool { self >= "a" && self <= "z" }
    fileprivate var isAsciiDigit: Bool { self >= "0" && self <= "9" }
}

// MARK: - 身分

/// §5：`source` 只是宣稱。不符一律拒絕並稽核；App／host **不得**「幫忙修正」後執行。
enum AIPIdentityDecision: Equatable {
    case accept
    case reject(bound: AIPParty, claimed: AIPParty)

    var isAccept: Bool {
        if case .accept = self { return true }
        return false
    }
}

func aipBindIdentity(bound: AIPParty, claimed: AIPParty) -> AIPIdentityDecision {
    bound == claimed ? .accept : .reject(bound: bound, claimed: claimed)
}

// MARK: - 離線政策

/// §8 的固定歸類表。未知 name → `dropIfOffline`（最保守：不排隊、不重播）。
func aipOfflinePolicy(_ name: String, hasConsentGrant: Bool = false) -> AIPOfflinePolicy {
    if hasConsentGrant { return .requireReconfirmation }
    if name.hasPrefix("character.interaction.touch") { return .expireByDeadline }
    if name.hasPrefix("character.interaction.") { return .dropIfOffline }
    if name.hasPrefix("character.behavior.") { return .dropIfOffline }
    if name.hasPrefix("character.preference.") { return .queueIdempotent }
    // `character.session.approval`（approval-request 的線上名字）同時符合
    // `character.session.` 前綴，所以這條**必須**先判斷；反過來排的話，唯一真正存在的
    // approval name 會被歸成 `state-reconcile`＝離線後可以自動對齊——那是人類決定，
    // 不得自動重送。與 `crates/interaction-aip/src/offline.rs` 同一條界線。
    if name.hasPrefix("approval.") || name.hasSuffix(".approval") { return .requireReconfirmation }
    if name.hasPrefix("character.session.") { return .stateReconcile }
    if name.hasPrefix("task.") || name.hasPrefix("runtime.") { return .stateReconcile }
    return .dropIfOffline
}

// MARK: - Outcome 誠實階梯

extension AIPOutcome {
    /// 終態（之後不得再變）。
    var isTerminal: Bool {
        switch self {
        case .applied, .observed, .verified, .rejected, .expired, .cancelConfirmed, .failed:
            return true
        default:
            return false
        }
    }

    /// `verified` 只能由 Runtime 的人類驗證路徑產生；App／adapter 一律不得宣告。
    var isRuntimeOnly: Bool { self == .verified }

    /// 合法遷移：只能往前，終態黏住，`observed`／`acknowledged` 永遠走不到 `verified`。
    func canTransition(to next: AIPOutcome) -> Bool {
        if isTerminal { return false }
        switch (self, next) {
        case (.received, .accepted), (.received, .rejected), (.received, .expired),
            (.accepted, .acknowledged), (.accepted, .applied), (.accepted, .observed),
            (.accepted, .claimedCompleted), (.accepted, .expired), (.accepted, .failed),
            (.accepted, .cancelRequested), (.accepted, .cancelConfirmed),
            (.acknowledged, .observed), (.acknowledged, .failed), (.acknowledged, .expired),
            (.acknowledged, .cancelRequested), (.acknowledged, .cancelConfirmed),
            (.claimedCompleted, .verified), (.claimedCompleted, .failed),
            (.cancelRequested, .cancelConfirmed), (.cancelRequested, .failed):
            return true
        default:
            return false
        }
    }

    /// 各 profile 只用自己適合的子集（§3）。
    func isAllowed(forProfile profile: String) -> Bool {
        switch profile {
        case "event":
            return [.received, .accepted, .applied, .rejected, .expired].contains(self)
        case "command":
            return [
                .received, .accepted, .acknowledged, .observed, .rejected, .expired, .failed,
                .cancelRequested, .cancelConfirmed,
            ].contains(self)
        case "state":
            return [.applied, .rejected].contains(self)
        default:
            return false
        }
    }
}

// MARK: - 去重（有界）

/// §7 有界去重環（每個 (session, source) 一份）。滿了淘汰最舊，永遠不會無界成長。
struct AIPDedupeRing {
    private let cap: Int
    private var order: [String] = []
    private var seen: Set<String> = []

    init(cap: Int = AIPLimits.dedupeRing) {
        self.cap = min(max(cap, 1), AIPLimits.dedupeRing)
    }

    /// `true` = 第一次看到（已記下）；`false` = 重複，不得重新套用。
    mutating func note(_ messageId: String) -> Bool {
        if seen.contains(messageId) { return false }
        if order.count >= cap, let oldest = order.first {
            order.removeFirst()
            seen.remove(oldest)
        }
        order.append(messageId)
        seen.insert(messageId)
        return true
    }

    func has(_ messageId: String) -> Bool { seen.contains(messageId) }
    var count: Int { order.count }
}

// MARK: - 版本

struct AIPNegotiatedVersion: Equatable {
    let specVersion: String
    let newerMinor: Bool
}

enum AIPVersion {
    /// 語法是**精確**的，不是「大概像」：前後空白／換行不得被 trim 掉
    ///（Swift 的 `.whitespaces` 連換行都不含，容忍它只會讓三個語言的界線不一樣），
    /// major／minor 溢出 u32 一律回 nil → `schema-invalid`（看不懂的字串不叫
    ///「不支援的版本」）。對齊 `crates/interaction-aip/src/version.rs::parse_spec_version`。
    static func parse(_ value: String) -> (major: Int, minor: Int)? {
        guard value.hasPrefix("aip/") else { return nil }
        let rest = value.dropFirst("aip/".count)
        let parts = rest.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 2,
            !parts[0].isEmpty, !parts[1].isEmpty,
            parts[0].allSatisfy(\.isAsciiDigit), parts[1].allSatisfy(\.isAsciiDigit),
            let major = UInt32(parts[0]), let minor = UInt32(parts[1])
        else { return nil }
        return (Int(major), Int(minor))
    }

    static var local: (major: Int, minor: Int) { parse(AIPConstants.specVersion) ?? (1, 0) }

    /// §4.1：major 不同一律拒絕（不猜）；minor 較新時取 min 並標 `newerMinor`。
    static func negotiate(_ remote: String) -> Result<AIPNegotiatedVersion, AIPFailure> {
        guard let (major, minor) = parse(remote) else {
            return .failure(AIPFailure(.schemaInvalid, "specVersion must look like aip/<major>.<minor>"))
        }
        let localVersion = local
        guard major == localVersion.major else {
            return .failure(
                AIPFailure(
                    .unsupportedVersion,
                    "unsupported major \(major); this build speaks aip/\(localVersion.major).x"))
        }
        return .success(
            AIPNegotiatedVersion(
                specVersion: "aip/\(major).\(min(minor, localVersion.minor))",
                newerMinor: minor > localVersion.minor))
    }
}

// MARK: - 解析與驗證

extension AIPEnvelope {
    /// 大小上限（§11）→ JSON 解析 → 時間戳形狀。未知的頂層欄位保留在 `extra`。
    static func parse(_ data: Data) -> Result<AIPEnvelope, AIPFailure> {
        guard data.count <= AIPLimits.maxMessageBytes else {
            return .failure(
                AIPFailure(.messageTooLarge, "message exceeds \(AIPLimits.maxMessageBytes) bytes"))
        }
        let envelope: AIPEnvelope
        do {
            envelope = try JSONDecoder().decode(AIPEnvelope.self, from: data)
        } catch {
            // §5：不回顯輸入，只說類別。
            return .failure(AIPFailure(.schemaInvalid, "invalid envelope (data)"))
        }
        guard AIPTime.parse(envelope.occurredAt) != nil else {
            return .failure(AIPFailure(.schemaInvalid, "invalid envelope (data)"))
        }
        if let expiresAt = envelope.expiresAt, AIPTime.parse(expiresAt) == nil {
            return .failure(AIPFailure(.schemaInvalid, "invalid envelope (data)"))
        }
        if let failure = checkIntegerLiterals(data) { return .failure(failure) }
        return .success(envelope)
    }

    /// §1／§6：`sequence`／`baseRevision`／`payload.revision` 這些欄位是**整數**。
    ///
    /// Foundation 的 `JSONDecoder` 會把 `2.0`（甚至 `1e3`）悄悄收成 `UInt64`，但權威 host
    /// 的 serde 不收（`Value::as_u64` 對浮點字面回 `None`）——同一則訊息在兩端會得到
    /// 相反的結論。所以這裡對**原文**再看一次字面形狀（`SemanticJSON` 逐字保留數字）。
    private static func checkIntegerLiterals(_ data: Data) -> AIPFailure? {
        guard let root = SemanticJSON.parse(String(decoding: data, as: UTF8.self)) else {
            // 這裡解析不出來不代表訊息有問題（上面的 JSONDecoder 已經過了）：不重複報錯。
            return nil
        }
        let payload = root["payload"]
        let integerFields: [SemanticJSON?] = [
            root["sequence"], root["baseRevision"],
            payload?["revision"], payload?["baseRevision"], payload?["sequence"],
            payload?["sessionEpoch"], payload?["lastRevision"], payload?["lastSequence"],
        ]
        for field in integerFields {
            guard case .number(let raw)? = field, !isIntegerLiteral(raw) else { continue }
            // §5：不回顯輸入，只說類別。
            return AIPFailure(.schemaInvalid, "integer fields must be written as JSON integers")
        }
        return nil
    }

    /// `-?[0-9]+`。小數點與指數都不是整數字面（serde 的 u64／i64 也是這條界線）。
    private static func isIntegerLiteral(_ raw: String) -> Bool {
        var body = Substring(raw)
        if body.hasPrefix("-") { body = body.dropFirst() }
        return !body.isEmpty && body.allSatisfy(\.isAsciiDigit)
    }

    func encode() -> Result<Data, AIPFailure> {
        guard let data = try? JSONEncoder().encode(self) else {
            return .failure(AIPFailure(.`internal`, "envelope encode failed"))
        }
        guard data.count <= AIPLimits.maxMessageBytes else {
            return .failure(
                AIPFailure(.messageTooLarge, "message exceeds \(AIPLimits.maxMessageBytes) bytes"))
        }
        return .success(data)
    }

    /// §7：`expiresAt` 已過（含等於）→ 過期。沒有 expiresAt → 不過期。
    func isExpired(now: Date) -> Bool {
        guard let expiresAt, let deadline = AIPTime.parse(expiresAt) else { return false }
        return deadline <= now
    }

    /// §2.2 profile 必填 ＋ §11 上限 ＋ §4 版本 ＋ name 語法。
    /// 順序固定並與 Rust 一致；第一個失敗即回，未知的一律不執行。回 nil 表示通過。
    func validate() -> AIPFailure? {
        if case .failure(let error) = AIPVersion.negotiate(specVersion) { return error }
        guard messageType.isKnown else {
            // §5：未知 type 的原字串是呼叫端可控的資料，只留在本地稽核，不進錯誤訊息。
            return AIPFailure(
                .unsupportedMessageType, "messageType is not one of the 12 known AIP message types")
        }
        if let failure = Self.checkId(messageId) { return failure }
        guard AIPName.isValid(name) else {
            return AIPFailure(.schemaInvalid, "name violates grammar")
        }
        if let failure = Self.checkId(source.id) { return failure }
        guard source.kind.isKnown else {
            return AIPFailure(.schemaInvalid, "source.kind unknown")
        }
        if let target, let failure = Self.checkId(target.id) { return failure }
        for value in [sessionId, correlationId, causationId, consentGrantId] {
            if let value, let failure = Self.checkId(value) { return failure }
        }
        if let failure = Self.checkPayload(payload) { return failure }

        let body = payload?.aipObject ?? [:]
        func need(_ condition: Bool, _ what: String) -> AIPFailure? {
            condition ? nil : AIPFailure(.schemaInvalid, "\(messageType.rawValue) requires \(what)")
        }

        switch messageType {
        case .event:
            if let failure = need(sessionId != nil, "sessionId") { return failure }
            if name.hasPrefix("character.interaction.") {
                return need(expiresAt != nil, "expiresAt for interaction events")
            }
            return nil
        case .command:
            if let failure = need(sessionId != nil, "sessionId") { return failure }
            if let failure = need(target != nil, "target") { return failure }
            if let failure = need(correlationId != nil, "correlationId") { return failure }
            return need(expiresAt != nil, "expiresAt")
        case .query:
            return need(target != nil, "target")
        case .response:
            return need(causationId != nil, "causationId")
        case .result:
            if let failure = need(causationId != nil, "causationId") { return failure }
            let status = body["status"]?.stringValue
            let known = status.flatMap { AIPOutcome(rawValue: $0) } != nil
            return need(known, "a known payload.status")
        case .state:
            if let failure = need(sessionId != nil, "sessionId") { return failure }
            if let failure = need(sequence != nil, "sequence") { return failure }
            // Rust 用 `Value::as_u64`：非負整數才算數（超出 Int 範圍的數字不得讓進程崩潰）。
            if let failure = need(body["revision"]?.uint64Value != nil, "payload.revision") {
                return failure
            }
            if body["kind"]?.stringValue == "patch" {
                return need(baseRevision != nil, "baseRevision for patches")
            }
            return nil
        case .cancel:
            return need(
                causationId != nil || body["messageId"]?.stringValue != nil,
                "causationId or payload.messageId")
        case .approvalRequest:
            if let failure = need(correlationId != nil, "correlationId") { return failure }
            if let failure = need(expiresAt != nil, "expiresAt") { return failure }
            return need(target?.kind == .human, "target{kind:human}")
        case .approvalResult:
            return need(causationId != nil, "causationId")
        case .`error`:
            return need(body["code"]?.stringValue != nil, "payload.code")
        case .heartbeat, .capability:
            return nil
        case .unknown:
            return AIPFailure(
                .unsupportedMessageType, "messageType is not one of the 12 known AIP message types")
        }
    }

    private static func checkId(_ value: String) -> AIPFailure? {
        // §11 的字元上限在 Rust 數 `chars()`、TS 數 code point：Swift 的 `String.count`
        // 數的是 grapheme cluster，同一個 id 會得到相反的結論，所以這裡也數 scalar。
        guard !value.isEmpty, value.unicodeScalars.count <= AIPLimits.maxIdChars else {
            return AIPFailure(
                .schemaInvalid, "identifiers must be 1..=\(AIPLimits.maxIdChars) chars")
        }
        let bad = value.unicodeScalars.contains { scalar in
            CharacterSet.whitespacesAndNewlines.contains(scalar)
                || CharacterSet.controlCharacters.contains(scalar)
        }
        if bad {
            return AIPFailure(.schemaInvalid, "an identifier contains whitespace or control characters")
        }
        return nil
    }

    /// payload：大小、巢狀深度、字串長度（§11）。順序與 Rust 相同：先量大小，再走樹。
    static func checkPayload(_ payload: JSONValue?) -> AIPFailure? {
        let value = payload ?? .null
        let encoder = JSONEncoder()
        let size = (try? encoder.encode(value))?.count ?? Int.max
        guard size <= AIPLimits.maxPayloadBytes else {
            return AIPFailure(.payloadTooLarge, "payload exceeds \(AIPLimits.maxPayloadBytes) bytes")
        }
        return walk(value, depth: 1)
    }

    private static func walk(_ value: JSONValue, depth: Int) -> AIPFailure? {
        guard depth <= AIPLimits.maxJsonDepth else {
            return AIPFailure(.schemaInvalid, "payload nesting too deep")
        }
        if case .string(let text) = value {
            if text.unicodeScalars.count > AIPLimits.maxStringChars {
                return AIPFailure(
                    .schemaInvalid, "payload string exceeds \(AIPLimits.maxStringChars) chars")
            }
            return nil
        }
        if let items = value.aipArray {
            for item in items {
                if let failure = walk(item, depth: depth + 1) { return failure }
            }
            return nil
        }
        if let object = value.aipObject {
            for item in object.values {
                if let failure = walk(item, depth: depth + 1) { return failure }
            }
            return nil
        }
        return nil
    }
}

// MARK: - 完整檢查

enum AIPCheck {
    /// 一則 wire bytes 走完整條檢查：大小 → 解析 → profile／上限／版本驗證。
    static func evaluate(_ data: Data) -> Result<AIPEnvelope, AIPFailure> {
        switch AIPEnvelope.parse(data) {
        case .failure(let error):
            return .failure(error)
        case .success(let envelope):
            if let failure = envelope.validate() { return .failure(failure) }
            return .success(envelope)
        }
    }
}
