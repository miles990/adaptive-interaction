//
//  CanonicalVectorsTests.swift
//  InteractionCompanionTests
//
//  三端共用的 **canonical 向量**：`crates/interaction-aip/tests/fixtures/manifest.json` 的
//  `canonicalVectors` 段（由 `scripts/aip-codegen.mjs` 內嵌成 `AIPFixtures`）。
//
//  `StateHashConformanceTests` 跑的是真的 `SemanticState`——鍵全是 ASCII 欄位名，所以在
//  「鍵序」這件事上它其實什麼都沒證明：ASCII 鍵在 UTF-8 位元組序、Unicode 排序、UTF-16
//  code unit 序底下長得一模一樣。這一份補上的是非 ASCII 鍵、補充平面鍵、需要跳脫的鍵與值，
//  以及數字字面的邊界。
//
//  權威值由 Rust 的 `crates/interaction-aip/tests/canonical_vectors.rs` 產生。對不上就是
//  App 這一側的 canonical 實作有漏洞——後果是「hash 不符 → 要 snapshot」的無限迴圈，
//  不是一個顯眼的錯誤。
//
//  這裡刻意走 `SemanticJSON` 的逐字掃描器（不先過 `JSONSerialization`）：數字字面
//  （`1.0`／`-0.0`／`1e+16`）一旦被 Double 洗過就再也寫不回去，而那正是 App 收到 wss
//  訊息時走的那一條路徑。
//

import XCTest

@testable import InteractionCompanion

final class CanonicalVectorsTests: XCTestCase {

    /// manifest 的 `canonicalVectors` 段。
    private func canonicalVectors() throws -> [SemanticJSON] {
        let manifest = try XCTUnwrap(
            SemanticJSON.parse(AIPFixtures.manifest), "manifest.json 解析失敗")
        return try XCTUnwrap(
            manifest["canonicalVectors"]?.arrayValue, "manifest.json 缺 `canonicalVectors` 段")
    }

    /// 逐筆比對 canonical 文字與 SHA-256。
    func testEveryCanonicalVectorMatchesTheRustOutput() throws {
        let vectors = try canonicalVectors()
        XCTAssertGreaterThanOrEqual(
            vectors.count, 8, "canonical 向量至少要 8 筆；少於這個數字表示 codegen 沒重生")

        var seen = Set<String>()
        for vector in vectors {
            let id = try XCTUnwrap(vector["id"]?.stringValue)
            let input = try XCTUnwrap(vector["input"], "\(id) 缺 input")
            let wantCanonical = try XCTUnwrap(vector["canonical"]?.stringValue, "\(id) 缺 canonical")
            let wantHash = try XCTUnwrap(vector["sha256"]?.stringValue, "\(id) 缺 sha256")

            XCTAssertEqual(
                input.canonicalJSON, wantCanonical,
                "\(id)：Swift 的 canonical 文字與 Rust 不同（鍵序／跳脫／數字字面）")
            XCTAssertEqual(
                input.canonicalSHA256, wantHash,
                "\(id)：Swift 與 Rust 對同一份輸入算出不同的 SHA-256")
            seen.insert(wantHash)
        }
        XCTAssertEqual(seen.count, vectors.count, "每一筆向量的 sha256 都必須不同")
    }

    /// 鍵序是 **UTF-8 位元組序（＝ code point 序）**，不是 UTF-16 code unit 序。
    ///
    /// 對抗審查 `hash-numeric-contract-017`：TypeScript 端曾經用 UTF-16 code unit 序，
    /// 補充平面鍵（代理對開頭 0xD800）會排到 U+F801..U+FFFF 的 BMP 鍵之前。Swift 的
    /// `String` 預設 `<` 也不是位元組序，所以 `canonicalJSON` 明確比 `Array(key.utf8)`。
    func testKeyOrderIsCodePointOrderNotUTF16Order() throws {
        let vector = try XCTUnwrap(
            try canonicalVectors().first { $0["id"]?.stringValue == "code-point-order-not-utf16" },
            "manifest 缺 code-point-order-not-utf16 向量")
        let input = try XCTUnwrap(vector["input"]?.objectValue)
        let keys = Array(input.keys)

        let byCodePoint = keys.sorted { Array($0.utf8).lexicographicallyPrecedes(Array($1.utf8)) }
        let byUTF16 = keys.sorted { Array($0.utf16).lexicographicallyPrecedes(Array($1.utf16)) }
        XCTAssertNotEqual(
            byCodePoint, byUTF16, "這筆向量沒有分開兩種排序，等於證明不了任何事")

        // 向量的值就是 code point 序的位置：排錯了會直接看出錯在第幾格。
        let positions = byCodePoint.compactMap { input[$0]?.uintValue }
        XCTAssertEqual(
            positions, (0..<UInt64(byCodePoint.count)).map { $0 },
            "canonical 的鍵序不是 code point 序")
    }

