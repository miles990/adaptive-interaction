// 角色互動記憶（spec §11 第一類）。
//
// 只記「跟角色玩」這件事：最喜歡的玩具、近期玩耍、常被關掉的反應、
// 互動熟悉度。全部是純函式、有界資料，存在 DesktopPrefs 裡。
//
// 邊界（不可違反）：
//   - 這裡不 import 任何 api／knowledge／runtime 模組：互動記憶不會外流，
//     也不會自動升級成正式知識（測試釘死）。
//   - 單一事件不推論人格：familiarity 只隨「互動過的天數」緩升，
//     玩一次玩具不會改變個性，個性仍只由表現度＋persona 派生。
//   - 有界：最多 8 種玩具、20 筆近期事件；超過就丟最舊/最少的。

/** 最多記幾種玩具。 */
export const MAX_TOYS = 8;
/** 最多記幾筆近期互動事件。 */
export const MAX_EVENTS = 20;
/** 熟悉度爬滿需要的互動天數（緩升）。 */
export const FAMILIARITY_DAYS = 30;

const DAY_MS = 24 * 60 * 60 * 1000;

export interface ToyCount {
  kind: string;
  count: number;
}

export interface MemoryEvent {
  at: number;
  /** `play`（玩玩具）／`disabled`（關掉某個反應）／`greet`（打招呼）。 */
  kind: "play" | "disabled" | "greet";
  detail: string;
}

export interface InteractionMemory {
  /** 玩過的玩具次數（依次數排序，最多 MAX_TOYS 種）。 */
  toys: ToyCount[];
  /** 常被關掉的反應（次數）。 */
  disabledReactions: ToyCount[];
  /** 近期事件（新的在後，最多 MAX_EVENTS 筆）。 */
  events: MemoryEvent[];
  /** 互動過的天數（同一天多次只算一次）。 */
  daysSeen: number;
  /** 最後一次互動的日序（epoch day）。 */
  lastDay: number;
  lastSeenAt: number;
}

export function emptyMemory(): InteractionMemory {
  return {
    toys: [],
    disabledReactions: [],
    events: [],
    daysSeen: 0,
    lastDay: -1,
    lastSeenAt: 0,
  };
}

const clampCount = (n: unknown): number => {
  const v = Math.floor(Number(n));
  return Number.isFinite(v) && v > 0 ? Math.min(1_000_000, v) : 0;
};

/** 任意輸入（含舊版/損壞的 prefs）→ 有界合法記憶。 */
export function sanitizeMemory(input: unknown): InteractionMemory {
  const m = (input ?? {}) as Partial<InteractionMemory>;
  const counts = (list: unknown): ToyCount[] =>
    (Array.isArray(list) ? list : [])
      .map((x) => ({ kind: String((x as ToyCount)?.kind ?? ""), count: clampCount((x as ToyCount)?.count) }))
      .filter((x) => x.kind !== "" && x.count > 0)
      .sort((a, b) => b.count - a.count)
      .slice(0, MAX_TOYS);
  const events: MemoryEvent[] = (Array.isArray(m.events) ? m.events : [])
    .filter((e) => e && typeof e === "object")
    .map(
      (e): MemoryEvent => ({
        at: Number.isFinite(Number(e.at)) ? Number(e.at) : 0,
        kind: e.kind === "disabled" || e.kind === "greet" ? e.kind : "play",
        detail: String(e.detail ?? "").slice(0, 48),
      })
    )
    .slice(-MAX_EVENTS);
  const daysSeen = clampCount(m.daysSeen);
  return {
    toys: counts(m.toys),
    disabledReactions: counts(m.disabledReactions),
    events,
    daysSeen,
    lastDay: Number.isFinite(Number(m.lastDay)) ? Number(m.lastDay) : -1,
    lastSeenAt: Number.isFinite(Number(m.lastSeenAt)) ? Number(m.lastSeenAt) : 0,
  };
}

function bump(list: ToyCount[], kind: string): ToyCount[] {
  const found = list.find((t) => t.kind === kind);
  const next = found
    ? list.map((t) => (t.kind === kind ? { ...t, count: t.count + 1 } : t))
    : [...list, { kind, count: 1 }];
  // 依次數排序後截斷：滿了就擠掉最少玩的那一種。
  return next.sort((a, b) => b.count - a.count).slice(0, MAX_TOYS);
}

