import XCTest
@testable import InteractionCompanion

final class SemanticStateConformanceTests: XCTestCase {
    private func corpus() throws -> SemanticJSON {
        try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(AIPFixtures.files["semantic-state-cases.json"])))
    }
    func testSharedStateAcceptanceCanonicalHashProjectionAndActualAdoption() throws {
        let entries = try XCTUnwrap(try corpus()["states"]?.arrayValue)
        XCTAssertGreaterThanOrEqual(entries.count, 26)
        for entry in entries {
            let id = try XCTUnwrap(entry["id"]?.stringValue)
            let raw = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(entry["wire"]?.stringValue)))
            let accepts = try XCTUnwrap(entry["accept"]?.boolValue)
            let validated = ValidatedSemanticState.validate(raw)
            XCTAssertEqual(validated != nil, accepts, id)
            XCTAssertEqual(raw.canonicalJSON, entry["canonical"]?.stringValue, id)
            XCTAssertEqual(raw.canonicalSHA256, entry["hash"]?.stringValue, id)
            let message = SessionStateMessage(kind: .snapshot(state: raw), revision: 1, epoch: 1, hash: raw.canonicalSHA256, sessionId: "session.home")
            let result = SessionDecisions.apply(message, to: SessionSyncLocal())
            XCTAssertEqual(result.decision, accepts ? .apply : .rejectInvalid, id)
            if let validated, let expected = entry["projection"] {
                let projected = CharacterSemanticState.project(validated)
                XCTAssertEqual(projected.characterId, expected["characterId"]?.stringValue, id)
                XCTAssertEqual(projected.mood, CharacterMood(wire: try XCTUnwrap(expected["mood"]?.stringValue)), id)
                XCTAssertEqual(projected.moodIntensity, expected["moodIntensity"]?.doubleValue, id)
                XCTAssertEqual(projected.activity, CharacterActivity(wire: try XCTUnwrap(expected["activity"]?.stringValue)), id)
                XCTAssertEqual(projected.truth, CharacterTruth(wire: try XCTUnwrap(expected["truth"]?.stringValue)), id)
                XCTAssertEqual(projected.reducedMotion, expected["reducedMotion"]?.boolValue, id)
                XCTAssertEqual(projected.lastInteractionKind, expected["lastInteractionKind"]?.stringValue, id)
            }
        }
    }
    func testSharedMergePatchResultsAreValidatedBeforeAdoption() throws {
        for entry in try XCTUnwrap(try corpus()["patches"]?.arrayValue) {
            let id = try XCTUnwrap(entry["id"]?.stringValue)
            let base = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(entry["baseWire"]?.stringValue)))
            let patch = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(entry["patchWire"]?.stringValue)))
            let merged = SemanticJSON.mergePatch(base, patch)
            XCTAssertEqual(merged.canonicalJSON, entry["canonical"]?.stringValue, id)
            XCTAssertEqual(merged.canonicalSHA256, entry["hash"]?.stringValue, id)
            var local = SessionSyncLocal()
            local.state = base; local.stateHash = base.canonicalSHA256; local.revision = 1; local.epoch = 1
            let message = SessionStateMessage(kind: .patch(patch: patch, baseRevision: 1), revision: 2, epoch: 1, hash: merged.canonicalSHA256)
            let outcome = SessionDecisions.apply(message, to: local)
            XCTAssertEqual(outcome.decision, entry["accept"]?.boolValue == true ? .apply : .rejectInvalid, id)
            XCTAssertEqual(local.state, base, "input state must not be changed")
        }
    }
    func testReleasedSnapshotAndPatchRemainCompatible() throws {
        var contents: [String: SemanticJSON] = [:]
        let prefix = "releases/v0.7.0/"
        let index = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(AIPFixtures.files[prefix + "manifest.json"])))
        XCTAssertEqual(index["sourceSha"]?.stringValue, "630b4291f6a59444cfb1d8185f757f9dcda9ecc4")
        for entry in try XCTUnwrap(index["files"]?.arrayValue) {
            let name = try XCTUnwrap(entry["file"]?.stringValue)
            contents[name] = .string(try XCTUnwrap(AIPFixtures.files[prefix + name]) + "\n")
        }
        XCTAssertEqual(SemanticJSON.object(contents).canonicalSHA256, "f043a8c9fbbd4eb5441a4069d009cc2c8e31a1b22f0a4c3102b46f356d72e1b0")
        let snapshot = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(AIPFixtures.files[prefix + "state-snapshot.json"])))
        let raw = try XCTUnwrap(snapshot["payload"]?["state"])
        XCTAssertNotNil(ValidatedSemanticState.validate(raw))
        let patch = try XCTUnwrap(SemanticJSON.parse(try XCTUnwrap(AIPFixtures.files[prefix + "state-patch.json"])))
        let merged = SemanticJSON.mergePatch(raw, try XCTUnwrap(patch["payload"]?["patch"]))
        XCTAssertNotNil(ValidatedSemanticState.validate(merged))
        XCTAssertEqual(merged.canonicalSHA256, patch["payload"]?["hash"]?.stringValue)
    }
}
