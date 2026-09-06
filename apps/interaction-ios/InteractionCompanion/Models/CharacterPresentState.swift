// Pure presentation vocabulary shared by the renderer and contract conformance runner.
enum CharacterPresentState: String, CaseIterable {
    case idle
    case working
    case waiting
    case verifiedSuccess = "verified-success"
    case failed
    case unknown
    case emergency
}