function pushEvent(events: MemoryEvent[], e: MemoryEvent): MemoryEvent[] {
  return [...events, e].slice(-MAX_EVENTS);
}

/** 玩了一次某個玩具。 */
export function notePlay(mem: InteractionMemory, toyKind: string, nowMs: number): InteractionMemory {
  const kind = String(toyKind ?? "").slice(0, 24);
  if (!kind) return mem;
  return {
    ...mem,
    toys: bump(mem.toys, kind),
    events: pushEvent(mem.events, { at: nowMs, kind: "play", detail: kind }),
    lastSeenAt: nowMs,
  };
}

/** 使用者關掉了某個反應（玩耍、游標互動、氣泡…）。 */
export function noteReactionDisabled(
  mem: InteractionMemory,
  reaction: string,
  nowMs: number
): InteractionMemory {
  const key = String(reaction ?? "").slice(0, 24);
  if (!key) return mem;
  return {
    ...mem,
    disabledReactions: bump(mem.disabledReactions, key),
    events: pushEvent(mem.events, { at: nowMs, kind: "disabled", detail: key }),
    lastSeenAt: nowMs,
  };
}

/**
 * 這次有互動（角色視窗開著並被使用）。同一天只算一次，
 * 所以熟悉度隨「天數」緩升，單一事件不會推高。
 */
export function noteSession(mem: InteractionMemory, nowMs: number): InteractionMemory {
  const day = Math.floor(nowMs / DAY_MS);
  if (day === mem.lastDay) return { ...mem, lastSeenAt: nowMs };
  return {
    ...mem,
    daysSeen: Math.min(FAMILIARITY_DAYS * 4, mem.daysSeen + 1),
    lastDay: day,
    lastSeenAt: nowMs,
  };
}

/** 熟悉度 0..1（只由互動天數決定；只影響呈現，永不影響權限）。 */
export function familiarity(mem: InteractionMemory): number {
  return Math.max(0, Math.min(1, mem.daysSeen / FAMILIARITY_DAYS));
}

/** 最喜歡的玩具（次數最多；沒玩過就 null）。 */
export function favoriteToy(mem: InteractionMemory): string | null {
  return mem.toys.length > 0 ? mem.toys[0].kind : null;
}

/** 最近玩過的玩具（新到舊、去重）。 */
export function recentPlay(mem: InteractionMemory, limit = 3): string[] {
  const out: string[] = [];
  for (let i = mem.events.length - 1; i >= 0 && out.length < limit; i--) {
    const e = mem.events[i];
    if (e.kind === "play" && !out.includes(e.detail)) out.push(e.detail);
  }
  return out;
}

/** 最常被關掉的反應。 */
export function mostDisabledReaction(mem: InteractionMemory): string | null {
  return mem.disabledReactions.length > 0 ? mem.disabledReactions[0].kind : null;
}

const TOY_LABEL: Record<string, string> = {
  yarn: "毛球",
  paper: "紙團",
  plane: "紙飛機",
  light: "光點",
  wand: "逗貓棒",
  trinket: "小物件",
};

const REACTION_LABEL: Record<string, string> = {
  play: "玩耍",
  cursorPlay: "游標互動",
  approach: "主動靠近",
  deskMove: "自主散步",
  bubbles: "說話氣泡",
  sound: "音效",
  drag: "被拖曳",
};

/** 「小樞記得：…」人話摘要（沒東西可說就回空陣列，不硬湊）。 */
export function memorySummary(mem: InteractionMemory): string[] {
  const lines: string[] = [];
  const fav = favoriteToy(mem);
  if (fav) {
    const count = mem.toys[0].count;
    lines.push(`最喜歡的玩具是${TOY_LABEL[fav] ?? fav}（玩過 ${count} 次）`);
  }
  const recent = recentPlay(mem, 3).filter((k) => k !== fav);
  if (recent.length > 0) {
    lines.push(`最近也玩了${recent.map((k) => TOY_LABEL[k] ?? k).join("、")}`);
  }
  const off = mostDisabledReaction(mem);
  if (off) {
    lines.push(`你比較常關掉「${REACTION_LABEL[off] ?? off}」，所以我會少做這件事`);
  }
  if (mem.daysSeen > 0) {
    lines.push(`我們一起待過 ${mem.daysSeen} 天（熟悉度 ${Math.round(familiarity(mem) * 100)}%）`);
  }
  return lines;
}
