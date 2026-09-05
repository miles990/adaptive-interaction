// 深連結盤點（維護性 §6 第十條）：「有幾種方式可以進到某一頁」必須是**可列舉**的，
// 而且每一種都真的落到正確的頁面。
//
// 控制中心沒有 URL router（Tauri 的 WebviewUrl::App 不帶 query，見 main.tsx），
// 所以「路由」就是一個 tab id 字串。它可以從六個地方進來：
//
//   1. 側邊欄一級導覽      routing.SIMPLE_NAV（5 個）＋進階模式的 ADVANCED_NAV（9 個）
//   2. 窄視窗底部導覽      routing.NARROW_PRIMARY（4 個）＋NARROW_MORE_ITEMS（5 個）
//   3. 舊錨點相容表        routing.LEGACY_ANCHORS 的 13 個 key（舊書籤／舊深連結）
//   4. 狀態列／桌面角色    Rust 端 `emit("navigate", <tab>)`（tray「外觀與語言…」＝
//                          settings；角色視窗 companion_open_control_center 帶 activity）
//   5. ⌘K 全域搜尋        components/GlobalSearch.tsx 的 PAGES 表與能力／指令項
//   6. 通知中心「前往」    後端 inbox item 給的 `route` 字串（**未經白名單**）
//
// 這裡把六條路徑逐一釘住：id → 導覽高亮（navAnchorFor）、→ 標題（titleFor）、
// → 真正渲染出來的頁面與它的內部分頁（PageBody）。任何人新增一個入口卻忘了
// 接上 PageBody，或把 PageBody 的 case 刪掉，這個檔案就會紅。
//
// 相容路由在**已掛載**元件上的分頁切換由 compat-routes.test.tsx 涵蓋；這裡只管
// 「每一個深連結都到得了、到對地方」的完整盤點。

import React from "react";
import { readFileSync } from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

// PageBody 直接掛載的每一個頁面都換成可辨識的 stub：這個檔案要驗的是「分派」，
// 不是各頁面的內容（那是各頁自己的測試）。stub 順便把 `initial` prop 印出來，
// 因為「舊 id 進到哪一個內部分頁」正是深連結契約的一部分。
// `vi.hoisted`：vi.mock 的 factory 會被提升到檔案最上面，共用的 stub 產生器
// 必須跟著提升，否則 factory 執行時它還沒初始化。
const { stub } = vi.hoisted(() => ({
  stub: (name: string) =>
    function Stub(props: Record<string, unknown>) {
      return React.createElement(
        "div",
        {
          "data-testid": `page-${name}`,
          "data-initial": String(props.initial ?? ""),
          // 深連結帶的「是哪一台裝置」：頁面收不到就等於那個參數是死的。
          "data-focus-device": String(props.focusDeviceId ?? ""),
        },
        `STUB ${name}`
      );
    },
}));

vi.mock("../pages/HomePage", () => ({ HomePage: stub("home"), PermissionMap: () => <div /> }));
vi.mock("../pages/CompanionPage", () => ({ CompanionPage: stub("companion") }));
vi.mock("../pages/WorkPage", () => ({ WorkPage: stub("work") }));
vi.mock("../pages/ConnectPage", () => ({
  ConnectPage: stub("connect"),
  // App.tsx／NotificationPanel 也從這個模組拿收件匣工具，mock 必須補齊。
  loadDecisionInbox: () => Promise.resolve({}),
  decisionPage: () => ({ shown: [], notShown: 0, exact: true }),
}));
vi.mock("../pages/MorePage", () => ({ MorePage: stub("more"), MORE_TABS: [] }));
vi.mock("../pages/CapabilitiesPage", () => ({
  CapabilitiesPage: (props: { kind: string }) => (
    <div data-testid="page-capabilities-single" data-kind={props.kind} />
  ),
}));
vi.mock("../pages/Overview", () => ({ OverviewPage: stub("adv-overview") }));
vi.mock("../pages/Receptors", () => ({ ReceptorsPage: stub("adv-receptors") }));
vi.mock("../pages/Actuators", () => ({ ActuatorsPage: stub("adv-actuators") }));
vi.mock("../pages/Tools", () => ({ ToolsPage: stub("adv-tools") }));
vi.mock("../pages/Recipes", () => ({ RecipesPage: stub("adv-recipes") }));
vi.mock("../pages/Policy", () => ({ PolicyPage: stub("adv-policy") }));
vi.mock("../pages/Timeline", () => ({ TimelinePage: stub("adv-timeline") }));
vi.mock("../pages/ProvidersAdvanced", () => ({ ProvidersAdvancedPage: stub("adv-providers") }));
vi.mock("../pages/KnowledgeAdvanced", () => ({ KnowledgeAdvancedPage: stub("adv-knowledge") }));

