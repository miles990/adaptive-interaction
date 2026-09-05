// 陪伴預設的**兩段寫入**計畫與恢復標記（M4）。
//
// 套用一個檔位要寫兩個地方：桌面偏好（表現程度＋勿擾）與後端的主動說話模式。
// 中間任何一段失敗、回應遺失、或程式被關掉，使用者都不該只憑「上面亮著哪個檔位」
// 去猜到底生效了什麼。這個純函式模組把交易本身描述出來：
//   - `beginPresetOp` 產生這次交易的計畫（含要一起原子寫入的 marker）；
//   - `shouldResumePendingOp` 決定「重開之後還能不能安全地補送第二段」；
//   - `projectPresetStatus` 把（有效值、marker、忙碌、恢復中、讀回失敗）投影成
//     使用者看得懂的五種狀態，其中沒有一種會冒充「已完成」。
//
// 這裡只測純函式：不碰 api、不碰 desktop、不碰 React。

import { describe, expect, it } from "vitest";
import {
  beginPresetOp,
  markerOf,
  projectPresetStatus,
  readPendingPresetOp,
  shouldResumePendingOp,
  type CompanionPresetStatus,
  type PresetOpMarker,
} from "../companion/applyPresetPlan";
import { COMPANION_PRESETS, presetDefinition, type CompanionPresetChoice } from "../companion/presets";

const NOW = 1_700_000_000_000;

function markerFor(id: "quiet" | "natural" | "lively", nowMs = NOW): PresetOpMarker {
  const plan = beginPresetOp(id, nowMs);
  if (!plan) throw new Error(`beginPresetOp(${id}) must plan`);
  return markerOf(plan);
}

describe("beginPresetOp：一次交易的計畫", () => {
  it("每個檔位都產生「要寫哪兩段」的計畫，內容就是預設本身", () => {
    for (const preset of COMPANION_PRESETS) {
      const plan = beginPresetOp(preset.id, NOW);
      expect(plan).not.toBeNull();
      expect(plan!.presetId).toBe(preset.id);
      expect(plan!.issuedAtMs).toBe(NOW);
      // 第一段：只有既有的兩個桌面偏好欄位（鍵集合不得長大）。
      expect(Object.keys(plan!.prefs).sort()).toEqual([
        "companionDoNotDisturb",
        "companionExpressiveness",
      ]);
      expect(plan!.prefs.companionExpressiveness).toBe(preset.state.expressiveness);
      expect(plan!.prefs.companionDoNotDisturb).toBe(preset.state.doNotDisturb);
      // 第二段：只有 mode。
      expect(Object.keys(plan!.proactive)).toEqual(["mode"]);
      expect(plan!.proactive.mode).toBe(preset.state.proactiveMode);
    }
  });

  it("未知的檔位不猜（回 null），opId 有界且不同時間的交易不同名", () => {
    expect(beginPresetOp("custom", NOW)).toBeNull();
    expect(beginPresetOp("", NOW)).toBeNull();
    expect(beginPresetOp("quiet-ish", NOW)).toBeNull();

    const a = beginPresetOp("quiet", NOW)!;
    const b = beginPresetOp("quiet", NOW + 1)!;
    expect(a.opId).not.toBe(b.opId);
    expect(a.opId.length).toBeGreaterThan(0);
    expect(a.opId.length).toBeLessThanOrEqual(64);
    // 時間壞掉（NaN／負數）不得產生無界或不合法的 opId。
    for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, -1]) {
      const plan = beginPresetOp("quiet", bad)!;
      expect(plan.issuedAtMs).toBe(0);
      expect(plan.opId.length).toBeLessThanOrEqual(64);
    }
  });

  it("marker 只帶恢復需要的東西：opId／presetId／第二段的 patch／發起時間", () => {
    const marker = markerFor("lively");
    expect(Object.keys(marker).sort()).toEqual(["issuedAtMs", "opId", "presetId", "proactivePatch"]);
    expect(Object.keys(marker.proactivePatch)).toEqual(["mode"]);
    expect(marker.proactivePatch.mode).toBe("lively");
  });
});

