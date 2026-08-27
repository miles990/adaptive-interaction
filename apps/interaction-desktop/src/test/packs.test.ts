// Persona / world / story pack invariants:
// safety wording is immutable, packs are data-only and bounded, story
// chapters fire once, quiet expressiveness suppresses casual bubbles.

import { describe, expect, it } from "vitest";
import {
  behaviorFor,
  DEFAULT_LINES,
  FIXED_SAFETY_LINES,
  nextChapter,
  PersonaPack,
  resolveLine,
  SAFETY_KEYS,
  StoryPack,
  validatePersonaPack,
  validateStoryPack,
} from "../companion/packs";

const persona = (lines: Record<string, string[]>): PersonaPack => ({
  schemaVersion: "1.0",
  kind: "persona-pack",
  id: "test-pack",
  name: { "zh-TW": "測試" },
  lines,
});

describe("persona packs", () => {
  it("safety wording is immutable even when a pack tries to override it", () => {
    const malicious = persona({
      emergency: ["沒事啦，繼續玩吧！"],
      blocked: ["其實可以執行喔"],
      unknown: ["一定成功了"],
      succeeded: ["任務節點完成。"],
    });
    // Validation flags the attempt…
    const issues = validatePersonaPack(malicious);
    expect(issues.some((i) => i.includes("safety-critical"))).toBe(true);
    // …and even if such a pack were loaded anyway, resolution ignores it.
    for (const key of SAFETY_KEYS) {
      expect(resolveLine(key, malicious)).toBe(FIXED_SAFETY_LINES[key]);
    }
    // Non-safety keys still restyle normally.
    expect(resolveLine("succeeded", malicious, () => 0)).toBe("任務節點完成。");
  });

  it("verified-success and failure wording are frozen (verification claims immutable)", () => {
    const p = persona({
      "succeeded-verified": ["其實根本沒驗證，但我說成功了！"],
      failed: ["沒有失敗喔～"],
    });
    // The validator flags both as safety-critical…
    const issues = validatePersonaPack(p);
    expect(issues.some((i) => i.includes("safety-critical"))).toBe(true);
    // …and resolution ignores the override.
    expect(resolveLine("succeeded-verified", p)).toBe(FIXED_SAFETY_LINES["succeeded-verified"]);
    expect(resolveLine("failed", p)).toBe(FIXED_SAFETY_LINES["failed"]);
  });

  it("valid packs restyle non-safety lines; missing keys fall back to defaults", () => {
    const p = persona({ succeeded: ["航線這一段已通過。"] });
    expect(validatePersonaPack(p)).toEqual([]);
    expect(resolveLine("succeeded", p, () => 0)).toBe("航線這一段已通過。");
    // Key not in the pack → built-in default.
    expect(resolveLine("text-received", p, () => 0)).toBe(DEFAULT_LINES["text-received"][0]);
    // No persona at all → defaults.
    expect(resolveLine("succeeded", null, () => 0)).toBe(DEFAULT_LINES.succeeded[0]);
  });

  it("rejects oversized and malformed packs (data-only, bounded)", () => {
    expect(validatePersonaPack(null).length).toBeGreaterThan(0);
    expect(
      validatePersonaPack(persona({ x: ["y".repeat(300)] })).some((i) =>
        i.includes("too long")
      )
    ).toBe(true);
    expect(
      validatePersonaPack(persona({ x: [] })).some((i) => i.includes("non-empty"))
    ).toBe(true);
    const tooMany: Record<string, string[]> = {};
    for (let i = 0; i < 70; i++) tooMany[`k${i}`] = ["a"];
    expect(
      validatePersonaPack(persona(tooMany)).some((i) => i.includes("too many line keys"))
    ).toBe(true);
    // Oversized pack lines are also filtered at resolve time.
    const sneaky = persona({ succeeded: ["長".repeat(500)] });
    expect(resolveLine("succeeded", sneaky, () => 0)).toBe(DEFAULT_LINES.succeeded[0]);
  });
});

describe("story packs", () => {
  const story: StoryPack = {
    schemaVersion: "1.0",
    kind: "story-pack",
    id: "story-test",
    name: { "zh-TW": "測試" },
    chapters: [
      { id: "meet", trigger: "first-meeting", line: "初次見面。", skippable: true },
      { id: "verified", trigger: "first-verified-success", line: "第一次驗證成功。" },
    ],
  };

  it("chapters fire exactly once", () => {
    const seen: Record<string, boolean> = {};
    const first = nextChapter(story, "first-meeting", seen);
    expect(first?.id).toBe("meet");
    seen[first!.id] = true;
    expect(nextChapter(story, "first-meeting", seen)).toBeNull();
    // Independent triggers track independently.
    expect(nextChapter(story, "first-verified-success", seen)?.id).toBe("verified");
  });

  it("validates chapter shape and refuses unknown triggers", () => {
    expect(validateStoryPack(story)).toEqual([]);
    const bad = {
      ...story,
      chapters: [{ id: "x", trigger: "guilt-user-for-leaving", line: "別走…" }],
    };
    expect(validateStoryPack(bad).some((i) => i.includes("unknown trigger"))).toBe(true);
  });
});

describe("behavior tuning", () => {
  it("quiet expressiveness suppresses casual bubbles; lively speeds up", () => {
    expect(behaviorFor("quiet").allowCasualBubbles).toBe(false);
    expect(behaviorFor("natural").allowCasualBubbles).toBe(true);
    expect(behaviorFor("lively").bubbleCooldownMs).toBeLessThan(
      behaviorFor("natural").bubbleCooldownMs
    );
    expect(behaviorFor("quiet").blinkIntervalMs).toBeGreaterThan(
      behaviorFor("lively").blinkIntervalMs
    );
  });
});

// 出貨的 pack 檔必須通過自己的驗證器 — 否則 CompanionApp 會靜默退回
// DEFAULT_LINES，persona 語句整包死亡（v0.3 曾因 succeeded-verified 被列為
// 安全鍵後未同步清理出貨 pack 而發生）。
describe("shipped packs validate cleanly", () => {
  it("all shipped persona packs pass validation", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const dir = path.resolve(__dirname, "../../public/packs");
    const personaFiles = fs
      .readdirSync(dir)
      .filter((f) => f.startsWith("persona-") && f.endsWith(".json"));
    expect(personaFiles.length).toBeGreaterThan(0);
    for (const f of personaFiles) {
      const raw = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
      expect({ file: f, issues: validatePersonaPack(raw) }).toEqual({ file: f, issues: [] });
    }
  });

  it("all shipped story packs pass validation", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const dir = path.resolve(__dirname, "../../public/packs");
    const storyFiles = fs
      .readdirSync(dir)
      .filter((f) => f.startsWith("story-") && f.endsWith(".json"));
    expect(storyFiles.length).toBeGreaterThan(0);
    for (const f of storyFiles) {
      const raw = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
      expect({ file: f, issues: validateStoryPack(raw) }).toEqual({ file: f, issues: [] });
    }
  });
});
