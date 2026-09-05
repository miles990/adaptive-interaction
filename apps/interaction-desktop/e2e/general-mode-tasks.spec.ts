// 一般模式的**必測任務**驗收（M3 §4.4）。
//
// 這一支不驗「有沒有那個字」，驗的是「一個普通人要做完這件事，得做幾個決定、
// 按幾下、來回幾次」，以及「不可省略的安全步驟有沒有被省掉」。每個任務都用
// `taskMetrics.ts` 的同一份計數規則量測，數字會附在測試附件與 console，
// 供 `docs/releases/v0.6.x-general-mode-tasks.md` 引用。
//
// ⚠️ 誠實邊界：
//   * 手機全部是【模擬 iPhone（fixture）】——`crates/interaction-runtime/examples/fake_iphone.rs`，
//     程序外的假手機。**iPhone 真機驗收仍然是零**，本檔的任何數字都不得寫成真機。
//   * 這是**瀏覽器模式**的控制中心：桌面角色偏好（換角色、陪伴程度、勿擾、顯示／隱藏）
//     住在 Tauri host，瀏覽器只驗得到「誠實降級」這一面（負向）。正向效果由
//     `scripts/tauri-ax-walkthrough.sh`（真 Tauri 視窗，AX 驅動）驗，
//     不得用這裡的通過冒充，也不得反過來用那邊的通過說這裡也做得到。
//   * 這一支自己起一支 daemon（隔離的家與埠號）：它會配對／移除手機、改安靜時段、
//     取消工作、按下緊急停止——放在共用 daemon 上會污染別的 spec。
//     `INTERACT_AI_MOBILE_ADVERTISE=0`：不對區網廣播、只綁 127.0.0.1。
//
// 刻意**不**用 serial 模式：任務彼此獨立（每一支需要手機就自己確保有手機），
// 一個任務紅了不應該讓其餘十幾個任務變成「已跳過」而失去證據。

import { test, expect, APIRequestContext, Locator, Page } from "@playwright/test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { CHARACTER_SYNC_PROJECTION } from "../src/statusProjection";
import {
  aipCapability,
  aipResume,
  aipTouch,
  api,
  appUrl,
  beginPairingFromUi,
  DESKTOP,
  FAKE_IPHONE_LABEL,
  makeWorkdir,
  makeWorkRoot,
  memberPresence,
  NARROW,
  navigateTo,
  openApp,
  openNarrow,
  PAGES,
  repoRoot,
  spawnDaemon,
  spawnFakeIphone,
  waitCharacterSession,
  waitSessionState,
  type FakeIphone,
  type SpawnedDaemon,
} from "./helpers";
import {
  formatTaskMetrics,
  TASK_METRICS_HEADER,
  taskMetricsRow,
  TaskMetrics,
  withinDecisionTarget,
  type TaskMetricSnapshot,
} from "./taskMetrics";

const PORT = 18795;
const COMPANION = PAGES[1];
const WORK = PAGES[2];
const CONNECT = PAGES[3];

/** 有 online 遠端成員時**誠實**可以說的那幾句（fixture 只宣告三個 intent，拿不到綠勾）。 */
const ONLINE_HONEST = [
  CHARACTER_SYNC_PROJECTION["capability-unknown"].headline,
  CHARACTER_SYNC_PROJECTION["partial-capability"].headline,
  CHARACTER_SYNC_PROJECTION.synced.headline,
];

// ---------------------------------------------------------------------------
// 分類漂移防線（M5）
//
// 每個任務都要**事先宣告**它應該落在哪一種結果，spec 才有辦法在「有一天它變了」
// 的時候變紅。四種結果只有這幾個字（誠實階梯）：
//
//   * `completed`         —— 真的做完了，而且後端事實對得上。
//   * `correctly-blocked` —— 這個模式下**做不到**，畫面誠實拒絕／降級，**後端零變動**。
//   * `failed`            —— 該做到卻沒做到（測試紅）。
//   * `not-run`           —— 這一輪沒有跑到（只會出現在結果摘要裡，不會是斷言結果）。
//
// 為什麼要有這張表：`correctly-blocked` 很容易在某次改動之後悄悄變成「其實按下去
// 有效了」（例如哪天瀏覽器模式也能寫桌面偏好），那時候文件裡「這個模式做不到」
// 就變成謊話；反過來，`completed` 的任務退化成拒絕也一樣要被抓到。`record()` 會拿
// 實際結果跟這張表對，不一致就紅。
// ---------------------------------------------------------------------------

/** 一個任務在這一輪可能的結果分類（斷言只會產生前兩種；紅了就是 `failed`）。 */
type TaskClassification = "completed" | "correctly-blocked";
/**
 * 摘要 JSON 會出現的分類。`not-run` 有兩種來源，都不是「通過」：
 * 這一輪根本沒跑到那一支測試，或者跑到了、但**這一輪沒有可做的事**
 * （例如 fixture agent 已經自己收尾，畫面上沒有可取消的工作）。
 */
type RecordedClassification = TaskClassification | "not-run";

interface TaskDeclaration {
  /** 事先宣告的分類。 */
  expected: TaskClassification;
  /** 證據等級（文件表格的第三欄）。 */
  evidence: string;
  /** 限制（文件表格的第四欄）。 */
  limits: string;
  /**
   * 這個任務宣告 `completed` 時**一定**要附上的證據標籤。
   *
   * 任務的定義就是那件事本身（「取消」＝後端真的變成 cancelled）：沒有證據就不得
   * 掛 completed，也不准用「反正它就是沒得取消」帶過——那一條路是 `not-run`
   *（對抗審查 general-mode-ux-029）。
   */
  completionEvidence?: string;
}

/** 瀏覽器模式的證據等級（這一支唯一會產出的那一種）。 */
const BROWSER_EVIDENCE =
  "瀏覽器模式（Playwright Chromium ＋ 真 interact-ai daemon，隔離的家與埠號 18795）";
const SIMULATED_PHONE = "手機是【模擬 iPhone（fixture）】，非 iPhone 真機";
const FIXTURE_AGENT = "AI 幫手是 fixture 子程序，非真 Codex／Claude Code";
/** 桌面偏好住在 Tauri host：瀏覽器模式只驗得到誠實降級。 */
const TAURI_ONLY = "正向效果是 Tauri-only（見「真 Tauri 視窗走查」章節）";

const TASK_MANIFEST: Record<string, TaskDeclaration> = {
  第一次使用桌面角色: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: "—" },
  稍後連手機: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: SIMULATED_PHONE },
  手機與桌面互動: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: SIMULATED_PHONE },
  暫時離線後恢復: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: SIMULATED_PHONE },
  等待離線逾時: {
    expected: "completed",
    evidence: BROWSER_EVIDENCE,
    limits: `${SIMULATED_PHONE}；presence 逾時無可調參數，只能真的等 45 秒`,
  },
  主動移除手機: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: SIMULATED_PHONE },
  撤銷後重新連線: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: SIMULATED_PHONE },
  更換角色: { expected: "correctly-blocked", evidence: BROWSER_EVIDENCE, limits: TAURI_ONLY },
  調整陪伴程度: { expected: "correctly-blocked", evidence: BROWSER_EVIDENCE, limits: TAURI_ONLY },
  設定安靜時段: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: "—" },
  暫停主動對話: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: "—" },
  取消進行中的工作: {
    expected: "completed",
    evidence: BROWSER_EVIDENCE,
    limits: FIXTURE_AGENT,
    completionEvidence: "後端 cancelled",
  },
  "390px：看角色頁並連手機": {
    expected: "completed",
    evidence: BROWSER_EVIDENCE,
    limits: SIMULATED_PHONE,
  },
  "390px：設定安靜時段": { expected: "completed", evidence: BROWSER_EVIDENCE, limits: "—" },
  緊急停止: { expected: "completed", evidence: BROWSER_EVIDENCE, limits: "—" },
};

/** 這一輪某個任務的結果（寫進 `test-results/` 的摘要 JSON）。 */
interface TaskOutcome {
  task: string;
  viewport: string;
  expected: TaskClassification;
  actual: RecordedClassification;
  evidence: string;
  limits: string;
  /** `correctly-blocked` 才有：後端零變動與畫面誠實文案的證據。 */
  blocked?: { backendUnchanged: boolean; honestText: string };
  /** 宣告 `completed` 時做到的那件事（`completionEvidence` 要求時必填）。 */
  completedVia?: string;
  /** `not-run` 才有：這一輪為什麼沒有可做的事（不是「通過」，也不是「失敗」）。 */
  notRun?: { reason: string };
  metrics: TaskMetricSnapshot;
}

const outcomes: TaskOutcome[] = [];

