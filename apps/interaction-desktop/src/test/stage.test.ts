// StageRenderer 的 bounded hit regions（companion-gameplay-032，TS 端）。
//
// 舊行為：只回報「角色 ∪ 所有玩具」的一個聯集矩形，於是把毛球丟到遠處時，
// 角色與玩具之間那一大條空白既不穿透桌面、點下去也毫無反應。
// 新行為：角色／每個使魔／每個玩具各一個 bounded region，空白區屬於桌面。
//
// 這支測 StageRenderer 這一層（純函式層在 hitRegions.test.ts）：
//   - interactiveRegions() 的內容與界限
//   - onHitRegions 的節流／force 心跳（沿用 hitRectReportPolicy 的意圖）
//   - 拖曳中角色 region 持續存在
//   - Reduced Motion 下仍正確
//   - 與 sendHitRegions 串起來的 invoke payload 形狀
//   - 舊的聯集 onHitRect 仍然可用（相容）

import fs from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { machineStageFlags, StageRenderer } from "../companion/rig/stage";
import {
  HitRegion,
  MAX_HIT_REGIONS,
  resetHitRegionIpcMode,
  resetHitRegionWarnings,
  sendHitRegions,
} from "../companion/hitRegions";

beforeEach(() => {
  resetHitRegionWarnings();
  resetHitRegionIpcMode();
});

/** jsdom 沒有 canvas 2D：可鏈式 stub（stage 只呼叫繪圖指令，不讀像素）。 */
function stubCanvas(w = 416, h = 216): HTMLCanvasElement {
  const store: Record<string | symbol, unknown> = {};
  const ctx: unknown = new Proxy(store, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return () => ctx;
    },
    set(target, prop, value) {
      target[prop] = value;
      return true;
    },
  });
  return {
    clientWidth: w,
    clientHeight: h,
    width: w,
    height: h,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: w, height: h }),
  } as unknown as HTMLCanvasElement;
}

function makeStage(opts: { rng?: () => number; reduced?: boolean } = {}) {
  const clock = { t: 1_000 };
  const stage = new StageRenderer(stubCanvas(), "maid-classic", 1, {
    autoStart: false,
    rng: opts.rng ?? (() => 0.9),
    now: () => clock.t,
  });
  stage.setAnimation("idle");
  stage.setMachineFlags(machineStageFlags("idle", null, "idle", true));
  if (opts.reduced) stage.setReducedMotion(true);
  const frames = (n: number, stepMs = 16) => {
    for (let i = 0; i < n; i++) {
      clock.t += stepMs;
      stage.renderFrame(clock.t);
    }
  };
  return { stage, clock, frames };
}

const inAny = (regions: HitRegion[], x: number, y: number) =>
  regions.some((g) => x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h);

const inRect = (r: { x: number; y: number; w: number; h: number }, x: number, y: number) =>
  x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;

/** 讓玩具停在遠處：關掉遊玩與桌面移動，角色不會去追它。 */
function stillStageWithToy(reduced = false) {
  const h = makeStage({ reduced });
  h.stage.setToggles({ play: false, cursorPlay: false, deskMove: false });
  expect(h.stage.spawnToy("yarn")).toBe(true);
  h.frames(40, 16);
  return h;
}

