// 跨頁整合：後端 activity inbox 的 `pendingCountExact`（activity.rs）與
// 新事件 `sensor.stop-uncertain` 的誠實呈現。
//
// 規則（不可違反）：
// - `pendingCountExact === false` 代表 pendingCount 只是**下限**：徽章要說「至少 N」，
//   而且**任何介面都不得**在這個旗標為 false 時說「目前沒有待決定事項」／
//   「現在沒有需要你決定的事」。
// - 舊 daemon 不送這個欄位（undefined）＝ 精確，行為與以前完全一樣。
// - `sensor.stop-uncertain`＝要求停止但沒等到確認：它是「待你決定」，不是純歷史。

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { api } from "../api";
import { AppStateProvider } from "../appstate";
import { inboxBadgeLabel, inboxBadgeText, NotificationPanel } from "../App";
import { decisionPage, localStopLine } from "../pages/ConnectPage";
import { NowStrip } from "../pages/HomePage";
import {
  inboxItemTitle,
  INBOX_STATUSES,
  isPendingCountExact,
  PENDING_INCOMPLETE_NOTE,
  pendingCountLabel,
  projectInboxStatus,
} from "../statusProjection";

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------
// 純投影
// ---------------------------------------------------------------------------

describe("pendingCountExact：純投影", () => {
  it("缺席＝精確（舊 daemon 不會被誤判成不精確）", () => {
    expect(isPendingCountExact({ pendingCount: 3 })).toBe(true);
    expect(isPendingCountExact(null)).toBe(true);
    expect(isPendingCountExact({ pendingCountExact: true })).toBe(true);
  });

  it("false＝不精確；不是布林的值也一律當成不精確（寧可說「至少」）", () => {
    expect(isPendingCountExact({ pendingCountExact: false })).toBe(false);
    expect(isPendingCountExact({ pendingCountExact: "maybe" })).toBe(false);
    expect(isPendingCountExact({ pendingCountExact: null })).toBe(false);
  });

  it("不精確時數字一定要加「至少」", () => {
    expect(pendingCountLabel(3, true)).toBe("3 項");
    expect(pendingCountLabel(3, false)).toBe("至少 3 項");
    expect(pendingCountLabel(0, false)).toBe("至少 0 項");
  });

  it("decisionPage 一併回報 exact，讓三個介面用同一份真相", () => {
    const items = [{ kind: "action-result", itemId: "a", needsDecision: true }];
    expect(decisionPage({ items, pendingCount: 1 }, 10).exact).toBe(true);
    expect(decisionPage({ items, pendingCount: 1, pendingCountExact: false }, 10).exact).toBe(
      false
    );
    // 沒有任何項目、pendingCount 0，但後端說不精確 → 不得被當成「沒有待決定」。
    const empty = decisionPage({ items: [], pendingCount: 0, pendingCountExact: false }, 10);
    expect(empty).toMatchObject({ shown: [], notShown: 0, pendingCount: 0, exact: false });
  });
});

// ---------------------------------------------------------------------------
// 右上角通知中心（App.tsx）
// ---------------------------------------------------------------------------

