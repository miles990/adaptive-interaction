//
//  CharacterSemantic.swift
//  InteractionCompanion
//
//  AIP Character Profile 的 Swift 鏡射：語意狀態（`docs/aip/character-session.md` §3）、
//  RFC 7396 merge patch、canonical JSON 的 SHA-256，以及要顯示給人看的投影。
//
//  權威實作是 Rust crate `interaction-session`（`patch.rs`／`state.rs`）。這裡只是同一組
//  純函式的 Swift 版本：對同一份 state JSON 必須得到同一個 hash 與同一個決策。
//
//  誠實不變量（對應 repo CLAUDE.md 與 docs/aip/README.md）：
//  - 綠色勾號只在 `truth == verified` 出現；`observed`／`applied` 都不是 verified。
//  - `emergency` 是安全訊息：只能加嚴、不能被語意狀態蓋掉，文案固定。
//  - 未知的 mood／activity／truth 一律保留原字串並顯示「未知」，不猜、不美化。
//  - 所有集合有界（成員數、巢狀深度、輸入長度）。
//
//  為什麼需要「逐字保留的 JSON」：host 的 hash 是對 **serde_json 寫出來的文字**取的，
//  而 `mood.intensity` 是 f64——值為 0 時 serde_json 寫 `0.0`，一般 JSON 解析器讀進
//  Double 之後再寫出來卻是 `0`，兩邊 canonical 文字就不一樣、hash 就對不上，
//  App 會陷入「hash 不符 → 要 snapshot」的迴圈。所以數字一律保留原始字面。
//

import CryptoKit
import Foundation

// MARK: - 逐字保留的 JSON

/// 保留原始數字字面的 JSON 值。
///
/// 只用在角色語意狀態這條路上（snapshot／patch／hash）；一般 wire 訊息仍用 `JSONValue`。
/// `Equatable` 比較數字時比的是**字面**：`0` 與 `0.0` 視為不同，這正是 hash 需要的語意。
enum SemanticJSON: Equatable {
    case null
    case bool(Bool)
    /// 原始數字字面（例如 `0`、`0.0`、`0.45`、`1e3`）。
    case number(raw: String)
    case string(String)
    case array([SemanticJSON])
    case object([String: SemanticJSON])

    /// 巢狀深度上限（有界；超過即拒絕，不遞迴到爆堆疊）。
    static let maxDepth = 32
    /// 可解析的最大輸入位元組數（wss frame 上限的同一個量級）。
    static let maxInputBytes = 131_072

    // MARK: 取值

    subscript(key: String) -> SemanticJSON? {
        if case .object(let map) = self { return map[key] }
        return nil
    }

    var objectValue: [String: SemanticJSON]? {
        if case .object(let map) = self { return map }
        return nil
    }

    var arrayValue: [SemanticJSON]? {
        if case .array(let items) = self { return items }
        return nil
    }

    var stringValue: String? {
        if case .string(let text) = self { return text }
        return nil
    }

