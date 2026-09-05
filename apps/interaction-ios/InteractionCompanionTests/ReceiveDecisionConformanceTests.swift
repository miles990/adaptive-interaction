//
//  ReceiveDecisionConformanceTests.swift
//  InteractionCompanionTests
//
//  AIP 1.0 接收端決策表的跨語言 conformance（`docs/aip/character-session.md` §7.2）。
//
//  這個檔案是 `receiveDecisions` fixture 的**第三個消費者**：Rust
//  （`crates/interaction-session/tests/receive_decisions_from_json.rs`）與 TypeScript
//  讀同一段、對同一個 `expect` 交答案。只讀 JSON、不認識任何產生器——證明的是
//  「別的語言照著這份檔案做，會得到同一個結論」。
//
//  **任何案例都不得跳過**：解析不出來就是失敗，不是 skip；最後還會斷言真的逐筆跑過。
//
//  下半部是客戶端（`SessionClient`）照著同一張表走的行為測試：resume 中途失敗只保留
//  前綴、snapshot 缺 hash 一律不套用、舊連線世代的遲到品先被丟掉。
//

import XCTest

@testable import InteractionCompanion

final class ReceiveDecisionConformanceTests: XCTestCase {

    // MARK: - fixture 解析（只讀 JSON）

    private func manifestObject() throws -> [String: Any] {
        let text = AIPFixtures.manifest
        XCTAssertFalse(text.isEmpty, "codegen 沒有內嵌 manifest.json")
        let data = try XCTUnwrap(text.data(using: .utf8))
        return try XCTUnwrap(
            try JSONSerialization.jsonObject(with: data) as? [String: Any],
            "manifest.json 必須是 JSON 物件")
    }

    private func receiveDecisionCases() throws -> [[String: Any]] {
        let manifest = try manifestObject()
        return try XCTUnwrap(
            manifest["receiveDecisions"] as? [[String: Any]],
            "manifest.json 缺少 receiveDecisions 段")
    }

    private func u64(_ value: Any?) -> UInt64? {
        guard let number = value as? NSNumber else { return nil }
        // JSON 的非負整數：負數與浮點都不是合法的 revision／epoch／世代。
        guard number.doubleValue >= 0, number.doubleValue == number.doubleValue.rounded() else {
            return nil
        }
        return number.uint64Value
    }

    private func requiredU64(_ container: [String: Any], _ key: String, _ id: String) throws
        -> UInt64
    {
        try XCTUnwrap(u64(container[key]), "\(id)：缺少非負整數欄位 `\(key)`")
    }

    private func view(_ raw: [String: Any], _ id: String) throws -> SessionReceiverView {
        SessionReceiverView(
            hasState: try XCTUnwrap(raw["hasState"] as? Bool, "\(id)：local 缺 hasState"),
            sessionId: raw["sessionId"] as? String,
            epoch: try requiredU64(raw, "epoch", id),
            revision: try requiredU64(raw, "revision", id),
            stateHash: raw["hash"] as? String,
            connectionGeneration: try requiredU64(raw, "connectionGeneration", id))
    }

    private func incoming(_ raw: [String: Any], _ id: String) throws -> SessionIncomingState {
        let kind = try XCTUnwrap(
            SessionStateKind.parse(raw["kind"] as? String), "\(id)：incoming 的 kind 不認得")
        return SessionIncomingState(
            kind: kind,
            sessionId: raw["sessionId"] as? String,
            epoch: try requiredU64(raw, "epoch", id),
            revision: try requiredU64(raw, "revision", id),
            baseRevision: u64(raw["baseRevision"]),
            reason: raw["reason"] as? String,
            hash: raw["hash"] as? String,
            computedHash: raw["computedHash"] as? String,
            statePresent: raw["statePresent"] as? Bool ?? false,
            arrivedOnGeneration: try requiredU64(raw, "arrivedOnGeneration", id),
            viaAuthoritativeReply: raw["viaAuthoritativeReply"] as? Bool ?? false)
    }

