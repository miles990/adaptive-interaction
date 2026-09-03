//
//  ReconnectHintTests.swift
//  InteractionCompanionTests
//
//  v0.5.1 迴歸測試,對應真機(iPhone 11 / iOS 26.3.1)實測到的兩個限制:
//  (a) 系統終止 App 後冷啟動不會自動重連;
//  (b) 桌面 IP 變更後 App 對舊位址反覆重連,畫面只有底層錯誤字串,
//      沒有告訴使用者「需要重新配對」。
//
//  這裡只測純函式與純決策,不開任何 socket:
//  - ReconnectDiagnosis.evaluate(failures:) 的分類與門檻
//  - ConnectionFailureKind.classify(...) 對 URLError / POSIX 錯誤的分類
//  - ColdStartConnectDecision.shouldAutoConnect(...) 的冷啟動決策
//  - 不變量:冷啟動自動重連**不得**恢復任何感測(SensorCenter 預設全關)
//

import XCTest
@testable import InteractionCompanion

final class ReconnectHintTests: XCTestCase {

    // MARK: 工具

    private func failures(_ kinds: [ConnectionFailureKind],
                          spacingSeconds: TimeInterval = 5) -> [ConnectionFailure] {
        kinds.enumerated().map { index, kind in
            ConnectionFailure(kind: kind, at: TimeInterval(index) * spacingSeconds)
        }
    }

    // MARK: 門檻與分類

