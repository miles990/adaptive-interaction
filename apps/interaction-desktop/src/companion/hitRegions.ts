// Bounded hit regions（companion-gameplay-032）——純函式層。
//
// 舊行為：renderer 只回報「角色 ∪ 所有玩具」的**一個聯集矩形**，Rust 端就用
// 它決定點擊穿透。把毛球丟到遠處時，角色與玩具中間那一大條空白既不穿透桌面、
// 點下去也毫無反應——常駐視窗在桌面上挖了一個看不見的洞。
//
// 新行為：角色本體／每個使魔／每個玩具／每個真的可互動的 UI 面各自一個
// bounded region，聯集內的空白區屬於桌面。Rust 端（`companion_hit_regions`）
// 只在游標落在**某一個** region 內時才攔截。
//
// 這裡的上限與 `src-tauri/src/lib.rs` 是同一組（數量／單框尺寸／總面積／頻率）：
// Rust 才是權威，TS 先截斷只是為了「不要送出必然被拒的報告」——被拒的報告會讓
// Rust 保留上一份（fail-closed），畫面就會拿著舊框做判定。
// `src/test/hitRegions.test.ts` 會直接讀 lib.rs 比對這些常數。
//
// 座標一律是 CSS px。stage 產出的是 **canvas 相對**，`translateRegions` 之後才是
// **視窗相對**（Rust 端要的就是視窗相對的 logical px）。

// ---------------------------------------------------------------------------
// 型別與上限
// ---------------------------------------------------------------------------