    var boolValue: Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }

    var doubleValue: Double? {
        if case .number(let raw) = self { return Double(raw) }
        return nil
    }

    var uintValue: UInt64? {
        guard case .number(let raw) = self else { return nil }
        if let exact = UInt64(raw) { return exact }
        // `1.0` 這種浮點寫法也接受，但只在真的是非負整數時。
        guard let value = Double(raw), value >= 0, value.rounded() == value,
            value <= Double(UInt64.max)
        else { return nil }
        return UInt64(value)
    }

    var isNull: Bool {
        if case .null = self { return true }
        return false
    }

    // MARK: Canonical JSON（與 `interaction_aip::canonical_json` 同一個輸出）

    /// 鍵以 UTF-8 位元組序排序、無空白、數字逐字保留。
    var canonicalJSON: String {
        switch self {
        case .null:
            return "null"
        case .bool(let value):
            return value ? "true" : "false"
        case .number(let raw):
            return raw
        case .string(let text):
            return Self.encodeString(text)
        case .array(let items):
            return "[" + items.map(\.canonicalJSON).joined(separator: ",") + "]"
        case .object(let map):
            // Rust 對 `&String` 排序＝UTF-8 位元組序；Swift 的 `<` 是 Unicode 排序，
            // 兩者對非 ASCII 鍵會分歧，所以這裡明確比位元組。
            let keys = map.keys.sorted { left, right in
                Array(left.utf8).lexicographicallyPrecedes(Array(right.utf8))
            }
            let parts = keys.map { key in
                Self.encodeString(key) + ":" + (map[key]?.canonicalJSON ?? "null")
            }
            return "{" + parts.joined(separator: ",") + "}"
        }
    }

    /// Canonical JSON 的 SHA-256（小寫十六進位）——與 host 的 `state_hash` 同一個值。
    var canonicalSHA256: String {
        Hex.encode(Data(SHA256.hash(data: Data(canonicalJSON.utf8))))
    }

    /// serde_json 相容的字串輸出：只跳脫 `"`、`\` 與控制字元，非 ASCII 與 `/` 原樣輸出。
    static func encodeString(_ text: String) -> String {
        var out = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\"":
                out += "\\\""
            case "\\":
                out += "\\\\"
            case "\u{08}":
                out += "\\b"
            case "\u{0C}":
                out += "\\f"
            case "\n":
                out += "\\n"
            case "\r":
                out += "\\r"
            case "\t":
                out += "\\t"
            default:
                if scalar.value < 0x20 {
                    out += String(format: "\\u%04x", scalar.value)
                } else {
                    out.unicodeScalars.append(scalar)
                }
            }
        }
        return out + "\""
    }

    // MARK: RFC 7396 Merge Patch

    /// `null` 代表刪除鍵，物件遞迴合併，其他型別整體取代（與 `interaction_session::apply_patch` 同）。
    static func mergePatch(_ base: SemanticJSON, _ patch: SemanticJSON) -> SemanticJSON {
        guard case .object(let patchMap) = patch else { return patch }
        var out = base.objectValue ?? [:]
        for (key, value) in patchMap {
            if value.isNull {
                out.removeValue(forKey: key)
            } else {
                out[key] = mergePatch(out[key] ?? .null, value)
            }
        }
        return .object(out)
    }

    // MARK: 解析

    /// 解析一段 JSON 文字，逐字保留數字字面。格式不合、太深或太大一律回 `nil`（不猜）。
    static func parse(_ text: String) -> SemanticJSON? {
        var scanner = SemanticJSONScanner(text)
        guard scanner.isWithinSizeLimit else { return nil }
        guard let value = scanner.parseValue(depth: 1) else { return nil }
        scanner.skipWhitespace()
        guard scanner.isAtEnd else { return nil }
        return value
    }
}

/// 逐字掃描器（純值型別，沒有共享狀態）。
private struct SemanticJSONScanner {
    private let bytes: [UInt8]
    private var index = 0

    init(_ text: String) {
        bytes = Array(text.utf8)
    }

    var isWithinSizeLimit: Bool { bytes.count <= SemanticJSON.maxInputBytes }
    var isAtEnd: Bool { index >= bytes.count }

    mutating func skipWhitespace() {
        while index < bytes.count {
            switch bytes[index] {
            case 0x20, 0x09, 0x0A, 0x0D: index += 1
            default: return
            }
        }
    }

    mutating func parseValue(depth: Int) -> SemanticJSON? {
        guard depth <= SemanticJSON.maxDepth else { return nil }
        skipWhitespace()
        guard index < bytes.count else { return nil }
        switch bytes[index] {
        case UInt8(ascii: "{"):
            return parseObject(depth: depth)
        case UInt8(ascii: "["):
            return parseArray(depth: depth)
        case UInt8(ascii: "\""):
            return parseString().map { SemanticJSON.string($0) }
        case UInt8(ascii: "t"):
            return match("true") ? .bool(true) : nil
        case UInt8(ascii: "f"):
            return match("false") ? .bool(false) : nil
        case UInt8(ascii: "n"):
            return match("null") ? SemanticJSON.null : nil
        default:
            return parseNumber()
        }
    }

