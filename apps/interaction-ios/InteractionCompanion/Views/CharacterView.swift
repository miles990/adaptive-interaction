//
//  CharacterView.swift
//  InteractionCompanion
//
//  角色分頁:簡化版陪伴角色(圓潤貓耳剪影)+ 狀態顯示 + 觸控事件。
//
//  誠實不變量:
//  - 綠色勾號「只」在 verified-success 出現;completed ≠ verified。
//  - unknown 明確標示「未知」,不猜測、不美化。
//  - emergency 固定文案「緊急停止中」。
//  - 此為簡化 2D 呈現,不冒充桌面版小樞的完整動畫形象。
//  - 點擊/長按 → observation("iphone.touch", kind: tap|longpress)。
//

import SwiftUI

struct CharacterView: View {
    @EnvironmentObject private var character: CharacterState
    @EnvironmentObject private var connection: ConnectionManager

    @State private var lastTouchNote: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                if !isConnected {
                    Label("未連線:以下狀態可能不是桌面端的即時狀態", systemImage: "wifi.slash")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                        .padding(.horizontal)
                }

                Spacer(minLength: 8)

                ZStack {
                    CatSilhouetteShape()
                        .fill(stateColor.gradient)
                        .frame(width: 200, height: 200)
                        .shadow(color: stateColor.opacity(0.35), radius: 18)
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
                .accessibilityLabel("陪伴角色,目前狀態:\(stateLabel)")

                Text(stateLabel)
                    .font(.title2.bold())
                    .foregroundStyle(stateColor)

                if character.state == .emergency {
                    // 固定文案,不得改寫或淡化
                    Text("緊急停止中")
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

                Text("此為簡化狀態顯示,非桌面版小樞完整形象;綠色勾號僅在「已驗證成功」出現。")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 8)
            }
            .navigationTitle("角色")
        }
    }

    private var isConnected: Bool {
        if case .connected = connection.phase { return true }
        return false
    }

    private func sendTouch(kind: String) {
        connection.send(.observation(
            receptor: "iphone.touch",
            facts: ["kind": .string(kind)],
            at: nil))
        lastTouchNote = isConnected
            ? "已送出觸控事件:\(kind)"
            : "未連線,觸控事件未送出(已丟棄)"
    }

    // MARK: 狀態外觀

    private var stateColor: Color {
        switch character.state {
        case .idle: return Color(red: 0.45, green: 0.55, blue: 0.75)
        case .working: return .orange
        case .waiting: return .yellow
        case .verifiedSuccess: return .green
        case .failed: return .red
        case .unknown: return .gray
        case .emergency: return .red
        }
    }

    private var stateLabel: String {
        switch character.state {
        case .idle: return "待機"
        case .working: return "工作中"
        case .waiting: return "等待中"
        case .verifiedSuccess: return "已驗證成功"
        case .failed: return "失敗"
        case .unknown: return "未知"
        case .emergency: return "緊急停止中"
        }
    }

    @ViewBuilder
    private var stateBadge: some View {
        switch character.state {
        case .verifiedSuccess:
            // 唯一允許出現綠色勾號的狀態
            Image(systemName: "checkmark.seal.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .green)
        case .failed:
            Image(systemName: "xmark.circle.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .red)
        case .unknown:
            Image(systemName: "questionmark.circle.fill")
                .font(.system(size: 44))
                .foregroundStyle(.white, .gray)
        case .emergency:
            Image(systemName: "octagon.fill")
                .font(.system(size: 44))
                .foregroundStyle(.red)
                .overlay {
                    Image(systemName: "hand.raised.fill")
                        .font(.system(size: 20))
                        .foregroundStyle(.white)
                }
        case .working:
            ProgressView()
                .controlSize(.large)
                .tint(.white)
        case .waiting:
            Image(systemName: "hourglass")
                .font(.system(size: 40))
                .foregroundStyle(.white)
        case .idle:
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
