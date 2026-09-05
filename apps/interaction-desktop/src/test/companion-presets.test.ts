// 陪伴預設（M3 §4.1）：安靜／自然／活潑／自訂只是**既有欄位**的可解釋組合。
// 這一組測試逐一釘住「套用預設只寫那幾個欄位」——不覆蓋其它自訂值、不改費用上限、
// 不啟用任何權限、不換 AI 幫手；反推不吻合時一律是「自訂」並顯示有效值。

import { describe, expect, it } from "vitest";
import {
  applyCompanionPreset,
  COMPANION_PRESETS,
  COMPANION_PRESET_PREFS_KEYS,
  COMPANION_PRESET_PROACTIVE_KEYS,
  describeCompanionState,
  presetDefinition,
  presetFor,
  type CompanionPresetInputs,
} from "../companion/presets";

const NATURAL: CompanionPresetInputs = {
  expressiveness: "natural",
  doNotDisturb: false,
  proactiveMode: "natural",
};

describe("陪伴預設：反推目前是哪一個", () => {
  it("三個預設各自反推得回自己", () => {
    expect(COMPANION_PRESETS.map((p) => p.id)).toEqual(["quiet", "natural", "lively"]);
    for (const preset of COMPANION_PRESETS) {
      expect(presetFor(preset.state)).toBe(preset.id);
    }
  });

  it("任何一個欄位不吻合就是「自訂」，且有效值逐項可讀", () => {
    expect(presetFor({ ...NATURAL, doNotDisturb: true })).toBe("custom");
    expect(presetFor({ ...NATURAL, proactiveMode: "off" })).toBe("custom");
    expect(presetFor({ ...NATURAL, expressiveness: "lively" })).toBe("custom");
    // 未知／缺值不得被硬塞進某個預設。
    expect(presetFor({})).toBe("custom");
    expect(presetFor({ ...NATURAL, expressiveness: "unknown-level" })).toBe("custom");

    const lines = describeCompanionState({ ...NATURAL, doNotDisturb: true, proactiveMode: "custom" });
    expect(lines).toEqual([
      "表現程度：自然",
      "勿擾：開啟",
      "主動說話：自訂",
    ]);
    // 未知值誠實顯示「不明」，不假裝是某個已知檔位。
    expect(describeCompanionState({ expressiveness: "zzz", doNotDisturb: null, proactiveMode: null })).toEqual([
      "表現程度：不明",
      "勿擾：不明",
      "主動說話：不明",
    ]);
  });

  it("每個預設都有人看得懂的名稱與一行說明", () => {
    for (const preset of COMPANION_PRESETS) {
      const def = presetDefinition(preset.id);
      expect(def?.label.length).toBeGreaterThan(0);
      expect(def?.summary.length).toBeGreaterThan(4);
    }
    expect(presetDefinition("custom")).toBeNull();
  });
});

describe("陪伴預設：套用只寫那幾個既有欄位", () => {
  it("只有 companionExpressiveness／companionDoNotDisturb ＋ 主動對話 mode", () => {
    expect(COMPANION_PRESET_PREFS_KEYS).toEqual(["companionExpressiveness", "companionDoNotDisturb"]);
    expect(COMPANION_PRESET_PROACTIVE_KEYS).toEqual(["mode"]);
    for (const preset of COMPANION_PRESETS) {
      const patch = applyCompanionPreset(preset.id);
      expect(patch).not.toBeNull();
      expect(Object.keys(patch!.prefs).sort()).toEqual(["companionDoNotDisturb", "companionExpressiveness"]);
      expect(Object.keys(patch!.proactive)).toEqual(["mode"]);
    }
    expect(applyCompanionPreset("custom")).toBeNull();
  });

  it("不改費用上限／不換 AI 幫手／不啟用任何權限／不動其它自訂值", () => {
    const forbiddenProactive = [
      "dailyGenerativeCostUsd",
      "dailyGenerativeSessions",
      "generativeAgent",
      "maxPerHour",
      "minIntervalMinutes",
      "mergeWindowSeconds",
      "custom",
      "noFollowUp",
      "dndDefer",
    ];
    const forbiddenPrefs = [
      "companionPack",
      "companionPersona",
      "companionVisible",
      "companionSound",
      "companionBubbles",
      "companionPreferences",
      "companionProactiveQuietUntil",
      "companionInteractionMemory",
    ];
    for (const preset of COMPANION_PRESETS) {
      const patch = applyCompanionPreset(preset.id)!;
      for (const key of forbiddenProactive) expect(patch.proactive).not.toHaveProperty(key);
      for (const key of forbiddenPrefs) expect(patch.prefs).not.toHaveProperty(key);
    }
  });

  it("套用後反推得回同一個預設（來回一致）", () => {
    for (const preset of COMPANION_PRESETS) {
      const patch = applyCompanionPreset(preset.id)!;
      expect(
        presetFor({
          expressiveness: patch.prefs.companionExpressiveness,
          doNotDisturb: patch.prefs.companionDoNotDisturb,
          proactiveMode: patch.proactive.mode,
        })
      ).toBe(preset.id);
    }
  });

  it("安靜預設不得把主動對話整個關掉（必要訊息仍要送得出來）", () => {
    const quiet = applyCompanionPreset("quiet")!;
    expect(quiet.proactive.mode).toBe("necessary");
    expect(quiet.proactive.mode).not.toBe("off");
  });

  it("預設模組是純函式：不 import 任何裝置／API／角色專屬模組", async () => {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const source = fs.readFileSync(path.resolve("src/companion/presets.ts"), "utf8");
    expect(source).not.toMatch(/from "\.\.\/api"/);
    expect(source).not.toMatch(/from "\.\.\/desktop"/);
    expect(source).not.toContain("小樞");
    expect(source).not.toContain("shu");
  });
});