    private mutating func match(_ literal: String) -> Bool {
        let expected = Array(literal.utf8)
        guard index + expected.count <= bytes.count else { return false }
        for (offset, byte) in expected.enumerated() where bytes[index + offset] != byte {
            return false
        }
        index += expected.count
        return true
    }

    private mutating func parseObject(depth: Int) -> SemanticJSON? {
        index += 1  // '{'
        var map: [String: SemanticJSON] = [:]
        skipWhitespace()
        if index < bytes.count, bytes[index] == UInt8(ascii: "}") {
            index += 1
            return .object(map)
        }
        while true {
            skipWhitespace()
            guard index < bytes.count, bytes[index] == UInt8(ascii: "\""),
                let key = parseString()
            else { return nil }
            skipWhitespace()
            guard index < bytes.count, bytes[index] == UInt8(ascii: ":") else { return nil }
            index += 1
            guard let value = parseValue(depth: depth + 1) else { return nil }
            map[key] = value  // 重複鍵：後者覆蓋（與 serde_json 相同）
            skipWhitespace()
            guard index < bytes.count else { return nil }
            if bytes[index] == UInt8(ascii: ",") {
                index += 1
                continue
            }
            if bytes[index] == UInt8(ascii: "}") {
                index += 1
                return .object(map)
            }
            return nil
        }
    }

    private mutating func parseArray(depth: Int) -> SemanticJSON? {
        index += 1  // '['
        var items: [SemanticJSON] = []
        skipWhitespace()
        if index < bytes.count, bytes[index] == UInt8(ascii: "]") {
            index += 1
            return .array(items)
        }
        while true {
            guard let value = parseValue(depth: depth + 1) else { return nil }
            items.append(value)
            skipWhitespace()
            guard index < bytes.count else { return nil }
            if bytes[index] == UInt8(ascii: ",") {
                index += 1
                continue
            }
            if bytes[index] == UInt8(ascii: "]") {
                index += 1
                return .array(items)
            }
            return nil
        }
    }

    private mutating func parseNumber() -> SemanticJSON? {
        let start = index
        if index < bytes.count, bytes[index] == UInt8(ascii: "-") { index += 1 }
        var digits = 0
        while index < bytes.count, bytes[index] >= UInt8(ascii: "0"), bytes[index] <= UInt8(ascii: "9") {
            index += 1
            digits += 1
        }
        guard digits > 0 else { return nil }
        if index < bytes.count, bytes[index] == UInt8(ascii: ".") {
            index += 1
            var fraction = 0
            while index < bytes.count, bytes[index] >= UInt8(ascii: "0"),
                bytes[index] <= UInt8(ascii: "9")
            {
                index += 1
                fraction += 1
            }
            guard fraction > 0 else { return nil }
        }
        if index < bytes.count, bytes[index] == UInt8(ascii: "e") || bytes[index] == UInt8(ascii: "E") {
            index += 1
            if index < bytes.count, bytes[index] == UInt8(ascii: "+") || bytes[index] == UInt8(ascii: "-") {
                index += 1
            }
            var exponent = 0
            while index < bytes.count, bytes[index] >= UInt8(ascii: "0"),
                bytes[index] <= UInt8(ascii: "9")
            {
                index += 1
                exponent += 1
            }
            guard exponent > 0 else { return nil }
        }
        guard let raw = String(bytes: bytes[start..<index], encoding: .utf8) else { return nil }
        return .number(raw: raw)
    }

