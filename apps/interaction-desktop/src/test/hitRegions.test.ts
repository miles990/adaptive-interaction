// companion-gameplay-032（TS 端）：Tauri host 收「多個 bounded hit regions」，
// 聯集內的空白區要能點穿到桌面。
//
// 這支測純函式層（src/companion/hitRegions.ts）：
//   1  regions 計算（角色／使魔／玩具／UI 各一個矩形，跟游標走的玩具不算）
//   2  去重（完全重疊、被包住的框）
//   3  上限截斷（數量／單框尺寸／總面積）＋只 console.warn 一次
//   4  與上一份比較（回報節流政策，沿用 hitRectReportPolicy 的意圖）
//   5  拖曳中角色 region 持續存在且略放大
//   6  IPC payload 形狀（mock invoke 斷言 companion_hit_regions 的參數）
//   7  TS 上限與 Rust（src-tauri/src/lib.rs）一致

import fs from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  capRegions,
  CHAR_REGION_H,
  CHAR_REGION_HALF_W,
  CHAR_REGION_TOP,
  dedupeRegions,
  FAMILIAR_REGION_H,
  FAMILIAR_REGION_HALF_W,
  FAMILIAR_REGION_TOP,
  HIT_REGION_DRAG_PAD,
  HIT_REGION_MAX_QUIET_MS,
  HIT_REGION_MIN_INTERVAL_MS,
  HIT_REGION_MOVE_EPS,
  HitRegion,
  hitRegionsPayload,
  hitRegionsReportPolicy,
  MAX_HIT_REGION_TOTAL_AREA_FRACTION,
  MAX_HIT_REGION_WINDOW_FRACTION,
  MAX_HIT_REGIONS,
  mergeHitRegions,
  prepareHitRegions,
  regionsEqual,
  resetHitRegionIpcMode,
  resetHitRegionWarnings,
  sendHitRegions,
  stageHitRegions,
  StageRegionInput,
  TOY_REGION_HALF,
  translateRegions,
  unionRegion,
} from "../companion/hitRegions";

beforeEach(() => {
  resetHitRegionWarnings();
  resetHitRegionIpcMode();
});

const r = (id: string, x: number, y: number, w: number, h: number): HitRegion => ({ id, x, y, w, h });

function input(over: Partial<StageRegionInput> = {}): StageRegionInput {
  return {
    scale: 1,
    ground: 170,
    charX: 100,
    familiars: [],
    toys: [],
    dragging: false,
    ...over,
  };
}

// ---------------------------------------------------------------------------
// 1. regions 計算
// ---------------------------------------------------------------------------

