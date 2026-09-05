//
//  StateHashConformanceTests.swift
//  InteractionCompanionTests
//
//  三端共用的 **state hash conformance**：`crates/interaction-aip/tests/fixtures/manifest.json`
//  的 `stateHashes` 段（由 `scripts/aip-codegen.mjs` 內嵌成 `AIPFixtures`）。
//
//  同一份 `state`，Rust（`interaction_session::state_hash`）、TypeScript 與 Swift 必須算出
//  同一個 SHA-256。算不出來就是這一端的 canonical 實作有漏洞，不是 fixture 的問題——
//  App 這一側算錯的後果是「hash 不符 → 要 snapshot」的無限迴圈，而不是一個顯眼的錯誤。
//
//  這裡刻意從 **fixture 的原始文字**用 `SemanticJSON` 的逐字掃描器解析，不先過
//  `JSONSerialization`：後者把 `0.0` 讀成 `Double` 之後就再也寫不回 `0.0`（會寫 `0`），
//  數字字面一丟失，hash 就永遠對不上。這條路徑正是 App 收到 wss 訊息時走的那一條。
//

import XCTest

@testable import InteractionCompanion

final class StateHashConformanceTests: XCTestCase {

    /// manifest 的 `stateHashes` 索引（id → 檔名）。
    private func stateHashEntries() throws -> [SemanticJSON] {
        let manifest = try XCTUnwrap(
            SemanticJSON.parse(AIPFixtures.manifest), "manifest.json 解析失敗")
        return try XCTUnwrap(
            manifest["stateHashes"]?.arrayValue, "manifest.json 缺 `stateHashes` 段")
    }

    /// 9 個情境（含亂序輸入、unicode 與 `-0.0`）逐一比對 canonical 文字與 SHA-256。
    func testEveryStateHashFixtureMatchesTheRustCanonicalHash() throws {
        let entries = try stateHashEntries()
        XCTAssertGreaterThanOrEqual(
            entries.count, 9, "stateHashes 至少要涵蓋 9 個情境；少於這個數字表示 codegen 沒重生")

        var hashesByID: [String: String] = [:]
        for entry in entries {
            let id = try XCTUnwrap(entry["id"]?.stringValue)
            let file = try XCTUnwrap(entry["file"]?.stringValue)
            let text = try XCTUnwrap(AIPFixtures.files[file], "缺少內嵌 fixture \(file)")
            // 逐字掃描器直接吃原始文字：數字字面（`0.0`／`1.0`／`-0.0`）原樣保留。
            let doc = try XCTUnwrap(SemanticJSON.parse(text), "\(file) 逐字解析失敗")
            let state = try XCTUnwrap(doc["state"], "\(file) 沒有 state")
            let wantCanonical = try XCTUnwrap(doc["canonical"]?.stringValue)
            let wantHash = try XCTUnwrap(doc["hash"]?.stringValue)

            XCTAssertEqual(
                state.canonicalJSON, wantCanonical,
                "\(file)：Swift 的 canonical 文字與 Rust 不同（鍵序／跳脫／數字字面）")
            XCTAssertEqual(
                state.canonicalSHA256, wantHash,
                "\(file)：Swift 與 Rust 對同一份 state 算出不同的 SHA-256")
            hashesByID[id] = wantHash
        }

        // 「同一份 state、不同的輸入排版」必須收斂到同一個 hash——消費端得自己排序、去空白。
        XCTAssertEqual(
            hashesByID["fresh"], hashesByID["unsorted-input"],
            "亂序、帶空白的同一份 state 必須算出同一個 hash")
        // 其餘情境彼此不得碰撞（否則這組 fixture 證明不了任何事）。
        XCTAssertEqual(
            Set(hashesByID.values).count, hashesByID.count - 1,
            "除了 fresh／unsorted-input 這一對，每個情境的 hash 都必須不同")
    }

    /// `-0.0`：canonical 文字是 `-0.0`，與 `0.0` 是**不同**的 hash。
    ///
    /// host 永不產生它（`clamp_unit` 正規化、`restore` 拒絕，見 `crates/interaction-session/src/state.rs`），
    /// 但萬一在線上遇到，App 也必須跟 Rust 得到同一個答案，而不是各算各的。
    func testNegativeZeroIntensityHashesDifferentlyFromPositiveZero() throws {
        let negative = try XCTUnwrap(
            AIPFixtures.files["state-hash-intensity-negative-zero.json"])
        let fresh = try XCTUnwrap(AIPFixtures.files["state-hash-fresh.json"])
        let negativeState = try XCTUnwrap(SemanticJSON.parse(negative)?["state"])
        let freshState = try XCTUnwrap(SemanticJSON.parse(fresh)?["state"])

        XCTAssertTrue(
            negativeState.canonicalJSON.contains(#""intensity":-0.0"#),
            "負零的字面必須原樣保留")
        XCTAssertTrue(
            freshState.canonicalJSON.contains(#""intensity":0.0"#),
            "正零的字面是 `0.0`，不是 `0`")
        XCTAssertNotEqual(negativeState.canonicalSHA256, freshState.canonicalSHA256)
        // 值相同、字面不同：這正是為什麼 host 端要把兩個零收斂成 `0.0`。
        XCTAssertEqual(negativeState["mood"]?["intensity"]?.doubleValue, 0)
        XCTAssertEqual(freshState["mood"]?["intensity"]?.doubleValue, 0)
    }

    /// manifest 標成 `semanticValid: false` 的 fixture 只有負零那一個——它是「hash 算得出來、
    /// 但 host 不接受」的唯一情境。多出來的話表示契約變了，三端都要重新對答案。
    func testOnlyTheNegativeZeroFixtureIsMarkedSemanticallyInvalid() throws {
        let invalid = try stateHashEntries()
            .filter { $0["semanticValid"]?.boolValue == false }
            .compactMap { $0["id"]?.stringValue }
        XCTAssertEqual(invalid, ["intensity-negative-zero"])
    }
}
