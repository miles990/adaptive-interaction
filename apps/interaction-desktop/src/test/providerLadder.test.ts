// Phase 7 對抗審查（第二輪）缺陷的 regression test：spec §9.3「UI 必須清楚
// 顯示『只發現』『已配對』『已測試』『已啟用』的差異；掃描到 metadata 不等
// 於連線完成」。四階文案由純函式決定，這裡直接釘住它。

import { describe, expect, it } from "vitest";
import {
  parseProviderDetail,
  providerProgress,
  testedSummary,
} from "../pages/CapabilitiesHub";

const testedOk = {
  at: "2026-08-28T02:00:00Z",
  how: "handshake",
  ok: true,
  note: "裝置連線握手完成（hello 身分＋pair-ok）：受器 desk-light.status 讀取成功",
};

describe("provider 四階誠實階梯", () => {
  it("只發現：掃描到 metadata 不宣稱連線或測試", () => {
    const p = providerProgress({ state: "discovered", enabledCapabilities: 0 });
    expect(p.stage).toBe("discovered");
    expect(p.label).toBe("只發現");
    expect(p.hint).toContain("metadata");
    expect(p.hint).not.toContain("已測試");
  });

  it("已配對：配對完成仍不等於安裝或測試", () => {
    const p = providerProgress({ state: "paired", enabledCapabilities: 0 });
    expect(p.stage).toBe("paired");
    expect(p.label).toBe("已配對");
    expect(p.hint).toContain("還沒測試過");
  });

  it("已安裝設定：宣告式裝置有設定檔，但還沒連線", () => {
    const p = providerProgress({ state: "installed", enabledCapabilities: 0 });
    expect(p.stage).toBe("installed");
    expect(p.label).toBe("已安裝設定，尚未連線");
    expect(p.hint).toContain("設定檔存在不等於連線完成");
  });

  it("已連線但未測試：available 不冒充已測試", () => {
    const p = providerProgress({ state: "available", enabledCapabilities: 0 });
    expect(p.stage).toBe("connected");
    expect(p.label).toBe("已連線，尚未測試");
    expect(p.kind).toBe("pending");
  });

  it("已測試：有成功證據但能力還沒啟用", () => {
    const p = providerProgress({
      state: "available",
      tested: testedOk,
      enabledCapabilities: 0,
    });
    expect(p.stage).toBe("tested");
    expect(p.label).toBe("已測試");
    expect(p.hint).toContain("還沒啟用");
  });

  it("已啟用：測試通過＋有能力真的開著", () => {
    const p = providerProgress({
      state: "available",
      tested: testedOk,
      enabledCapabilities: 2,
    });
    expect(p.stage).toBe("enabled");
    expect(p.label).toBe("已啟用");
    expect(p.kind).toBe("ok");
    expect(p.hint).toContain("2 項能力");
  });

  it("啟用了但沒測過／測失敗：不給綠燈，說出原因", () => {
    const untested = providerProgress({ state: "available", enabledCapabilities: 1 });
    expect(untested.label).toBe("已啟用（尚未測試）");
    expect(untested.kind).toBe("warn");

    const failed = providerProgress({
      state: "available",
      tested: { at: testedOk.at, how: "human", ok: false, note: "receptor x is disabled" },
      enabledCapabilities: 1,
    });
    expect(failed.label).toBe("已啟用（上次測試沒過）");
    expect(failed.hint).toContain("receptor x is disabled");
  });

  it("停用／撤銷等狀態維持原本的人話", () => {
    expect(providerProgress({ state: "revoked", enabledCapabilities: 0 }).label).toBe("已撤銷");
    expect(providerProgress({ state: "disconnected", enabledCapabilities: 3 }).stage).toBe("stopped");
  });
});

describe("parseProviderDetail", () => {
  it("純文字註記原樣保留（向後相容）", () => {
    expect(parseProviderDetail("re-armed on restart requires explicit enable")).toEqual({
      note: "re-armed on restart requires explicit enable",
    });
    expect(parseProviderDetail(undefined)).toEqual({});
    expect(parseProviderDetail("")).toEqual({});
  });

  it("JSON 註記帶證據時，註記與證據分開讀得到", () => {
    const detail = JSON.stringify({ note: "paired via code", tested: testedOk });
    expect(parseProviderDetail(detail)).toEqual({ note: "paired via code", tested: testedOk });
  });

  it("形狀不對的 tested 不當成證據（不臆造已測試）", () => {
    const detail = JSON.stringify({ note: "x", tested: { how: "handshake" } });
    expect(parseProviderDetail(detail).tested).toBeUndefined();
  });
});

describe("testedSummary", () => {
  it("沒測過就說沒測過", () => {
    expect(testedSummary(undefined)).toContain("還沒測試過");
  });

  it("測過就寫明時間、結果與來源", () => {
    const summary = testedSummary(testedOk);
    expect(summary).toContain("成功");
    expect(summary).toContain("裝置連線握手");
  });
});