describe("stageHitRegions：角色／使魔／玩具各一個矩形（不是一個大聯集）", () => {
  it("只有角色時：一個框，幾何與 charHitRect 相同", () => {
    expect(stageHitRegions(input())).toEqual([
      r("character", 100 - CHAR_REGION_HALF_W, 170 - CHAR_REGION_TOP, CHAR_REGION_HALF_W * 2, CHAR_REGION_H),
    ]);
  });

  it("使魔與玩具各自一個框；角色永遠排第一", () => {
    const out = stageHitRegions(
      input({
        familiars: [
          { id: "f1", x: 40 },
          { id: "f2", x: 220 },
        ],
        toys: [{ id: 7, x: 300, y: 120, cursorToy: false, grabbed: null }],
      })
    );
    expect(out.map((g) => g.id)).toEqual(["character", "familiar:f1", "familiar:f2", "toy:7"]);
    expect(out[1]).toEqual(
      r("familiar:f1", 40 - FAMILIAR_REGION_HALF_W, 170 - FAMILIAR_REGION_TOP, FAMILIAR_REGION_HALF_W * 2, FAMILIAR_REGION_H)
    );
    expect(out[3]).toEqual(r("toy:7", 300 - TOY_REGION_HALF, 120 - TOY_REGION_HALF, TOY_REGION_HALF * 2, TOY_REGION_HALF * 2));
  });

  it("聯集裡的空白區不在任何 region 內（companion-gameplay-032 的核心）", () => {
    const out = stageHitRegions(
      input({ toys: [{ id: 1, x: 300, y: 120, cursorToy: false, grabbed: null }] })
    );
    const inside = (x: number, y: number) =>
      out.some((g) => x >= g.x && x < g.x + g.w && y >= g.y && y < g.y + g.h);
    expect(inside(100, 100)).toBe(true); // 角色身上
    expect(inside(300, 120)).toBe(true); // 玩具身上
    expect(inside(200, 120)).toBe(false); // 兩者之間的空白：要能穿透
    // 一個聯集矩形會把它吃掉——這正是要修的 bug。
    const u = unionRegion(out)!;
    expect(200 >= u.x && 200 < u.x + u.w && 120 >= u.y && 120 < u.y + u.h).toBe(true);
  });

  it("跟游標走的玩具（光點／逗貓棒）不算，除非被玩家抓著", () => {
    const light = { id: 1, x: 30, y: 40, cursorToy: true, grabbed: null as null };
    expect(stageHitRegions(input({ toys: [light] })).map((g) => g.id)).toEqual(["character"]);
    expect(
      stageHitRegions(input({ toys: [{ ...light, grabbed: "player" as const }] })).map((g) => g.id)
    ).toEqual(["character", "toy:1"]);
  });

  it("scale 會等比放大所有框（CSS px）", () => {
    const out = stageHitRegions(input({ scale: 2, toys: [{ id: 1, x: 300, y: 120, cursorToy: false, grabbed: null }] }));
    expect(out[0]).toEqual(r("character", (100 - CHAR_REGION_HALF_W) * 2, (170 - CHAR_REGION_TOP) * 2, CHAR_REGION_HALF_W * 4, CHAR_REGION_H * 2));
    expect(out[1].x).toBe((300 - TOY_REGION_HALF) * 2);
  });

  it("使魔框涵蓋走路時的上下抖動（Reduced Motion 開關都是同一個框）", () => {
    // drawFamiliar：身體中心 ground-10、耳尖 ground-21、走路時再往上抖 ≤2px。
    expect(FAMILIAR_REGION_TOP).toBeGreaterThanOrEqual(23);
    // 底部貼地（身體 ellipse ry=8 → ground-2）。
    expect(FAMILIAR_REGION_H).toBeGreaterThanOrEqual(FAMILIAR_REGION_TOP - 2);
  });
});

// ---------------------------------------------------------------------------
// 5. 拖曳
// ---------------------------------------------------------------------------

describe("拖曳中：角色 region 持續存在且略放大", () => {
  it("dragging=true 時角色框仍在第一位，且四邊各外擴 HIT_REGION_DRAG_PAD", () => {
    const still = stageHitRegions(input())[0];
    const drag = stageHitRegions(input({ dragging: true }))[0];
    expect(drag.id).toBe("character");
    expect(drag.x).toBe(still.x - HIT_REGION_DRAG_PAD);
    expect(drag.y).toBe(still.y - HIT_REGION_DRAG_PAD);
    expect(drag.w).toBe(still.w + HIT_REGION_DRAG_PAD * 2);
    expect(drag.h).toBe(still.h + HIT_REGION_DRAG_PAD * 2);
  });

  it("被玩家抓著的玩具也略放大（游標甩得比框快時不會掉出去）", () => {
    const toy = { id: 3, x: 200, y: 100, cursorToy: false, grabbed: "player" as const };
    const grabbed = stageHitRegions(input({ dragging: true, toys: [toy] }))[1];
    expect(grabbed.w).toBe(TOY_REGION_HALF * 2 + HIT_REGION_DRAG_PAD * 2);
    const loose = stageHitRegions(input({ toys: [{ ...toy, grabbed: null }] }))[1];
    expect(loose.w).toBe(TOY_REGION_HALF * 2);
  });

  it("放大是有界的（不會變成整個視窗的聯集）", () => {
    expect(HIT_REGION_DRAG_PAD).toBeGreaterThan(0);
    expect(HIT_REGION_DRAG_PAD).toBeLessThanOrEqual(16);
  });
});