let daemon: SpawnedDaemon | null = null;
const phones: FakeIphone[] = [];
/** 目前這一輪還活著的模擬手機（任務之間共用；沒有就現配一台）。 */
let phone: FakeIphone | null = null;
/** 全部任務的量測結果（afterAll 印成表格，文件直接引用）。 */
const measured: TaskMetricSnapshot[] = [];

test.beforeAll(async () => {
  daemon = await spawnDaemon({
    port: PORT,
    label: "general-mode-tasks",
    env: {
      // 「取消工作」任務需要 agent；用 Rust 測試的 fixture 子程序（模擬 agent，非真 CLI）。
      INTERACT_AI_CODEX_BIN: join(
        repoRoot(),
        "crates/interaction-runtime/tests/fixtures/fake_codex.sh"
      ),
      INTERACT_AI_CLAUDE_BIN: join(
        repoRoot(),
        "crates/interaction-runtime/tests/fixtures/fake_claude.sh"
      ),
    },
  });
});

test.afterAll(async () => {
  for (const p of phones) p.kill();
  phones.length = 0;
  phone = null;
  daemon?.kill();
  daemon = null;
  if (measured.length > 0) {
    // 文件（docs/releases/v0.6.x-general-mode-tasks.md）的表格直接抄這一段。
    const lines = [
      "",
      "=== 一般模式任務量測（本輪實跑；模擬 iPhone（fixture）／瀏覽器模式控制中心）===",
      ...TASK_METRICS_HEADER,
      ...measured.map(taskMetricsRow),
      "",
    ];
    console.log(lines.join("\n"));
  }
  writeOutcomeSummary();
});

/**
 * 任務 → 分類 → 證據等級的摘要 JSON。
 *
 * 沒跑到的任務誠實寫成 `not-run`（不是「通過」，也不是「失敗」）：這一支刻意不是
 * serial，單獨跑某幾個任務時其餘就是沒跑，摘要必須說得出這件事。
 */
function writeOutcomeSummary(): void {
  const seen = new Set(outcomes.map((o) => o.task));
  const missing = Object.entries(TASK_MANIFEST)
    .filter(([task]) => !seen.has(task))
    .map(([task, decl]) => ({
      task,
      expected: decl.expected,
      actual: "not-run" as const,
      evidence: decl.evidence,
      limits: decl.limits,
    }));
  const summary = {
    generatedAt: new Date().toISOString(),
    spec: "apps/interaction-desktop/e2e/general-mode-tasks.spec.ts",
    mode: BROWSER_EVIDENCE,
    honesty: {
      phone: "全部是【模擬 iPhone（fixture）】；iPhone 真機驗收仍然是零",
      agent: FIXTURE_AGENT,
      desktopPrefs:
        "桌面角色偏好（換角色、陪伴程度、勿擾、顯示／隱藏）住在 Tauri host；瀏覽器模式只驗得到誠實降級",
      classifications:
        "completed／correctly-blocked／failed（測試紅）／not-run（本輪沒跑到，" +
        "或跑到了但這一輪沒有可做的事——後者會附 `notRun.reason`）",
    },
    tasks: [...outcomes, ...missing],
  };
  try {
    const dir = join(process.cwd(), "test-results");
    mkdirSync(dir, { recursive: true });
    const file = join(dir, "general-mode-task-outcomes.json");
    // Playwright 在有測試失敗之後會換一個 worker：module 層的 `outcomes` 會歸零，
    // `afterAll` 也會再跑一次。直接覆寫會讓前半段跑過的任務在摘要裡憑空變成
    // `not-run`——那是假的。所以先讀回既有的檔案，以「有結果的那一筆」為準合併。
    let merged: unknown[] = summary.tasks;
    try {
      const previous = JSON.parse(readFileSync(file, "utf8")) as {
        tasks?: { task: string; actual: string }[];
      };
      if (Array.isArray(previous.tasks)) {
        const byTask = new Map<string, unknown>(previous.tasks.map((t) => [t.task, t]));
        for (const row of summary.tasks) {
          const earlier = byTask.get(row.task) as { actual?: string } | undefined;
          // 這一輪沒跑到、但上一個 worker 跑過 → 保留上一個 worker 的結果。
          // 帶著 `notRun.reason` 的那一種是**跑到了、但沒有可做的事**，是這一輪的實際觀察，
          // 不得被舊結果蓋掉。
          const observed = "notRun" in row;
          if (row.actual === "not-run" && !observed && earlier && earlier.actual !== "not-run") {
            continue;
          }
          byTask.set(row.task, row);
        }
        merged = [...byTask.values()];
      }
    } catch {
      /* 沒有既有檔案（第一次寫）或內容壞掉：就用這一輪的結果 */
    }
    writeFileSync(file, `${JSON.stringify({ ...summary, tasks: merged }, null, 2)}\n`, "utf8");
    console.log(`[任務分類摘要] ${file}`);
  } catch (e) {
    // 摘要寫不出來不得讓已經跑完的任務結果消失：印出來，至少 log 裡有。
    console.log(`[任務分類摘要] 寫檔失敗：${String(e)}`);
    console.log(JSON.stringify(summary, null, 2));
  }
}

/** 這一支自己的 daemon（所有 API 呼叫都要帶）。 */
function target(): { base: string; token: string } {
  return { base: daemon!.api, token: daemon!.token };
}

function openHere(page: Page) {
  return openApp(page, appUrl(daemon!.api, daemon!.token));
}

/**
 * 收尾：把這一輪的量測與**分類**寫進附件、印一行、檢查決策數在目標區間內，
 * 並拿實際分類跟 `TASK_MANIFEST` 事先宣告的對照（漂移就紅）。
 *
 * `correctly-blocked` 一定要附證據：後端零變動（同一段狀態的前後 JSON 相等）＋
 * 畫面上真的有一句誠實文案＋**動作的目標**沒有長出綠勾。少任何一項都算沒有證明「被正確擋下」，
 * 這裡直接讓它失敗——不准用「反正它就是做不到」帶過。
 */
async function record(
  m: TaskMetrics,
  outcome: {
    actual: RecordedClassification;
    maxDecisions?: number;
    /** 宣告 `completed` 時做到的那件事（宣告 `completionEvidence` 的任務必填）。 */
    completedVia?: string;
    /** `actual: "not-run"` 必填：這一輪為什麼沒有可做的事。 */
    notRun?: { reason: string };
    blocked?: {
      /** 動作之前的後端狀態（JSON 字串）。 */
      backendBefore: string;
      /** 動作之後的後端狀態（必須與 before 一模一樣）。 */
      backendAfter: string;
      /** 畫面上那句誠實拒絕／降級的文字。 */
      honestText: string;
      /**
       * **這個動作的目標**——用來確認它沒有長出替失敗背書的綠勾。
       *
       * 範圍要收斂到動作真正指向的那個東西（被按下的那張卡片、那一格設定），不是整頁：
       * 頁面上同時存在別的、**真實**的成功狀態（手機已同步、另一個角色「使用中」），
       * 把它們一起掃進來會讓這個檢查變成噪音，然後遲早被人放寬掉。
       */
      scope: Locator;
    };
  }
): Promise<void> {
  const snapshot = m.snapshot();
  const declared = TASK_MANIFEST[snapshot.task];
  expect(
    declared,
    `任務「${snapshot.task}」沒有在 TASK_MANIFEST 宣告分類與證據等級`
  ).toBeTruthy();
  // `not-run` 不是漂移：它說的是「這一輪沒有可做的事」，所以不拿它跟宣告的分類對照，
  // 但一定要說出原因，而且不會被算成做完（摘要與量測表都分得出來）。
  if (outcome.actual === "not-run") {
    const reason = outcome.notRun?.reason.trim() ?? "";
    expect(reason.length, `「${snapshot.task}」記成 not-run 卻沒有說原因`).toBeGreaterThan(0);
    expect(
      outcome.blocked,
      `「${snapshot.task}」是 not-run，不該附「被擋下」的證據`
    ).toBeUndefined();
    outcomes.push({
      task: snapshot.task,
      viewport: snapshot.viewport,
      expected: declared.expected,
      actual: "not-run",
      evidence: declared.evidence,
      limits: declared.limits,
      notRun: { reason },
      metrics: snapshot,
    });
    const notRunLine = `${formatTaskMetrics(snapshot)}｜分類 not-run（${reason}）`;
    console.log(notRunLine);
    await test.info().attach(`task-metrics-${snapshot.task}`, {
      body: JSON.stringify(outcomes[outcomes.length - 1], null, 2),
      contentType: "application/json",
    });
    return;
  }

  expect(
    outcome.actual,
    `任務「${snapshot.task}」的分類漂移了：宣告 ${declared.expected}，這一輪是 ${outcome.actual}`
  ).toBe(declared.expected);

  if (outcome.actual === "completed" && declared.completionEvidence) {
    expect(
      outcome.completedVia,
      `「${snapshot.task}」宣告 completed 就要附上「${declared.completionEvidence}」的證據`
    ).toBe(declared.completionEvidence);
  }

  let blocked: TaskOutcome["blocked"];
  if (outcome.actual === "correctly-blocked") {
    const evidence = outcome.blocked;
    expect(evidence, `「${snapshot.task}」宣告被正確擋下，卻沒有附任何證據`).toBeTruthy();
    expect(
      evidence!.backendAfter,
      `「${snapshot.task}」被擋下時後端狀態卻變了（correctly-blocked 的定義就是零變動）`
    ).toBe(evidence!.backendBefore);
    expect(
      evidence!.honestText.trim().length,
      `「${snapshot.task}」被擋下時畫面沒有任何誠實文案`
    ).toBeGreaterThan(0);
    await expect(
      evidence!.scope.locator(".badge-ok"),
      `「${snapshot.task}」被擋下時不得出現綠勾`
    ).toHaveCount(0);
    blocked = { backendUnchanged: true, honestText: evidence!.honestText.trim().slice(0, 300) };
  } else {
    expect(
      outcome.blocked,
      `「${snapshot.task}」是 completed，不該附「被擋下」的證據`
    ).toBeUndefined();
  }

  measured.push(snapshot);
  outcomes.push({
    task: snapshot.task,
    viewport: snapshot.viewport,
    expected: declared.expected,
    actual: outcome.actual,
    evidence: declared.evidence,
    limits: declared.limits,
    ...(blocked ? { blocked } : {}),
    ...(outcome.completedVia ? { completedVia: outcome.completedVia } : {}),
    metrics: snapshot,
  });

  const line = `${formatTaskMetrics(snapshot)}｜分類 ${outcome.actual}`;
  console.log(line);
  await test.info().attach(`task-metrics-${snapshot.task}`, {
    body: JSON.stringify(outcomes[outcomes.length - 1], null, 2),
    contentType: "application/json",
  });
  expect(
    withinDecisionTarget(snapshot, outcome.maxDecisions),
    `${line}：主要決策超過目標上限`
  ).toBe(true);
}