    func testFourConsecutiveConnectivityFailuresSuggestRepair() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.connectivity, .connectivity, .connectivity, .connectivity]))
        XCTAssertEqual(diagnosis, .suggestRepair(reason: .hostAddressLikelyChanged))
    }

    func testThreeConnectivityFailuresKeepRetrying() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.connectivity, .connectivity, .connectivity]))
        XCTAssertEqual(diagnosis, .keepRetrying)
    }

    func testEmptyHistoryKeepsRetrying() {
        XCTAssertEqual(ReconnectDiagnosis.evaluate(failures: []), .keepRetrying)
    }

    /// 夾雜 auth-fail:撤銷有自己的既有文案,絕不可混淆成「位址已變更」。
    /// auth-fail 會打斷連續失敗串,之後只累積 2 次 → 未達門檻。
    func testAuthFailInTheMiddleBreaksTheRunSoNoRepairSuggestion() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.connectivity, .connectivity, .authRejected,
                                .connectivity, .connectivity]))
        XCTAssertEqual(diagnosis, .keepRetrying)
    }

    /// 最新一筆是 auth-fail:桌面端明確拒絕,權威高於任何連線層猜測。
    func testAuthFailAfterManyTimeoutsIsAuthoritative() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.connectivity, .connectivity, .connectivity,
                                .connectivity, .connectivity, .authRejected]))
        XCTAssertEqual(diagnosis, .keepRetrying)
    }

    /// TLS 指紋不符維持既有文案(可能是憑證輪替或中間人),不得說成 IP 變更。
    func testTlsMismatchIsNeverReportedAsAddressChange() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.tlsMismatch, .tlsMismatch, .tlsMismatch,
                                .tlsMismatch, .tlsMismatch]))
        XCTAssertEqual(diagnosis, .keepRetrying)
    }

    /// 次數未達 4 次,但同一串連線層失敗已持續 ≥ 60 秒 → 一樣建議重新配對。
    func testSustainedConnectivityFailuresOverSixtySecondsSuggestRepair() {
        let sustained = [
            ConnectionFailure(kind: .connectivity, at: 0),
            ConnectionFailure(kind: .connectivity, at: 61),
        ]
        XCTAssertEqual(ReconnectDiagnosis.evaluate(failures: sustained),
                       .suggestRepair(reason: .hostAddressLikelyChanged))
    }

    func testTwoConnectivityFailuresWithinSixtySecondsKeepRetrying() {
        let quick = [
            ConnectionFailure(kind: .connectivity, at: 0),
            ConnectionFailure(kind: .connectivity, at: 59),
        ]
        XCTAssertEqual(ReconnectDiagnosis.evaluate(failures: quick), .keepRetrying)
    }

    /// `.other`(協定錯誤、送出失敗…)不足以推論位址變更。
    func testOtherFailuresNeverSuggestRepair() {
        let diagnosis = ReconnectDiagnosis.evaluate(
            failures: failures([.other, .other, .other, .other, .other, .other]))
        XCTAssertEqual(diagnosis, .keepRetrying)
    }

    // MARK: 固定文案

    func testRepairMessageIsTheFixedHonestCopy() {
        XCTAssertEqual(ReconnectDiagnosis.RepairReason.hostAddressLikelyChanged.message,
                       "連不上桌面：可能是桌面的網路位址已變更。請在桌面重新產生配對碼並重新配對。")
    }

    // MARK: 錯誤分類

    func testUrlErrorsThatMeanCannotReachTheDesktopAreConnectivity() {
        let codes: [URLError.Code] = [.timedOut, .cannotFindHost, .cannotConnectToHost,
                                      .networkConnectionLost, .notConnectedToInternet,
                                      .dnsLookupFailed]
        for code in codes {
            XCTAssertEqual(
                ConnectionFailureKind.classify(error: URLError(code), pinningRejected: false),
                .connectivity,
                "URLError \(code.rawValue) 應歸類為連線層失敗")
        }
    }

    func testPosixConnectionRefusedAndHostUnreachableAreConnectivity() {
        for code in [ECONNREFUSED, EHOSTUNREACH, ENETUNREACH, ETIMEDOUT] {
            let error = NSError(domain: NSPOSIXErrorDomain, code: Int(code))
            XCTAssertEqual(
                ConnectionFailureKind.classify(error: error, pinningRejected: false),
                .connectivity,
                "POSIX \(code) 應歸類為連線層失敗")
        }
    }

    /// 指紋比對失敗時 URLSession 只回報 `cancelled`;必須靠 pinning 旗標分類,
    /// 否則會被誤當成連線層失敗、進而錯誤地建議「位址已變更」。
    func testCancelledWithPinningRejectedIsTlsMismatchNotConnectivity() {
        XCTAssertEqual(
            ConnectionFailureKind.classify(error: URLError(.cancelled), pinningRejected: true),
            .tlsMismatch)
    }

    func testCancelledWithoutPinningRejectionIsOther() {
        XCTAssertEqual(
            ConnectionFailureKind.classify(error: URLError(.cancelled), pinningRejected: false),
            .other)
    }

    func testCertificateErrorsAreTlsMismatchNotConnectivity() {
        for code in [URLError.Code.serverCertificateUntrusted,
                     .secureConnectionFailed,
                     .serverCertificateHasBadDate,
                     .serverCertificateHasUnknownRoot,
                     .serverCertificateNotYetValid] {
            XCTAssertEqual(
                ConnectionFailureKind.classify(error: URLError(code), pinningRejected: false),
                .tlsMismatch,
                "URLError \(code.rawValue) 不是連線層失敗")
        }
    }

    /// 端到端:一連串真的 timeout 錯誤(照 classify 分類)最後要建議重新配對。
    func testTimeoutSequenceClassifiedThenDiagnosedSuggestsRepair() {
        let history = (0..<4).map { index in
            ConnectionFailure(
                kind: .classify(error: URLError(.timedOut), pinningRejected: false),
                at: TimeInterval(index) * 3)
        }
        XCTAssertEqual(ReconnectDiagnosis.evaluate(failures: history),
                       .suggestRepair(reason: .hostAddressLikelyChanged))
    }

    // MARK: 冷啟動決策

    func testColdStartConnectsWhenPairedAndUserLastWantedConnection() {
        XCTAssertTrue(ColdStartConnectDecision.shouldAutoConnect(hasPairing: true,
                                                                storedIntent: true))
    }

    func testColdStartDoesNotConnectWithoutPairing() {
        XCTAssertFalse(ColdStartConnectDecision.shouldAutoConnect(hasPairing: false,
                                                                 storedIntent: true))
        XCTAssertFalse(ColdStartConnectDecision.shouldAutoConnect(hasPairing: false,
                                                                 storedIntent: nil))
    }

    /// 使用者按過「立即中斷」→ 意圖為 false,冷啟動不得自作主張連回去。
    func testColdStartRespectsExplicitUserDisconnect() {
        XCTAssertFalse(ColdStartConnectDecision.shouldAutoConnect(hasPairing: true,
                                                                 storedIntent: false))
    }

    /// 從 v0.5.0 升級上來的使用者 Keychain 有配對但沒有意圖旗標:
    /// 視為「想要連線」,否則升級後反而更難用。
    func testColdStartTreatsMissingIntentAsWantingConnection() {
        XCTAssertTrue(ColdStartConnectDecision.shouldAutoConnect(hasPairing: true,
                                                                storedIntent: nil))
    }

    // MARK: 不變量:自動重連不得恢復感測

    /// 冷啟動自動重連只走 connect 路徑;感測中心是全新的,全部旗標必須是關的。
    /// (麥克風 / 位置 / 動作 / 電池 / BLE 閘道皆預設 OFF,重連不會打開任何一項。)
    @MainActor
    func testColdStartAutoConnectNeverResumesAnySensor() {
        let sensors = SensorCenter()
        let flags = sensors.snapshotFlags()
        XCTAssertFalse(flags.motion)
        XCTAssertFalse(flags.battery)
        XCTAssertFalse(flags.micLevel)
        XCTAssertFalse(flags.location)
        XCTAssertFalse(flags.bleGateway)
    }
}