describe("interactiveRegions：角色／使魔／玩具各一個框（不是一個聯集）", () => {
  it("角色與遠處玩具之間的空白不在任何 region 內，但確實在舊聯集框內", () => {
    const { stage } = stillStageWithToy();
    const regions = stage.interactiveRegions();
    const char = regions.find((g) => g.id === "character");
    const toy = regions.find((g) => g.id.startsWith("toy:"));
    expect(char).toBeDefined();
    expect(toy).toBeDefined();
    // 玩具生在角色左邊（world.w*0.25）——先確認兩個框真的沒有重疊。
    const toyRight = toy!.x + toy!.w;
    expect(toyRight).toBeLessThan(char!.x);
    const gapX = (toyRight + char!.x) / 2;
    const gapY = toy!.y + toy!.h / 2;
    expect(inAny(regions, gapX, gapY)).toBe(false); // 新：空白屬於桌面
    expect(inRect(stage.interactiveBounds(), gapX, gapY)).toBe(true); // 舊：聯集吃掉它
    // 角色身上與玩具身上都還是要攔截。
    expect(inAny(regions, char!.x + char!.w / 2, char!.y + char!.h / 2)).toBe(true);
    expect(inAny(regions, toy!.x + toy!.w / 2, gapY)).toBe(true);
    stage.destroy();
  });

  it("每個使魔各一個框，且各自分開", () => {
    const { stage, frames } = makeStage();
    stage.setToggles({ play: false, cursorPlay: false, deskMove: false });
    stage.setFamiliars([
      { id: "a", name: "a", palette: "maid-classic" },
      { id: "b", name: "b", palette: "maid-classic" },
      { id: "c", name: "c", palette: "maid-classic" },
    ]);
    frames(5, 16);
    const ids = stage.interactiveRegions().map((g) => g.id);
    expect(ids).toContain("familiar:a");
    expect(ids).toContain("familiar:b");
    expect(ids).toContain("familiar:c");
    expect(ids[0]).toBe("character"); // 角色永遠第一（Rust 端截斷時先留前面的）
    stage.destroy();
  });

  it("跟游標走的光點不成框（游標永遠在它底下，會把整條路都吃掉）", () => {
    const { stage, frames } = makeStage();
    stage.setToggles({ play: false, cursorPlay: false, deskMove: false });
    expect(stage.spawnToy("light")).toBe(true);
    stage.pointerMove(30, 40);
    frames(20, 16);
    const regions = stage.interactiveRegions();
    expect(regions.map((g) => g.id)).toEqual(["character"]);
    expect(inAny(regions, 30, 40)).toBe(false);
    stage.destroy();
  });

  it("滿載（3 使魔＋4 玩具）也在 Rust 的 region 數上限內", () => {
    const { stage, frames } = makeStage();
    stage.setToggles({ play: false, cursorPlay: false, deskMove: false });
    stage.setFamiliars([
      { id: "a", name: "a", palette: "maid-classic" },
      { id: "b", name: "b", palette: "maid-classic" },
      { id: "c", name: "c", palette: "maid-classic" },
    ]);
    for (const k of ["yarn", "paper", "plane", "trinket"] as const) stage.spawnToy(k);
    frames(10, 16);
    const regions = stage.interactiveRegions();
    expect(regions.length).toBeGreaterThanOrEqual(8);
    expect(regions.length).toBeLessThanOrEqual(MAX_HIT_REGIONS);
    stage.destroy();
  });
});

describe("拖曳中：角色 region 持續存在且略放大", () => {
  it("抓著玩具移動時，角色框仍在（而且比放手時大）", () => {
    const { stage, frames } = stillStageWithToy();
    const before = stage.interactiveRegions().find((g) => g.id === "character")!;
    const toy = stage.toyPoints()[0];
    expect(stage.pointerDown(toy.x, toy.y)).toBe("toy");
    stage.pointerMove(toy.x + 30, toy.y - 20);
    frames(3, 16);
    const during = stage.interactiveRegions().find((g) => g.id === "character");
    expect(during).toBeDefined();
    expect(during!.w).toBeGreaterThan(before.w);
    expect(during!.h).toBeGreaterThan(before.h);
    // 拖曳中的玩具框也還在（游標甩得比框快時不會掉出去）。
    expect(stage.interactiveRegions().some((g) => g.id.startsWith("toy:"))).toBe(true);
    stage.pointerUp();
    stage.destroy();
  });
});

describe("Reduced Motion 下 regions 仍正確", () => {
  it("角色與玩具各自成框、空白仍可穿透，force 心跳照樣回報", () => {
    const { stage } = stillStageWithToy(true);
    const regions = stage.interactiveRegions();
    const char = regions.find((g) => g.id === "character")!;
    const toy = regions.find((g) => g.id.startsWith("toy:"))!;
    const gapX = (toy.x + toy.w + char.x) / 2;
    const gapY = toy.y + toy.h / 2;
    expect(toy.x + toy.w).toBeLessThan(char.x);
    expect(inAny(regions, gapX, gapY)).toBe(false);
    const reports: HitRegion[][] = [];
    stage.onHitRegions((rs) => reports.push(rs));
    stage.reportHitRect(true);
    expect(reports).toHaveLength(1);
    expect(reports[0].map((g) => g.id)).toEqual(regions.map((g) => g.id));
    stage.destroy();
  });
});