function syncCard(page: Page): Locator {
  return page.getByTestId("character-sync");
}

/** 開到角色頁並回傳同步卡（等它真的讀到狀態）。 */
async function openSyncCard(page: Page, narrow: boolean): Promise<Locator> {
  await navigateTo(page, COMPANION, narrow);
  const card = syncCard(page);
  await card.scrollIntoViewIfNeeded();
  await expect(card).toBeVisible({ timeout: 20_000 });
  return card;
}

/** 同步卡現在說的那一句（badge 文字）。 */
async function syncHeadline(card: Locator): Promise<string> {
  return (await card.locator(".badge").first().innerText()).trim();
}

/** 同步卡的「下一步」按鈕（沒有下一步時不存在——這本身也是被驗的事實）。 */
function syncAction(card: Locator): Locator {
  return card.getByTestId("character-sync-action");
}

/**
 * 從角色頁的同步卡一鍵到配對區，再配一台【模擬 iPhone（fixture）】。
 * 這條路徑本身就是任務「稍後連手機」：不必自己去翻連接與權限的第二層分頁。
 */
async function pairFromSyncCard(
  page: Page,
  request: APIRequestContext,
  m: TaskMetrics,
  narrow: boolean
): Promise<FakeIphone> {
  const card = await openSyncCard(page, narrow);
  m.visit("companion");
  const action = syncAction(card);
  await expect(action, "同步卡在沒有手機時必須給一個「下一步」").toBeVisible({ timeout: 20_000 });
  await expect(action).toHaveAttribute("data-action", /connect-phone|reconfirm-device/);
  m.decide(`同步卡的下一步：${(await action.innerText()).trim()}`);
  await action.click();
  await expect(page.locator(".topbar-title")).toHaveText(CONNECT.label, { timeout: 20_000 });
  m.visit("connect");
  // 一鍵就到配對區：不需要再自己切到「裝置與來源」分頁。
  const begin = page.getByRole("button", { name: "開始配對（5 分鐘內有效）" });
  await expect(begin, "同步卡的下一步必須直接落在配對區").toBeVisible({ timeout: 20_000 });
  m.decide("開始配對（5 分鐘內有效）");
  const pairing = await beginPairingFromUi(page, request, target());
  expect(pairing.code).toMatch(/^\d{6}$/);
  const fixture = await spawnFakeIphone({
    port: pairing.port,
    fingerprint: pairing.fingerprint,
    code: pairing.code,
  });
  phones.push(fixture);
  phone = fixture;
  return fixture;
}

/** 需要一台已經是 session 成員的模擬手機時用（沒有就現配一台）。 */
async function ensureMemberPhone(
  page: Page,
  request: APIRequestContext,
  m: TaskMetrics,
  narrow = false
): Promise<FakeIphone> {
  if (phone) {
    const presence = memberPresence(
      await waitCharacterSession(request, () => true, 5_000, target()),
      phone.deviceId
    );
    if (presence === "online") return phone;
  }
  const fixture = await pairFromSyncCard(page, request, m, narrow);
  await aipCapability(fixture);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "online",
    20_000,
    target()
  );
  return fixture;
}

