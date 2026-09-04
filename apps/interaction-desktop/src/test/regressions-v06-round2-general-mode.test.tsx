// v0.6.0 第二輪對抗審查（desktop-and-tauri 組）：一般模式文案的誠實度回歸。
//
//   capability-consent-052 ／ general-mode-ux-022
//       「部分能力目前不可用」看的是自報 role，不是協商結果；協商結果根本沒送到桌面，
//       所以一台把 intent 協商成 unsupported 的裝置拿到唯一的綠色「已同步」。
//   general-mode-ux-024  live 模式讀不到權威狀態時完全沒有自動重試（永遠停在 pending）。
//   general-mode-ux-025  連接頁手機卡只看 presence，和角色頁對同一台裝置自相矛盾。
//   general-mode-ux-026  多裝置時「連上但尚未同步／已撤銷」被綠色「已同步」蓋掉。

import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { render, screen, waitFor } from "@testing-library/react";
import {
  CHARACTER_SYNC_PROJECTION,
  characterSyncDeviceLine,
  characterSyncMembers,
  projectCharacterSession,
  type CharacterSyncMember,
  type CharacterSyncSignals,
} from "../statusProjection";

const DEVICE_ID = "iphone-87b42264";
const FIXTURE_PHONE = "模擬 iPhone（fixture）";

function signals(overrides: Partial<CharacterSyncSignals> = {}): CharacterSyncSignals {
  return {
    enabled: true,
    failedReads: 0,
    revokedDevice: false,
    connectedButNotSynced: false,
    storeReset: false,
    ...overrides,
  };
}

function snapshot(members: Record<string, unknown>[]): Record<string, unknown> {
  return {
    specVersion: "aip/1.0",
    messageType: "state",
    name: "character.session.snapshot",
    payload: {
      kind: "snapshot",
      state: {
        characterId: "character",
        mood: { kind: "neutral", intensity: 0 },
        activity: "idle",
        truth: { state: "none" },
        members,
        reducedMotion: false,
      },
    },
  };
}

function remoteMember(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    party: { kind: "device", id: DEVICE_ID },
    role: "remote-renderer",
    presence: "online",
    lastSeenAt: "2026-09-04T12:30:00.000Z",
    ...over,
  };
}

describe("capability-consent-052／general-mode-ux-022：綠勾只給真的", () => {
  it("拿不到協商結果時不得顯示綠色「已同步」", () => {
    const snap = snapshot([remoteMember()]);
    const members = characterSyncMembers(snap, { [DEVICE_ID]: FIXTURE_PHONE });
    const p = projectCharacterSession(snap, members, signals());
    expect(p.state).not.toBe("synced");
    expect(p.tone).not.toBe("ok");
    expect(p.headline).toBe("iPhone 已連接，能力核對中");
  });

  it("協商結果說有 unsupported intent → 部分能力目前不可用", () => {
    const snap = snapshot([
      remoteMember({ negotiated: { intents: { celebrate: "exact", settle: "unsupported" } } }),
    ]);
    const members = characterSyncMembers(snap, { [DEVICE_ID]: FIXTURE_PHONE });
    expect(members[0]?.degraded).toBe(true);
    const p = projectCharacterSession(snap, members, signals());
    expect(p.state).toBe("partial-capability");
    expect(p.headline).toBe("部分能力目前不可用");
  });

  it("協商結果說每個 intent 都做得到 → 才給綠色「已同步」", () => {
    const snap = snapshot([
      remoteMember({ negotiated: { intents: { celebrate: "exact", settle: "exact" } } }),
    ]);
    const members = characterSyncMembers(snap, { [DEVICE_ID]: FIXTURE_PHONE });
    expect(members[0]?.degraded).toBe(false);
    const p = projectCharacterSession(snap, members, signals());
    expect(p.state).toBe("synced");
    expect(p.tone).toBe("ok");
  });

  it("runtime 換成 unsupportedIntents 計數欄位也認得（同一條資料路徑的兩種寫法）", () => {
    const snap = snapshot([remoteMember({ unsupportedIntents: 0 })]);
    const members = characterSyncMembers(snap, { [DEVICE_ID]: FIXTURE_PHONE });
    expect(members[0]?.degraded).toBe(false);
    expect(projectCharacterSession(snap, members, signals()).state).toBe("synced");
    const snap2 = snapshot([remoteMember({ unsupportedIntents: 2 })]);
    const members2 = characterSyncMembers(snap2, { [DEVICE_ID]: FIXTURE_PHONE });
    expect(members2[0]?.degraded).toBe(true);
    expect(projectCharacterSession(snap2, members2, signals()).state).toBe("partial-capability");
  });
});

