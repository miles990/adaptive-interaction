// Runs the actual pure Swift parser, generated DTO, validator and projection on
// macOS. This is a native contract test, not iPhone/simulator/device acceptance.
import Foundation

@main
struct SemanticConformanceRunner {
    static func main() throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let fixtures = root.appendingPathComponent("crates/interaction-aip/tests/fixtures")
        func read(_ path: String) throws -> SemanticJSON {
            let text = try String(contentsOf: fixtures.appendingPathComponent(path), encoding: .utf8)
            guard let parsed = SemanticJSON.parse(text) else { fatalError("invalid fixture \(path)") }
            return parsed
        }
        let corpus = try read("semantic-state-cases.json")
        var passed = 0
        for entry in corpus["states"]?.arrayValue ?? [] {
            guard let wire = entry["wire"]?.stringValue, let state = SemanticJSON.parse(wire),
                (ValidatedSemanticState.validate(state) != nil) == entry["accept"]?.boolValue,
                state.canonicalJSON == entry["canonical"]?.stringValue,
                state.canonicalSHA256 == entry["hash"]?.stringValue else {
                fatalError("semantic state fixture failed: \(entry["id"]?.stringValue ?? "missing-id")")
            }
            passed += 1
        }
        for entry in corpus["patches"]?.arrayValue ?? [] {
            guard let baseText = entry["baseWire"]?.stringValue, let base = SemanticJSON.parse(baseText),
                let patchText = entry["patchWire"]?.stringValue, let patch = SemanticJSON.parse(patchText) else { fatalError("bad patch fixture") }
            let merged = SemanticJSON.mergePatch(base, patch)
            guard (ValidatedSemanticState.validate(merged) != nil) == entry["accept"]?.boolValue,
                merged.canonicalJSON == entry["canonical"]?.stringValue,
                merged.canonicalSHA256 == entry["hash"]?.stringValue else { fatalError("patch fixture failed") }
            passed += 1
        }
        let manifest = try read("manifest.json")
        for entry in manifest["stateHashes"]?.arrayValue ?? [] {
            guard let file = entry["file"]?.stringValue else { fatalError("bad state hash index") }
            let data = try read(file)
            guard let state = data["state"], state.canonicalJSON == data["canonical"]?.stringValue,
                state.canonicalSHA256 == data["hash"]?.stringValue else { fatalError("state hash fixture failed \(file)") }
            if entry["semanticValid"]?.boolValue == true {
                guard let checked = ValidatedSemanticState.validate(state) else { fatalError("generated state rejected \(file)") }
                _ = CharacterSemanticState.project(checked)
            }
            passed += 1
        }
        let released = try read("releases/v0.7.0/manifest.json")
        var contents: [String: SemanticJSON] = [:]
        for item in released["files"]?.arrayValue ?? [] {
            guard let name = item["file"]?.stringValue else { fatalError("release file missing") }
            contents[name] = .string(try String(contentsOf: fixtures.appendingPathComponent("releases/v0.7.0/" + name), encoding: .utf8))
        }
        guard SemanticJSON.object(contents).canonicalSHA256 == "f043a8c9fbbd4eb5441a4069d009cc2c8e31a1b22f0a4c3102b46f356d72e1b0" else { fatalError("released corpus was changed") }
        passed += 1
        print("Swift native semantic conformance: \(passed) passed, 0 failed")
    }
}