/** 這一支自己的 daemon 上，label 為 `label` 的那一筆工作階段（有界輪詢；找不到就失敗）。 */
async function sessionByLabel(
  request: APIRequestContext,
  label: string
): Promise<Record<string, unknown>> {
  const deadline = Date.now() + 30_000;
  for (;;) {
    const list = (await api(
      request,
      "GET",
      "/v1/agent-sessions",
      undefined,
      target()
    )) as Record<string, unknown>[];
    const found = list.find((s) => String(s.label ?? "") === label);
    if (found) return found;
    if (Date.now() > deadline) {
      throw new Error(`30 秒內找不到 label 為「${label}」的工作階段`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** 一般模式的畫面不得外洩技術詞（X5；每個任務結束前都便宜地再確認一次）。 */
async function expectNoTechnicalTerms(scope: Locator): Promise<void> {
  const text = (await scope.innerText()).toLowerCase();
  expect(text).not.toMatch(/revision|sequence|epoch|schema|token/);
}

/** 390px 不得產生水平捲動（documentElement 的可捲寬度不超過視窗寬）。 */
async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const measurements = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(
    measurements.scrollWidth,
    `橫向溢出：scrollWidth=${measurements.scrollWidth} > innerWidth=${measurements.innerWidth}`
  ).toBeLessThanOrEqual(measurements.innerWidth);
}

/** 目前可見（且不在收合區塊裡）的可互動控制項數量。 */
async function visibleControls(page: Page, selector: string): Promise<number> {
  return page.locator(selector).evaluate((root) => {
    const INTERACTIVE = 'button, input, select, textarea, a[href], summary, [role="button"]';
    const inClosedDetails = (el: Element): boolean => {
      let n: Element | null = el.parentElement;
      while (n) {
        if (n.tagName === "DETAILS" && !(n as HTMLDetailsElement).open && el.tagName !== "SUMMARY") {
          return true;
        }
        n = n.parentElement;
      }
      return false;
    };
    return Array.from(root.querySelectorAll<HTMLElement>(INTERACTIVE)).filter(
      (el) => !inClosedDetails(el) && el.offsetParent !== null
    ).length;
  });
}

// ---------------------------------------------------------------------------
// 任務 1：第一次使用桌面角色（首次設定精靈）
// ---------------------------------------------------------------------------

test("任務 1：第一次使用——走完首次設定精靈，安全確認一步都不能少", async ({ page, request }) => {
  test.setTimeout(180_000);
  const m = new TaskMetrics("第一次使用桌面角色", "desktop");
  await page.setViewportSize(DESKTOP);
  await page.goto(appUrl(daemon!.api, daemon!.token));

  const wizard = page.getByRole("dialog", { name: "首次設定" });
  await expect(wizard, "全新的 daemon 第一次打開必須是精靈").toBeVisible({ timeout: 60_000 });
  m.visit("onboarding");

  await expect(wizard.getByRole("heading", { name: "選擇角色與陪伴方式" })).toBeVisible({
    timeout: 20_000,
  });
  m.decide("第 1 步：角色與陪伴方式");
  await wizard.getByRole("button", { name: "下一步" }).click();

  await expect(wizard.getByRole("heading", { name: /幫忙工作嗎？/ })).toBeVisible({
    timeout: 20_000,
  });
  m.decide("第 2 步：要不要幫忙工作");
  await wizard.getByRole("button", { name: "下一步" }).click();

  await expect(wizard.getByRole("heading", { name: "確認安全與權限預設" })).toBeVisible({
    timeout: 20_000,
  });
  m.decide("第 3 步：安全與權限預設");
  await wizard.getByRole("button", { name: "完成設定" }).click();

  // 套用前確認是安全步驟：按「套用」之前後端什麼都沒改，這一步不得被省掉。
  const confirm = page.getByRole("dialog", { name: "套用前確認" });
  await expect(confirm, "完成設定之前必須有一次套用前確認").toBeVisible({ timeout: 20_000 });
  m.safety("套用前確認");
  await confirm.getByRole("button", { name: "套用", exact: true }).click();
  await expect(confirm).toBeHidden({ timeout: 20_000 });

  const desktopNav = page.getByRole("navigation", { name: "主要導覽" });
  const firstSuccess = page.getByRole("dialog", { name: "首次成功體驗" });
  await Promise.race([
    firstSuccess.waitFor({ state: "visible", timeout: 20_000 }),
    desktopNav.waitFor({ state: "visible", timeout: 20_000 }),
  ]);
  if (await firstSuccess.isVisible().catch(() => false)) {
    m.click("關閉首次成功體驗");
    await firstSuccess.getByRole("button", { name: "完成", exact: true }).click();
  }
  await expect(desktopNav).toBeVisible({ timeout: 20_000 });
  m.visit("home");

  // 後端事實：精靈真的完成了（不是只有畫面關掉）。
  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    onboardingCompleted?: boolean;
  };
  expect(status.onboardingCompleted).toBe(true);

  // 桌面角色在「現在」第一屏就看得到（第一次使用的人不必先去找它）。
  await expect(page.getByTestId("now-character")).toBeVisible({ timeout: 20_000 });
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 2＋3：稍後連手機（同步卡的下一步一鍵到配對區）／手機與桌面互動
// ---------------------------------------------------------------------------

test("任務 2＋3：稍後連手機（同步卡 → 配對區 → 模擬 iPhone（fixture））、然後在手機上摸一下角色", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);

  const connectTask = new TaskMetrics("稍後連手機", "desktop");
  connectTask.note("手機是【模擬 iPhone（fixture）】，不是 iPhone 真機。");
  connectTask.visit("home");
  const fixture = await pairFromSyncCard(page, request, connectTask, false);
  // 配對只是「連上」；成為角色同步的成員要手機端送一次 capability（重新確認）。
  await aipCapability(fixture);
  const joined = await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "online",
    30_000,
    target()
  );
  const card = await openSyncCard(page, false);
  connectTask.visit("companion");
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .not.toBe(CHARACTER_SYNC_PROJECTION["no-device"].headline);
  expect(ONLINE_HONEST, "手機連上之後同步卡必須說得出它連上了").toContain(
    await syncHeadline(card)
  );
  const members = card.getByRole("list", { name: "同步中的裝置" });
  await expect(members.getByText(FAKE_IPHONE_LABEL)).toBeVisible();
  await expectNoTechnicalTerms(card);
  await record(connectTask, { actual: "completed" });

  // --- 任務 3：手機與桌面互動（桌面端零決策：不必為了「收到互動」再去設定什麼）。
  const touchTask = new TaskMetrics("手機與桌面互動", "desktop");
  touchTask.note("互動由模擬 iPhone（fixture）發起；桌面端不需要任何決策。");
  touchTask.visit("companion");
  const beforeRevision = Number(joined.revision ?? 0);
  const result = await aipTouch(fixture, "tap");
  expect(result.status).toBe("applied");
  const touched = await waitCharacterSession(
    request,
    (payload) => Number(payload.revision ?? 0) > beforeRevision,
    20_000,
    target()
  );
  expect(Number(touched.revision)).toBeGreaterThan(beforeRevision);
  // SSE 會把卡片推到最新：使用者盯著畫面就看得到，不必重新整理（零點擊）。
  await expect(card.getByText(/摸了摸角色/)).toBeVisible({ timeout: 30_000 });
  await record(touchTask, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 4：暫時離線 → 重新連線恢復
// ---------------------------------------------------------------------------

test("任務 4：暫時離線 → 重新連線——畫面誠實說「正在重新連線」，接回來就恢復", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("暫時離線後恢復", "desktop");
  m.note("斷線／重連由模擬 iPhone（fixture）驅動；桌面端不需要任何決策。");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const fixture = await ensureMemberPhone(page, request, m);
  const card = await openSyncCard(page, false);
  m.visit("companion");

  const seen = fixture.events.length;
  fixture.send({ op: "disconnect" });
  await fixture.waitForEvent((e) => e.event === "disconnected", 20_000, seen);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "reconnecting",
    20_000,
    target()
  );
  await expect(card.getByText(CHARACTER_SYNC_PROJECTION.reconnecting.headline)).toBeVisible({
    timeout: 30_000,
  });
  await expect(card.locator(".badge-ok"), "重新連線中不得給綠勾").toHaveCount(0);

  const reconnected = fixture.events.length;
  fixture.send({ op: "reconnect" });
  await fixture.waitForEvent((e) => e.event === "connected", 20_000, reconnected);
  const snapshot = await aipCapability(fixture);
  const replay = await aipResume(fixture, {
    lastRevision: Number(snapshot.revision ?? 0),
    lastSequence: Number(snapshot.sequence ?? 0),
    epoch: Number(snapshot.sessionEpoch ?? 0),
  });
  expect(["patches", "snapshot"]).toContain(String(replay.kind));
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "online",
    20_000,
    target()
  );
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .not.toBe(CHARACTER_SYNC_PROJECTION.reconnecting.headline);
  expect(ONLINE_HONEST, "接回來之後同步卡必須說得出它回來了").toContain(await syncHeadline(card));
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 5：完整逾時路徑（presence 逾時 45 s）→「iPhone 暫時離線」
// ---------------------------------------------------------------------------

test("任務 5：手機一直沒回來——45 秒 presence 逾時之後畫面從「重新連線中」轉成「暫時離線」", async ({
  page,
  request,
}) => {
  // presence 逾時是 45 s（`interaction-session` 的 `presence_timeout_ms`，沒有可調的
  // 環境變數），加上 watchdog sweep 的間隔，所以這一支必須真的等。
  test.setTimeout(300_000);
  const m = new TaskMetrics("等待離線逾時", "desktop");
  m.note("模擬 iPhone（fixture）斷線後不重連；等的是真的 45 秒 presence 逾時。");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const fixture = await ensureMemberPhone(page, request, m);
  const card = await openSyncCard(page, false);
  m.visit("companion");

  const seen = fixture.events.length;
  fixture.send({ op: "disconnect" });
  await fixture.waitForEvent((e) => e.event === "disconnected", 20_000, seen);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "reconnecting",
    20_000,
    target()
  );
  // 後端真相：逾時之後成員仍在名單上，但 presence 是 offline（不是無聲消失）。
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "offline",
    120_000,
    target()
  );
  await expect(card.getByText(CHARACTER_SYNC_PROJECTION.offline.headline)).toBeVisible({
    timeout: 60_000,
  });
  await expect(card.locator(".badge-ok"), "離線不得給綠勾").toHaveCount(0);

  // 收尾：接回來，後面的任務才有一台在線的手機可用。
  const reconnected = fixture.events.length;
  fixture.send({ op: "reconnect" });
  await fixture.waitForEvent((e) => e.event === "connected", 20_000, reconnected);
  await aipCapability(fixture);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "online",
    30_000,
    target()
  );
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 6：主動移除手機 → 中性終態（不是永遠亮著的待辦）
// ---------------------------------------------------------------------------

test("任務 6：主動移除手機——移除是正常終態「目前只在這台電腦使用」，不是永遠要求重配", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("主動移除手機", "desktop");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const fixture = await ensureMemberPhone(page, request, m);

  await navigateTo(page, CONNECT, false);
  m.visit("connect");
  const phoneCard = page.locator(`[data-testid="phone-card-${fixture.deviceId}"]`).first();
  await phoneCard.scrollIntoViewIfNeeded();
  await expect(phoneCard).toBeVisible({ timeout: 20_000 });
  m.decide("移除此手機");
  await phoneCard.getByRole("button", { name: "移除此手機" }).click();
  // 二段確認：不可單鍵誤觸（安全步驟，不是可以簡化掉的決策）。
  m.safety("確定移除？");
  await phoneCard.getByRole("button", { name: /確定移除？/ }).click();
  await expect(page.locator(`[data-testid="phone-card-${fixture.deviceId}"]`)).toHaveCount(0, {
    timeout: 20_000,
  });

  // 後端事實：不再是 session 成員。
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === null,
    30_000,
    target()
  );

  // 畫面事實：中性的終態句，而且下一步是「連接手機」（不是「去重新確認」）。
  const card = await openSyncCard(page, false);
  m.visit("companion");
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .toBe(CHARACTER_SYNC_PROJECTION["local-only"].headline);
  await expect(card.getByText(/不會自動回來/)).toBeVisible();
  await expect(card.locator(".badge-ok")).toHaveCount(0);
  await expect(syncAction(card)).toHaveAttribute("data-action", "connect-phone");
  await expectNoTechnicalTerms(card);

  fixture.kill();
  phone = null;
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 7：撤銷之後重新連線（重新配對＋重新確認）
// ---------------------------------------------------------------------------

test("任務 7：移除之後又想用手機——重新配對，重新確認之後才回到同步", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("撤銷後重新連線", "desktop");
  m.note("重新配對的是另一台【模擬 iPhone（fixture）】；被移除的裝置不會自動回來。");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  m.visit("home");

  const fixture = await pairFromSyncCard(page, request, m, false);
  // 配對完成但還沒送 capability：連著、卻不是成員——畫面必須說「需要重新確認裝置」，
  // 而且指名是哪一台（只給名字，不給裝置識別碼）。
  const card = await openSyncCard(page, false);
  m.visit("companion");
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .toBe(CHARACTER_SYNC_PROJECTION["needs-reconfirmation"].headline);
  await expect(card.getByText(new RegExp(FAKE_IPHONE_LABEL))).toBeVisible();
  await expect(syncAction(card)).toHaveAttribute("data-action", "reconfirm-device");

  // 手機端重新確認（fixture 的 capability＝使用者在手機上按下「同意」的那一步）。
  await aipCapability(fixture);
  await waitCharacterSession(
    request,
    (payload) => memberPresence(payload, fixture.deviceId) === "online",
    30_000,
    target()
  );
  await expect
    .poll(async () => syncHeadline(card), { timeout: 30_000 })
    .not.toBe(CHARACTER_SYNC_PROJECTION["needs-reconfirmation"].headline);
  expect(ONLINE_HONEST).toContain(await syncHeadline(card));
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 8：更換角色（瀏覽器模式＝誠實拒絕；正向效果是 Tauri 驗收）
// ---------------------------------------------------------------------------

test("任務 8：更換角色——瀏覽器模式誠實拒絕，不假裝換成功（正向效果由真 Tauri 走查驗）", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const m = new TaskMetrics("更換角色", "desktop");
  m.note(
    "瀏覽器模式：桌面角色偏好住在 Tauri host，這裡只驗得到誠實拒絕；真的換成功由 scripts/tauri-ax-walkthrough.sh 的真 Tauri 走查驗。"
  );
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const before = JSON.stringify(
    ((await api(request, "GET", "/v1/status", undefined, target())) as Record<string, unknown>)
      .characterProtocol ?? null
  );

  await navigateTo(page, COMPANION, false);
  m.visit("companion");
  // 首屏看不到整個角色庫：要換角色得先展開（一次點擊換來一個乾淨的首屏）。
  const library = page.locator('details[data-disclosure="library"]');
  await expect(library).toBeVisible({ timeout: 20_000 });
  expect(await library.evaluate((el) => (el as HTMLDetailsElement).open)).toBe(false);
  m.click("展開「更換或加入角色」");
  await library.locator("summary").click();
  const candidate = library.locator("article.character-card:not(.active)").first();
  await candidate.scrollIntoViewIfNeeded();
  await expect(candidate).toBeVisible({ timeout: 20_000 });
  m.decide("選用另一個角色");
  await candidate.getByRole("button", { name: "選用" }).click();

  // 誠實：畫面出現錯誤，使用中的角色不變，後端角色狀態一個位元都沒動。
  const alert = page.locator(".character-page").getByRole("alert").first();
  await expect(alert).toBeVisible({ timeout: 20_000 });
  await expect(page.locator("article.character-card.active")).toHaveCount(1);
  // 被按下的那一張卡片**沒有**變成使用中：失敗就不能長出「使用中」的綠勾。
  await expect(candidate, "換角色失敗時，被選的那個角色不得變成使用中").not.toHaveClass(
    /\bactive\b/
  );
  const after = JSON.stringify(
    ((await api(request, "GET", "/v1/status", undefined, target())) as Record<string, unknown>)
      .characterProtocol ?? null
  );
  expect(after, "換角色失敗時不得改到後端角色狀態").toBe(before);
  // 分類：correctly-blocked。哪天瀏覽器模式真的換得動（或改成無聲失敗），
  // `record()` 會拿這份證據跟 TASK_MANIFEST 對，兩種漂移都會紅。
  await record(m, {
    actual: "correctly-blocked",
    blocked: {
      backendBefore: before,
      backendAfter: after,
      honestText: await alert.innerText(),
      // 綠勾檢查看的是**這個動作的目標**：範圍就是那張被按下的卡片。放大到整個角色庫
      // 會掃到另一個角色的「使用中」——那是真的，不該讓這個檢查變紅。
      scope: candidate,
    },
  });
});

// ---------------------------------------------------------------------------
// 任務 9：調整陪伴程度（瀏覽器模式＝誠實降級）
// ---------------------------------------------------------------------------

test("任務 9：調整陪伴程度——瀏覽器模式照實說需要桌面版，不給按了沒用的檔位", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const m = new TaskMetrics("調整陪伴程度", "desktop");
  m.note(
    "陪伴預設（安靜／自然／活潑）寫的是桌面偏好，只有 Tauri 控制中心有；瀏覽器模式驗的是誠實降級。正向切換屬於 Tauri 驗收（見「真 Tauri 視窗走查」章節）。"
  );
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  // 套用一個檔位的**第二段**寫的是後端主動說話模式：拿它當「後端零變動」的證據。
  const before = JSON.stringify(
    await api(request, "GET", "/v1/proactive-dialogue", undefined, target())
  );
  await navigateTo(page, COMPANION, false);
  m.visit("companion");

  // 首屏第二格就是「陪伴方式」：不必展開任何東西就看得到現在是什麼情況。
  const companionship = page.locator("section.section", { hasText: "陪伴方式" }).first();
  await expect(companionship).toBeVisible({ timeout: 20_000 });
  const honest = companionship
    .getByText("桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。")
    .first();
  await expect(honest, "沒有桌面 host 時要照實說，而不是給一排按了沒用的檔位").toBeVisible();
  await expect(
    companionship.getByRole("group", { name: "陪伴方式" }),
    "瀏覽器模式不得渲染假的陪伴檔位"
  ).toHaveCount(0);
  const after = JSON.stringify(
    await api(request, "GET", "/v1/proactive-dialogue", undefined, target())
  );
  await record(m, {
    actual: "correctly-blocked",
    blocked: {
      backendBefore: before,
      backendAfter: after,
      honestText: await honest.innerText(),
      scope: companionship,
    },
  });
});

// ---------------------------------------------------------------------------
// 任務 10：設定安靜時段（這一項瀏覽器也能真的做完）
// ---------------------------------------------------------------------------

test("任務 10：設定安靜時段——存進 policy，而且 status 的安靜時段真的變成生效中", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const m = new TaskMetrics("設定安靜時段", "desktop");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  await navigateTo(page, COMPANION, false);
  m.visit("companion");

  const quiet = page.locator('details[data-disclosure="quiet"]');
  await expect(quiet).toBeVisible({ timeout: 20_000 });
  // 收合摘要必須先講清楚現在的有效值（收起來 ≠ 看不到）。
  await expect(quiet.locator("summary")).toContainText("安靜時段：");
  m.click("展開「安靜與勿擾」");
  await quiet.locator("summary").click();

  const fieldset = quiet.locator("fieldset", { hasText: "安靜時段" });
  await expect(fieldset).toBeVisible({ timeout: 20_000 });
  m.decide("啟用安靜時段");
  await fieldset.getByRole("checkbox").check();
  const times = fieldset.locator('input[type="time"]');
  await expect(times).toHaveCount(2, { timeout: 20_000 });
  m.decide("設定安靜的起訖時間（整天）");
  await times.nth(0).fill("00:00");
  await times.nth(0).blur();
  await times.nth(1).fill("23:59");
  await times.nth(1).blur();

  // 後端事實 1：policy 真的存了一段安靜時段。
  await expect
    .poll(
      async () => {
        const policy = (await api(request, "GET", "/v1/policy", undefined, target())) as Record<
          string,
          unknown
        >;
        const hours = policy.quietHours;
        return Array.isArray(hours) ? hours.length : 0;
      },
      { timeout: 20_000 }
    )
    .toBeGreaterThan(0);
  // 後端事實 2：status 說現在正在安靜時段內（設定不是只存起來好看的）。
  await expect
    .poll(
      async () =>
        (
          (await api(request, "GET", "/v1/status", undefined, target())) as {
            quietHours?: boolean;
          }
        ).quietHours,
      { timeout: 20_000 }
    )
    .toBe(true);
  await expect(quiet.getByText("已儲存，立即生效。")).toBeVisible({ timeout: 20_000 });

  // 收尾：關掉，不要污染後面的任務（也順便驗它關得掉）。
  await fieldset.getByRole("checkbox").uncheck();
  await expect
    .poll(
      async () =>
        (
          (await api(request, "GET", "/v1/status", undefined, target())) as {
            quietHours?: boolean;
          }
        ).quietHours,
      { timeout: 20_000 }
    )
    .toBe(false);
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 14：暫停主動對話（Home 一鍵暫停 → 後端真的暫停 → 恢復）
//
// 這一項**不得**用「設定安靜時段」代打：那是另一個機制（安靜時段是排程，暫停是
// 使用者現在就要它閉嘴），兩者的有效值也分開存。所以這裡除了驗 `/v1/pause` 之外，
// 還要驗安靜時段一個位元都沒被順手改掉——否則「暫停」就變成偷偷設了一段安靜時間。
// ---------------------------------------------------------------------------

test("任務 14：暫停主動對話——一鍵暫停，/v1/pause 真的是暫停，恢復也是真的", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const m = new TaskMetrics("暫停主動對話", "desktop");
  m.note("暫停與安靜時段是兩件事：這一支同時驗「暫停真的生效」與「安靜時段沒被順手改掉」。");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  m.start();
  m.visit("home");

  const pauseState = async () =>
    (await api(request, "GET", "/v1/pause", undefined, target())) as { paused?: boolean };
  const quietHours = async () =>
    JSON.stringify(
      ((await api(request, "GET", "/v1/policy", undefined, target())) as Record<string, unknown>)
        .quietHours ?? null
    );

  expect((await pauseState()).paused, "任務開始前不該已經是暫停狀態").toBe(false);
  const quietBefore = await quietHours();

  // 「暫停主動互動」就在「現在」的快速操作裡：不必先去角色頁、也不必展開任何東西。
  const home = page.locator(".home");
  const group = home.getByRole("group", { name: "暫停或恢復主動互動" });
  await expect(group).toBeVisible({ timeout: 20_000 });
  const pauseButton = group.getByRole("button", { name: "暫停主動互動" });
  await expect(pauseButton).toBeVisible({ timeout: 20_000 });
  m.decide("暫停主動互動");
  await pauseButton.click();

  // 畫面說暫停了；後端也真的暫停了（畫面說了不算）。
  await expect(page.getByText("主動互動已暫停").first()).toBeVisible({ timeout: 20_000 });
  await expect.poll(async () => (await pauseState()).paused, { timeout: 20_000 }).toBe(true);
  // 暫停 ≠ 安靜時段：policy 的安靜時段不得被順手改掉。
  expect(await quietHours(), "「暫停主動互動」不得偷偷改到安靜時段").toBe(quietBefore);
  // 暫停不是緊急停止：安全機制沒有被牽動。
  expect(
    ((await api(request, "GET", "/v1/status", undefined, target())) as {
      emergencyStop?: boolean;
    }).emergencyStop,
    "暫停主動互動不得順便觸發緊急停止"
  ).toBe(false);

  // 恢復同樣是一個決策，而且後端要真的恢復（不是畫面上換一顆按鈕）。
  const resume = group.getByRole("button", { name: "恢復主動互動" });
  await expect(resume, "暫停之後同一個位置要變成「恢復主動互動」").toBeVisible({
    timeout: 20_000,
  });
  m.decide("恢復主動互動");
  await resume.click();
  await expect.poll(async () => (await pauseState()).paused, { timeout: 20_000 }).toBe(false);
  await expect(group.getByRole("button", { name: "暫停主動互動" })).toBeVisible({
    timeout: 20_000,
  });
  expect(await quietHours(), "恢復也不得改到安靜時段").toBe(quietBefore);
  await record(m, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 任務 11：取消一件進行中的工作
// ---------------------------------------------------------------------------

test("任務 11：取消工作——按一次中斷，後端真的取消（不是畫面上停了）", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("取消進行中的工作", "desktop");
  m.note("agent 是 fixture 子程序（模擬 agent，非真 Codex／Claude Code）。");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  await navigateTo(page, WORK, false);
  m.visit("work");

  const label = `任務驗收：先跑一下再讓我中斷它 ${Date.now()}`;
  // fixture agent 的副作用（fake-pid 等）一律關在這個隔離目錄裡，不寫到別處。
  const dir = makeWorkdir(makeWorkRoot("interaction-e2e-tasks-"), "cancel", "turns");
  const composer = page.getByLabel(/幫你做什麼/);
  await expect(composer).toBeVisible({ timeout: 20_000 });
  m.decide("寫下要交代的事");
  await composer.fill(label);
  m.decide("選擇工作的資料夾");
  await page.getByLabel("加入檔案或選擇資料夾").fill(dir);
  m.decide("選這是哪一種工作");
  await page.getByLabel("這是哪一種工作").selectOption({ label: "程式工作" });
  const start = page.getByRole("button", { name: "開始", exact: true });
  await expect(start).toBeEnabled({ timeout: 20_000 });
  m.decide("開始");
  await start.click();

  const session = await sessionByLabel(request, label);
  const sessionId = String(session.sessionId);
  await waitSessionState(
    request,
    sessionId,
    ["active", "claimed-completed", "waiting-for-consent", "unknown"],
    60_000,
    target()
  );

  const card = page.locator(".provider-card", { hasText: label });
  await card.scrollIntoViewIfNeeded();
  await expect(card).toBeVisible({ timeout: 20_000 });
  const interrupt = card.getByRole("button", { name: "暫停／中斷目前工作" });
  if (await interrupt.isVisible().catch(() => false)) {
    m.decide("暫停／中斷目前工作");
    await interrupt.click();
    await expect(page.getByText("已送出中斷指令。")).toBeVisible({ timeout: 20_000 });
    await waitSessionState(request, sessionId, ["cancelled"], 60_000, target());
    // 使用者不重新載入也要看到它停了。
    await expect(card.getByText("已取消", { exact: true })).toBeVisible({ timeout: 30_000 });
    await record(m, { actual: "completed", completedVia: "後端 cancelled" });
  } else {
    // fixture agent 已經自己收尾：這時候該有的是「關閉」，不是假的中斷鈕。
    // 這一條路**沒有取消任何東西**，後端也不會出現 `cancelled`：這個任務的定義就是
    // 「取消」，沒取消到就不是 completed，誠實記成 not-run（對抗審查 general-mode-ux-029）。
    m.decide("關閉這個工作階段");
    await card.getByRole("button", { name: "關閉", exact: true }).click();
    await expect(page.getByText(/工作階段已關閉/)).toBeVisible({ timeout: 20_000 });
    await record(m, {
      actual: "not-run",
      notRun: { reason: "fixture agent 已經自己收尾，這一輪沒有可取消的工作（沒有中斷鈕）" },
    });
  }
});

// ---------------------------------------------------------------------------
// 任務 12：390px——同一組任務在窄視窗上也做得完，而且不產生橫向捲動
// ---------------------------------------------------------------------------

test("任務 12（390px）：角色頁首屏、同步卡下一步、安靜時段都做得完，且無橫向溢出", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("390px：看角色頁並連手機", "narrow");
  m.note("同一組任務在 390px 走一次；手機是【模擬 iPhone（fixture）】。");
  // `openNarrow` 會把首次設定精靈走完（單獨跑這一支時 daemon 是全新的，
  // 直接 goto 會卡在精靈上）——不假設前面的任務一定跑過。
  await openNarrow(page, appUrl(daemon!.api, daemon!.token));
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 30_000,
  });
  m.visit("home");
  await expectNoHorizontalOverflow(page);

  const card = await openSyncCard(page, true);
  m.visit("companion");
  await expectNoHorizontalOverflow(page);
  // 390px 的角色頁首屏：可見控制項收斂到個位數（收合 ≠ 刪功能，展開後全都還在）。
  const controls = await visibleControls(page, ".character-page");
  await test.info().attach("390px-character-page-visible-controls", {
    body: String(controls),
    contentType: "text/plain",
  });
  expect(controls, `390px 角色頁可見控制項 ${controls} 個，超過首屏收斂的目標`).toBeLessThanOrEqual(
    12
  );
  // 卡片不超出視窗寬。
  const box = await card.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x + box!.width).toBeLessThanOrEqual(NARROW.width);
  await expectNoTechnicalTerms(card);

  // 同步卡的下一步在 390px 也按得到，而且落點必須就是那個 action id 承諾的地方
  // （這一支跑在共用 daemon 上，前面的任務可能留著一台手機，所以不假設是哪一態）。
  const action = syncAction(card);
  await action.scrollIntoViewIfNeeded();
  await expect(action).toBeVisible({ timeout: 20_000 });
  const actionId = (await action.getAttribute("data-action")) ?? "";
  m.decide(`同步卡的下一步：${(await action.innerText()).trim()}`);
  await action.click();
  await expect(page.locator(".topbar-title")).toHaveText(CONNECT.label, { timeout: 20_000 });
  m.visit("connect");
  if (["connect-phone", "reconfirm-device", "safe-reconnect"].includes(actionId)) {
    // 落點是配對區：一鍵就能開始配對，不必自己去翻第二層分頁。
    await expect(page.getByRole("button", { name: "開始配對（5 分鐘內有效）" })).toBeVisible({
      timeout: 20_000,
    });
  } else {
    // 落點是裝置清單（`view-capabilities`／`open-devices`：去看那台裝置少了什麼）。
    expect(["view-capabilities", "open-devices"], `未知的 action id：${actionId}`).toContain(
      actionId
    );
    await expect(page.getByRole("region", { name: "已連接的裝置" })).toBeVisible({
      timeout: 20_000,
    });
  }
  await expectNoHorizontalOverflow(page);
  await record(m, { actual: "completed" });

  // --- 390px 的安靜時段（同一個任務在窄視窗再做一次）。
  const quietTask = new TaskMetrics("390px：設定安靜時段", "narrow");
  await navigateTo(page, COMPANION, true);
  quietTask.visit("companion");
  const quiet = page.locator('details[data-disclosure="quiet"]');
  await quiet.scrollIntoViewIfNeeded();
  quietTask.click("展開「安靜與勿擾」");
  await quiet.locator("summary").click();
  const fieldset = quiet.locator("fieldset", { hasText: "安靜時段" });
  await fieldset.scrollIntoViewIfNeeded();
  quietTask.decide("啟用安靜時段");
  await fieldset.getByRole("checkbox").check();
  await expect
    .poll(
      async () => {
        const policy = (await api(request, "GET", "/v1/policy", undefined, target())) as Record<
          string,
          unknown
        >;
        return Array.isArray(policy.quietHours) ? policy.quietHours.length : 0;
      },
      { timeout: 20_000 }
    )
    .toBeGreaterThan(0);
  await expectNoHorizontalOverflow(page);
  // 收尾。
  await fieldset.getByRole("checkbox").uncheck();
  await record(quietTask, { actual: "completed" });
});