    private mutating func parseString() -> String? {
        index += 1  // 開頭的引號
        var out: [UInt8] = []
        while index < bytes.count {
            let byte = bytes[index]
            if byte == UInt8(ascii: "\"") {
                index += 1
                return String(bytes: out, encoding: .utf8)
            }
            if byte == UInt8(ascii: "\\") {
                index += 1
                guard index < bytes.count else { return nil }
                let escape = bytes[index]
                index += 1
                switch escape {
                case UInt8(ascii: "\""): out.append(UInt8(ascii: "\""))
                case UInt8(ascii: "\\"): out.append(UInt8(ascii: "\\"))
                case UInt8(ascii: "/"): out.append(UInt8(ascii: "/"))
                case UInt8(ascii: "b"): out.append(0x08)
                case UInt8(ascii: "f"): out.append(0x0C)
                case UInt8(ascii: "n"): out.append(0x0A)
                case UInt8(ascii: "r"): out.append(0x0D)
                case UInt8(ascii: "t"): out.append(0x09)
                case UInt8(ascii: "u"):
                    guard let scalar = parseUnicodeEscape() else { return nil }
                    out.append(contentsOf: Array(String(scalar).utf8))
                default:
                    return nil
                }
                continue
            }
            if byte < 0x20 { return nil }  // 未跳脫的控制字元不合法
            out.append(byte)
            index += 1
        }
        return nil
    }

    /// `\uXXXX`（含代理對）。已消耗 `u`，游標停在第一個十六進位數字。
    private mutating func parseUnicodeEscape() -> Unicode.Scalar? {
        guard let first = readHex4() else { return nil }
        if first >= 0xD800, first <= 0xDBFF {
            guard index + 1 < bytes.count, bytes[index] == UInt8(ascii: "\\"),
                bytes[index + 1] == UInt8(ascii: "u")
            else { return nil }
            index += 2
            guard let second = readHex4(), second >= 0xDC00, second <= 0xDFFF else { return nil }
            let combined = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            return Unicode.Scalar(combined)
        }
        if first >= 0xDC00, first <= 0xDFFF { return nil }  // 孤兒低位代理
        return Unicode.Scalar(first)
    }

    private mutating func readHex4() -> UInt32? {
        guard index + 4 <= bytes.count else { return nil }
        var value: UInt32 = 0
        for _ in 0..<4 {
            let byte = bytes[index]
            let digit: UInt32
            switch byte {
            case UInt8(ascii: "0")...UInt8(ascii: "9"): digit = UInt32(byte - UInt8(ascii: "0"))
            case UInt8(ascii: "a")...UInt8(ascii: "f"): digit = UInt32(byte - UInt8(ascii: "a")) + 10
            case UInt8(ascii: "A")...UInt8(ascii: "F"): digit = UInt32(byte - UInt8(ascii: "A")) + 10
            default: return nil
            }
            value = value * 16 + digit
            index += 1
        }
        return value
    }
}

// MARK: - 語意詞彙（§3）

/// `mood.kind`（7）。未知值保留原字串，不猜。
enum CharacterMood: Equatable {
    case neutral, happy, playful, proud, tired, alert, down
    case unknown(String)

    init(wire: String) {
        switch wire {
        case "neutral": self = .neutral
        case "happy": self = .happy
        case "playful": self = .playful
        case "proud": self = .proud
        case "tired": self = .tired
        case "alert": self = .alert
        case "down": self = .down
        default: self = .unknown(wire)
        }
    }
}

/// `activity`（7）。未知值保留原字串。
enum CharacterActivity: Equatable {
    case idle, reacting, working, waiting, celebrating, resting, frozen
    case unknown(String)

    init(wire: String) {
        switch wire {
        case "idle": self = .idle
        case "reacting": self = .reacting
        case "working": self = .working
        case "waiting": self = .waiting
        case "celebrating": self = .celebrating
        case "resting": self = .resting
        case "frozen": self = .frozen
        default: self = .unknown(wire)
        }
    }

    /// 一般模式的人話。未知一律說「未知」。
    var text: String {
        switch self {
        case .idle: return "待機"
        case .reacting: return "回應中"
        case .working: return "工作中"
        case .waiting: return "等待中"
        case .celebrating: return "慶祝中"
        case .resting: return "休息中"
        case .frozen: return "已凍結"
        case .unknown: return "未知"
        }
    }
}