describe("通知中心徽章：不精確時說「至少 N」", () => {
  it("精確時就是數字本身", () => {
    expect(inboxBadgeText({ pendingCount: 4 })).toBe("4");
    expect(inboxBadgeLabel({ pendingCount: 4 })).toBe("4 項");
  });

  it("pendingCountExact:false → 徽章與 aria-label 都要說「至少」", () => {
    expect(inboxBadgeText({ pendingCount: 1000, pendingCountExact: false })).toBe("至少 1000");
    expect(inboxBadgeLabel({ pendingCount: 1000, pendingCountExact: false })).toBe("至少 1000 項");
  });

  it("面板：pendingCountExact:false 且本頁空的時候，絕不說「目前沒有待決定事項」", () => {
    render(
      <NotificationPanel
        inbox={{ pendingCount: 0, count: 0, items: [], pendingCountExact: false }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(screen.queryByText("目前沒有待決定事項。")).toBeNull();
    expect(screen.getByText(new RegExp(PENDING_INCOMPLETE_NOTE))).toBeTruthy();
  });

  it("面板：真的精確且沒有待決定時，照樣說「目前沒有待決定事項」", () => {
    render(
      <NotificationPanel
        inbox={{ pendingCount: 0, count: 0, items: [] }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(screen.getByText("目前沒有待決定事項。")).toBeTruthy();
    expect(screen.queryByText(new RegExp(PENDING_INCOMPLETE_NOTE))).toBeNull();
  });

  it("面板：有裝不下的待決定時，不精確要說「至少還有 N 項」", () => {
    render(
      <NotificationPanel
        inbox={{
          pendingCount: 1200,
          items: [
            {
              kind: "action-result",
              itemId: "a",
              status: "uncertain",
              title: "送到 iPhone 的結果未知",
              route: "activity",
              needsDecision: true,
            },
          ],
          pendingCountExact: false,
        }}
        onClose={() => {}}
        onNavigate={() => {}}
      />
    );
    expect(screen.getByText(/至少還有 1199 項待決定不在這一頁/)).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// 首頁「待我決定」（HomePage NowStrip）
// ---------------------------------------------------------------------------

function stubNowStrip(inbox: Record<string, unknown>) {
  vi.spyOn(api, "agentSessionsList").mockResolvedValue([]);
  vi.spyOn(api, "activityInbox").mockResolvedValue(inbox);
}

async function renderNowStrip(inbox: Record<string, unknown>) {
  stubNowStrip(inbox);
  const utils = render(
    <AppStateProvider ready refreshKey={0}>
      <NowStrip refreshKey={0} status={{ activeSensors: [] }} onNavigate={() => {}} />
    </AppStateProvider>
  );
  return utils;
}

describe("首頁「待我決定」：不精確時不得顯示綠色的 0 項", () => {
  it("pendingCountExact:false、pendingCount:0 → 「至少 0 項」＋未載入說明", async () => {
    await renderNowStrip({ pendingCount: 0, items: [], pendingCountExact: false });
    const card = await screen.findByTestId("now-decisions");
    expect(within(card).getByText("至少 0 項")).toBeTruthy();
    expect(within(card).getByText(new RegExp(PENDING_INCOMPLETE_NOTE))).toBeTruthy();
    expect(within(card).queryByText("0 項")).toBeNull();
  });

  it("精確且為 0 → 維持原本的綠色「0 項」，不多嚇人一句", async () => {
    await renderNowStrip({ pendingCount: 0, items: [] });
    const card = await screen.findByTestId("now-decisions");
    expect(within(card).getByText("0 項")).toBeTruthy();
    expect(within(card).queryByText(new RegExp(PENDING_INCOMPLETE_NOTE))).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// 新事件 sensor.stop-uncertain
// ---------------------------------------------------------------------------

describe("sensor.stop-uncertain：要求停止 ≠ 已停止", () => {
  it("投影成「停止結果不確定」，而且算待你決定", () => {
    const p = projectInboxStatus("sensor.stop-uncertain");
    expect(p.known).toBe(true);
    expect(p.label).toBe("停止結果不確定");
    expect(p.needsDecision).toBe(true);
    expect(p.badge).toBe("warn");
    // 不得被講成已停止／已完成。
    expect(p.kind).toBe("unknown");
  });

  it("列在 INBOX_STATUSES 裡（介面認得，不會落到「結果不確定」的兜底）", () => {
    expect(INBOX_STATUSES).toContain("sensor.stop-uncertain");
  });

  it("後端已給人話標題就照用（含裝置名）", () => {
    expect(
      inboxItemTitle({
        kind: "safety-event",
        status: "sensor.stop-uncertain",
        title: "感測停止結果不確定：Alex 的 iPhone",
      })
    ).toBe("感測停止結果不確定：Alex 的 iPhone");
  });

  it("舊 daemon 把原始 event_type 當標題時，翻成人話而不是印 `sensor.stop-uncertain`", () => {
    const title = inboxItemTitle({
      kind: "safety-event",
      status: "sensor.stop-uncertain",
      title: "sensor.stop-uncertain",
      detail: { payload: { sensor: "iphone.mic-level" } },
    });
    expect(title).toBe("感測停止結果不確定：麥克風");
    expect(title).not.toContain("sensor.");
  });
});

// ---------------------------------------------------------------------------
// SensorStopReport.local 是物件，不是布林
// ---------------------------------------------------------------------------

describe("停止所有感測：local 是 {microphone: stopped|idle}", () => {
  it("stopped＝本來在擷取、現在停了", () => {
    expect(localStopLine({ stopped: true, local: { microphone: "stopped" } })).toBe(
      "這台電腦：已停止本機感測（麥克風）。"
    );
  });

  it("idle＝本來就沒在擷取（不是失敗，也不能講成剛剛停下來）", () => {
    expect(localStopLine({ stopped: true, local: { microphone: "idle" } })).toBe(
      "這台電腦：本機本來就沒有在感測。"
    );
  });

  it("認不得的值＝結果不確定，不猜成功", () => {
    expect(localStopLine({ local: { microphone: "???" } })).toContain("結果不確定");
    expect(localStopLine({ local: {} })).toContain("結果不確定");
  });

  it("舊 daemon 的布林值仍然看得懂（相容）", () => {
    expect(localStopLine({ local: true } as never)).toBe("這台電腦：已停止本機感測。");
    expect(localStopLine({ local: false } as never)).toContain("結果不確定");
  });
});