    /// `incomingBatchChain`：從本地 revision 起連續 `count` 則 patch
    /// （用來釘住 `maxResumePatches` 的邊界，不必把幾百則訊息寫進檔案）。
    private func chain(_ spec: [String: Any], view: SessionReceiverView, _ id: String) throws
        -> [SessionIncomingState]
    {
        XCTAssertEqual(spec["kind"] as? String, "patch", "\(id)：目前只支援 patch 鏈")
        let count = Int(try requiredU64(spec, "count", id))
        return (0..<count).map { index in
            let base = view.revision + UInt64(index)
            return SessionIncomingState(
                kind: .patch,
                sessionId: view.sessionId,
                epoch: view.epoch,
                revision: base + 1,
                baseRevision: base,
                reason: nil,
                hash: nil,
                computedHash: nil,
                statePresent: false,
                arrivedOnGeneration: view.connectionGeneration,
                viaAuthoritativeReply: false)
        }
    }

    // MARK: - 逐筆對答案

    func testEveryReceiveDecisionFixtureReachesTheDocumentedDecision() throws {
        let entries = try receiveDecisionCases()
        XCTAssertGreaterThanOrEqual(
            entries.count, 32, "receiveDecisions 至少要 32 個具名案例，實際 \(entries.count)")

        var exercised = 0
        for entry in entries {
            let id = try XCTUnwrap(entry["id"] as? String, "案例缺少 id")
            let local = try view(
                try XCTUnwrap(entry["local"] as? [String: Any], "\(id)：缺 local"), id)
            let expect = try XCTUnwrap(entry["expect"] as? [String: Any], "\(id)：缺 expect")

            // 進來之前已經連續失敗幾次（有界 realign）。
            var budget = SessionRealignBudget()
            for _ in 0..<Int(u64(entry["budgetBefore"]) ?? 0) {
                budget = budget.observing(
                    .realign(reason: .epochChanged), viaAuthoritativeReply: true)
            }

            let decision: SessionReceiveDecision
            let after: SessionReceiverView
            var applied: Int?
            var skipped: Int?
            var stoppedAt: Int?
            let viaAuthoritative: Bool

            if let single = entry["incoming"] as? [String: Any] {
                let message = try incoming(single, id)
                decision = SessionDecisions.decideReceive(view: local, incoming: message)
                after = SessionDecisions.advance(
                    view: local, incoming: message, decision: decision)
                viaAuthoritative = message.viaAuthoritativeReply
            } else {
                let items: [SessionIncomingState]
                if let batch = entry["incomingBatch"] as? [[String: Any]] {
                    items = try batch.map { try incoming($0, id) }
                } else if let spec = entry["incomingBatchChain"] as? [String: Any] {
                    items = try chain(spec, view: local, id)
                } else {
                    XCTFail("\(id)：案例既沒有 incoming 也沒有 incomingBatch")
                    continue
                }
                let batch = SessionDecisions.decideResumeBatch(view: local, items: items)
                decision = batch.outcome
                after = batch.view
                applied = batch.applied
                skipped = batch.skipped
                stoppedAt = batch.stoppedAt
                viaAuthoritative = true
            }

            XCTAssertEqual(
                decision.wireName, expect["decision"] as? String, "\(id)：決策不同")
            XCTAssertEqual(
                decision.realignReason?.rawValue, expect["reason"] as? String,
                "\(id)：realign 原因不同")
            XCTAssertEqual(
                after.revision, try requiredU64(expect, "revisionAfter", id),
                "\(id)：套用後的 revision 不同")
            XCTAssertEqual(
                after.epoch, try requiredU64(expect, "epochAfter", id),
                "\(id)：套用後的 epoch 不同")
            if let applied {
                XCTAssertEqual(
                    UInt64(applied), try requiredU64(expect, "applied", id), "\(id)：套用筆數不同")
            }
            if let skipped {
                XCTAssertEqual(
                    UInt64(skipped), try requiredU64(expect, "skipped", id), "\(id)：跳過筆數不同")
            }
            XCTAssertEqual(
                stoppedAt.map(UInt64.init), u64(expect["stoppedAt"]), "\(id)：中止位置不同")

            budget = budget.observing(decision, viaAuthoritativeReply: viaAuthoritative)
            XCTAssertEqual(
                UInt64(budget.attempts), try requiredU64(expect, "budgetAfter", id),
                "\(id)：realign 預算不同")
            XCTAssertEqual(
                budget.isUnrecoverable ? "unrecoverable" : "ok", expect["budget"] as? String,
                "\(id)：realign 預算的結論不同")

            // 不採用狀態的決策**不得**動到本地副本（這條規則值得單獨釘死）。
            if !decision.adoptsState, entry["incoming"] != nil {
                XCTAssertEqual(after, local, "\(id)：不採用的決策不得改變本地狀態")
            }
            exercised += 1
        }
        XCTAssertEqual(exercised, entries.count, "每一個案例都要跑到，不得跳過")
    }