/// CPP `truthState`（15）。`verified` 只可能來自 Runtime 的人類驗證路徑。
enum CharacterTruth: Equatable {
    case none, queued, working, waitingInput, waitingConsent, blocked, claimed
    case verified, failed, timedOut, expired, unknownState, cancelled, emergency, offline
    case unrecognized(String)

    init(wire: String) {
        switch wire {
        case "none": self = .none
        case "queued": self = .queued
        case "working": self = .working
        case "waiting-input": self = .waitingInput
        case "waiting-consent": self = .waitingConsent
        case "blocked": self = .blocked
        case "claimed": self = .claimed
        case "verified": self = .verified
        case "failed": self = .failed
        case "timed-out": self = .timedOut
        case "expired": self = .expired
        case "unknown": self = .unknownState
        case "cancelled": self = .cancelled
        case "emergency": self = .emergency
        case "offline": self = .offline
        default: self = .unrecognized(wire)
        }
    }

    /// 一般模式的人話。`claimed` 絕不寫成「完成」——那還沒被人驗證過。
    var text: String? {
        switch self {
        case .none: return nil
        case .queued: return "已排隊"
        case .working: return "工作中"
        case .waitingInput: return "等待你的輸入"
        case .waitingConsent: return "等待你的同意"
        case .blocked: return "被安全政策擋下"
        case .claimed: return "宣稱完成（尚未驗證）"
        case .verified: return "已驗證成功"
        case .failed: return "失敗"
        case .timedOut: return "逾時"
        case .expired: return "已過期"
        case .unknownState: return "未知"
        case .cancelled: return "已取消"
        case .emergency: return "緊急停止中"
        case .offline: return "離線"
        case .unrecognized: return "未知"
        }
    }
}

/// §3 `members[]` 的投影。
struct CharacterMember: Equatable {
    let partyKind: String
    let partyId: String
    let role: String
    let presence: String
}

/// §3 `SemanticState` 的唯讀鏡射。缺必填欄位就投影不出來（未知不執行）。
struct CharacterSemanticState: Equatable {
    var characterId: String
    var mood: CharacterMood
    var moodIntensity: Double
    var activity: CharacterActivity
    var truth: CharacterTruth
    var reducedMotion: Bool
    var members: [CharacterMember]
    var lastInteractionKind: String?

    /// 成員數上限（AIP §11 `MAX_MEMBERS`）；超過即拒絕整份狀態，不截斷後假裝正常。
    static let maxMembers = AIPLimits.maxMembers

    static func project(_ json: SemanticJSON) -> CharacterSemanticState? {
        guard let characterId = json["characterId"]?.stringValue,
            let moodKind = json["mood"]?["kind"]?.stringValue,
            let activity = json["activity"]?.stringValue,
            let truth = json["truth"]?["state"]?.stringValue
        else { return nil }
        let rawMembers = json["members"]?.arrayValue ?? []
        guard rawMembers.count <= maxMembers else { return nil }
        var members: [CharacterMember] = []
        members.reserveCapacity(rawMembers.count)
        for entry in rawMembers {
            guard let kind = entry["party"]?["kind"]?.stringValue,
                let id = entry["party"]?["id"]?.stringValue,
                let role = entry["role"]?.stringValue,
                let presence = entry["presence"]?.stringValue
            else { return nil }
            members.append(
                CharacterMember(partyKind: kind, partyId: id, role: role, presence: presence))
        }
        return CharacterSemanticState(
            characterId: characterId,
            mood: CharacterMood(wire: moodKind),
            moodIntensity: json["mood"]?["intensity"]?.doubleValue ?? 0,
            activity: CharacterActivity(wire: activity),
            truth: CharacterTruth(wire: truth),
            reducedMotion: json["reducedMotion"]?.boolValue ?? false,
            members: members,
            lastInteractionKind: json["lastInteraction"]?["kind"]?.stringValue)
    }
}

