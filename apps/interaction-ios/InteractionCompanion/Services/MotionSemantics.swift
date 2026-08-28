//
//  MotionSemantics.swift
//  InteractionCompanion
//
//  CoreMotion → 語意事件(lifted / shaken / placed / rotated)。
//  隱私不變量:
//  - 只送語意事件,絕不送原始軌跡。
//  - 原始樣本僅存在於記憶體中的 3 秒滑動視窗;不落盤、不外送。
//  - 每種事件 debounce ≥ 1.5 秒。
//
//  MotionClassifier 為純函式核心(不碰 CoreMotion),可直接單元測試:
//  餵入 (加速度, 姿態) 樣本序列,驗證輸出事件。
//

import Foundation
import simd
import CoreMotion

// MARK: - 純分類器核心

/// 單一動作樣本(裝置座標系;加速度單位 g)。
struct MotionSample {
    /// 單調時間(秒)
    let t: TimeInterval
    /// 使用者加速度(已扣除重力,單位 g)
    let userAccel: SIMD3<Double>
    /// 重力向量(單位 g,指向地心)
    let gravity: SIMD3<Double>
    /// 姿態(弧度)
    let yaw: Double
    let pitch: Double
    let roll: Double
    /// 角速度大小(rad/s)
    let rotationRateMagnitude: Double
}

enum MotionEventKind: String, CaseIterable {
    case lifted
    case shaken
    case placed
    case rotated
}

struct MotionEvent: Equatable {
    let kind: MotionEventKind
    let at: TimeInterval
}

/// 純分類器:狀態只有 3 秒滑動視窗與各事件的 debounce 時間戳。
final class MotionClassifier {
    /// 視窗上限:3 秒。超齡樣本立即丟棄——這是隱私承諾,不是效能最佳化。
    static let windowSeconds: TimeInterval = 3.0
    /// 每種事件至少間隔 1.5 秒
    static let debounceSeconds: TimeInterval = 1.5

    // 門檻(單位 g / rad / 秒);依一般 iPhone 30Hz deviceMotion 調校的保守初值
    private let liftUpAccelThreshold = 0.12       // 持續向上加速度平均
    private let liftSustainSeconds = 0.35
    private let liftAttitudeDelta = 0.25          // pitch/roll 合併變化
    private let shakeMagnitudeThreshold = 0.9     // 單樣本 |userAccel| 視為爆發
    private let shakeBurstCountThreshold = 6      // 1 秒內爆發樣本數
    private let stillMagnitudeThreshold = 0.05
    private let stillRotationThreshold = 0.15
    private let stillSustainSeconds = 0.8
    private let priorMotionThreshold = 0.25       // placed 之前必須真的有動過
    private let rotatedYawDelta = Double.pi / 3   // 60°

    /// 對外唯讀,供單元測試驗證「視窗跨度 ≤ 3 秒」的隱私承諾
    private(set) var window: [MotionSample] = []
    private var lastEmitted: [MotionEventKind: TimeInterval] = [:]

    func reset() {
        window.removeAll()
        lastEmitted.removeAll()
    }

    /// 餵入一筆樣本,回傳本次新產生的語意事件(可能為空)。
    func ingest(_ sample: MotionSample) -> [MotionEvent] {
        window.append(sample)
        // 滑動視窗:丟棄超過 3 秒的樣本(唯一的原始資料保存處)
        let cutoff = sample.t - Self.windowSeconds
        while let first = window.first, first.t < cutoff {
            window.removeFirst()
        }

        var events: [MotionEvent] = []
        let now = sample.t

        let shaking = detectShaken(now: now)
        if shaking, canEmit(.shaken, at: now) {
            events.append(emit(.shaken, at: now))
        }
        // shaken 期間不判 lifted / rotated,避免高頻雜訊誤判
        if !shaking {
            if detectLifted(now: now), canEmit(.lifted, at: now) {
                events.append(emit(.lifted, at: now))
            }
            if detectRotated(now: now), canEmit(.rotated, at: now) {
                events.append(emit(.rotated, at: now))
            }
        }
        if detectPlaced(now: now), canEmit(.placed, at: now) {
            events.append(emit(.placed, at: now))
        }
        return events
    }

    // MARK: 個別偵測規則