// ---------------------------------------------------------------------------
// 2. 去重
// ---------------------------------------------------------------------------

describe("dedupeRegions", () => {
  it("完全一樣的框只留一個（先到先留）", () => {
    const out = dedupeRegions([r("a", 0, 0, 10, 10), r("b", 0, 0, 10, 10), r("c", 40, 0, 10, 10)]);
    expect(out.map((g) => g.id)).toEqual(["a", "c"]);
  });

  it("被前一個框完全包住的框拿掉（聯集不變、數量變少）", () => {
    const out = dedupeRegions([r("big", 0, 0, 100, 100), r("small", 10, 10, 20, 20)]);
    expect(out.map((g) => g.id)).toEqual(["big"]);
  });

  it("只是重疊（不是包住）就兩個都留", () => {
    const out = dedupeRegions([r("a", 0, 0, 100, 100), r("b", 90, 90, 40, 40)]);
    expect(out.map((g) => g.id)).toEqual(["a", "b"]);
  });

  it("退化的框（NaN／寬高 ≤0）直接丟掉", () => {
    const out = dedupeRegions([
      r("ok", 0, 0, 10, 10),
      r("nan", Number.NaN, 0, 10, 10),
      r("zero", 20, 0, 0, 10),
      r("neg", 30, 0, 10, -1),
    ]);
    expect(out.map((g) => g.id)).toEqual(["ok"]);
  });
});

// ---------------------------------------------------------------------------
// 3. 上限截斷
// ---------------------------------------------------------------------------