import { PageBody } from "../App";
import {
  ADVANCED_NAV,
  LEGACY_ANCHORS,
  moreSheetCurrent,
  NARROW_MORE_ITEMS,
  NARROW_PRIMARY,
  navAnchorFor,
  simpleNavFor,
  SIMPLE_NAV,
  titleFor,
  type Tab,
} from "../routing";
import { useNavigation } from "../useNavigation";
import { characterNameFallback, NEUTRAL_CHARACTER_ICON } from "../characterName";

afterEach(() => {
  vi.restoreAllMocks();
});

/** 渲染一個深連結目標，回傳它落到的頁面 stub 與內部分頁。 */
function open(tab: Tab): { page: string; initial: string } {
  const view = render(
    <PageBody
      tab={tab}
      refreshKey={0}
      events={[]}
      advanced={false}
      onNavigate={() => {}}
      onRerunOnboarding={() => {}}
    />
  );
  const hit = view.container.querySelector("[data-testid^='page-']");
  if (!hit) {
    const fallback = view.container.textContent ?? "";
    view.unmount();
    return { page: `NO-PAGE(${fallback.slice(0, 40)})`, initial: "" };
  }
  const page = (hit.getAttribute("data-testid") ?? "").replace(/^page-/, "");
  const initial = hit.getAttribute("data-initial") ?? hit.getAttribute("data-kind") ?? "";
  view.unmount();
  return { page, initial };
}

/**
 * 全部深連結 → 期望落點。**這張表就是盤點結果**：新增入口要同時加在這裡，
 * 否則下面「盤點完整」的測試會抓到漏網的 id。
 */
const EXPECTED: Record<string, { page: string; initial: string; anchor: string; title: string }> = {
  // 一級入口
  home: { page: "home", initial: "", anchor: "home", title: "現在" },
  companion: { page: "companion", initial: "", anchor: "companion", title: characterNameFallback },
  work: { page: "work", initial: "sessions", anchor: "work", title: "工作" },
  connect: { page: "connect", initial: "devices", anchor: "connect", title: "連接與權限" },
  more: { page: "more", initial: "memory", anchor: "more", title: "更多" },
  // 舊錨點（LEGACY_ANCHORS）：折疊到一級入口，內容進到對應的內部分頁
  ai: { page: "work", initial: "sessions", anchor: "work", title: "工作" },
  automations: { page: "work", initial: "automations", anchor: "work", title: "工作" },
  capabilities: { page: "connect", initial: "devices", anchor: "connect", title: "連接與權限" },
  safety: { page: "connect", initial: "safety", anchor: "connect", title: "連接與權限" },
  senses: { page: "capabilities-single", initial: "receptor", anchor: "connect", title: "連接與權限" },
  responses: { page: "capabilities-single", initial: "actuator", anchor: "connect", title: "連接與權限" },
  toolops: {
    page: "capabilities-single",
    initial: "tool-operation",
    anchor: "connect",
    title: "連接與權限",
  },
  memory: { page: "more", initial: "memory", anchor: "more", title: "更多" },
  activity: { page: "more", initial: "activity", anchor: "more", title: "更多" },
  settings: { page: "more", initial: "settings", anchor: "more", title: "更多" },
  backup: { page: "more", initial: "backup", anchor: "more", title: "更多" },
  manage: { page: "more", initial: "manage", anchor: "more", title: "更多" },
  "advanced-features": { page: "more", initial: "advanced-features", anchor: "more", title: "更多" },
  // 進階模式的原始技術頁
  "adv-overview": { page: "adv-overview", initial: "", anchor: "adv-overview", title: "總覽（原始）" },
  "adv-receptors": { page: "adv-receptors", initial: "", anchor: "adv-receptors", title: "受器" },
  "adv-actuators": { page: "adv-actuators", initial: "", anchor: "adv-actuators", title: "動器" },
  "adv-tools": { page: "adv-tools", initial: "", anchor: "adv-tools", title: "工具" },
  "adv-recipes": { page: "adv-recipes", initial: "", anchor: "adv-recipes", title: "配方 YAML" },
  "adv-policy": { page: "adv-policy", initial: "", anchor: "adv-policy", title: "政策／同意" },
  "adv-timeline": { page: "adv-timeline", initial: "", anchor: "adv-timeline", title: "時間軸" },
  "adv-providers": {
    page: "adv-providers",
    initial: "",
    anchor: "adv-providers",
    title: "Provider Registry",
  },
  "adv-knowledge": {
    page: "adv-knowledge",
    initial: "",
    anchor: "adv-knowledge",
    title: "Knowledge Graph",
  },
};

