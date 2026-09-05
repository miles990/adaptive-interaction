// 一般模式任務量測（M3 §4.4）的計數規則。
//
// e2e 的任務 spec 會用同一份純函式計數「主要決策／點擊／回頭／安全步驟」，
// 文件（docs/releases/v0.6.x-general-mode-tasks.md）的前後對照表也由它產出。
// 這裡釘住的是**規則本身**，因為數字一旦被引用進文件，計法就不能悄悄改：
//
//   - 主要決策 = 使用者必須自己選的事（同時也是一次點擊）。
//   - 安全步驟 = 不可省略的確認（二段確認、套用前確認）：算點擊，**不算決策**。
//     它不是「可以簡化掉的選擇」，把它併進決策數會讓「省掉安全步驟」看起來像進步。
//   - 回頭 = 導覽回到先前造訪過的頁面（連續停在同一頁不算）。
//   - 目標區間 3–5 個主要決策；超出時 `withinDecisionTarget` 必須回 false（不四捨五入）。

import { describe, expect, it } from "vitest";
import {
  TaskMetrics,
  formatTaskMetrics,
  taskMetricsRow,
  withinDecisionTarget,
} from "../../e2e/taskMetrics";

describe("任務量測：計數規則", () => {
  it("主要決策同時計入決策數與點擊數", () => {
    const m = new TaskMetrics("連接手機", "desktop");
    m.decide("從同步卡的下一步去連接手機");
    m.decide("開始配對");
    const s = m.snapshot();
    expect(s.decisions).toBe(2);
    expect(s.clicks).toBe(2);
    expect(s.safetySteps).toBe(0);
  });

  it("安全步驟算點擊但不算決策（省掉安全步驟不得看起來像進步）", () => {
    const m = new TaskMetrics("緊急停止", "desktop");
    m.decide("按下緊急停止");
    m.safety("二段確認：立即停止一切？");
    const s = m.snapshot();
    expect(s.decisions).toBe(1);
    expect(s.safetySteps).toBe(1);
    expect(s.clicks).toBe(2);
  });

  it("純操作點擊（找路、展開）只計點擊", () => {
    const m = new TaskMetrics("設定安靜時段", "desktop");
    m.click("展開「安靜與勿擾」");
    const s = m.snapshot();
    expect(s.clicks).toBe(1);
    expect(s.decisions).toBe(0);
  });

  it("回頭＝導覽回到造訪過的頁；第一次到訪與連續停在同一頁都不算", () => {
    const m = new TaskMetrics("連接手機", "desktop");
    m.visit("companion");
    m.visit("companion");
    m.visit("connect");
    expect(m.snapshot().backtracks).toBe(0);
    m.visit("companion");
    expect(m.snapshot().backtracks).toBe(1);
    m.visit("connect");
    expect(m.snapshot().backtracks).toBe(2);
  });

  it("每一步都留下可讀的軌跡（報告要說得出決策是哪幾個）", () => {
    const m = new TaskMetrics("更換角色", "narrow");
    m.visit("companion");
    m.click("展開「更換或加入角色」");
    m.decide("選用另一個角色");
    m.note("瀏覽器模式：桌面角色偏好住在 Tauri host，這裡只驗誠實拒絕");
    const s = m.snapshot();
    expect(s.viewport).toBe("narrow");
    expect(s.steps).toEqual([
      "到達：companion",
      "點擊：展開「更換或加入角色」",
      "決策：選用另一個角色",
    ]);
    expect(s.notes).toEqual([
      "瀏覽器模式：桌面角色偏好住在 Tauri host，這裡只驗誠實拒絕",
    ]);
  });

  it("目標區間 3–5 個主要決策：超過就不算達標", () => {
    const build = (decisions: number) => {
      const m = new TaskMetrics("t", "desktop");
      for (let i = 0; i < decisions; i += 1) m.decide(`d${i}`);
      return m.snapshot();
    };
    expect(withinDecisionTarget(build(3))).toBe(true);
    expect(withinDecisionTarget(build(5))).toBe(true);
    expect(withinDecisionTarget(build(6))).toBe(false);
    // 0 個決策（例如「手機端動作，桌面不必做任何事」）也在目標內：目標是上限，
    // 不是逼使用者一定要做滿三個選擇。
    expect(withinDecisionTarget(build(0))).toBe(true);
  });

  it("摘要行與表格列都帶著四個數字與視窗尺寸", () => {
    const m = new TaskMetrics("設定安靜時段", "narrow");
    m.visit("companion");
    m.click("展開「安靜與勿擾」");
    m.decide("啟用安靜時段");
    m.safety("套用前確認");
    const s = m.snapshot();
    const line = formatTaskMetrics(s);
    expect(line).toContain("設定安靜時段");
    expect(line).toContain("390px");
    expect(line).toMatch(/決策 1/);
    expect(line).toMatch(/點擊 3/);
    expect(line).toMatch(/回頭 0/);
    expect(line).toMatch(/安全步驟 1/);
    const row = taskMetricsRow(s);
    expect(row.startsWith("|")).toBe(true);
    expect(row.split("|").length).toBe(8);
    expect(row).toContain("設定安靜時段");
  });
});
