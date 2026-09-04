//
//  CharacterView.swift
//  InteractionCompanion
//
//  角色分頁:簡化版陪伴角色(圓潤貓耳剪影)+ 語意狀態顯示 + 觸控事件。
//
//  誠實不變量:
//  - 綠色勾號「只」在 truth = verified 出現;completed ≠ verified。
//  - unknown 明確標示「未知」,不猜測、不美化。
//  - emergency 固定文案「緊急停止中」,而且**取兩條路徑的聯集**:語意狀態或
//    舊的 character.present 任一邊說緊急就是緊急(安全訊息只能加嚴)。
//  - 此為簡化 2D 呈現,不冒充桌面版角色的完整動畫形象。
//  - Behavior Intent 只做**本地**動畫:播完才回 observed;不支援的一律 rejected。
//  - Reduced Motion 開啟時不做位移/縮放,只換顏色。
//  - haptic **不由** intent 觸發(震動只走受 governor 管的 haptic.pulse 動器)。
//  - 點擊/長按:已協商 → AIP 語意事件;未協商(舊桌面)→ 既有 iphone.touch 觀察;
//    未連線 → 丟棄並誠實說明。
//  - 進階細節(版本號等)不在這一頁,只在「連線」頁的診斷折疊區。
//

import SwiftUI

struct CharacterView: View {
    @EnvironmentObject private var character: CharacterState
    @EnvironmentObject private var connection: ConnectionManager
    @EnvironmentObject private var session: SessionClient

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var lastTouchNote: String?
    /// 目前這一則 intent 在畫面上的效果(純資料;決策在 `CharacterPlaybackEffect.plan`)。
    @State private var effect = CharacterPlaybackEffect()
    @State private var playback: Task<Void, Never>?
    #if DEBUG
    @State private var debugTouchEmitted = false
    #endif