// ---------------------------------------------------------------------------
// 可及性：鍵盤、收合區塊、對話框、Reduced Motion
// ---------------------------------------------------------------------------

test("可及性：鍵盤走得到同步卡的「下一步」，而且按 Enter 真的到得了配對區", async ({ page }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  await navigateTo(page, COMPANION, false);
  const action = syncAction(syncCard(page));
  await expect(action).toBeVisible({ timeout: 20_000 });

  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  let focused = false;
  for (let i = 0; i < 80 && !focused; i += 1) {
    await page.keyboard.press("Tab");
    focused = await action.evaluate((el) => el === document.activeElement);
  }
  expect(focused, "80 次 Tab 之內沒有走到同步卡的「下一步」").toBe(true);
  await page.keyboard.press("Enter");
  await expect(page.locator(".topbar-title")).toHaveText(CONNECT.label, { timeout: 20_000 });
});

test("可及性：角色頁的收合區塊用 Enter／Space 展開得了，而且焦點不會掉", async ({ page }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  await navigateTo(page, COMPANION, false);
  const quiet = page.locator('details[data-disclosure="quiet"]');
  const summary = quiet.locator("summary");
  await expect(summary).toBeVisible({ timeout: 20_000 });
  // 每一個收合區塊都有可及名稱（螢幕閱讀器唸得出這是什麼）。
  for (const id of ["appearance", "quiet", "proactive", "library"]) {
    const text = await page.locator(`details[data-disclosure="${id}"] summary`).innerText();
    expect(text.trim().length, `收合區塊 ${id} 沒有可讀的標題`).toBeGreaterThan(0);
  }

  await summary.focus();
  await expect(summary).toBeFocused();
  await page.keyboard.press("Enter");
  await expect
    .poll(async () => quiet.evaluate((el) => (el as HTMLDetailsElement).open), { timeout: 5_000 })
    .toBe(true);
  await expect(summary, "展開後焦點必須還在原地（不能掉到 body）").toBeFocused();
  await page.keyboard.press("Space");
  await expect
    .poll(async () => quiet.evaluate((el) => (el as HTMLDetailsElement).open), { timeout: 5_000 })
    .toBe(false);
  await expect(summary).toBeFocused();
});

