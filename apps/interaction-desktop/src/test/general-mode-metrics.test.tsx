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

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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
    // 欄位固定 9 欄（M5 起含求助、失敗後恢復、耗時）。
    expect(row.split("|").length).toBe(11);
    expect(row).toContain("設定安靜時段");
  });
});

// ---------------------------------------------------------------------------
// M5：耗時／求助／失敗後恢復
//
// 三個欄位的定義（文件與 e2e 都照這一份）：
//   * `durationMs`：從量測開始（建構或 `start()`）到 `snapshot()` 的實際經過時間。
//     時鐘可注入——測試要釘住的是「怎麼算」，不是「跑多快」。
//   * `helpRequested`：任務中「必須另外去找說明才做得下去」的次數（自動化＝打開說明
//     區塊／點進說明；受測者腳本＝開口問主持人）。**不自動計入點擊**：口頭求助沒有
//     點擊，自動化裡真的按了什麼就自己再呼叫一次 `click()`。同一個定義兩種情境都成立。
//   * `recoveredFromFailure`：任務中是否至少發生過一次「出現失敗／被拒絕之後，靠畫面
//     上的指示回到可繼續的狀態」。它是**布林**：恢復過就是恢復過，次數多不代表更好。
// ---------------------------------------------------------------------------

describe("任務量測：耗時、求助與失敗後恢復", () => {
  /** 可注入的假時鐘（毫秒）。 */
  function fakeClock(start = 1_000) {
    let now = start;
    return {
      now: () => now,
      advance: (ms: number) => {
        now += ms;
      },
    };
  }

  it("durationMs 從建構算到 snapshot（時鐘可注入）", () => {
    const clock = fakeClock();
    const m = new TaskMetrics("暫停主動對話", "desktop", { now: clock.now });
    clock.advance(2_500);
    expect(m.snapshot().durationMs).toBe(2_500);
    clock.advance(1_500);
    expect(m.snapshot().durationMs).toBe(4_000);
  });

  it("start() 重設起點（前置準備不算進任務耗時）", () => {
    const clock = fakeClock();
    const m = new TaskMetrics("暫停主動對話", "desktop", { now: clock.now });
    clock.advance(10_000); // 前置：起 daemon、走完精靈
    m.start();
    clock.advance(800);
    expect(m.snapshot().durationMs).toBe(800);
  });

  it("時鐘倒退時 durationMs 不得為負（退回 0）", () => {
    let now = 5_000;
    const m = new TaskMetrics("t", "desktop", { now: () => now });
    now = 4_000;
    expect(m.snapshot().durationMs).toBe(0);
  });

  it("help() 只計求助，不會偷偷變成一次點擊或決策", () => {
    const m = new TaskMetrics("設定安靜時段", "desktop");
    m.help("打開「哪些提示不受安靜影響」");
    const s = m.snapshot();
    expect(s.helpRequested).toBe(1);
    expect(s.clicks).toBe(0);
    expect(s.decisions).toBe(0);
    expect(s.steps).toEqual(["求助：打開「哪些提示不受安靜影響」"]);
  });

  it("recover() 是布林：恢復過就是 true，重複呼叫仍是 true，但每次都留下軌跡", () => {
    const m = new TaskMetrics("更換角色", "desktop");
    expect(m.snapshot().recoveredFromFailure).toBe(false);
    m.recover("讀了誠實錯誤後改用桌面版");
    expect(m.snapshot().recoveredFromFailure).toBe(true);
    m.recover("第二次");
    const s = m.snapshot();
    expect(s.recoveredFromFailure).toBe(true);
    expect(s.steps).toEqual(["恢復：讀了誠實錯誤後改用桌面版", "恢復：第二次"]);
  });

  it("摘要行與表格列帶上三個新欄位（表格固定 9 欄）", () => {
    const clock = fakeClock();
    const m = new TaskMetrics("暫停主動對話", "desktop", { now: clock.now });
    m.visit("home");
    m.decide("暫停主動互動");
    m.help("讀「暫停期間仍會執行你的直接要求」");
    m.recover("暫停失敗後按恢復再試一次");
    clock.advance(3_400);
    const s = m.snapshot();
    const line = formatTaskMetrics(s);
    expect(line).toMatch(/求助 1/);
    expect(line).toMatch(/失敗後恢復 是/);
    expect(line).toMatch(/耗時 3\.4 s/);
    const row = taskMetricsRow(s);
    // | 任務 | 視窗 | 決策 | 點擊 | 回頭 | 安全步驟 | 求助 | 失敗後恢復 | 耗時 |
    expect(row.split("|").length).toBe(11);
    expect(row).toContain("| 1 | 是 | 3.4 |");
  });
});

// ---------------------------------------------------------------------------
// 任務 spec 的分類誠實（來源層棘輪）
// ---------------------------------------------------------------------------
//
// e2e 只有在真的跑到某一條分支時才驗得到那條分支。「取消進行中的工作」的 else 分支
// （fixture agent 已經自己收尾，畫面上根本沒有中斷鈕）不會取消任何東西，卻曾經照樣以
// `completed` 收尾——文件 §1 的第 12 列宣稱「按『暫停／中斷目前工作』→ 後端 cancelled」，
// 兩條路徑卻收斂成同一個分類，一個沒做到的任務可以合法地拿到 completed
//（對抗審查 general-mode-ux-029）。這一支從原始碼把那條分支釘住。

describe("任務 spec：沒做到就不得記成 completed", () => {
  const source = readFileSync(resolve("e2e/general-mode-tasks.spec.ts"), "utf8");

  /** 取出「取消進行中的工作」那一支測試的內文。 */
  function cancelTaskBody(): string {
    const start = source.indexOf('new TaskMetrics("取消進行中的工作"');
    expect(start, "找不到「取消進行中的工作」這一支測試").toBeGreaterThan(0);
    const end = source.indexOf("\ntest(", start);
    return source.slice(start, end === -1 ? undefined : end);
  }

  it("找不到中斷鈕（沒有可取消的工作）的那一條路記成 not-run", () => {
    const body = cancelTaskBody();
    const split = body.indexOf("} else {");
    expect(split, "任務 11 應該還有「沒有中斷鈕」的那一條路").toBeGreaterThan(0);
    const withInterrupt = body.slice(0, split);
    const withoutInterrupt = body.slice(split);
    expect(withInterrupt).toContain('actual: "completed"');
    expect(withoutInterrupt).toContain('actual: "not-run"');
    expect(withoutInterrupt).not.toContain('actual: "completed"');
  });

  it("宣告 completed 的那一條路要附上「真的取消到了」的證據", () => {
    const body = cancelTaskBody();
    expect(body).toContain('waitSessionState(request, sessionId, ["cancelled"]');
    expect(body).toMatch(/completedVia:\s*"/);
  });
});