describe("onHitRegions 的回報節流（沿用 hitRectReportPolicy 的意圖）", () => {
  it("角色走動時每幾幀回報一次——不是每幀，也不是每 500ms", () => {
    const { stage, frames } = makeStage({ rng: () => 0.001 }); // 觸發散步
    const reports: HitRegion[][] = [];
    stage.onHitRegions((rs) => reports.push(rs));
    frames(60, 16.7); // ~1 秒
    stage.destroy();
    expect(reports.length).toBeLessThan(60);
    expect(reports.length).toBeGreaterThanOrEqual(10);
  });

  it("force 心跳一定回報（rAF 停擺時仍有一次）", () => {
    const { stage } = makeStage();
    const reports: HitRegion[][] = [];
    stage.onHitRegions((rs) => reports.push(rs));
    stage.reportHitRect(true);
    stage.reportHitRect(true); // 節流窗內，但 force 仍然回報
    stage.destroy();
    expect(reports).toHaveLength(2);
  });

  it("pause() 之後不再回報（隱藏／CPP suspend 不打 IPC）", () => {
    const { stage } = makeStage();
    const reports: HitRegion[][] = [];
    stage.onHitRegions((rs) => reports.push(rs));
    stage.pause();
    stage.reportHitRect(true);
    expect(reports).toHaveLength(0);
    stage.destroy();
  });

  it("舊的聯集回呼仍然可用（相容），內容＝interactiveBounds", () => {
    const { stage } = stillStageWithToy();
    const rects: { x: number; y: number; w: number; h: number }[] = [];
    stage.onHitRect((r) => rects.push(r));
    stage.reportHitRect(true);
    expect(rects).toHaveLength(1);
    expect(rects[0]).toEqual(stage.interactiveBounds());
    stage.destroy();
  });
});

describe("stage regions → Tauri IPC 的 payload 形狀", () => {
  it("invoke 收到 companion_hit_regions 與 {regions:[{id,x,y,w,h}]}", async () => {
    const { stage } = stillStageWithToy();
    const invoke = vi.fn(async () => undefined);
    let sent: Promise<unknown> = Promise.resolve();
    stage.onHitRegions((rs) => {
      sent = sendHitRegions(invoke, rs);
    });
    stage.reportHitRect(true);
    await sent;
    stage.destroy();
    expect(invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = invoke.mock.calls[0] as unknown as [string, { regions: HitRegion[] }];
    expect(cmd).toBe("companion_hit_regions");
    expect(Array.isArray(args.regions)).toBe(true);
    expect(args.regions.length).toBeGreaterThanOrEqual(2);
    for (const g of args.regions) {
      expect(Object.keys(g).sort()).toEqual(["h", "id", "w", "x", "y"]);
      expect(Number.isFinite(g.x) && Number.isFinite(g.y)).toBe(true);
      expect(g.w).toBeGreaterThan(0);
      expect(g.h).toBeGreaterThan(0);
    }
    expect(args.regions[0].id).toBe("character");
  });
});

describe("CompanionApp 的接線（原始碼層級的守衛）", () => {
  const src = fs.readFileSync(path.resolve("src/companion/CompanionApp.tsx"), "utf8");

  it("遊玩場路徑改走 onHitRegions ＋ sendHitRegions（不是每次都送聯集框）", () => {
    expect(src).toContain("onHitRegions(");
    expect(src).toContain("sendHitRegions(");
  });

  it("UI 面（快捷選單／氣泡／可信文字）自己有 region，不再讓整窗吃掉游標", () => {
    expect(src).toContain("companion-menu");
    expect(src).toContain("uiHitRegions");
    // set_interactive 只留真的需要整窗的：文字輸入與拖放確認。
    const call = src.slice(src.indexOf('invoke("companion_set_interactive"'));
    const args = call.slice(0, call.indexOf("});"));
    expect(args).toContain("inputOpen");
    expect(args).toContain("dropPreview");
    expect(args).not.toContain("menuOpen");
    expect(args).not.toContain("trustedText");
  });

  it("regions 在送出前先過 prepareHitRegions（TS 端先截斷）", () => {
    expect(src).toContain("prepareHitRegions(");
    expect(src).toContain("mergeHitRegions(");
  });
});