    /// U+2028／U+2029／U+007F／U+00A0／`/` 在 serde_json 是**不跳脫**的。
    ///
    /// 這些字元在 manifest 檔案裡是以 `\uXXXX` 寫的（Swift 原始碼裡不能有裸的 U+2028），
    /// 解析回來之後必須是那個字元本身，canonical 文字裡也必須是原樣。
    func testPassthroughCharactersAreNotEscaped() throws {
        let vector = try XCTUnwrap(
            try canonicalVectors().first { $0["id"]?.stringValue == "unescaped-passthrough" },
            "manifest 缺 unescaped-passthrough 向量")
        let text = try XCTUnwrap(vector["input"]).canonicalJSON

        for scalar: Unicode.Scalar in ["\u{2028}", "\u{2029}", "\u{007F}", "\u{00A0}", "/"] {
            XCTAssertTrue(
                text.unicodeScalars.contains(scalar),
                "U+\(String(format: "%04X", scalar.value)) 必須原樣出現在 canonical 文字裡")
        }
        XCTAssertFalse(text.contains("\\u2028"), "U+2028 不得被跳脫")
        XCTAssertFalse(text.contains("\\/"), "`/` 不得被跳脫")
    }

    /// 控制字元的跳脫形式：`\b \t \n \f \r` 用短寫，其餘 < U+0020 用**小寫** `\u00xx`。
    func testControlCharactersUseSerdeJsonEscapes() throws {
        let vector = try XCTUnwrap(
            try canonicalVectors().first { $0["id"]?.stringValue == "escaped-keys-and-values" },
            "manifest 缺 escaped-keys-and-values 向量")
        let text = try XCTUnwrap(vector["input"]).canonicalJSON

        for expected in ["\\b", "\\t", "\\n", "\\f", "\\r", "\\\"", "\\\\", "\\u0000", "\\u001f"] {
            XCTAssertTrue(text.contains(expected), "canonical 文字缺跳脫形式 \(expected)")
        }
        XCTAssertFalse(text.contains("\\u001F"), "\\u00xx 必須是小寫十六進位")
    }

    /// 數字字面逐字保留：整數沒有小數點、f64 有（或是 `1e+16` 這種指數形）。
    ///
    /// Swift 這一端不需要知道哪些欄位是 f64（`SemanticJSON` 存的是原始字面），但**必須**
    /// 不要多做正規化——把 `1e+16` 讀成 Double 再印回去就會變成別的字串。
    func testNumberLiteralsArePreservedVerbatim() throws {
        let vectors = try canonicalVectors()
        let integers = try XCTUnwrap(
            vectors.first { $0["id"]?.stringValue == "numbers-integers" },
            "manifest 缺 numbers-integers 向量")
        let exponents = try XCTUnwrap(
            vectors.first { $0["id"]?.stringValue == "numbers-exponent-forms" },
            "manifest 缺 numbers-exponent-forms 向量")

        let integerText = try XCTUnwrap(integers["input"]).canonicalJSON
        XCTAssertTrue(integerText.contains("\"max-safe\":9007199254740992"), "整數不得帶小數點")
        XCTAssertFalse(integerText.contains("9007199254740992.0"), "整數不得被寫成 f64")

        let exponentText = try XCTUnwrap(exponents["input"]).canonicalJSON
        XCTAssertTrue(exponentText.contains("1e+16"), "缺 k = 16 的科學記號")
        XCTAssertTrue(exponentText.contains("1e-6"), "缺 k = -6 的科學記號")
        XCTAssertTrue(exponentText.contains("0.00001"), "缺 k = -5 的固定小數")
        XCTAssertTrue(exponentText.contains("1000000000000000.0"), "缺 k = 15 的固定小數")
    }
}