test("可及性：對話框開著的時候，Escape 收得掉，而且停止的方式一直在", async ({ page, request }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);

  // 1. 通知中心是一個真的 modal（焦點陷阱）：開著時 Escape 一定收得掉。
  const bell = page.getByRole("button", { name: /通知中心，(\d+|未知) 項待決定/ });
  await expect(bell).toBeVisible({ timeout: 20_000 });
  await bell.click();
  const panel = page.getByRole("dialog", { name: "通知中心" });
  await expect(panel).toBeVisible({ timeout: 20_000 });
  // 對話框內部一定有一個收起來的辦法（不是只能用滑鼠點外面）。
  await expect(panel.getByRole("button", { name: "關閉" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden({ timeout: 10_000 });

  // 2. 收掉之後，頂部列的「緊急停止」立刻是鍵盤可達的（不必先找路）。
  const estop = page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true });
  await expect(estop).toBeVisible();
  await estop.focus();
  await expect(estop).toBeFocused();

  // 3. ⌘K 的指令面板本身也是對話框，而「緊急停止」永遠在它的清單裡——
  //    而且是二段確認：只按第一下不會真的停（後端狀態一個位元都沒動）。
  await page.keyboard.press("ControlOrMeta+k");
  const search = page.getByRole("dialog", { name: "全域搜尋" });
  await expect(search).toBeVisible({ timeout: 20_000 });
  await expect(search.getByText("緊急停止", { exact: true }).first()).toBeVisible();
  await expect(search.getByText(/再確認一次/).first()).toBeVisible();
  // 面板是真 modal：容器一掛載就拿到焦點（Escape 立刻有人接），30 ms 後才把焦點交給搜尋框。
  const searchInput = search.getByPlaceholder(/搜尋設定/);
  await expect(searchInput).toBeFocused({ timeout: 10_000 });
  await page.keyboard.press("Escape");
  await expect(search).toBeHidden({ timeout: 10_000 });
  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(status.emergencyStop, "只看了指令清單不得觸發緊急停止").toBe(false);
});