describe("深連結盤點：每一個進到某一頁的方式", () => {
  it("盤點表涵蓋所有入口的 id（沒有沒被盤點到的深連結）", () => {
    const declared = new Set([
      ...SIMPLE_NAV.map((t) => t.id),
      ...ADVANCED_NAV.map((t) => t.id),
      ...NARROW_PRIMARY,
      ...NARROW_MORE_ITEMS.map((t) => t.id),
      ...Object.keys(LEGACY_ANCHORS),
    ]);
    const missing = [...declared].filter((id) => !(id in EXPECTED));
    expect(missing, "有入口沒有寫進 EXPECTED 盤點表").toEqual([]);
    // 反向：盤點表不得列出根本不存在的入口（除了 PageBody 自有的 adv-* 與一級入口，
    // 這兩類已經在 declared 裡）。
    const stale = Object.keys(EXPECTED).filter((id) => !declared.has(id));
    expect(stale, "盤點表列了不存在的入口").toEqual([]);
  });

  it.each(Object.entries(EXPECTED))(
    "%s → 正確的頁面、內部分頁、導覽高亮與標題",
    (tab, want) => {
      const got = open(tab);
      expect(got.page, `${tab} 應落在 ${want.page}`).toBe(want.page);
      expect(got.initial, `${tab} 的內部分頁`).toBe(want.initial);
      expect(navAnchorFor(tab), `${tab} 的導覽高亮`).toBe(want.anchor);
      expect(titleFor(tab), `${tab} 的標題`).toBe(want.title);
      expect(titleFor(tab), `${tab} 不得沒有標題`).not.toBe("未知頁面");
    }
  );

  it("舊錨點一個都不能少：13 個 key 全部仍可用且折疊到現有一級入口", () => {
    const anchors = new Set(SIMPLE_NAV.map((t) => t.id));
    expect(Object.keys(LEGACY_ANCHORS).sort()).toEqual(
      [
        "advanced-features",
        "ai",
        "activity",
        "automations",
        "backup",
        "capabilities",
        "manage",
        "memory",
        "responses",
        "safety",
        "senses",
        "settings",
        "toolops",
      ].sort()
    );
    for (const [legacy, target] of Object.entries(LEGACY_ANCHORS)) {
      expect(anchors.has(target), `${legacy} 折疊到不存在的入口 ${target}`).toBe(true);
      expect(open(legacy).page, `${legacy} 不得掉進「找不到這個頁面」`).not.toMatch(/^NO-PAGE/);
    }
  });
});