    /// lifted:近 liftSustainSeconds 內「沿反重力方向」的平均加速度為正且夠大,
    /// 且視窗內姿態(pitch+roll)有明顯變化。
    private func detectLifted(now: TimeInterval) -> Bool {
        let recent = window.filter { $0.t >= now - liftSustainSeconds }
        guard recent.count >= 3, let oldest = window.first, let newest = window.last else {
            return false
        }
        var upSum = 0.0
        for sample in recent {
            let gravityLength = simd_length(sample.gravity)
            guard gravityLength > 0.5 else { return false }  // 資料異常時不猜
            let up = -sample.gravity / gravityLength
            upSum += simd_dot(sample.userAccel, up)
        }
        let upMean = upSum / Double(recent.count)
        let attitudeDelta = abs(newest.pitch - oldest.pitch) + abs(newest.roll - oldest.roll)
        return upMean > liftUpAccelThreshold && attitudeDelta > liftAttitudeDelta
    }

    /// shaken:近 1 秒內高強度加速度爆發樣本數達門檻(高頻來回)。
    private func detectShaken(now: TimeInterval) -> Bool {
        let recent = window.filter { $0.t >= now - 1.0 }
        guard recent.count >= shakeBurstCountThreshold else { return false }
        let bursts = recent.filter { simd_length($0.userAccel) > shakeMagnitudeThreshold }
        return bursts.count >= shakeBurstCountThreshold
    }

    /// placed:先前有動作,之後連續 stillSustainSeconds 靜止。
    private func detectPlaced(now: TimeInterval) -> Bool {
        let stillStart = now - stillSustainSeconds
        let stillPart = window.filter { $0.t >= stillStart }
        let earlierPart = window.filter { $0.t < stillStart }
        guard stillPart.count >= 3, !earlierPart.isEmpty else { return false }
        let isStill = stillPart.allSatisfy {
            simd_length($0.userAccel) < stillMagnitudeThreshold
                && $0.rotationRateMagnitude < stillRotationThreshold
        }
        guard isStill else { return false }
        let movedBefore = earlierPart.contains {
            simd_length($0.userAccel) > priorMotionThreshold
        }
        return movedBefore
    }

    /// rotated:視窗內 yaw 累積變化(逐樣本 unwrap)超過 60°。
    private func detectRotated(now: TimeInterval) -> Bool {
        guard window.count >= 3 else { return false }
        var accumulated = 0.0
        for index in 1..<window.count {
            let delta = window[index].yaw - window[index - 1].yaw
            // wrap-around 校正到 (-π, π]
            accumulated += atan2(sin(delta), cos(delta))
        }
        return abs(accumulated) > rotatedYawDelta
    }

    // MARK: debounce

    private func canEmit(_ kind: MotionEventKind, at t: TimeInterval) -> Bool {
        guard let last = lastEmitted[kind] else { return true }
        return t - last >= Self.debounceSeconds
    }

    private func emit(_ kind: MotionEventKind, at t: TimeInterval) -> MotionEvent {
        lastEmitted[kind] = t
        return MotionEvent(kind: kind, at: t)
    }
}

// MARK: - CoreMotion 包裝

/// CMMotionManager → MotionClassifier 的薄包裝。
/// 預設不啟動;只有使用者在 UI 明確開啟動作感測後才呼叫 start()。
final class MotionService {
    private let manager = CMMotionManager()
    private let classifier = MotionClassifier()
    private(set) var running = false

    /// 每個語意事件回呼一次(主執行緒)。
    var onEvent: ((MotionEventKind) -> Void)?

    /// 本裝置是否支援 deviceMotion(不可假設所有 iPhone 相同)。
    var isAvailable: Bool {
        manager.isDeviceMotionAvailable
    }

    func start() {
        guard !running else { return }
        guard manager.isDeviceMotionAvailable else {
            // 誠實:不可用就是不可用,不啟動、不模擬
            return
        }
        running = true
        manager.deviceMotionUpdateInterval = 1.0 / 30.0
        manager.startDeviceMotionUpdates(to: .main) { [weak self] motion, _ in
            guard let self, let motion else { return }
            let sample = MotionSample(
                t: motion.timestamp,
                userAccel: SIMD3(motion.userAcceleration.x,
                                 motion.userAcceleration.y,
                                 motion.userAcceleration.z),
                gravity: SIMD3(motion.gravity.x, motion.gravity.y, motion.gravity.z),
                yaw: motion.attitude.yaw,
                pitch: motion.attitude.pitch,
                roll: motion.attitude.roll,
                rotationRateMagnitude: simd_length(
                    SIMD3(motion.rotationRate.x, motion.rotationRate.y, motion.rotationRate.z))
            )
            for event in self.classifier.ingest(sample) {
                self.onEvent?(event.kind)
            }
        }
    }

    func stop() {
        guard running else { return }
        running = false
        manager.stopDeviceMotionUpdates()
        classifier.reset()  // 停止時立刻清空滑動視窗
    }
}