// MARK: - Behavior Intent（§5）

/// 1.0 的四個 intent。本 App 全部宣告支援；未知的一律 `unsupported`，不假裝播過。
enum BehaviorIntent: String, CaseIterable, Equatable {
    case reactHappilyToTouch = "react-happily-to-touch"
    case celebrate = "celebrate"
    case settle = "settle"
    case idle = "idle"

    /// 本地動畫長度（毫秒）。播完才回 `observed`。
    var playbackMs: Int {
        switch self {
        case .reactHappilyToTouch: return 450
        case .celebrate: return 700
        case .settle, .idle: return 300
        }
    }
}

/// 正在本地播放的一個 intent（供 CharacterView 做動畫）。
struct PlayingIntent: Equatable {
    let messageId: String
    let intent: BehaviorIntent
    let intensity: Double
    let interruptible: Bool
}

/// 一則 Behavior Intent 在本機的呈現效果（純資料，View 只負責畫）。
///
/// 為什麼要把它拆出來：`observed` 的定義是「**呈現完成**」
/// （`docs/aip/iphone-companion.md` §4 第 4 點）。如果 Reduced Motion 把某個 intent
/// 的唯一效果整個關掉，那次播放就只剩一段 sleep，回 `observed` 等於謊稱演過
/// （誠實階梯：呈現完成才是 observed）。做成純資料，「這次到底有沒有東西可看」
/// 才會變成一句可以被測試斷言的事實，而不是埋在 View 的 `if` 裡。
struct CharacterPlaybackEffect: Equatable {
    /// 色彩效果。Reduced Motion 開啟時這是唯一可用的呈現手段（換色不是位移）。
    enum Highlight: Equatable {
        case none
        /// 慶祝：整隻換成高亮色。
        case celebrate
        /// 回應觸摸。
        case react
    }

    /// 縮放倍率（1 ＝ 不縮放）。Reduced Motion 開啟時**永遠**是 1。
    var scale: Double = 1
    var highlight: Highlight = .none

    /// 這次播放對畫面有沒有可見的變化。
    /// `settle`／`idle` 的呈現本來就是「回到靜止」，不適用這個判斷。
    var hasVisibleChange: Bool { scale != 1 || highlight != .none }

    /// §4 的表格：
    /// `react-happily-to-touch` → 一次縮放脈衝（幅度隨 intensity）；
    /// Reduced Motion 開啟 → 不縮放，**只換顏色**（所以那一格必須真的有顏色可換）。
    /// `celebrate` → 色彩閃一次（兩種模式相同）。`settle`／`idle` → 回到靜止。
    static func plan(intent: BehaviorIntent, intensity: Double, reduceMotion: Bool)
        -> CharacterPlaybackEffect
    {
        let strength = min(max(intensity, 0), 1)
        switch intent {
        case .reactHappilyToTouch:
            return CharacterPlaybackEffect(
                scale: reduceMotion ? 1 : 1 + 0.12 * strength, highlight: .react)
        case .celebrate:
            return CharacterPlaybackEffect(scale: 1, highlight: .celebrate)
        case .settle, .idle:
            return CharacterPlaybackEffect()
        }
    }
}

// MARK: - 呈現投影（可測；View 只負責畫）

/// 角色外觀的語意色調。View 再把它對應到實際顏色。
enum CharacterTone: Equatable {
    case neutral, happy, playful, proud, tired, alert, down, unknown, emergency
}

/// 要顯示給人看的一份角色狀態。**沒有**任何技術詞。
struct CharacterPresentation: Equatable {
    /// 主標題（活動或安全狀態）。
    var headline: String
    /// 次要說明（真相；沒有就不顯示）。
    var detail: String?
    var tone: CharacterTone
    /// 綠色勾號：**只有** `truth == verified` 才是 true。
    var showsVerifiedCheck: Bool
    /// 緊急停止：固定文案，不得改寫或淡化。
    var isEmergency: Bool
    /// 資料來源是不是角色同步（false＝退回舊的 `character.present` 路徑）。
    var fromSession: Bool