describe("深連結：狀態列／桌面角色（Rust emit(\"navigate\", …)）", () => {
  const repo = (rel: string) => readFileSync(path.resolve(rel), "utf8");

  it("Rust 與角色視窗送出的 tab 字面值都是可路由的深連結", () => {
    const lib = repo("src-tauri/src/lib.rs");
    const companion = repo("src/companion/CompanionApp.tsx");
    // 狀態列「外觀與語言…」→ emit("navigate", "settings")
    const emitted = [...lib.matchAll(/emit\("navigate",\s*"([a-z-]+)"\)/g)].map((m) => m[1]);
    // 角色視窗 → invoke("companion_open_control_center", { tab: "activity" })
    const invoked = [
      ...companion.matchAll(/companion_open_control_center",\s*\{\s*tab:\s*"([a-z-]+)"/g),
    ].map((m) => m[1]);
    expect(emitted, "lib.rs 應至少有一個 navigate 深連結").toContain("settings");
    expect(invoked, "角色視窗應能開到活動紀錄").toContain("activity");
    for (const tab of [...emitted, ...invoked]) {
      expect(tab in EXPECTED, `Rust／角色視窗送出的 "${tab}" 不在深連結盤點表裡`).toBe(true);
      expect(open(tab).page, `"${tab}" 不得掉進「找不到這個頁面」`).not.toMatch(/^NO-PAGE/);
    }
  });

  it("控制中心真的有把 desktop 的 navigate 事件接到導覽上", () => {
    // 事件 → goTo 的接線在 App.tsx 的 Shell；接線被拆掉的話狀態列選單會變成死點擊。
    const app = repo("src/App.tsx");
    expect(app).toContain("onNavigate((t) => goTo(t))");
  });
});

describe("深連結：⌘K 全域搜尋的頁面表", () => {
  it("GlobalSearch 列出的每一頁都在盤點表裡，也都渲染得出來", () => {
    const source = readFileSync(path.resolve("src/components/GlobalSearch.tsx"), "utf8");
    const table = source.slice(source.indexOf("const PAGES"));
    const ids = [...table.slice(0, table.indexOf("];")).matchAll(/id:\s*"([a-z-]+)"/g)].map(
      (m) => m[1]
    );
    expect(ids.length, "PAGES 表應該被掃到").toBeGreaterThan(10);
    for (const id of ids) {
      expect(id in EXPECTED, `⌘K 的 "${id}" 不在深連結盤點表裡`).toBe(true);
      expect(open(id).page, `⌘K 的 "${id}" 不得掉進「找不到這個頁面」`).not.toMatch(/^NO-PAGE/);
    }
    // ⌘K 也會直接導到這幾個 id（能力項、安全指令），同樣要可路由。
    for (const id of ["capabilities", "safety", "memory", "activity", "ai"]) {
      expect(open(id).page).not.toMatch(/^NO-PAGE/);
    }
  });
});

describe("深連結：通知中心「前往」（後端給的 route，未經白名單）", () => {
  it("認得的 route 到得了該頁", () => {
    expect(open("ai").page).toBe("work");
    expect(open("activity").page).toBe("more");
  });

  it("不認得的 route 不靜默空白：說不確定，並留一條回得去的路", () => {
    const goTo = vi.fn();
    render(
      <PageBody
        tab="a-route-from-a-newer-daemon"
        refreshKey={0}
        events={[]}
        advanced={false}
        onNavigate={goTo}
        onRerunOnboarding={() => {}}
      />
    );
    expect(screen.getByRole("alert")).toHaveTextContent("找不到這個頁面");
    screen.getByRole("button", { name: "回到「現在」" }).click();
    expect(goTo).toHaveBeenCalledWith("home");
  });
});

describe("角色改名後，'companion' 仍是同一個 route", () => {
  it("導覽第二項換的是 label／icon，id 永遠是 companion", () => {
    const renamed = simpleNavFor({ name: "小助手", icon: "sparkles" });
    expect(renamed.map((t) => t.id)).toEqual(SIMPLE_NAV.map((t) => t.id));
    expect(renamed[1]).toEqual({ id: "companion", label: "小助手", icon: "sparkles" });
    // 靜態表本身仍是中立值（不被執行期改名污染）。
    expect(SIMPLE_NAV[1]).toEqual({
      id: "companion",
      label: characterNameFallback,
      icon: NEUTRAL_CHARACTER_ICON,
    });
  });

  it("改名只影響標題文字，不影響 anchor 與落點", () => {
    expect(titleFor("companion", "小助手")).toBe("小助手");
    expect(titleFor("companion")).toBe(characterNameFallback);
    expect(navAnchorFor("companion")).toBe("companion");
    expect(open("companion").page).toBe("companion");
  });
});

describe("窄視窗「更多」選單的深連結", () => {
  it("每個細項都是可路由的 id，且高亮用未折疊的原始路由", () => {
    for (const item of NARROW_MORE_ITEMS) {
      expect(item.id in EXPECTED, `${item.id} 不在盤點表裡`).toBe(true);
      expect(open(item.id).page).not.toMatch(/^NO-PAGE/);
      expect(moreSheetCurrent(item.id)).toBe(item.id);
    }
    // 裸的 `more` 對應 PageBody 的預設分頁（記憶與資料），高亮才不會全部熄掉。
    expect(moreSheetCurrent("more")).toBe("memory");
    expect(open("more").initial).toBe("memory");
  });

  it("底部導覽的 4 個一級入口都是 SIMPLE_NAV 的成員", () => {
    const ids = new Set(SIMPLE_NAV.map((t) => t.id));
    for (const id of NARROW_PRIMARY) expect(ids.has(id), `${id} 不是一級入口`).toBe(true);
  });
});

describe("深連結可以帶「落點參數」，但 route id 不變", () => {
  it("goTo('connect', { hub: 'providers' }) 一步到配對區；下一次不帶就回到預設分頁", () => {
    let nav: ReturnType<typeof useNavigation> | null = null;
    function Probe() {
      nav = useNavigation("home");
      return (
        <PageBody
          tab={nav.tab}
          refreshKey={0}
          events={[]}
          advanced={false}
          onNavigate={nav.goTo}
          navOptions={nav.options}
          onRerunOnboarding={() => {}}
        />
      );
    }
    render(<Probe />);
    // 角色同步卡的「連接手機／去重新確認」：同一個 route id（connect），落點是第二層的配對區。
    act(() => nav!.goTo("connect", { hub: "providers", deviceId: "iphone-1" }));
    expect(nav!.tab).toBe("connect");
    expect(screen.getByTestId("page-connect").getAttribute("data-initial")).toBe("providers");
    // 「去重新確認」還帶著是哪一台：ConnectPage 收得到才有辦法把人帶到那張卡片
    //（對抗審查 general-mode-ux-014：算出來卻沒有人消費就是名不副實的落點）。
    expect(screen.getByTestId("page-connect").getAttribute("data-focus-device")).toBe("iphone-1");
    // 參數只對這一次導覽有效：側邊欄再按一次「連接與權限」就回到預設的「裝置與能力」。
    act(() => nav!.goTo("connect"));
    expect(screen.getByTestId("page-connect").getAttribute("data-initial")).toBe("devices");
    expect(screen.getByTestId("page-connect").getAttribute("data-focus-device")).toBe("");
    // 不認得的 hub 值不會變成別的分頁（只認 providers）。
    act(() => nav!.goTo("connect", { hub: "nope" }));
    expect(screen.getByTestId("page-connect").getAttribute("data-initial")).toBe("devices");
  });
});

describe("深連結導到「已經在的那一頁」也要有作用", () => {
  it("useNavigation 的 mountKey 每次導覽都改變（重複的深連結會重新掛載）", () => {
    let nav: ReturnType<typeof useNavigation> | null = null;
    function Probe() {
      nav = useNavigation("connect");
      return <div>{nav.mountKey}</div>;
    }
    render(<Probe />);
    const first = nav!.mountKey;
    // 緊急停止中在安全頁重複按「前往解除」：路由沒變，但內部分頁必須被拉回 safety。
    act(() => nav!.goTo("safety"));
    const second = nav!.mountKey;
    act(() => nav!.goTo("safety"));
    const third = nav!.mountKey;
    expect(second).not.toBe(first);
    expect(third).not.toBe(second);
    expect(nav!.tab).toBe("safety");
  });
});
