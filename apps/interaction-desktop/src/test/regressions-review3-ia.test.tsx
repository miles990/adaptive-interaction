// 第三輪對抗審查（0c845e0）ia-settings／agent-honesty 的 regression tests。
// 每一項都對應一個已確認的缺陷，沒有修復就會失敗：
//  * ia-settings-008：淺色主題下 .companion-sensor-label 對比僅約 3.7:1（低於
//    11px 粗體文字所需的 4.5:1）。角色視窗本身無法讀使用者的 data-theme 選擇
//    （main.tsx 只對主視窗寫入，不在本檔獨占清單內，這裡只驗對比度）。
//  * ia-settings-009：未知路由 PageBody 回 null、titleFor 回空字串——靜默空白。
//  * ia-settings-010：通知中心宣告 aria-modal="true" 但不是真 modal。
//  * ia-settings-011：狀態列「設定…」與安全頁「回應方式」是 v0.5 IA 已不存在的頁名。
//  * agent-honesty-026：已關閉但確實通過人工驗證的工作，AiPage 顯示「這一輪尚未檢查」。

import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { api, HumanCapabilities, HumanCard } from "../api";
import { AppStateProvider } from "../appstate";
import { NotificationPanel, PageBody, titleFor, type Tab } from "../App";
import { SafetyPage } from "../pages/SafetyPage";

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------
// ia-settings-008：淺色主題角色感測標籤對比 ≥ 4.5:1
// ---------------------------------------------------------------------------