    private var presentation: CharacterPresentation {
        CharacterPresentation.resolve(
            session: session.presentation,
            negotiated: session.negotiated,
            legacy: character.state)
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                syncLine

                Spacer(minLength: 8)

                ZStack {
                    CatSilhouetteShape()
                        .fill(bodyColor.gradient)
                        .frame(width: 200, height: 200)
                        .shadow(color: bodyColor.opacity(0.35), radius: 18)
                        .scaleEffect(CGFloat(effect.scale))
                    stateBadge
                        .offset(y: 26)
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    sendTouch(kind: "tap")
                }
                .onLongPressGesture(minimumDuration: 0.6) {
                    sendTouch(kind: "longpress")
                }
                .accessibilityLabel("陪伴角色,目前狀態:\(presentation.headline)")

                Text(presentation.headline)
                    .font(.title2.bold())
                    .foregroundStyle(bodyColor)

                if let detail = presentation.detail {
                    Text(detail)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                if presentation.isEmergency {
                    // 固定文案,不得改寫或淡化
                    Text(CharacterPresentation.emergencyText)
                        .font(.headline)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .background(Color.red.opacity(0.15), in: Capsule())
                        .foregroundStyle(.red)
                }

                if let note = lastTouchNote {
                    Text(note)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Text("此為簡化狀態顯示,非桌面版角色的完整形象;綠色勾號僅在「已驗證成功」出現。")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 8)
            }
            .navigationTitle("角色")
        }
        .onChange(of: session.nowPlaying) { _, playing in
            startPlayback(playing)
        }
        .onDisappear {
            // 離開角色頁＝不再看著角色(§4 character.interaction.dismiss)。
            // 只有已協商時才有這個語意事件;舊路徑沒有對應訊息,不硬造。
            session.dismiss()
        }
        #if DEBUG
        .task {
            await emitDebugTouchIfRequested()
        }
        #endif
    }

    #if DEBUG
    /// DEBUG 限定(release 不編入):`--emit-touch tap|longpress` 或
    /// `INTERACT_EMIT_TOUCH`。模擬器沒有觸控注入,這個入口讓自動化驗收
    /// **替使用者按一次角色**——走的是同一條 `sendTouch` 路徑,不繞過協商、
    /// 連線或政策檢查,而且每次啟動只送一次。
    ///
    /// 等待協商最多 10 秒:等不到就什麼都不做(誠實:不會退回舊路徑假裝成功)。
    private func emitDebugTouchIfRequested() async {
        guard !debugTouchEmitted else { return }
        let arguments = CommandLine.arguments
        var kind: String?
        if let index = arguments.firstIndex(of: "--emit-touch"),
           arguments.indices.contains(index + 1) {
            kind = arguments[index + 1]
        } else if let value = ProcessInfo.processInfo.environment["INTERACT_EMIT_TOUCH"] {
            kind = value
        }
        guard let kind, ["tap", "longpress"].contains(kind) else { return }
        debugTouchEmitted = true
        for _ in 0..<100 {
            if session.negotiated { break }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        guard session.negotiated else {
            lastTouchNote = "尚未與桌面完成角色同步,未送出自動觸控"
            return
        }
        sendTouch(kind: kind)
    }
    #endif

    // MARK: 同步狀態(一行人話,不含任何技術詞)

    @ViewBuilder
    private var syncLine: some View {
        switch session.syncStatus {
        case .synced:
            Label(session.syncStatus.text, systemImage: "checkmark.circle")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .padding(.horizontal)
        case .offline:
            Label(session.syncStatus.text, systemImage: "wifi.slash")
                .font(.footnote)
                .foregroundStyle(.orange)
                .padding(.horizontal)
        case .notNegotiated, .resuming, .partialCapabilities:
            Label(session.syncStatus.text, systemImage: "arrow.triangle.2.circlepath")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .padding(.horizontal)
        case .unrecoverable:
            Label(session.syncStatus.text, systemImage: "exclamationmark.triangle")
                .font(.footnote)
                .foregroundStyle(.orange)
                .padding(.horizontal)
        }
    }

    // MARK: 觸控

    private func sendTouch(kind: String) {
        lastTouchNote = session.touch(kind: kind)
    }

    // MARK: Behavior Intent 的本地動畫

    /// 播一次動畫,**播完**才回 `observed`;被新的 intent 中斷就不回(誠實)。
    private func startPlayback(_ playing: PlayingIntent?) {
        playback?.cancel()
        resetMotion()
        guard let playing else { return }
        playback = Task { @MainActor in
            let completed = await play(playing)
            guard completed, !Task.isCancelled else { return }
            session.intentDidFinishPlaying(messageId: playing.messageId)
        }
    }

    @MainActor
    private func play(_ playing: PlayingIntent) async -> Bool {
        let seconds = Double(playing.intent.playbackMs) / 1000
        let planned = CharacterPlaybackEffect.plan(
            intent: playing.intent, intensity: playing.intensity, reduceMotion: reduceMotion)
        // Reduced Motion 下 `react-happily-to-touch` 只剩換色(§4 的表格),
        // 但**一定有**東西可看——沒有呈現就不能回 observed。
        let duration = seconds * (planned.scale == 1 ? 0.4 : 0.5)
        withAnimation(planned.scale == 1 ? .easeInOut(duration: duration) : .spring(duration: duration)) {
            effect = planned
        }
        do {
            try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        } catch {
            return false  // 被取消:沒播完就不能說 observed
        }
        withAnimation(.easeOut(duration: 0.2)) {
            resetMotion()
        }
        return true
    }

    private func resetMotion() {
        effect = CharacterPlaybackEffect()
    }

    // MARK: 外觀

    private var bodyColor: Color {
        Self.bodyColor(presentation: presentation, effect: effect)
    }

    /// 目前該畫什麼顏色(純函式,可測)。
    ///
    /// 緊急停止**永遠**壓過任何播放效果:安全訊息只能加嚴,不能被一段動畫蓋掉。
    static func bodyColor(presentation: CharacterPresentation, effect: CharacterPlaybackEffect)
        -> Color
    {
        let rgb = components(presentation: presentation, effect: effect)
        return Color(red: rgb.red, green: rgb.green, blue: rgb.blue)
    }

    /// 同上,但回 RGB 分量——測試要能斷言「顏色真的變了」,不能只比 `Color` 的相等性。
    static func components(presentation: CharacterPresentation, effect: CharacterPlaybackEffect)
        -> RGB
    {
        if presentation.isEmergency { return components(for: .emergency) }
        switch effect.highlight {
        case .none: return components(for: presentation.tone)
        case .celebrate: return RGB(red: 1.0, green: 0.85, blue: 0.20)
        // 回應觸摸:把語意色調往亮處推。Reduced Motion 下這是**唯一**的呈現手段,
        // 所以它必須真的與靜止時不同(evidence-honesty-011)。
        case .react: return components(for: presentation.tone).lightened(by: 0.35)
        }
    }

    /// 語意色調 → 顏色。顏色只是呈現,語意在 `CharacterPresentation`。
    static func color(for tone: CharacterTone) -> Color {
        let rgb = components(for: tone)
        return Color(red: rgb.red, green: rgb.green, blue: rgb.blue)
    }

    /// 語意色調 → RGB 分量(0…1)。
    static func components(for tone: CharacterTone) -> RGB {
        switch tone {
        case .neutral: return RGB(red: 0.45, green: 0.55, blue: 0.75)
        case .happy: return RGB(red: 0.98, green: 0.68, blue: 0.25)
        case .playful: return RGB(red: 0.95, green: 0.45, blue: 0.65)
        case .proud: return RGB(red: 0.20, green: 0.68, blue: 0.33)
        case .tired: return RGB(red: 0.55, green: 0.55, blue: 0.62)
        case .alert: return RGB(red: 1.0, green: 0.58, blue: 0.0)
        case .down: return RGB(red: 0.42, green: 0.48, blue: 0.70)
        case .unknown: return RGB(red: 0.56, green: 0.56, blue: 0.58)
        case .emergency: return RGB(red: 0.90, green: 0.22, blue: 0.21)
        }
    }

    /// 呈現用的 RGB(0…1)。與語意無關,只是為了讓顏色變化可以被斷言。
    struct RGB: Equatable {
        var red: Double
        var green: Double
        var blue: Double

        /// 往白色推 `amount`(0…1)。
        func lightened(by amount: Double) -> RGB {
            let t = min(max(amount, 0), 1)
            return RGB(
                red: red + (1 - red) * t,
                green: green + (1 - green) * t,
                blue: blue + (1 - blue) * t)
        }
    }

    @ViewBuilder
    private var stateBadge: some View {
        if presentation.isEmergency {
            Image(systemName: "octagon.fill")
                .font(.system(size: 44))
                .foregroundStyle(.red)
                .overlay {
                    Image(systemName: "hand.raised.fill")
                        .font(.system(size: 20))
                        .foregroundStyle(.white)
                }
        } else if presentation.showsVerifiedCheck {
            // 唯一允許出現綠色勾號的狀態
            Image(systemName: "checkmark.seal.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .green)
        } else if presentation.tone == .unknown {
            Image(systemName: "questionmark.circle.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .gray)
        } else if presentation.tone == .down {
            Image(systemName: "xmark.circle.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .red)
        } else if presentation.tone == .alert {
            ProgressView()
                .controlSize(.large)
                .tint(.white)
        } else if presentation.tone == .tired {
            Image(systemName: "hourglass")
                .font(.system(size: 40))
                .foregroundStyle(.white)
        } else {
            Image(systemName: "zzz")
                .font(.system(size: 36))
                .foregroundStyle(.white.opacity(0.85))
        }
    }
}

// MARK: - 圓潤貓耳剪影

/// 簡單的 2D 貓耳頭形:圓潤頭部 + 兩個帶弧度的耳朵。
struct CatSilhouetteShape: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        let width = rect.width
        let height = rect.height

        // 頭:下方 72% 的圓角橢圓
        let headRect = CGRect(x: rect.minX + width * 0.05,
                              y: rect.minY + height * 0.28,
                              width: width * 0.9,
                              height: height * 0.68)
        path.addEllipse(in: headRect)

        // 左耳
        var leftEar = Path()
        leftEar.move(to: CGPoint(x: rect.minX + width * 0.16, y: rect.minY + height * 0.42))
        leftEar.addQuadCurve(
            to: CGPoint(x: rect.minX + width * 0.26, y: rect.minY + height * 0.06),
            control: CGPoint(x: rect.minX + width * 0.12, y: rect.minY + height * 0.16))
        leftEar.addQuadCurve(
            to: CGPoint(x: rect.minX + width * 0.48, y: rect.minY + height * 0.32),
            control: CGPoint(x: rect.minX + width * 0.42, y: rect.minY + height * 0.14))
        leftEar.closeSubpath()
        path.addPath(leftEar)

        // 右耳(鏡像)
        var rightEar = Path()
        rightEar.move(to: CGPoint(x: rect.minX + width * 0.84, y: rect.minY + height * 0.42))
        rightEar.addQuadCurve(
            to: CGPoint(x: rect.minX + width * 0.74, y: rect.minY + height * 0.06),
            control: CGPoint(x: rect.minX + width * 0.88, y: rect.minY + height * 0.16))
        rightEar.addQuadCurve(
            to: CGPoint(x: rect.minX + width * 0.52, y: rect.minY + height * 0.32),
            control: CGPoint(x: rect.minX + width * 0.58, y: rect.minY + height * 0.14))
        rightEar.closeSubpath()
        path.addPath(rightEar)

        return path
    }
}