test("可及性：套用前確認對話框開著時 Escape 收得掉，而且什麼都還沒被套用", async ({
  page,
  request,
}) => {
  // 精靈的「套用前確認」是整個 App 裡**唯一**會擋住主畫面（含頂部列的緊急停止）的
  // 對話框：精靈本身取代整個外殼，所以對話框開著時「停止的方式」不在畫面上。
  // 這一支要釘住的就是那條逃生路徑真的存在而且是誠實的：
  //   1. Escape 收得掉（不是只能用滑鼠點「取消」）；
  //   2. 收掉之後**後端一個位元都沒動**（套用前確認的承諾）；
  //   3. 從精靈按「略過」回到主畫面之後，緊急停止立刻是鍵盤可達的。
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const policyBefore = JSON.stringify(await api(request, "GET", "/v1/policy", undefined, target()));

  // 重新執行首次設定（更多 → 外觀與語言）：不改任何東西，只是把精靈叫回來。
  await navigateTo(page, PAGES[4], false);
  await page.getByRole("tab", { name: "外觀與語言" }).click();
  await page.getByRole("button", { name: "重新執行首次設定" }).click();
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  await expect(wizard).toBeVisible({ timeout: 20_000 });
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: /幫忙工作嗎？/ })).toBeVisible({
    timeout: 20_000,
  });
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: "確認安全與權限預設" })).toBeVisible({
    timeout: 20_000,
  });
  await wizard.getByRole("button", { name: "完成設定" }).click();

  // 1. 真的是 modal（有名字、aria-modal），而且 Escape 收得掉。
  const confirm = page.getByRole("dialog", { name: "套用前確認" });
  await expect(confirm).toBeVisible({ timeout: 20_000 });
  await expect(confirm).toHaveAttribute("aria-modal", "true");
  await expect(confirm.getByRole("button", { name: "取消" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(confirm).toBeHidden({ timeout: 10_000 });

  // 2. 取消＝什麼都沒套用（policy 與緊急停止狀態一個位元都沒動）。
  expect(
    JSON.stringify(await api(request, "GET", "/v1/policy", undefined, target())),
    "按 Escape 取消套用之後不得改到任何設定"
  ).toBe(policyBefore);
  expect(
    ((await api(request, "GET", "/v1/status", undefined, target())) as {
      emergencyStop?: boolean;
    }).emergencyStop
  ).toBe(false);

  // 3. 精靈還在（沒有把使用者關在半途），而且「略過」是鍵盤可達的逃生口；
  //    回到主畫面之後緊急停止立刻可聚焦。
  const skip = wizard.getByRole("button", { name: /略過/ });
  await skip.focus();
  await expect(skip).toBeFocused();
  await skip.click();
  const estop = page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true });
  await expect(estop).toBeVisible({ timeout: 20_000 });
  await estop.focus();
  await expect(estop).toBeFocused();
});

