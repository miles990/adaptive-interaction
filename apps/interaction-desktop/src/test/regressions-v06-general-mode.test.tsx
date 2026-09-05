// v0.6.0 一般模式守門測試：角色同步進來之後，一般模式的三條紅線不得鬆動。
//
//   1. 主入口永遠恰好 5 個（角色同步不是第六個入口，它住在角色頁裡）。
//   2. 一般模式不出現技術詞（revision／sequence／epoch／schema／token／…）；
//      這些只在進階模式的「連接診斷」。
//   3. 誠實階梯不因為新卡片鬆動：claimed ≠ verified、綠勾只給真的已同步／已驗證。
//
// 模擬 iPhone（fixture）在文案上永遠自帶標籤，投影不得把它寫成真機。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const FIXTURE_PHONE = "模擬 iPhone（fixture）";

const mockApi = vi.hoisted(() => ({
  characterSessionSnapshot: vi.fn(),
  characterSessionDiagnostics: vi.fn(),
  characterSessionEvent: vi.fn(),
  mobileStatus: vi.fn(),
  providersList: vi.fn(),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { SIMPLE_NAV, simpleNavFor } from "../App";
import { CharacterSyncCard } from "../components/CharacterSyncCard";
import {
  CHARACTER_SYNC_PROJECTION,
  CHARACTER_SYNC_STATES,
  characterSyncPresenceLabel,
  projectCharacterSession,
  projectWorkState,
  type CharacterSyncMember,
} from "../statusProjection";
import { call, configureHttp } from "../transport";
import {
  buildTouchEnvelope,
  CHARACTER_SESSION_ID,
  DESKTOP_SURFACE,
  sendCharacterTouch,
  TOUCH_TTL_MS,
} from "../companion/sessionTouch";
import { validateEnvelope } from "../aip/envelope";

/** 一般模式一個字都不能出現的技術詞。 */
const FORBIDDEN = /revision|sequence|epoch|schema|token|provider|lease|transport|uuid|payload|envelope/i;

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("守門：主入口仍然恰好五個", () => {
  it("角色同步沒有變成第六個一級入口", () => {
    expect(SIMPLE_NAV).toHaveLength(5);
    expect(SIMPLE_NAV.map((t) => t.id)).toEqual(["home", "companion", "work", "connect", "more"]);
    const runtime = simpleNavFor({ name: "小樞", icon: "sparkles" });
    expect(runtime).toHaveLength(5);
    expect(runtime.map((t) => t.id)).toEqual(["home", "companion", "work", "connect", "more"]);
    expect(runtime.some((t) => /同步/.test(t.label))).toBe(false);
  });
});

describe("守門：一般模式沒有技術詞", () => {
  it("十種同步文案（含 presence 標籤）全部是人話", () => {
    for (const state of CHARACTER_SYNC_STATES) {
      const p = CHARACTER_SYNC_PROJECTION[state];
      expect(`${p.headline}｜${p.detail}`).not.toMatch(FORBIDDEN);
    }
    for (const presence of ["online", "reconnecting", "offline", "??"]) {
      expect(characterSyncPresenceLabel(presence)).not.toMatch(FORBIDDEN);
    }
  });

  it("同步卡在一般模式的整段文字沒有技術詞，也不顯示任何診斷數字", async () => {
    // 接收端現在是嚴格解析（`src/aip/sessionClient.ts`）：缺 messageType／revision／sessionEpoch
    // 的 envelope 是 invalid，不會被當成 revision 0 的合法狀態——fixture 要長得像 Runtime 真的送的。
    mockApi.characterSessionSnapshot.mockResolvedValue({
      messageType: "state",
      name: "character.session.snapshot",
      sessionId: "session.home",
      payload: {
        kind: "snapshot",
        revision: 12,
        sessionEpoch: 1,
        state: {
          truth: { state: "none" },
          members: [
            {
              party: { kind: "device", id: "iphone-87b42264" },
              role: "remote-renderer",
              presence: "online",
              // 協商結果齊全（每個 host intent 都演得出來）才有資格給綠色「已同步」；
              // 沒有這個欄位時卡片只會說「已連接，能力核對中」
              //（對抗審查 capability-consent-052／general-mode-ux-022）。
              negotiated: { intents: { "react-happily-to-touch": "exact", celebrate: "exact", settle: "exact", idle: "exact" } },
            },
          ],
          lastInteraction: {
            name: "character.interaction.touch",
            kind: "tap",
            source: "device:iphone-87b42264",
          },
        },
      },
    });
    mockApi.mobileStatus.mockResolvedValue({
      devices: [{ deviceId: "iphone-87b42264", name: FIXTURE_PHONE, connected: true }],
    });
    mockApi.providersList.mockResolvedValue([]);
    render(<CharacterSyncCard refreshKey={0} advanced={false} />);
    const card = await screen.findByTestId("character-sync");
    await waitFor(() =>
      expect(card.textContent ?? "").toContain("iPhone 已連接，角色狀態已同步")
    );
    expect(card.textContent ?? "").not.toMatch(FORBIDDEN);
    // 模擬手機的名稱原樣顯示（標籤自帶，不得被改寫成真機）。
    expect(card.textContent ?? "").toContain(FIXTURE_PHONE);
    // 診斷會被讀（`storeNote` 是「紀錄曾損毀」唯一的來源，不得靜默），但一般模式
    // 不得出現「連接診斷」區塊，也不得印出任何診斷數字——上面兩行守的就是這件事。
    expect(card.querySelector(".tech-details")).toBeNull();
  });
});

describe("守門：誠實階梯不因為同步卡而鬆動", () => {
  it("claimed-completed 仍然不是 verified", () => {
    const claimed = projectWorkState("claimed-completed");
    const verified = projectWorkState("verified");
    expect(claimed.kind).toBe("claimed");
    expect(claimed.badge).not.toBe("ok");
    expect(claimed.needsDecision).toBe(true);
    expect(claimed.honesty).toBeTruthy();
    expect(verified.kind).toBe("verified");
    expect(verified.badge).toBe("ok");
  });

  it("只有真的已同步才給 ok；其餘八種一律不是綠勾", () => {
    for (const state of CHARACTER_SYNC_STATES) {
      const tone = CHARACTER_SYNC_PROJECTION[state].tone;
      if (state === "synced") expect(tone).toBe("ok");
      else expect(tone).not.toBe("ok");
    }
  });

  it("裝置說得再多也改不了同步結論：只有 online 才算已同步", () => {
    const member = (presence: string): CharacterSyncMember => ({
      name: FIXTURE_PHONE,
      remote: true,
      presence,
      canPresent: true,
      degraded: false,
    });
    const snapshot = { payload: { state: { truth: { state: "none" }, members: [] } } };
    const signals = {
      enabled: true,
      failedReads: 0,
      revokedDevice: false,
      connectedButNotSynced: false,
      storeReset: false,
    };
    for (const presence of ["offline", "reconnecting", "totally-fine"]) {
      expect(projectCharacterSession(snapshot, [member(presence)], signals).state).not.toBe(
        "synced"
      );
    }
  });
});

describe("守門：四條角色同步路由與 HTTP 綁定一致", () => {
  it("Tauri 指令名 → docs/aip/transport-bindings.md §2 的路由", async () => {
    configureHttp("http://127.0.0.1:8787", "test-token");
    const seen: { method: string; url: string }[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        seen.push({ method: init?.method ?? "GET", url: String(url) });
        return new Response("{}", { status: 200 });
      })
    );
    await call("character_session_snapshot");
    await call("character_session_resume", { lastRevision: 1, lastSequence: 2, epoch: 3 });
    await call("character_session_events", { envelope: {} });
    await call("character_session_diagnostics");
    expect(seen).toEqual([
      { method: "GET", url: "http://127.0.0.1:8787/v1/character-session" },
      { method: "POST", url: "http://127.0.0.1:8787/v1/character-session/resume" },
      { method: "POST", url: "http://127.0.0.1:8787/v1/character-session/events" },
      { method: "GET", url: "http://127.0.0.1:8787/v1/character-session/diagnostics" },
    ]);
  });
});

describe("守門：桌面角色被點擊 → Character Session 的語意事件", () => {
  it("信封的身分固定是可信 host surface，而且一定帶 5 秒 deadline", () => {
    const now = Date.UTC(2026, 8, 4, 12, 30, 0);
    const envelope = buildTouchEnvelope(now, "tap", "desktop-touch-1");
    expect(envelope.source).toEqual(DESKTOP_SURFACE);
    expect(envelope.source.kind).not.toBe("renderer");
    expect(envelope.sessionId).toBe(CHARACTER_SESSION_ID);
    expect(envelope.name).toBe("character.interaction.touch");
    expect(envelope.payload).toEqual({ kind: "tap" });
    expect(Date.parse(String(envelope.expiresAt)) - now).toBe(TOUCH_TTL_MS);
    expect(validateEnvelope(envelope).ok).toBe(true);
  });

  it("送出 ≠ 生效：只有後端說 applied 才是 applied，其餘照實回報", async () => {
    mockApi.characterSessionEvent.mockResolvedValue({ payload: { status: "applied" } });
    await expect(sendCharacterTouch("tap")).resolves.toBe("applied");
    mockApi.characterSessionEvent.mockResolvedValue({ payload: { status: "rejected", code: "not-a-member" } });
    await expect(sendCharacterTouch("tap")).resolves.toBe("rejected");
    mockApi.characterSessionEvent.mockResolvedValue({ payload: {} });
    await expect(sendCharacterTouch("tap")).resolves.toBe("unknown");
    mockApi.characterSessionEvent.mockRejectedValue(new Error("500: boom"));
    await expect(sendCharacterTouch("tap")).resolves.toBe("unknown");
  });

  it("每一則的訊息識別碼都不同（重送不會被當成同一次觸摸）", () => {
    const now = Date.now();
    const a = buildTouchEnvelope(now, "tap");
    const b = buildTouchEnvelope(now, "tap");
    expect(a.messageId).not.toBe(b.messageId);
    expect(a.messageId.length).toBeLessThanOrEqual(128);
  });
});