describe("上限截斷（與 Rust 同一組上限），並且只警告一次", () => {
  it("數量超過 MAX_HIT_REGIONS：截到上限、保留前面的（角色與 UI 在前）", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const many = Array.from({ length: 40 }, (_, i) => r(`g${i}`, i * 20, 0, 4, 4));
    const kept = capRegions(many);
    expect(kept).toHaveLength(MAX_HIT_REGIONS);
    expect(kept[0].id).toBe("g0");
    capRegions(many); // 第二次不再吵
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("prepareHitRegions：框被裁進視窗，整個在視窗外的丟掉", () => {
    const out = prepareHitRegions([r("a", -40, -10, 90, 60), r("far", 900, 10, 20, 20)], 520, 284);
    expect(out).toEqual([r("a", 0, 0, 50, 50)]);
  });

  it("prepareHitRegions：單框兩軸都 ≥80% 視窗＝整窗霸佔，丟掉並警告一次", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const out = prepareHitRegions(
      [r("char", 10, 10, 52, 124), r("grab", 0, 0, 520 * 0.85, 284 * 0.85)],
      520,
      284
    );
    expect(out.map((g) => g.id)).toEqual(["char"]);
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("prepareHitRegions：又寬又扁／又高又窄的長條是合法的", () => {
    expect(prepareHitRegions([r("bar", 0, 0, 520, 20)], 520, 284)).toHaveLength(1);
    expect(prepareHitRegions([r("bar", 0, 0, 20, 284)], 520, 284)).toHaveLength(1);
  });

  it("prepareHitRegions：總面積超過 80% 時從後面砍（角色留著），並警告一次", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const big = (id: string, x: number) => r(id, x, 0, 300, 260); // 78000 px²，視窗 147680 px²
    const out = prepareHitRegions([r("character", 0, 0, 52, 124), big("a", 0), big("b", 100)], 520, 284);
    expect(out.map((g) => g.id)).toEqual(["character", "a"]);
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("prepareHitRegions：全部不合法時回空陣列（呼叫端要跳過 IPC，不送空清單）", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(prepareHitRegions([r("far", 900, 900, 10, 10)], 520, 284)).toEqual([]);
    warn.mockRestore();
  });

  it("mergeHitRegions：角色第一、UI 其次、其餘在後", () => {
    const stage = [r("character", 0, 0, 10, 10), r("toy:1", 60, 0, 10, 10)];
    const ui = [r("ui:menu", 100, 0, 40, 40)];
    expect(mergeHitRegions(stage, ui).map((g) => g.id)).toEqual(["character", "ui:menu", "toy:1"]);
    expect(mergeHitRegions([], ui).map((g) => g.id)).toEqual(["ui:menu"]);
  });

  it("translateRegions：整批平移（canvas 相對 → 視窗相對）", () => {
    expect(translateRegions([r("a", 5, 6, 10, 10)], 100, 20)).toEqual([r("a", 105, 26, 10, 10)]);
  });
});

// ---------------------------------------------------------------------------
// 4. 與上一份比較 ＋ 回報節流
// ---------------------------------------------------------------------------

describe("regionsEqual／hitRegionsReportPolicy（沿用 hitRectReportPolicy 的意圖）", () => {
  const one = (x: number): HitRegion[] => [r("character", x, 10, 52, 124)];

  it("regionsEqual：數量不同、順序不同、位移超過 eps 都算不同", () => {
    expect(regionsEqual(one(0), one(0))).toBe(true);
    expect(regionsEqual(one(0), one(3), HIT_REGION_MOVE_EPS)).toBe(true);
    expect(regionsEqual(one(0), one(5), HIT_REGION_MOVE_EPS)).toBe(false);
    expect(regionsEqual(one(0), [...one(0), r("toy:1", 300, 0, 28, 28)])).toBe(false);
    expect(regionsEqual(null, one(0))).toBe(false);
  });

  it("第一次一定回報", () => {
    expect(hitRegionsReportPolicy(null, one(0), 0)).toBe(true);
    expect(hitRegionsReportPolicy(null, one(0), Number.POSITIVE_INFINITY)).toBe(true);
  });

  it("節流：50ms 內不回報（不得每幀 invoke）", () => {
    expect(hitRegionsReportPolicy(one(0), one(400), HIT_REGION_MIN_INTERVAL_MS - 1)).toBe(false);
    expect(hitRegionsReportPolicy(one(0), one(400), 16)).toBe(false);
  });

  it("位移 >4px 且過了節流窗就立刻回報", () => {
    expect(hitRegionsReportPolicy(one(0), one(4.5), HIT_REGION_MIN_INTERVAL_MS)).toBe(true);
    expect(hitRegionsReportPolicy(one(0), one(3), HIT_REGION_MIN_INTERVAL_MS)).toBe(false);
  });

  it("集合實質改變（多一個玩具／少一個使魔）就立刻回報", () => {
    const grew = [...one(0), r("toy:1", 300, 0, 28, 28)];
    expect(hitRegionsReportPolicy(one(0), grew, HIT_REGION_MIN_INTERVAL_MS)).toBe(true);
    expect(hitRegionsReportPolicy(grew, one(0), HIT_REGION_MIN_INTERVAL_MS)).toBe(true);
  });

  it("沒有變化也要在 60ms 內補一次（Rust 端的框永遠不會太舊）", () => {
    expect(hitRegionsReportPolicy(one(0), one(0), HIT_REGION_MAX_QUIET_MS)).toBe(true);
    expect(hitRegionsReportPolicy(one(0), one(0), HIT_REGION_MAX_QUIET_MS - 1)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// 6. IPC payload
// ---------------------------------------------------------------------------

describe("sendHitRegions：payload 形狀與 lib.rs 的 HitRegionInput 一致", () => {
  const regions = [r("character", 10, 20, 52, 124), r("toy:1", 300, 100, 28, 28)];

  it("送 companion_hit_regions，參數是 { regions: [{id,x,y,w,h}] }", async () => {
    const invoke = vi.fn(async () => undefined);
    expect(await sendHitRegions(invoke, regions)).toBe("regions");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("companion_hit_regions", {
      regions: [
        { id: "character", x: 10, y: 20, w: 52, h: 124 },
        { id: "toy:1", x: 300, y: 100, w: 28, h: 28 },
      ],
    });
    expect(hitRegionsPayload(regions)).toEqual({ regions });
  });

  it("空清單不送（Rust 端會拒絕空報告，也不該讓整窗變透明）", async () => {
    const invoke = vi.fn(async () => undefined);
    expect(await sendHitRegions(invoke, [])).toBe("skipped");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("舊 host（沒有這個命令）：退回舊 IPC companion_hit_rect，且只探測一次", async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === "companion_hit_regions") throw new Error("Command companion_hit_regions not found");
      return undefined;
    });
    expect(await sendHitRegions(invoke, regions)).toBe("rect");
    const u = unionRegion(regions)!;
    expect(invoke).toHaveBeenNthCalledWith(2, "companion_hit_rect", { x: u.x, y: u.y, w: u.w, h: u.h });
    // 第二次直接走舊 IPC，不再試新命令。
    invoke.mockClear();
    expect(await sendHitRegions(invoke, regions)).toBe("rect");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("companion_hit_rect", { x: u.x, y: u.y, w: u.w, h: u.h });
  });

  it("Rust 端驗證拒絕（不是缺命令）：不退回聯集——Rust 保留上一份才是 fail-closed", async () => {
    const invoke = vi.fn(async () => {
      throw new Error("hit regions may not cover the whole companion window");
    });
    expect(await sendHitRegions(invoke, regions)).toBe("rejected");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("companion_hit_regions", expect.anything());
  });
});

// ---------------------------------------------------------------------------
// 7. 與 Rust 的上限一致
// ---------------------------------------------------------------------------

describe("TS 上限＝Rust 上限（src-tauri/src/lib.rs 是唯一權威）", () => {
  const rust = fs.readFileSync(path.resolve("src-tauri/src/lib.rs"), "utf8");
  const constOf = (name: string): string => {
    const m = rust.match(new RegExp(`const ${name}:\\s*[A-Za-z0-9_]+\\s*=\\s*([^;]+);`));
    if (!m) throw new Error(`lib.rs 沒有 ${name}`);
    return m[1].trim();
  };

  it("region 數上限一致", () => {
    expect(Number(constOf("MAX_HIT_REGIONS"))).toBe(MAX_HIT_REGIONS);
  });

  it("單框／總面積的視窗佔比一致", () => {
    expect(Number(constOf("MAX_HIT_REGION_WINDOW_FRACTION"))).toBe(MAX_HIT_REGION_WINDOW_FRACTION);
    expect(Number(constOf("MAX_HIT_REGION_TOTAL_AREA_FRACTION"))).toBe(MAX_HIT_REGION_TOTAL_AREA_FRACTION);
  });

  it("TS 的回報頻率地板不低於 Rust 的 host 限流（誠實的回報不會被丟掉）", () => {
    const hostFloor = Number(constOf("MIN_HIT_REGION_INTERVAL_MS"));
    expect(hostFloor).toBeLessThanOrEqual(HIT_REGION_MIN_INTERVAL_MS);
    expect(HIT_REGION_MAX_QUIET_MS).toBeGreaterThanOrEqual(hostFloor);
  });

  it("Rust 端真的有 companion_hit_regions 命令，也還留著舊的 companion_hit_rect", () => {
    expect(rust).toContain("async fn companion_hit_regions(");
    expect(rust).toContain("async fn companion_hit_rect(");
    expect(rust).toMatch(/struct HitRegionInput \{[\s\S]*?\bid: String,[\s\S]*?\bx: f64,[\s\S]*?\by: f64,[\s\S]*?\bw: f64,[\s\S]*?\bh: f64,/);
  });
});