    /// fixture 裡的 hash 必須是真的 SHA-256 十六進位字串（別的語言可以自己重算來核對）。
    func testTheHashesInTheFixturesLookLikeSha256() throws {
        var seen = 0
        for entry in try receiveDecisionCases() {
            let local = entry["local"] as? [String: Any]
            let single = entry["incoming"] as? [String: Any]
            for hash in [
                local?["hash"] as? String, single?["hash"] as? String,
                single?["computedHash"] as? String,
            ].compactMap({ $0 }) {
                XCTAssertEqual(hash.count, 64, "hash 不是 sha-256 hex：\(hash)")
                XCTAssertTrue(
                    hash.allSatisfy { $0.isHexDigit && !$0.isUppercase },
                    "hash 不是小寫十六進位：\(hash)")
                seen += 1
            }
        }
        XCTAssertGreaterThanOrEqual(seen, 32, "案例裡的 hash 太少（\(seen)）")
    }

    /// 上限是 codegen 從 golden schema 讀來的，不得在 Swift 這一端手寫。
    func testTheBoundsComeFromTheGeneratedLimits() {
        XCTAssertEqual(AIPLimits.maxResumePatches, 512)
        XCTAssertEqual(AIPLimits.maxRealignAttempts, 3)
        XCTAssertEqual(SessionSyncLocal.resumeFailureLimit, AIPLimits.maxRealignAttempts)
    }

    // MARK: - 客戶端行為（同一張表，走真的收訊路徑）

    /// resume 回覆中途接不上：前面的照樣套用，斷點之後的**一則都不套用**，並再要一次對齊。
    @MainActor
    func testAResumeReplyThatFailsHalfwayKeepsThePrefixAndAsksAgain() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()

        // 三則 patch：第 1 則接得上（10 → 11）、第 2 則的 base 是 12（接不上 11）、第 3 則接在後面。
        try feed(
            client,
            resumeReply(
                patches: [
                    patchItem(revision: 11, baseRevision: 10, sequence: 11, epoch: 5),
                    patchItem(revision: 13, baseRevision: 12, sequence: 13, epoch: 5),
                    patchItem(revision: 14, baseRevision: 13, sequence: 14, epoch: 5),
                ]))

