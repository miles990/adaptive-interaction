// 一般模式任務量測（M3 §4.4）：純函式計數器。
//
// 為什麼要有這個檔：任務難度不能靠感覺講。「3–5 個主要決策」這個目標必須有一個
// 每一支 spec 都照著算的定義，而且這個定義要能被測試釘住（規則變了測試就紅），
// 否則文件裡的「10→3」隨時可以因為換了計法而變成好看的數字。
//
// 計數規則（`src/test/general-mode-metrics.test.tsx` 是它的合約）：
//
//   * **主要決策**（`decide`）：使用者必須自己選的事——選一個角色、決定開始配對、
//     決定啟用安靜時段。它同時也是一次點擊。
//   * **安全步驟**（`safety`）：不可省略的確認（緊急停止二段確認、移除裝置的
//     「確定移除？」、精靈的套用前確認）。算點擊，**不算決策**：它不是可以被
//     「簡化」掉的選擇，把它併進決策數會讓「省掉一個安全確認」看起來像進步。
//   * **點擊**（`click`）：找路、展開收合區塊、捲動到定位這類純操作。
//   * **回頭**（`visit`）：導覽落到先前造訪過的頁面就算一次；連續停在同一頁不算。
//     回頭多＝資訊放錯地方（要在兩個頁面之間來回才問得完一件事）。
//
// 這個檔刻意**不** import `@playwright/test`：它同時被 e2e spec 與 vitest 用，
// 必須在 jsdom 裡也能跑。

/** 量測時的視窗尺寸（文件裡的「桌面／390px」兩欄）。 */
export type TaskViewport = "desktop" | "narrow";

export interface TaskMetricSnapshot {
  task: string;
  viewport: TaskViewport;
  /** 主要決策數（目標 3–5）。 */
  decisions: number;
  /** 總點擊數（含決策與安全步驟）。 */
  clicks: number;
  /** 回頭次數（導覽回到造訪過的頁）。 */
  backtracks: number;
  /** 不可省略的安全步驟數（愈少**不**代表愈好）。 */
  safetySteps: number;
  /** 逐步軌跡（報告要說得出決策是哪幾個）。 */
  steps: string[];
  /** 誠實註記（例如「瀏覽器模式只驗得到負向」）。 */
  notes: string[];
}

/** 視窗尺寸的人話（文件與 console 都用這個字）。 */
export function viewportLabel(viewport: TaskViewport): string {
  return viewport === "narrow" ? "390px" : "桌面寬度";
}

export class TaskMetrics {
  private decisions = 0;
  private clicks = 0;
  private backtracks = 0;
  private safetySteps = 0;
  private readonly steps: string[] = [];
  private readonly notes: string[] = [];
  /** 造訪過的頁面（判斷「回頭」用）。 */
  private readonly visited = new Set<string>();
  private current: string | null = null;

  constructor(
    readonly task: string,
    readonly viewport: TaskViewport
  ) {}

  /** 一個主要決策（使用者必須自己選的事）；同時是一次點擊。 */
  decide(label: string): void {
    this.decisions += 1;
    this.clicks += 1;
    this.steps.push(`決策：${label}`);
  }

  /** 純操作點擊（找路、展開、切換分頁）。 */
  click(label: string): void {
    this.clicks += 1;
    this.steps.push(`點擊：${label}`);
  }

  /** 不可省略的安全步驟（二段確認、套用前確認）：算點擊，不算決策。 */
  safety(label: string): void {
    this.safetySteps += 1;
    this.clicks += 1;
    this.steps.push(`安全步驟：${label}`);
  }

  /** 導覽落點；回到造訪過的頁面＝一次回頭（連續停在同一頁不算）。 */
  visit(page: string): void {
    if (this.current === page) return;
    if (this.visited.has(page)) this.backtracks += 1;
    this.visited.add(page);
    this.current = page;
    this.steps.push(`到達：${page}`);
  }

  /** 誠實註記（不影響任何數字）。 */
  note(text: string): void {
    this.notes.push(text);
  }

  snapshot(): TaskMetricSnapshot {
    return {
      task: this.task,
      viewport: this.viewport,
      decisions: this.decisions,
      clicks: this.clicks,
      backtracks: this.backtracks,
      safetySteps: this.safetySteps,
      steps: [...this.steps],
      notes: [...this.notes],
    };
  }
}

/** 目標：一般任務 3–5 個主要決策。上限外就是沒達標（不四捨五入）。 */
export function withinDecisionTarget(snapshot: TaskMetricSnapshot, max = 5): boolean {
  return snapshot.decisions <= max;
}

/** 一行摘要（console 與測試附件用）。 */
export function formatTaskMetrics(snapshot: TaskMetricSnapshot): string {
  return [
    `[任務量測] ${snapshot.task}（${viewportLabel(snapshot.viewport)}）`,
    `決策 ${snapshot.decisions}`,
    `點擊 ${snapshot.clicks}`,
    `回頭 ${snapshot.backtracks}`,
    `安全步驟 ${snapshot.safetySteps}`,
  ].join("｜");
}

/** Markdown 表格列（文件的前後對照表直接貼）。 */
export function taskMetricsRow(snapshot: TaskMetricSnapshot): string {
  const cells = [
    snapshot.task,
    viewportLabel(snapshot.viewport),
    String(snapshot.decisions),
    String(snapshot.clicks),
    String(snapshot.backtracks),
    String(snapshot.safetySteps),
  ];
  return `| ${cells.join(" | ")} |`;
}
