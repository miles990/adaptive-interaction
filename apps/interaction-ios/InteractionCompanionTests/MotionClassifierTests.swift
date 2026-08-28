//
//  MotionClassifierTests.swift
//  InteractionCompanionTests
//
//  MotionClassifier 純核心行為測試。
//  這些案例已於交付時在 macOS 上以抽出核心的方式實際執行通過
//  (見 apps/interaction-ios/README.md「本機驗證了什麼」);
//  在 Xcode 中請將本檔加入 Unit Test target 執行。
//

import XCTest
import simd
@testable import InteractionCompanion

final class MotionClassifierTests: XCTestCase {
    private let hz = 30.0
    private var dt: Double { 1.0 / hz }

    private func sample(t: Double, accel: SIMD3<Double>,
                        gravity: SIMD3<Double> = SIMD3(0, 0, -1),
                        yaw: Double = 0, pitch: Double = 0, roll: Double = 0,
                        rot: Double = 0) -> MotionSample {
        MotionSample(t: t, userAccel: accel, gravity: gravity,
                     yaw: yaw, pitch: pitch, roll: roll, rotationRateMagnitude: rot)
    }

    func testLiftedFiresOnSustainedUpwardAccelWithAttitudeChange() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        for _ in 0..<15 {
            events += classifier.ingest(sample(t: t, accel: SIMD3(0, 0, 0.01)))
            t += dt
        }
        var pitch = 0.0
        for _ in 0..<18 {
            pitch += 0.03
            events += classifier.ingest(sample(t: t, accel: SIMD3(0, 0, 0.3), pitch: pitch))
            t += dt
        }
        XCTAssertTrue(events.contains { $0.kind == .lifted })
        XCTAssertFalse(events.contains { $0.kind == .shaken })
    }

    func testShakenFiresOnHighFrequencyBursts() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        for index in 0..<30 {
            let sign = index % 2 == 0 ? 1.0 : -1.0
            events += classifier.ingest(sample(t: t, accel: SIMD3(sign * 1.4, 0, 0)))
            t += dt
        }
        XCTAssertTrue(events.contains { $0.kind == .shaken })
        XCTAssertFalse(events.contains { $0.kind == .lifted },
                       "shaken 期間不得誤報 lifted")
    }

    func testPlacedFiresOnStillnessAfterMotion() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        for _ in 0..<15 {
            events += classifier.ingest(sample(t: t, accel: SIMD3(0.4, 0.1, 0)))
            t += dt
        }
        for _ in 0..<36 {
            events += classifier.ingest(sample(t: t, accel: SIMD3(0.01, 0, 0), rot: 0.02))
            t += dt
        }
        XCTAssertTrue(events.contains { $0.kind == .placed })
    }

    func testPureStillnessProducesNoEvents() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        for _ in 0..<90 {
            events += classifier.ingest(sample(t: t, accel: SIMD3(0.005, 0, 0)))
            t += dt
        }
        XCTAssertTrue(events.isEmpty)
    }

    func testRotatedFiresOnYawSweepPast60Degrees() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        var yaw = 0.0
        for _ in 0..<45 {
            yaw += 0.03
            events += classifier.ingest(
                sample(t: t, accel: SIMD3(0.02, 0, 0), yaw: yaw, rot: 0.9))
            t += dt
        }
        XCTAssertTrue(events.contains { $0.kind == .rotated })
    }

    func testRotatedHandlesYawWrapAround() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        var yaw = 3.0
        for _ in 0..<45 {
            yaw += 0.03
            if yaw > Double.pi { yaw -= 2 * Double.pi }
            events += classifier.ingest(
                sample(t: t, accel: SIMD3(0.02, 0, 0), yaw: yaw, rot: 0.9))
            t += dt
        }
        XCTAssertTrue(events.contains { $0.kind == .rotated },
                      "yaw 跨越 ±π 的旋轉也必須被偵測")
    }

    func testShakenDebounceIsAtLeastOnePointFiveSeconds() {
        let classifier = MotionClassifier()
        var events: [MotionEvent] = []
        var t = 0.0
        for index in 0..<90 {
            let sign = index % 2 == 0 ? 1.0 : -1.0
            events += classifier.ingest(sample(t: t, accel: SIMD3(sign * 1.4, 0, 0)))
            t += dt
        }
        let shakes = events.filter { $0.kind == .shaken }
        XCTAssertGreaterThanOrEqual(shakes.count, 2)
        for index in 1..<shakes.count {
            XCTAssertGreaterThanOrEqual(shakes[index].at - shakes[index - 1].at, 1.5)
        }
    }

    func testSlidingWindowNeverExceedsThreeSeconds() {
        // 隱私承諾:原始樣本只存 3 秒滑動視窗
        let classifier = MotionClassifier()
        var t = 0.0
        for _ in 0..<300 {
            _ = classifier.ingest(sample(t: t, accel: SIMD3(0.3, 0, 0)))
            t += dt
        }
        guard let first = classifier.window.first, let last = classifier.window.last else {
            XCTFail("視窗不應為空")
            return
        }
        XCTAssertLessThanOrEqual(last.t - first.t, 3.0 + dt)
    }
}