        XCTAssertEqual(client.advanced.revision, 11, "第 1 則必須已經套用")
        XCTAssertEqual(client.advanced.appliedStates, 2, "bootstrap ＋ 第 1 則 ＝ 2 則套用")
        XCTAssertEqual(transport.resumes.count, 1, "斷點必須觸發一次重新對齊")
        XCTAssertNotEqual(client.syncStatus, .synced, "一則沒套用就不得宣稱已同步")
    }

    /// 缺 hash 的 snapshot 一律不套用：AIP 1.0 的 snapshot 必帶 hash，沒有 legacy profile。
    ///
    /// 這一關在**決策表**（規則 2），不在 typed boundary：Rust 與 TypeScript 的 boundary
    /// 都放行缺 hash 的 state envelope，Swift 多擋一分就會讓同一則訊息在三端得到不同結論
    ///（`docs/aip/conformance.md` §1 說的正是不可以這樣）。所以下面先確認 boundary **收下**
    /// 它，再確認決策表**不套用**它。
    @MainActor
    func testASnapshotWithoutAHashIsNeverApplied() throws {
        // 1) 純決策層：規則 2。
        let localView = SessionReceiverView(
            hasState: true, sessionId: "session.home", epoch: 5, revision: 10,
            stateHash: "a", connectionGeneration: 1)
        let noHash = SessionIncomingState(
            kind: .snapshot, sessionId: "session.home", epoch: 5, revision: 11,
            statePresent: true, arrivedOnGeneration: 1)
        XCTAssertEqual(
            SessionDecisions.decideReceive(view: localView, incoming: noHash), .rejectInvalid)

        // 2) 缺 state 的 snapshot 同樣是規則 2（不是「讀不出來」）。
        var noState = noHash
        noState.hash = "h11"
        noState.computedHash = "h11"
        noState.statePresent = false
        XCTAssertEqual(
            SessionDecisions.decideReceive(view: localView, incoming: noState), .rejectInvalid)

        // 3) typed boundary 與三端同一條界線：缺 hash 的 state envelope **收得下**，
        //    分歧不在這裡。
        let text = snapshotEnvelopeText(
            revision: 11, sequence: 11, epoch: 5, includeHash: false)
        guard case .success(let envelope) = AIPCheck.evaluate(Data(text.utf8)) else {
            return XCTFail("boundary 不該比 Rust／TypeScript 嚴：缺 hash 的 state envelope 應該收下")
        }

        // 4) 收訊路徑：不套用、也不把它當成「已同步」。
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()
        let ignoredBefore = client.advanced.framesIgnored
        client.handleFrame(envelope, rawFrame: frame(text))
        XCTAssertEqual(client.advanced.revision, 10, "缺 hash 的快照不得改變本地 revision")
        XCTAssertGreaterThan(client.advanced.framesIgnored, ignoredBefore, "要誠實計數")
        XCTAssertEqual(transport.resumes.count, 0, "推播上的垃圾不是「接不上」，不必再問一次")
    }

    /// 被規則 2 擋下的**權威回覆**算一次對齊失敗：對方回答了、但答案沒用。
    /// 連續三次（`AIPLimits.maxRealignAttempts`）就要照實說「無法恢復」，不再自動重試。
    @MainActor
    func testABoundaryRejectedAuthoritativeReplyCostsOneRealignAttempt() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()

        for attempt in 1...AIPLimits.maxRealignAttempts {
            try feed(client, snapshotReplyWithoutHash(revision: 11 + UInt64(attempt), epoch: 5))
            XCTAssertEqual(client.advanced.revision, 10, "沒得核對的快照不得套用")
        }
        XCTAssertEqual(
            client.syncStatus, .unrecoverable,
            "連續 \(AIPLimits.maxRealignAttempts) 次拿不到能用的權威狀態就是「狀態未知」")
    }

    /// 規則 0：舊連線世代的遲到品先被丟掉——它宣告的 epoch 一定與本地不同，
    /// 任何 epoch 判斷都會被它騙過去。
    @MainActor
    func testAStateFromAPreviousConnectionGenerationIsIgnored() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        client.connectionDidConnect(generation: 7)
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5, generation: 7)
        transport.clear()

        // 上一條連線送出的 session-reset 現在才到（epoch 9、revision 1）。
        try feed(
            client,
            snapshotEnvelopeText(
                revision: 1, sequence: 1, epoch: 9, reason: "session-reset"),
            generation: 6)
        XCTAssertEqual(client.advanced.revision, 10, "舊世代的 reset 不得改寫本地狀態")
        XCTAssertEqual(client.advanced.epoch, 5)
        XCTAssertEqual(transport.resumes.count, 0, "舊連線的遲到品不觸發對齊")
        XCTAssertGreaterThan(client.advanced.staleConnectionFrames, 0, "要誠實計數")
    }

    /// 規則 1：別的 session 的狀態不是「比較舊」，是**不相干**——不套用，也不 realign
    ///（realign 只會再要一次別人的 session）。
    @MainActor
    func testAStateFromAnotherSessionIsRejectedWithoutAskingForResume() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()

        try feed(
            client,
            snapshotEnvelopeText(
                revision: 99, sequence: 99, epoch: 5, sessionId: "session.elsewhere"))
        XCTAssertEqual(client.advanced.revision, 10, "別的 session 的狀態不得套用")
        XCTAssertEqual(transport.resumes.count, 0, "身分不符不得 realign")
    }

    /// 規則 5：epoch 不同又沒有重建宣告 → 不猜、不套用，改要求重新對齊。
    @MainActor
    func testASnapshotFromADifferentEpochRealignsInsteadOfBeingApplied() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()

        try feed(client, snapshotEnvelopeText(revision: 11, sequence: 11, epoch: 6))
        XCTAssertEqual(client.advanced.revision, 10, "epoch 不同就不是可比的順序，不得靜默改寫")
        XCTAssertEqual(client.advanced.epoch, 5)
        XCTAssertEqual(transport.resumes.count, 1, "必須改要求重新對齊")
    }

    /// resume 回覆超過 `maxResumePatches`：整批不處理，**不**靜默截斷成「我以為我追上了」。
    ///
    /// 這一條只能在純函式上驗：一則真的帶 513 則補丁的 `response` 連 typed boundary
    /// 都過不了（payload 上限 32 KiB），權威 host 也會在超限時改回 snapshot
    ///（`character_session_resume_value`）。客戶端那道 guard 因此是縱深防禦，
    /// 由 `SessionDecisions.resumeExceedsBound` 這個共用判斷式定義，兩邊讀同一個上限。
    func testAResumeReplyBeyondTheBoundIsNotSilentlyTruncated() {
        XCTAssertFalse(SessionDecisions.resumeExceedsBound(AIPLimits.maxResumePatches))
        XCTAssertTrue(SessionDecisions.resumeExceedsBound(AIPLimits.maxResumePatches + 1))

        let view = SessionReceiverView(
            hasState: true, sessionId: "session.home", epoch: 5, revision: 30,
            stateHash: "h30", connectionGeneration: 7)
        func chainItems(_ count: Int) -> [SessionIncomingState] {
            (0..<count).map { index in
                let base = view.revision + UInt64(index)
                return SessionIncomingState(
                    kind: .patch, sessionId: view.sessionId, epoch: view.epoch,
                    revision: base + 1, baseRevision: base, arrivedOnGeneration: 7)
            }
        }
        let atBound = SessionDecisions.decideResumeBatch(
            view: view, items: chainItems(AIPLimits.maxResumePatches))
        XCTAssertEqual(atBound.applied, AIPLimits.maxResumePatches, "剛好在界上要全部套用")
        XCTAssertEqual(atBound.view.revision, 30 + UInt64(AIPLimits.maxResumePatches))

        let beyond = SessionDecisions.decideResumeBatch(
            view: view, items: chainItems(AIPLimits.maxResumePatches + 1))
        XCTAssertEqual(beyond.applied, 0, "超過上限就整批不處理")
        XCTAssertEqual(beyond.halted, .realign(reason: .resumeTooLong))
        XCTAssertEqual(beyond.view, view, "整批不處理就不得動到本地狀態")
    }

    /// resume 回覆裡的 patch 是**攤平**的（`{sequence, baseRevision, revision, patch, hash,
    /// sessionEpoch}`，沒有 `kind`）。認不得它等於每一次 resume 都白跑。
    @MainActor
    func testAFlatResumePatchItemWithoutAKindIsUnderstood() throws {
        let transport = Transport()
        let client = SessionClient()
        client.transport = transport
        try negotiate(client)
        try applyBootstrapSnapshot(client, revision: 10, epoch: 5)
        transport.clear()

        try feed(
            client,
            resumeReply(patches: [
                patchItem(revision: 11, baseRevision: 10, sequence: 11, epoch: 5)
            ]))
        XCTAssertEqual(client.advanced.revision, 11, "攤平的 patch 必須讀得懂並套用")
        XCTAssertEqual(client.syncStatus, .synced)
        XCTAssertEqual(transport.resumes.count, 0, "讀得懂就不必再問一次")
    }

    // MARK: - 小工具

    /// 記錄送出訊息的 mock transport。
    private final class Transport: SessionTransport {
        var isConnected = true
        var boundDeviceId: String? = "iphone-87b42264"
        private(set) var sent: [AIPEnvelope] = []

        @discardableResult
        func sendAip(_ envelope: AIPEnvelope) -> Bool {
            sent.append(envelope)
            return true
        }

        func sendObservation(receptor: String, facts: [String: JSONValue]) {}
        func sendStatusNow() {}

        func clear() { sent.removeAll() }
        var resumes: [AIPEnvelope] { sent.filter { $0.name == SessionNames.resume } }
        var snapshotQueries: [AIPEnvelope] {
            sent.filter { $0.name == SessionNames.snapshot && $0.messageType == .query }
        }
    }

    private func rawEnvelope(_ text: String) throws -> AIPEnvelope {
        switch AIPCheck.evaluate(Data(text.utf8)) {
        case .success(let envelope): return envelope
        case .failure(let error):
            XCTFail("信封應該通過驗證，卻回 \(error.code.rawValue)")
            throw error
        }
    }

    private func frame(_ envelopeText: String) -> String {
        #"{"type":"aip","envelope":"# + envelopeText + "}"
    }

    @MainActor
    private func feed(
        _ client: SessionClient, _ envelopeText: String, generation: UInt64? = nil
    ) throws {
        client.handleFrame(
            try rawEnvelope(envelopeText), rawFrame: frame(envelopeText),
            arrivedOnGeneration: generation)
    }

    @MainActor
    private func negotiate(_ client: SessionClient) throws {
        try feed(
            client,
            """
            {"specVersion":"aip/1.0","messageId":"aip-neg-1","messageType":"capability",
             "name":"character.session.capability",
             "source":{"kind":"session","id":"session.home"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"session.home","occurredAt":"2026-09-04T12:30:00.000Z",
             "payload":{"specVersion":"aip/1.0","newerMinor":false,"role":"remote-renderer",
                        "syncClass":"semantic","intents":{},"inputs":[],"unsupportedInputs":[],
                        "limits":{"maxMessageBytes":65536,"maxPayloadBytes":32768,
                                  "maxIntentsPerMinute":60}}}
            """)
        XCTAssertTrue(client.negotiated, "協商失敗，後面的斷言就沒有意義")
    }

    @MainActor
    private func applyBootstrapSnapshot(
        _ client: SessionClient, revision: UInt64, epoch: UInt64, generation: UInt64? = nil
    ) throws {
        try feed(
            client,
            snapshotEnvelopeText(revision: revision, sequence: revision, epoch: epoch),
            generation: generation)
        XCTAssertEqual(client.advanced.revision, revision, "bootstrap 快照必須被套用")
    }

    /// 這一節的權威狀態文字（hash 一定對它自己算，不寫死）。
    private static let stateText = """
        {"characterId":"ref-shape","mood":{"kind":"happy","intensity":0.5},"activity":"idle",\
        "attention":{"kind":"none"},"truth":{"state":"none"},"members":[],"reducedMotion":false}
        """

    private func stateHash() -> String {
        SemanticJSON.parse(Self.stateText)?.canonicalSHA256 ?? "?"
    }

    /// - Parameter includeHash: `false` ＝ 刻意不帶 hash（用來驗證邊界會擋下來）。
    private func snapshotEnvelopeText(
        revision: UInt64,
        sequence: UInt64,
        epoch: UInt64,
        sessionId: String = "session.home",
        reason: String? = nil,
        includeHash: Bool = true
    ) -> String {
        let fields = [
            includeHash ? #""hash":"\#(stateHash())""# : nil,
            reason.map { #""reason":"\#($0)""# },
        ].compactMap { $0 }
        let extra = fields.isEmpty ? "" : "," + fields.joined(separator: ",")
        return """
            {"specVersion":"aip/1.0","messageId":"aip-st-\(epoch)-\(revision)-\(sequence)",
             "messageType":"state","name":"character.session.snapshot",
             "source":{"kind":"session","id":"\(sessionId)"},
             "target":{"kind":"device","id":"iphone-87b42264"},
             "sessionId":"\(sessionId)","occurredAt":"2026-09-04T12:30:04.000Z",
             "sequence":\(sequence),
             "payload":{"kind":"snapshot","revision":\(revision),"sessionEpoch":\(epoch)\(extra),
                        "state":\(Self.stateText)}}
            """
    }

    /// resume 回覆裡的一則攤平 patch（`transport-bindings.md` §1.3）。
    /// `patch` 是空物件：套用後的狀態不變，宣告的 hash 因此就是原狀態的 hash。
    private func patchItem(
        revision: UInt64, baseRevision: UInt64, sequence: UInt64, epoch: UInt64
    ) -> String {
        """
        {"sequence":\(sequence),"baseRevision":\(baseRevision),"revision":\(revision),
         "patch":{},"hash":"\(stateHash())","sessionEpoch":\(epoch)}
        """
    }

    /// 桌面對 resume 的回覆，內容是一份**缺 hash** 的 snapshot（規則 2 的權威回覆版）。
    private func snapshotReplyWithoutHash(revision: UInt64, epoch: UInt64) -> String {
        """
        {"specVersion":"aip/1.0","messageId":"aip-resp-nohash-\(revision)",
         "messageType":"response","name":"character.session.resume",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","occurredAt":"2026-09-04T12:30:06.000Z",
         "causationId":"aip-resume-1",
         "payload":{"kind":"snapshot","revision":\(revision),"sessionEpoch":\(epoch),
                    "state":\(Self.stateText)}}
        """
    }

    private func resumeReply(patches: [String]) -> String {
        """
        {"specVersion":"aip/1.0","messageId":"aip-resp-\(patches.count)",
         "messageType":"response","name":"character.session.resume",
         "source":{"kind":"session","id":"session.home"},
         "target":{"kind":"device","id":"iphone-87b42264"},
         "sessionId":"session.home","occurredAt":"2026-09-04T12:30:05.000Z",
         "causationId":"aip-resume-1",
         "payload":{"kind":"patches","patches":[\(patches.joined(separator: ","))]}}
        """
    }
}
