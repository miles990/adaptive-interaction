import Foundation

/// A validated DTO is for reading. `raw` remains the sole hash/patch source and
/// retains every unknown field and number literal, even if DTO decoding ignores it.
struct ValidatedSemanticState: Equatable {
    let raw: SemanticJSON
    let wire: SemanticStateWire

    static func validate(_ raw: SemanticJSON) -> ValidatedSemanticState? {
        guard let schema = SemanticStateContract.schema,
            bounded(raw, depth: 1),
            raw.canonicalJSON.utf8.count <= AIPLimits.maxPayloadBytes,
            matches(raw, schema, root: schema),
            let wire = try? JSONDecoder().decode(SemanticStateWire.self, from: Data(raw.canonicalJSON.utf8))
        else { return nil }
        return ValidatedSemanticState(raw: raw, wire: wire)
    }

    private static func bounded(_ value: SemanticJSON, depth: Int) -> Bool {
        guard depth <= AIPLimits.maxJsonDepth - 1 else { return false }
        switch value {
        case .null: return false
        case .bool: return true
        case .number(let raw): return Double(raw)?.isFinite == true
        case .string(let text): return text.unicodeScalars.count <= AIPLimits.maxStringChars
        case .array(let values): return values.allSatisfy { bounded($0, depth: depth + 1) }
        case .object(let values): return values.allSatisfy { $0.key.unicodeScalars.count <= AIPLimits.maxStringChars && bounded($0.value, depth: depth + 1) }
        }
    }

    /// Mirrors chrono's structural RFC3339 parser, including a retained second
    /// 60. Calendar/time bounds are checked before Foundation can normalize them.
    private static func validTimestamp(_ text: String) -> Bool {
        let pattern = #"^([0-9]{4})-([0-9]{2})-([0-9]{2})[Tt ]([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.[0-9]+)?(?:[Zz]|([+−-])([0-9]{2}):([0-9]{2}))$"#
        guard let expression = try? NSRegularExpression(pattern: pattern),
            let match = expression.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)),
            match.range.length == text.utf16.count else { return false }
        func number(_ index: Int) -> Int? {
            guard let range = Range(match.range(at: index), in: text) else { return nil }
            return Int(text[range])
        }
        guard let year = number(1), let month = number(2), let day = number(3),
            let hour = number(4), let minute = number(5), let second = number(6) else { return false }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
        let days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        guard (1...12).contains(month), day >= 1, day <= days[month - 1],
            hour <= 23, minute <= 59, second <= 60 else { return false }
        if let offsetHour = number(8) {
            guard let offsetMinute = number(9), offsetHour <= 23, offsetMinute <= 59 else { return false }
        }
        return true
    }

    private static func matches(_ value: SemanticJSON, _ rule: SemanticJSON, root: SemanticJSON) -> Bool {
        if let reference = rule["$ref"]?.stringValue {
            guard let name = reference.split(separator: "/").last,
                let target = root["$defs"]?[String(name)] else { return false }
            return matches(value, target, root: root)
        }
        if let branches = rule["oneOf"]?.arrayValue, branches.filter({ matches(value, $0, root: root) }).count != 1 { return false }
        if let branches = rule["anyOf"]?.arrayValue, !branches.contains(where: { matches(value, $0, root: root) }) { return false }
        if let negative = rule["not"], matches(value, negative, root: root) { return false }
        if let expected = rule["const"], value != expected { return false }
        if let values = rule["enum"]?.arrayValue, !values.contains(value) { return false }
        switch rule["type"]?.stringValue {
        case "object":
            guard let object = value.objectValue else { return false }
            if let required = rule["required"]?.arrayValue,
                required.contains(where: { object[$0.stringValue ?? ""] == nil }) { return false }
            for (key, child) in rule["properties"]?.objectValue ?? [:] {
                if let field = object[key], !matches(field, child, root: root) { return false }
            }
        case "array":
            guard let array = value.arrayValue else { return false }
            if let max = rule["maxItems"]?.uintValue, array.count > max { return false }
            if let item = rule["items"], !array.allSatisfy({ matches($0, item, root: root) }) { return false }
        case "string":
            guard let text = value.stringValue else { return false }
            if rule["format"]?.stringValue == "date-time" {
                if !validTimestamp(text) { return false }
            }
        case "boolean": if value.boolValue == nil { return false }
        case "number", "integer":
            guard let number = value.doubleValue, number.isFinite else { return false }
            if rule["type"]?.stringValue == "integer", number.rounded() != number { return false }
            if let minimum = rule["minimum"]?.doubleValue, number < minimum { return false }
            if let maximum = rule["maximum"]?.doubleValue, number > maximum { return false }
            if rule["x-nonNegativeZero"]?.boolValue == true, number.sign == .minus { return false }
        default: break
        }
        return true
    }
}
