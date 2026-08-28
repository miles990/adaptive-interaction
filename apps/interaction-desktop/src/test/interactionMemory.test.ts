// v0.5：角色互動記憶（spec §11 第一類）。
//
// 釘死三件事：有界、單一事件不推論人格、不與 api/knowledge 相連。

import { describe, expect, it } from "vitest";
import memorySource from "../companion/interactionMemory?raw";
import {
  emptyMemory,
  familiarity,
  favoriteToy,
  MAX_EVENTS,
  MAX_TOYS,
  memorySummary,
  mostDisabledReaction,
  notePlay,
  noteReactionDisabled,
  noteSession,
  recentPlay,
  sanitizeMemory,
} from "../companion/interactionMemory";
import { personalityFor, tuningFor } from "../companion/personality";

const DAY = 24 * 60 * 60 * 1000;

describe("角色互動記憶：內容", () => {
  it("記得最喜歡的玩具（依次數）與最近玩了什麼", () => {
    let mem = emptyMemory();
    for (let i = 0; i < 3; i++) mem = notePlay(mem, "yarn", 1_000 + i);
    mem = notePlay(mem, "plane", 2_000);
    mem = notePlay(mem, "trinket", 3_000);
    expect(favoriteToy(mem)).toBe("yarn");
    expect(recentPlay(mem, 3)).toEqual(["trinket", "plane", "yarn"]);
  });

  it("記得常被關掉的反應", () => {
    let mem = emptyMemory();
    mem = noteReactionDisabled(mem, "sound", 1_000);
    mem = noteReactionDisabled(mem, "bubbles", 2_000);
    mem = noteReactionDisabled(mem, "sound", 3_000);
    expect(mostDisabledReaction(mem)).toBe("sound");
  });

  it("熟悉度隨互動天數緩升，同一天多次只算一次", () => {
    let mem = emptyMemory();
    const day0 = 40 * DAY;
    mem = noteSession(mem, day0);
    const afterFirst = familiarity(mem);
    mem = noteSession(mem, day0 + 3_600_000); // 同一天稍晚
    expect(familiarity(mem)).toBe(afterFirst);
    mem = noteSession(mem, day0 + DAY); // 隔天
    expect(familiarity(mem)).toBeGreaterThan(afterFirst);
    expect(familiarity(mem)).toBeLessThan(0.2); // 緩升，不會兩天就熟透
    for (let d = 2; d < 60; d++) mem = noteSession(mem, day0 + d * DAY);
    expect(familiarity(mem)).toBe(1); // 有上界
  });

  it("人話摘要；沒東西可說就不硬湊", () => {
    expect(memorySummary(emptyMemory())).toEqual([]);
    let mem = notePlay(emptyMemory(), "yarn", 1_000);
    mem = noteReactionDisabled(mem, "sound", 2_000);
    mem = noteSession(mem, 3 * DAY);
    const lines = memorySummary(mem);
    expect(lines.join("\n")).toContain("毛球");
    expect(lines.join("\n")).toContain("音效");
    expect(lines.some((l) => l.includes("熟悉度"))).toBe(true);
  });
});

describe("角色互動記憶：邊界", () => {
  it("有界：最多 8 種玩具、20 筆事件", () => {
    let mem = emptyMemory();
    for (let i = 0; i < 30; i++) mem = notePlay(mem, `toy-${i}`, 1_000 + i);
    expect(mem.toys.length).toBe(MAX_TOYS);
    expect(mem.events.length).toBe(MAX_EVENTS);
    // 留下的是最近的事件。
    expect(mem.events[mem.events.length - 1].detail).toBe("toy-29");
  });

  it("sanitizeMemory 接受任何損壞輸入並回傳有界資料", () => {
    expect(sanitizeMemory(undefined)).toEqual(emptyMemory());
    expect(sanitizeMemory("nonsense")).toEqual(emptyMemory());
    const dirty = sanitizeMemory({
      toys: [{ kind: "yarn", count: "3" }, { kind: "", count: 5 }, { kind: "x", count: -2 }],
      events: Array.from({ length: 50 }, (_, i) => ({ at: i, kind: "weird", detail: "y".repeat(90) })),
      daysSeen: -5,
      lastDay: "no",
    });
    expect(dirty.toys).toEqual([{ kind: "yarn", count: 3 }]);
    expect(dirty.events.length).toBe(MAX_EVENTS);
    expect(dirty.events[0].kind).toBe("play");
    expect(dirty.events[0].detail.length).toBe(48);
    expect(dirty.daysSeen).toBe(0);
    expect(dirty.lastDay).toBe(-1);
  });

  it("單一事件不改變個性（個性只由表現度＋persona 派生）", () => {
    const before = tuningFor(personalityFor("natural", "persona-shu"));
    let mem = emptyMemory();
    mem = notePlay(mem, "yarn", 1_000);
    mem = noteReactionDisabled(mem, "play", 2_000);
    mem = noteSession(mem, 3_000);
    const after = tuningFor(personalityFor("natural", "persona-shu"));
    expect(after).toEqual(before);
    // 而且熟悉度沒有因為「玩了一次」就跳動。
    expect(familiarity(notePlay(emptyMemory(), "yarn", 1_000))).toBe(0);
  });

  it("互動記憶模組不 import 任何 api／knowledge／runtime 模組", () => {
    const imports = memorySource.match(/^\s*import[^\n]*$/gm) ?? [];
    expect(imports).toEqual([]);
    expect(memorySource).not.toMatch(/from\s+["'].*(api|knowledge|transport|desktop)["']/);
    expect(memorySource).not.toContain("fetch(");
  });
});