/** 一個互動矩形。`id` 只用於診斷：Rust 端不會因為名字給任何額外權限。 */
export interface HitRegion {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** host 一次報告最多收幾個 region（= lib.rs MAX_HIT_REGIONS）。 */
export const MAX_HIT_REGIONS = 16;
/** 單一 region 不得在**兩軸**都 ≥ 這個視窗佔比（= lib.rs MAX_HIT_REGION_WINDOW_FRACTION）。 */
export const MAX_HIT_REGION_WINDOW_FRACTION = 0.8;
/** 所有 region 的面積合計不得超過這個視窗佔比（= lib.rs MAX_HIT_REGION_TOTAL_AREA_FRACTION）。 */
export const MAX_HIT_REGION_TOTAL_AREA_FRACTION = 0.8;

/** 兩次回報之間的最小間隔（不得每幀 invoke）。Rust 端的 host 限流是 45ms，比這個低，
 *  所以誠實的回報永遠不會被丟掉。 */
export const HIT_REGION_MIN_INTERVAL_MS = 50;
/** 沒有變化時的最長沉默：超過就補一次（Rust 端的框永遠不會太舊）。 */
export const HIT_REGION_MAX_QUIET_MS = 60;
/** 位移多少才值得立刻回報（px）。 */
export const HIT_REGION_MOVE_EPS = 4;
/** 拖曳中把「角色」與「被抓著的玩具」各外擴這麼多 px：游標甩得比框快時不會掉出去。 */
export const HIT_REGION_DRAG_PAD = 8;

// 幾何（邏輯 px，乘 scale 之後才是 CSS px）。與 rig/stage.ts 的繪製一致：
//   角色：charHitRect() 的 (x-26, ground-122, 52, 124)
//   使魔：drawFamiliar() 身體中心 ground-10、耳尖 ground-21、走路抖動 ≤2px
//   玩具：drawToy()／grabToyAt() 的抓取半徑（12~16）＋一點餘裕
/** 角色框半寬。 */
export const CHAR_REGION_HALF_W = 26;
/** 角色框頂端在 ground 之上多少。 */
export const CHAR_REGION_TOP = 122;
/** 角色框高。 */
export const CHAR_REGION_H = 124;
/** 使魔框半寬（含尾巴）。 */
export const FAMILIAR_REGION_HALF_W = 15;
/** 使魔框頂端在 ground 之上多少（耳尖 21 ＋ 走路抖動 2 ＋ 1 餘裕）。 */
export const FAMILIAR_REGION_TOP = 24;
/** 使魔框高（底部貼地）。 */
export const FAMILIAR_REGION_H = 24;
/** 玩具框半邊。 */
export const TOY_REGION_HALF = 14;

/** 遊玩場裡的一個玩具（只取算框需要的欄位）。 */
export interface StageRegionToy {
  id: number;
  x: number;
  y: number;
  /** 光點／逗貓棒：永遠在游標底下，不成框（否則整條路都被吃掉）。 */
  cursorToy: boolean;
  grabbed: "player" | "character" | null;
}

/** 算 stage regions 需要的世界狀態（邏輯 px；`scale` 轉成 CSS px）。 */
export interface StageRegionInput {
  scale: number;
  /** 地面 y（邏輯 px）。 */
  ground: number;
  /** 角色 x（邏輯 px）。 */
  charX: number;
  familiars: { id: string; x: number }[];
  toys: StageRegionToy[];
  /** 玩家正在拖曳玩具。 */
  dragging: boolean;
}

// ---------------------------------------------------------------------------
// 幾何小工具
// ---------------------------------------------------------------------------

function finiteRegion(g: HitRegion): boolean {
  return (
    Number.isFinite(g.x) &&
    Number.isFinite(g.y) &&
    Number.isFinite(g.w) &&
    Number.isFinite(g.h) &&
    g.w > 0 &&
    g.h > 0
  );
}

/** 四邊各外擴 `pad` px（pad 可為 0）。 */
export function padRegion(g: HitRegion, pad: number): HitRegion {
  if (!pad) return g;
  return { id: g.id, x: g.x - pad, y: g.y - pad, w: g.w + pad * 2, h: g.h + pad * 2 };
}

/** 所有 region 的包圍盒（＝舊的聯集框；沒有 region 時回 null）。 */
export function unionRegion(regions: HitRegion[], id = "union"): HitRegion | null {
  const valid = regions.filter(finiteRegion);
  if (valid.length === 0) return null;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const g of valid) {
    x0 = Math.min(x0, g.x);
    y0 = Math.min(y0, g.y);
    x1 = Math.max(x1, g.x + g.w);
    y1 = Math.max(y1, g.y + g.h);
  }
  return { id, x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** 整批平移（canvas 相對 → 視窗相對）。 */
export function translateRegions(regions: HitRegion[], dx: number, dy: number): HitRegion[] {
  return regions.map((g) => ({ ...g, x: g.x + dx, y: g.y + dy }));
}

/** a 是否完全包住 b（含邊界）。 */
function contains(a: HitRegion, b: HitRegion): boolean {
  return b.x >= a.x && b.y >= a.y && b.x + b.w <= a.x + a.w && b.y + b.h <= a.y + a.h;
}

// ---------------------------------------------------------------------------
// regions 計算
// ---------------------------------------------------------------------------

/**
 * 遊玩場的 bounded regions（canvas 相對 CSS px）。
 *
 * 順序固定：角色 → 使魔 → 玩具。Rust 端超過上限時只留**前面的**，所以角色一定
 * 在第一位（呼叫端再把 UI 面插在角色之後、使魔之前）。
 *
 * 跟游標走的玩具（光點／逗貓棒）不成框——它們永遠在游標底下，算進來的話
 * 游標所在處就永遠被攔截（對抗審查 companion-gameplay-004）。被玩家抓著時例外：
 * 那時它就是玩家手上的東西。
 */
export function stageHitRegions(input: StageRegionInput): HitRegion[] {
  const s = input.scale;
  const g = input.ground;
  const out: HitRegion[] = [];
  const character: HitRegion = {
    id: "character",
    x: (input.charX - CHAR_REGION_HALF_W) * s,
    y: (g - CHAR_REGION_TOP) * s,
    w: CHAR_REGION_HALF_W * 2 * s,
    h: CHAR_REGION_H * s,
  };
  // 拖曳中角色框仍在（而且略放大）：拖東西的時候整個視窗更容易變成互動面，
  // 但仍然是 bounded 的，不是「整窗吃掉游標」。
  out.push(input.dragging ? padRegion(character, HIT_REGION_DRAG_PAD) : character);
  for (const f of input.familiars) {
    out.push({
      id: `familiar:${f.id}`,
      x: (f.x - FAMILIAR_REGION_HALF_W) * s,
      y: (g - FAMILIAR_REGION_TOP) * s,
      w: FAMILIAR_REGION_HALF_W * 2 * s,
      h: FAMILIAR_REGION_H * s,
    });
  }
  for (const t of input.toys) {
    if (t.cursorToy && t.grabbed !== "player") continue;
    const box: HitRegion = {
      id: `toy:${t.id}`,
      x: (t.x - TOY_REGION_HALF) * s,
      y: (t.y - TOY_REGION_HALF) * s,
      w: TOY_REGION_HALF * 2 * s,
      h: TOY_REGION_HALF * 2 * s,
    };
    out.push(t.grabbed === "player" ? padRegion(box, HIT_REGION_DRAG_PAD) : box);
  }
  return out;
}

/** 角色第一、UI 面其次、其餘在後（Rust 端截斷時先留前面的）。 */
export function mergeHitRegions(stage: HitRegion[], ui: HitRegion[]): HitRegion[] {
  if (stage.length === 0) return [...ui];
  return [stage[0], ...ui, ...stage.slice(1)];
}

// ---------------------------------------------------------------------------
// 去重／截斷（警告只發一次）
// ---------------------------------------------------------------------------

const warned = new Set<string>();

/** 測試用：清掉「只警告一次」的記憶。 */
export function resetHitRegionWarnings(): void {
  warned.clear();
}

function warnOnce(key: string, message: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(message);
}

/**
 * 去重：丟掉退化的框（NaN／寬高 ≤0）、跟先前完全一樣的框，以及被先前的框
 * 完全包住的框（聯集不變、數量變少——上限是有限的資源）。
 */
export function dedupeRegions(regions: HitRegion[]): HitRegion[] {
  const out: HitRegion[] = [];
  for (const g of regions) {
    if (!finiteRegion(g)) continue;
    if (out.some((k) => contains(k, g))) continue;
    out.push(g);
  }
  return out;
}

/** 數量截斷（保留前面的），超過時 console.warn 一次。 */
export function capRegions(regions: HitRegion[], max = MAX_HIT_REGIONS): HitRegion[] {
  if (regions.length <= max) return regions;
  warnOnce(
    "count",
    `companion hit regions: ${regions.length} regions reported, keeping the first ${max} (host cap)`
  );
  return regions.slice(0, max);
}

/**
 * 送出前的最後把關（與 Rust 的 `sanitize_hit_regions` 同一組規則）：
 *   1. 裁進視窗；整個在視窗外的丟掉（玩具飛出畫面是正常的，不警告）
 *   2. 單框在兩軸都 ≥80% 視窗＝整窗霸佔，丟掉並警告一次
 *   3. 去重
 *   4. 數量截斷
 *   5. 總面積 >80% 視窗時從後面砍（角色與 UI 在前，最後被砍的是玩具）
 *
 * 回空陣列＝這一份沒有任何合法的框：呼叫端**不要**送（Rust 會拒絕空報告並保留
 * 上一份，這是刻意的 fail-closed）。
 */
export function prepareHitRegions(regions: HitRegion[], winW: number, winH: number): HitRegion[] {
  if (!Number.isFinite(winW) || !Number.isFinite(winH) || winW <= 0 || winH <= 0) return [];
  const clamped: HitRegion[] = [];
  for (const g of regions) {
    if (!finiteRegion(g)) continue;
    const x0 = Math.max(0, Math.min(winW, g.x));
    const y0 = Math.max(0, Math.min(winH, g.y));
    const x1 = Math.max(0, Math.min(winW, g.x + g.w));
    const y1 = Math.max(0, Math.min(winH, g.y + g.h));
    const w = x1 - x0;
    const h = y1 - y0;
    if (w <= 0 || h <= 0) continue; // 整個在視窗外
    if (w >= winW * MAX_HIT_REGION_WINDOW_FRACTION && h >= winH * MAX_HIT_REGION_WINDOW_FRACTION) {
      warnOnce(
        "oversize",
        `companion hit regions: dropped "${g.id}" — a single region may not cover the whole companion window`
      );
      continue;
    }
    clamped.push({ id: g.id, x: x0, y: y0, w, h });
  }
  const capped = capRegions(dedupeRegions(clamped));
  const budget = winW * winH * MAX_HIT_REGION_TOTAL_AREA_FRACTION;
  const out: HitRegion[] = [];
  let area = 0;
  let trimmed = false;
  for (const g of capped) {
    const next = area + g.w * g.h;
    if (next > budget) {
      trimmed = true;
      continue;
    }
    area = next;
    out.push(g);
  }
  if (trimmed) {
    warnOnce(
      "area",
      "companion hit regions: total area exceeded the host budget; the lowest-priority regions were dropped"
    );
  }
  return out;
}

// ---------------------------------------------------------------------------
// 與上一份比較 ＋ 回報節流
// ---------------------------------------------------------------------------

/** 兩份 regions 是否「實質相同」（順序有意義；每個框各軸差 ≤ eps 就算沒動）。 */
export function regionsEqual(
  a: HitRegion[] | null,
  b: HitRegion[] | null,
  eps = 0
): boolean {
  if (a === null || b === null) return a === b;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const p = a[i];
    const q = b[i];
    if (p.id !== q.id) return false;
    if (
      Math.abs(p.x - q.x) > eps ||
      Math.abs(p.y - q.y) > eps ||
      Math.abs(p.w - q.w) > eps ||
      Math.abs(p.h - q.h) > eps
    ) {
      return false;
    }
  }
  return true;
}

/**
 * 這一幀該不該把 regions 回報給 Rust？
 *
 * 有界節流（與 `hitRectReportPolicy` 同一個意圖）：至少隔 50ms；集合實質改變
 * （多／少一個框、任何框位移 >4px）就立刻報，否則 60ms 補一次，Rust 端的框
 * 永遠不會太舊。
 *
 * @param dtMs 距離上次回報的時間（首次回報傳 Infinity 或給 prev=null）。
 */
export function hitRegionsReportPolicy(
  prev: HitRegion[] | null,
  next: HitRegion[],
  dtMs: number
): boolean {
  if (!prev) return true;
  if (!Number.isFinite(dtMs)) return true;
  if (dtMs < HIT_REGION_MIN_INTERVAL_MS) return false;
  const changed = !regionsEqual(prev, next, HIT_REGION_MOVE_EPS);
  return changed || dtMs >= HIT_REGION_MAX_QUIET_MS;
}

// ---------------------------------------------------------------------------
// IPC
// ---------------------------------------------------------------------------

/** `companion_hit_regions` 的參數（形狀對應 lib.rs 的 `Vec<HitRegionInput>`）。 */
export function hitRegionsPayload(regions: HitRegion[]): { regions: HitRegion[] } {
  return { regions: regions.map((g) => ({ id: g.id, x: g.x, y: g.y, w: g.w, h: g.h })) };
}

export type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** 這個 host 有沒有 `companion_hit_regions`（舊版桌面只有單框 IPC）。 */
let ipcMode: "regions" | "rect" = "regions";

/** 測試用：把「host 支援哪個 IPC」的探測結果清掉。 */
export function resetHitRegionIpcMode(): void {
  ipcMode = "regions";
}

/** Tauri 對未註冊的命令回「… not found」；只有這種錯才代表要退回舊 IPC。 */
function unknownCommand(e: unknown): boolean {
  return /not found|unknown command|not allowed/i.test(String(e));
}

/**
 * 把 regions 送給 host。
 *
 * - 空清單不送：Rust 會拒絕空報告（保留上一份），而且「什麼都沒有」不該被
 *   解讀成「整窗透明」。
 * - 舊 host 沒有 `companion_hit_regions` 時**只探測一次**，之後一律走舊的
 *   `companion_hit_rect`（聯集框；功能退化但誠實）。
 * - Rust 端的**驗證**拒絕（框太大／不合法）不退回聯集：那樣等於用「整條聯集」
 *   蓋掉 host 已有的好資料。Rust 保留上一份才是 fail-closed。
 */
export async function sendHitRegions(
  invoke: InvokeFn,
  regions: HitRegion[]
): Promise<"regions" | "rect" | "rejected" | "skipped"> {
  if (regions.length === 0) return "skipped";
  if (ipcMode === "regions") {
    try {
      await invoke("companion_hit_regions", hitRegionsPayload(regions));
      return "regions";
    } catch (e) {
      if (!unknownCommand(e)) return "rejected";
      ipcMode = "rect";
    }
  }
  const u = unionRegion(regions);
  if (!u) return "skipped";
  try {
    await invoke("companion_hit_rect", { x: u.x, y: u.y, w: u.w, h: u.h });
  } catch {
    // 舊 IPC 也失敗：沒有第三條路，維持上一份（host 端仍是 fail-closed）。
  }
  return "rect";
}