describe("readPendingPresetOp：host 回來的 marker 一律驗過才用", () => {
  it("合法的 marker 原樣讀回", () => {
    const marker = markerFor("quiet");
    expect(readPendingPresetOp(marker)).toEqual(marker);
    expect(readPendingPresetOp({ ...marker, extra: "ignored" })).toEqual(marker);
  });

  it("缺欄位／型別錯／超界／未知檔位一律當作沒有 marker（不猜）", () => {
    const ok = markerFor("quiet");
    const bad: unknown[] = [
      null,
      undefined,
      "quiet",
      [ok],
      {},
      { ...ok, opId: "" },
      { ...ok, opId: "x".repeat(65) },
      { ...ok, opId: 1 },
      { ...ok, presetId: "custom" },
      { ...ok, presetId: "unknown" },
      { ...ok, proactivePatch: {} },
      { ...ok, proactivePatch: { mode: "" } },
      { ...ok, proactivePatch: { mode: "m".repeat(33) } },
      { ...ok, proactivePatch: { mode: "necessary", maxPerHour: 99 } },
      { ...ok, issuedAtMs: Number.NaN },
      { ...ok, issuedAtMs: -1 },
      { ...ok, issuedAtMs: "yesterday" },
    ];
    for (const value of bad) {
      expect(readPendingPresetOp(value), JSON.stringify(value ?? null)).toBeNull();
    }
  });
});

describe("shouldResumePendingOp：只有「使用者沒有改過」才補送", () => {
  it("marker 鎖定的兩個偏好欄位仍等於目前值 → 可以補送", () => {
    for (const preset of COMPANION_PRESETS) {
      const marker = markerFor(preset.id);
      expect(
        shouldResumePendingOp(marker, {
          expressiveness: preset.state.expressiveness,
          doNotDisturb: preset.state.doNotDisturb,
          // 主動說話模式是**還沒寫成功的那一段**，不參與判斷。
          proactiveMode: "off",
        })
      ).toBe(true);
    }
  });

  it("使用者事後改過任何一個目標欄位 → 不補送（不覆蓋使用者的修改）", () => {
    const marker = markerFor("quiet");
    const def = presetDefinition("quiet")!;
    expect(
      shouldResumePendingOp(marker, { expressiveness: "lively", doNotDisturb: def.state.doNotDisturb })
    ).toBe(false);
    expect(
      shouldResumePendingOp(marker, { expressiveness: def.state.expressiveness, doNotDisturb: false })
    ).toBe(false);
    // 缺值（讀不到桌面偏好）也不補送：不知道就不要動。
    expect(shouldResumePendingOp(marker, {})).toBe(false);
    expect(shouldResumePendingOp(marker, { expressiveness: def.state.expressiveness })).toBe(false);
  });

  it("marker 本身不合法，或與這一版的檔位定義不一致 → 不補送", () => {
    const marker = markerFor("natural");
    const def = presetDefinition("natural")!;
    const current = {
      expressiveness: def.state.expressiveness,
      doNotDisturb: def.state.doNotDisturb,
    };
    expect(shouldResumePendingOp(null, current)).toBe(false);
    expect(shouldResumePendingOp({ ...marker, opId: "" }, current)).toBe(false);
    // 設定檔被手改成別的模式：這一版不認得這個組合，寧可清掉也不代使用者送。
    expect(shouldResumePendingOp({ ...marker, proactivePatch: { mode: "off" } }, current)).toBe(false);
  });
});

describe("projectPresetStatus：五種狀態都不冒充「已完成」", () => {
  const marker = markerFor("quiet");
  const base = {
    presetChoice: "natural" as CompanionPresetChoice,
    pendingOp: null as PresetOpMarker | null,
    busy: false,
    recovering: false,
    readbackFailed: false,
  };
  const project = (over: Partial<typeof base>): CompanionPresetStatus =>
    projectPresetStatus({ ...base, ...over });

  it("沒有交易在跑：吻合檔位＝applied，不吻合＝custom-effective", () => {
    expect(project({})).toBe("applied");
    expect(project({ presetChoice: "custom" })).toBe("custom-effective");
  });

  it("有 marker 而有效值還不是目標 → partially-applied（半套用要說出來）", () => {
    expect(project({ pendingOp: marker, presetChoice: "custom" })).toBe("partially-applied");
    expect(project({ pendingOp: marker, presetChoice: "natural" })).toBe("partially-applied");
  });

  it("有 marker 但讀回已經等於目標 → 視為 applied（回應遺失但事情做到了）", () => {
    expect(project({ pendingOp: marker, presetChoice: "quiet" })).toBe("applied");
  });

  it("交易還在飛（busy）時不下「半套用」的判決：只說現在的有效值", () => {
    expect(project({ pendingOp: marker, presetChoice: "custom", busy: true })).toBe("custom-effective");
    expect(project({ pendingOp: marker, presetChoice: "quiet", busy: true })).toBe("applied");
  });

  it("補送中＝recovering；讀不回有效值＝unverified（最高優先，蓋過其它）", () => {
    expect(project({ pendingOp: marker, recovering: true, presetChoice: "custom" })).toBe("recovering");
    expect(project({ readbackFailed: true })).toBe("unverified");
    expect(project({ readbackFailed: true, recovering: true, pendingOp: marker })).toBe("unverified");
    expect(project({ readbackFailed: true, presetChoice: "natural" })).toBe("unverified");
  });
});
