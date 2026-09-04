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
        self.message = String(message.prefix(200))
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
        guard !name.isEmpty, name.count <= AIPLimits.maxNameChars else { return false }
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
    if name.hasPrefix("character.session.") { return .stateReconcile }
    if name.hasPrefix("task.") || name.hasPrefix("runtime.") { return .stateReconcile }
    if name == "approval.request" { return .requireReconfirmation }
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
    static func parse(_ value: String) -> (major: Int, minor: Int)? {
        let trimmed = value.trimmingCharacters(in: .whitespaces)
        guard trimmed.hasPrefix("aip/") else { return nil }
        let rest = trimmed.dropFirst("aip/".count)
        let parts = rest.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 2,
            !parts[0].isEmpty, !parts[1].isEmpty,
            parts[0].allSatisfy(\.isAsciiDigit), parts[1].allSatisfy(\.isAsciiDigit),
            let major = Int(parts[0]), let minor = Int(parts[1])
        else { return nil }
        return (major, minor)
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
        return .success(envelope)
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
            let revision = body["revision"]?.intValue
            if let failure = need(revision != nil && revision! >= 0, "payload.revision") {
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
        guard !value.isEmpty, value.count <= AIPLimits.maxIdChars else {
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
            if text.count > AIPLimits.maxStringChars {
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