describe("general-mode-ux-026：另一台裝置尚未同步／已撤銷時不得給綠色", () => {
  const full: Partial<CharacterSyncMember> = { canPresent: true, degraded: false };
  const member = (o: Partial<CharacterSyncMember> = {}): CharacterSyncMember => ({
    name: FIXTURE_PHONE,
    remote: true,
    presence: "online",
    canPresent: true,
    degraded: false,
    ...o,
  });

  it("有 online 成員但另一台連著卻不是成員：不得是 synced／ok", () => {
    const snap = snapshot([remoteMember({ negotiated: { intents: { settle: "exact" } } })]);
    const p = projectCharacterSession(snap, [member(full)], signals({ connectedButNotSynced: true }));
    expect(p.state).not.toBe("synced");
    expect(p.tone).not.toBe("ok");
  });

  it("被撤銷過的裝置只要重新連上來就會算成「連著但不是成員」，同樣不得給綠色", () => {
    const snap = snapshot([remoteMember({ negotiated: { intents: { settle: "exact" } } })]);
    const p = projectCharacterSession(
      snap,
      [member(full)],
      signals({ revokedDevice: true, connectedButNotSynced: true })
    );
    expect(p.state).toBe("needs-reconfirmation");
    expect(p.tone).not.toBe("ok");
  });

  it("只是「曾經有裝置被撤銷過」（現在沒連著）不得壓掉一台真的在線的裝置", () => {
    // `revokedDevice` 是歷史事實（provider 列永遠留著 revoked）。拿它當當下的結論
    // 會變成一個永遠亮著的假警報——那和「綠勾只給真的」是同一條誠實規則的兩面。
    const snap = snapshot([remoteMember({ negotiated: { intents: { settle: "exact" } } })]);
    const p = projectCharacterSession(snap, [member(full)], signals({ revokedDevice: true }));
    expect(p.state).toBe("synced");
  });
});

describe("general-mode-ux-025：連接頁手機卡與角色頁不得互相矛盾", () => {
  it("role 不是呈現者的裝置：手機卡不得寫「已同步」", () => {
    const snap = snapshot([remoteMember({ role: "input-device" })]);
    const members = characterSyncMembers(snap, { [DEVICE_ID]: FIXTURE_PHONE });
    const p = projectCharacterSession(snap, members, signals());
    expect(p.state).toBe("partial-capability");
    const line = characterSyncDeviceLine(snap, DEVICE_ID);
    expect(line).not.toBe("角色同步：已同步");
    expect(line).toContain("部分能力目前不可用");
  });

  it("拿不到協商結果的呈現者：手機卡也說「能力核對中」，和角色頁一致", () => {
    const snap = snapshot([remoteMember()]);
    expect(characterSyncDeviceLine(snap, DEVICE_ID)).toBe("角色同步：已連接，能力核對中");
  });

  it("協商結果齊全才寫「已同步」", () => {
    const snap = snapshot([remoteMember({ negotiated: { intents: { settle: "exact" } } })]);
    expect(characterSyncDeviceLine(snap, DEVICE_ID)).toBe("角色同步：已同步");
  });
});

// --- general-mode-ux-024：live 模式的自動重試 ------------------------------

const mockApi = vi.hoisted(() => ({
  characterSessionSnapshot: vi.fn(),
  characterSessionDiagnostics: vi.fn(),
  mobileStatus: vi.fn(),
  providersList: vi.fn(),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { CharacterSyncCard } from "../components/CharacterSyncCard";

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("general-mode-ux-024：live 模式讀不到權威狀態時要自己退避重試", () => {
  it("連續讀不到會自己重試到契約 §7.5 的「無法恢復，請重新連接」，不必使用者按三次", async () => {
    mockApi.characterSessionSnapshot.mockRejectedValue(new Error("500: boom"));
    mockApi.mobileStatus.mockResolvedValue({ devices: [] });
    mockApi.providersList.mockResolvedValue([]);
    mockApi.characterSessionDiagnostics.mockRejectedValue(new Error("500: boom"));
    render(<CharacterSyncCard refreshKey={0} advanced={false} sessionEvents={[]} />);
    await waitFor(() => expect(screen.getByText("同步尚未完成")).toBeInTheDocument());
    await waitFor(
      () => expect(screen.getByText("無法恢復，請重新連接")).toBeInTheDocument(),
      { timeout: 15_000 }
    );
    expect(mockApi.characterSessionSnapshot.mock.calls.length).toBeGreaterThanOrEqual(3);
    // 錯誤訊息不回顯後端原文。
    expect(document.body.textContent ?? "").not.toContain("boom");
  }, 20_000);
});

// --- evidence-honesty-015：第三份手抄清單 --------------------------------

describe("evidence-honesty-015：使用者指南的同步狀態表不得漏掉任何一態", () => {
  it("DESKTOP-GUIDE 的表格列出 CHARACTER_SYNC_PROJECTION 的每一句 headline", () => {
    const guide = readFileSync(join(__dirname, "../../../../docs/DESKTOP-GUIDE.md"), "utf8");
    const section = guide.slice(guide.indexOf("#### 角色同步"));
    const missing = Object.values(CHARACTER_SYNC_PROJECTION)
      .map((p) => p.headline)
      .filter((headline) => !section.includes(`| ${headline} |`));
    // 漏掉的那一態使用者永遠不會在指南裡看到說明——最需要誠實的
    //「角色同步紀錄曾損毀，已重新開始」正是被漏掉的那一列。
    expect(missing, "docs/DESKTOP-GUIDE.md 的同步狀態表少了這幾句").toEqual([]);
  });
});