test("可及性：暫停對話框與配對區開著時，Escape／緊急停止都沒有被關住", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const estop = page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true });

  // --- A. 「暫停一段時間…」是真 modal：Escape 收得掉，而且開關對話框不改後端。
  const pausedBefore = (
    (await api(request, "GET", "/v1/pause", undefined, target())) as { paused?: boolean }
  ).paused;
  await page.locator(".home").getByRole("button", { name: "暫停一段時間…" }).click();
  const pauseDialog = page.getByRole("dialog", { name: "暫停主動互動" });
  await expect(pauseDialog).toBeVisible({ timeout: 20_000 });
  await expect(pauseDialog).toHaveAttribute("aria-modal", "true");
  await expect(pauseDialog.getByRole("button", { name: "關閉" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pauseDialog).toBeHidden({ timeout: 10_000 });
  expect(
    ((await api(request, "GET", "/v1/pause", undefined, target())) as { paused?: boolean }).paused,
    "只是打開又關掉暫停對話框，不得改到後端的暫停狀態"
  ).toBe(pausedBefore);
  // 收掉之後緊急停止立刻可聚焦（不必先找路）。
  await estop.focus();
  await expect(estop).toBeFocused();

  // --- B. 配對區**不是** modal：配對進行中，緊急停止仍然在畫面上、仍然可聚焦。
  //     （這一版的配對是「連接與權限」頁上的一塊通知區，不是對話框——所以沒有
  //     「Escape 才收得掉」這回事；要釘住的是它不會把停止的路徑關起來。）
  await navigateTo(page, CONNECT, false);
  // 配對區住在「裝置與來源」那一格；預設落點不一定是它（深連結才會直接開）。
  const providersTab = page.getByRole("tab", { name: "裝置與來源" });
  if (await providersTab.isVisible().catch(() => false)) await providersTab.click();
  const pairing = await beginPairingFromUi(page, request, target());
  expect(pairing.code).toMatch(/^\d{6}$/);
  const notice = page.locator(".notice-box", { hasText: "輸入配對碼" });
  await expect(notice).toBeVisible();
  const insideModal = await notice.evaluate((el) =>
    Boolean(el.closest('[role="dialog"][aria-modal="true"]'))
  );
  expect(insideModal, "配對區若變成 modal，就必須同時提供 Escape 與停止的路徑").toBe(false);
  await expect(estop).toBeVisible();
  await estop.focus();
  await expect(estop).toBeFocused();
  // Escape 不會讓配對期無聲消失（使用者以為關掉了說明，其實碼作廢了）。
  await page.keyboard.press("Escape");
  await expect(notice).toBeVisible();
  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(status.emergencyStop, "這一支只是看畫面，不得觸發緊急停止").toBe(false);
});

test("可及性：⌘K 面板焦點離開搜尋框之後，Escape 一樣收得掉、Tab 逃不出面板", async ({ page, request }) => {
  // M3c 發現的缺陷：Escape 以前只掛在搜尋框上，Tab 一下（焦點落到第一個選項＝「緊急停止」）
  // 之後就關不掉，面板底下卻寫著「Esc 關閉」；overlay 也沒有焦點陷阱。
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  await page.keyboard.press("ControlOrMeta+k");
  const search = page.getByRole("dialog", { name: "全域搜尋" });
  await expect(search).toBeVisible({ timeout: 20_000 });
  await expect(search).toHaveAttribute("aria-modal", "true");
  const searchInput = search.getByPlaceholder(/搜尋設定/);
  await expect(searchInput).toBeFocused({ timeout: 10_000 });
  // 1. Tab → 第一個選項；焦點還在面板裡。
  await page.keyboard.press("Tab");
  const focusedInside = await page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"][aria-label="全域搜尋"]');
    return dialog?.contains(document.activeElement) ?? false;
  });
  expect(focusedInside, "Tab 之後焦點必須留在面板裡").toBe(true);
  const firstOption = search.getByRole("option").first();
  await expect(firstOption).toBeFocused();
  // 2. Shift+Tab 從搜尋框往前不會逃出面板（循環到最後一個可聚焦元素）。
  await searchInput.focus();
  await page.keyboard.press("Shift+Tab");
  const stillInside = await page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"][aria-label="全域搜尋"]');
    return dialog?.contains(document.activeElement) ?? false;
  });
  expect(stillInside, "Shift+Tab 不得逃出面板").toBe(true);
  // 3. 焦點在選項上時 Escape 收得掉；只看了清單，緊急停止沒有被觸發。
  await firstOption.focus();
  await expect(firstOption).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(search).toBeHidden({ timeout: 10_000 });
  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(status.emergencyStop, "在選項上按 Escape 不得觸發緊急停止").toBe(false);
});

test("可及性：Reduced Motion 下，角色頁首屏與同步卡說的話一字不變、也讀得到", async ({ page }) => {
  test.setTimeout(180_000);
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  const before = await syncHeadline(await openSyncCard(page, false));
  const allowed = Object.values(CHARACTER_SYNC_PROJECTION).map((p) => p.headline);
  expect(allowed, "同步卡的句子必須是契約固定文案").toContain(before);
  const summariesBefore = await page
    .locator(".character-page details[data-disclosure] summary")
    .allInnerTexts();

  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.reload();
  const card = await openSyncCard(page, false);
  expect(await syncHeadline(card), "減少動態不得改變同步卡說的事實").toBe(before);
  await expect(card.getByText(before, { exact: true }).first()).toBeVisible({ timeout: 20_000 });
  expect((await card.innerText()).trim().length).toBeGreaterThan(0);
  // 收合區塊的摘要（帶著有效值的那一行）在減少動態下同樣讀得到。
  const summariesAfter = await page
    .locator(".character-page details[data-disclosure] summary")
    .allInnerTexts();
  expect(summariesAfter).toEqual(summariesBefore);
  for (const text of summariesAfter) expect(text.trim().length).toBeGreaterThan(0);
  await page.emulateMedia({ reducedMotion: null });
});

// ---------------------------------------------------------------------------
// 任務 13：緊急停止（放最後——它會撤銷同意、取消工作、停掉感測）
// ---------------------------------------------------------------------------

test("任務 13：緊急停止——兩步按得完，後端真的停了，解除必須走安全流程", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const m = new TaskMetrics("緊急停止", "desktop");
  await page.setViewportSize(DESKTOP);
  await openHere(page);
  m.visit("home");

  const home = page.locator(".home");
  const quick = home.getByRole("button", { name: "緊急停止", exact: true });
  await quick.scrollIntoViewIfNeeded();
  await expect(quick).toBeVisible({ timeout: 20_000 });
  m.decide("緊急停止");
  await quick.click();
  m.safety("立即停止一切？（二段確認）");
  await home.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible({ timeout: 20_000 });

  const status = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(status.emergencyStop, "畫面說停了，後端就必須真的停了").toBe(true);

  // 解除不是一顆按鈕：必須走安全流程（而且不會自動恢復）。
  await page.locator(".topbar").getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  m.click("前往解除");
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  m.decide("我了解，解除緊急停止");
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  m.safety("確定解除？（二段確認）");
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  await expect(
    page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible({ timeout: 20_000 });
  const cleared = (await api(request, "GET", "/v1/status", undefined, target())) as {
    emergencyStop?: boolean;
  };
  expect(cleared.emergencyStop).toBe(false);
  await record(m, { actual: "completed" });
});