/** WCAG 2.x 相對亮度／對比比。 */
function relLuminance(hex: string): number {
  const n = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16) / 255);
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}
function contrastRatio(hexA: string, hexB: string): number {
  const [a, b] = [relLuminance(hexA), relLuminance(hexB)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
}

describe("ia-settings-008 角色感測標籤對比不足", () => {
  it(".companion-sensor-label 的 background/color 對比至少 4.5:1", () => {
    const css = fs.readFileSync(path.resolve("src/styles.css"), "utf8");
    const rule = css.match(/\.companion-sensor-label\s*\{([^}]*)\}/);
    expect(rule, "找不到 .companion-sensor-label 規則").not.toBeNull();
    const body = rule![1];
    const bg = body.match(/background:\s*(#[0-9a-fA-F]{6})/);
    const color = body.match(/color:\s*(#[0-9a-fA-F]{6})/);
    // 修復前：background 是 var(--warn)（隨主題變動、淺色主題下對比僅約 3.7:1），
    // 不是字面 hex——這個 match 本身就會在舊行為下失敗，逼修法採固定高對比色。
    expect(bg, "background 必須是固定字面色，不能隨主題變動到對比不足").not.toBeNull();
    expect(color, "color 必須是固定字面色").not.toBeNull();
    const ratio = contrastRatio(bg![1], color![1]);
    expect(ratio).toBeGreaterThanOrEqual(4.5);
  });
});

// ---------------------------------------------------------------------------
// ia-settings-009：未知路由不得靜默空白
// ---------------------------------------------------------------------------

function stubMinimalApis() {
  vi.spyOn(api, "uiPrefsGet").mockResolvedValue({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  });
  vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
}

describe("ia-settings-009 未知路由不得靜默空白", () => {
  it("titleFor 對未知 tab 不回傳空字串", () => {
    expect(titleFor("this-route-does-not-exist")).not.toBe("");
    expect(titleFor("this-route-does-not-exist").length).toBeGreaterThan(0);
  });

  it("PageBody 對未知 tab 渲染可見訊息與回到「現在」的按鈕，不是空白", async () => {
    stubMinimalApis();
    const onNavigate = vi.fn();
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <PageBody
          tab={"this-route-does-not-exist" as Tab}
          refreshKey={0}
          events={[]}
          advanced={false}
          onNavigate={onNavigate}
          onRerunOnboarding={() => {}}
        />
      </AppStateProvider>
    );
    // 舊行為：container.textContent === ""（PageBody default 分支回 null）。
    expect(document.body.textContent).not.toBe("");
    const back = await screen.findByRole("button", { name: /回到「現在」/ });
    fireEvent.click(back);
    expect(onNavigate).toHaveBeenCalledWith("home");
  });
});

// ---------------------------------------------------------------------------
// ia-settings-010：通知中心 aria-modal 與實際行為一致
// ---------------------------------------------------------------------------

describe("ia-settings-010 通知中心 aria-modal 與實際行為一致", () => {
  it("宣告 aria-modal 的同時真的是 modal：有 backdrop，點 backdrop（面板外）會關閉", () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();
    const { container } = render(
      <NotificationPanel
        inbox={{ pendingCount: 0, pendingCountExact: true, items: [] }}
        onClose={onClose}
        onNavigate={onNavigate}
      />
    );
    const panel = screen.getByRole("dialog", { name: "通知中心" });
    expect(panel).toHaveAttribute("aria-modal", "true");
    // 舊行為：aria-modal="true"，但沒有 backdrop（面板的父層直接是呼叫端的容器，
    // 不是 .dialog-backdrop），點面板外完全沒有反應——宣稱與行為不一致。
    const backdrop = container.querySelector(".dialog-backdrop");
    expect(backdrop, "必須有 .dialog-backdrop 把宣稱的 modal 做成真的 modal").not.toBeNull();
    expect(backdrop).toContainElement(panel);
    fireEvent.mouseDown(backdrop as Element);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// ia-settings-011：導覽文案殘留 v0.5 IA 已不存在的頁名
// ---------------------------------------------------------------------------

function card(overrides: Partial<HumanCard>): HumanCard {
  return {
    id: "test.cap",
    kind: "actuator",
    displayName: "測試能力",
    nameSource: "catalog",
    shortDescription: "一句說明。",
    descriptionSource: "catalog",
    icon: "bell",
    colorRole: "output",
    category: "notification",
    beginnerRecommended: false,
    badges: [],
    consent: { required: false },
    undescribed: false,
    availability: "available",
    requiresConsent: false,
    manifestHash: "0123456789abcdef",
    ...overrides,
  };
}

const HUMAN_WITH_DISABLED: HumanCapabilities = {
  locale: "zh-TW",
  catalogVersion: 1,
  capabilityVersion: 1,
  generatedAt: "2026-09-01T00:00:00Z",
  constraints: [],
  receptors: [],
  actuators: [
    card({ id: "notify.desktop", displayName: "桌面通知", availability: "disabled" }),
  ],
  toolOperations: [],
};

describe("ia-settings-011 導覽文案殘留舊頁名", () => {
  it('狀態列 tray 選單不再用查無對應頁面的「設定…」', () => {
    const src = fs.readFileSync(path.resolve("src-tauri/src/tray.rs"), "utf8");
    expect(src).not.toMatch(/"設定…"/);
    // id 保持 "settings"（lib.rs 的 navigate 事件依此路由），只有標籤文字要改。
    expect(src).toMatch(/"settings",\s*"[^"]+…?"/);
  });

  it("安全頁「先前已停用」文案不再指向不存在的「回應方式」一級頁，且附可用的導覽按鈕", async () => {
    stubMinimalApis();
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue(HUMAN_WITH_DISABLED);
    vi.spyOn(api, "status").mockResolvedValue({ emergencyStop: true });
    vi.spyOn(api, "auditTail").mockResolvedValue([]);
    const onNavigate = vi.fn();
    render(
      <AppStateProvider ready refreshKey={0}>
        <SafetyPage refreshKey={0} onNavigate={onNavigate} />
      </AppStateProvider>
    );
    const recover = await screen.findByRole("button", { name: /開始安全解除流程/ });
    await userEvent.click(recover);

    // 舊行為：一句「要用得先到『回應方式』重新啟用」，沒有任何導覽按鈕。
    expect(screen.queryByText(/「回應方式」重新啟用/)).not.toBeInTheDocument();
    const goDevices = await screen.findByRole("button", { name: /前往「裝置與能力」/ });
    await userEvent.click(goDevices);
    expect(onNavigate).toHaveBeenCalledWith("connect");
  });
});

// ---------------------------------------------------------------------------
// agent-honesty-026：已關閉但確實驗證過的工作，不得顯示「這一輪尚未檢查」
// ---------------------------------------------------------------------------

import { AgentSessionRecord } from "../api";
import { AiPage, closedWithVerifiedHumanCheck, verifiedForCurrentClaim } from "../pages/AiPage";
import { resetCharacterNameForTests } from "../characterName";

const CLOSED_BUT_VERIFIED: AgentSessionRecord = {
  sessionId: "sess-closed-verified-1",
  providerId: "provider.ai-agent.claude-code",
  agentId: "claude-code",
  label: "整理測試報告",
  state: "closed",
  lease: { issuedAt: "2026-01-01T00:00:00Z", expiresAt: "2026-01-01T00:30:00Z", renewable: true },
  dataScope: ["workspace:/Users/me/project"],
  toolScope: [],
  consentScope: [],
  budget: { maxMessages: 10, spentMessages: 3, maxCost: 0, spentCost: 0 },
  claimId: "claim-1",
  humanVerified: { at: "2026-01-01T00:20:00Z", claimId: "claim-1" },
  createdAt: "2026-01-01T00:00:00Z",
  closedAt: "2026-01-01T00:25:00Z",
};

describe("agent-honesty-026 已關閉但驗證過的工作不得顯示「尚未檢查」", () => {
  afterEach(() => resetCharacterNameForTests());

  it("closedWithVerifiedHumanCheck：closed 且 claim 對得上 → true；verifiedForCurrentClaim 仍是 false（沒有『這一輪』）", () => {
    expect(closedWithVerifiedHumanCheck(CLOSED_BUT_VERIFIED)).toBe(true);
    expect(verifiedForCurrentClaim(CLOSED_BUT_VERIFIED)).toBe(false);
  });

  it("AiPage 對已關閉且驗證過的 session 顯示「已確認並收尾」，不是「這一輪尚未檢查」", async () => {
    vi.spyOn(api, "agentsDiscoveries").mockResolvedValue({ agents: [] });
    vi.spyOn(api, "agentSessionsList").mockResolvedValue([CLOSED_BUT_VERIFIED]);
    vi.spyOn(api, "agentSessionMessages").mockResolvedValue([]);
    render(
      <AppStateProvider ready={false} refreshKey={0}>
        <AiPage refreshKey={0} advanced={false} onNavigate={() => {}} />
      </AppStateProvider>
    );
    await screen.findByText("整理測試報告");
    // 舊行為：record.humanVerified && !verified 一定成立（closed !== claimed-completed），
    // 因此一定會渲染這句與紀錄相反的話。
    await waitFor(() =>
      expect(screen.queryByText(/這一輪尚未檢查/)).not.toBeInTheDocument()
    );
    expect(screen.getByText(/已由你在.*確認並收尾/)).toBeInTheDocument();
  });
});