    /// 緊急停止的固定文案。
    static let emergencyText = "緊急停止中"

    /// 語意狀態 ＋ 舊路徑狀態 → 一份呈現。
    ///
    /// 規則：
    /// 1. 已協商且有語意狀態時以語意狀態為準（舊的 `character.present` 只是 hint）。
    /// 2. **緊急停止取兩邊的聯集**：任一邊說緊急就是緊急。安全訊息只能加嚴，
    ///    不能因為另一條路徑還沒更新就被淡化。
    /// 3. 綠色勾號只在語意狀態的 `truth == verified`（或舊路徑的 verified-success）出現。
    static func resolve(
        session: CharacterSemanticState?,
        negotiated: Bool,
        legacy: CharacterPresentState
    ) -> CharacterPresentation {
        let legacyEmergency = legacy == .emergency
        guard negotiated, let session else {
            return CharacterPresentation(
                headline: legacyHeadline(legacy),
                detail: nil,
                tone: legacyTone(legacy),
                showsVerifiedCheck: legacy == .verifiedSuccess,
                isEmergency: legacyEmergency,
                fromSession: false)
        }
        let emergency = legacyEmergency || session.truth == .emergency
        let tone: CharacterTone = emergency ? .emergency : tone(for: session.mood)
        return CharacterPresentation(
            headline: emergency ? emergencyText : session.activity.text,
            detail: emergency ? nil : session.truth.text,
            tone: tone,
            showsVerifiedCheck: !emergency && session.truth == .verified,
            isEmergency: emergency,
            fromSession: true)
    }

    private static func tone(for mood: CharacterMood) -> CharacterTone {
        switch mood {
        case .neutral: return .neutral
        case .happy: return .happy
        case .playful: return .playful
        case .proud: return .proud
        case .tired: return .tired
        case .alert: return .alert
        case .down: return .down
        case .unknown: return .unknown
        }
    }

    private static func legacyHeadline(_ state: CharacterPresentState) -> String {
        switch state {
        case .idle: return "待機"
        case .working: return "工作中"
        case .waiting: return "等待中"
        case .verifiedSuccess: return "已驗證成功"
        case .failed: return "失敗"
        case .unknown: return "未知"
        case .emergency: return emergencyText
        }
    }

    private static func legacyTone(_ state: CharacterPresentState) -> CharacterTone {
        switch state {
        case .idle: return .neutral
        case .working: return .alert
        case .waiting: return .tired
        case .verifiedSuccess: return .proud
        case .failed: return .down
        case .unknown: return .unknown
        case .emergency: return .emergency
        }
    }
}

// MARK: - 同步狀態（`docs/aip/character-session.md` §11 的人話）

/// 一行給人看的同步狀態。**不得**出現 revision／sequence／epoch 之類的技術詞。
enum SessionSyncStatus: Equatable {
    /// 沒有連線：角色頁顯示的東西可能不是桌面的即時狀態。
    case offline
    /// 已連線但還沒協商（例如桌面版本較舊，沒有角色同步）。
    case notNegotiated
    /// 一切正常。
    case synced
    /// 協商後有 intent 被標成 unsupported。
    case partialCapabilities
    /// 正在補齊落後的狀態。
    case resuming
    /// 連續補齊失敗。
    case unrecoverable

    var text: String {
        switch self {
        case .offline: return "未連線，角色狀態可能不是最新的"
        case .notNegotiated: return "這台桌面尚未提供角色同步"
        case .synced: return "已連接桌面，角色狀態已同步"
        case .partialCapabilities: return "部分能力目前不可用"
        case .resuming: return "同步尚未完成"
        case .unrecoverable: return "無法恢復，請重新連接"
        }
    }
}
