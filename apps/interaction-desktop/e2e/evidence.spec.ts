// 畫面證據：來自實際 App＋真實 Runtime 資料的截圖（v0.5 五入口 IA＋Phase 8 一般模式）。
//
// 誠實範圍（每張圖都適用）：
// - 全部是「瀏覽器版控制中心」（vite dev server＋headless Chromium）對 global-setup 起的
//   真 daemon 擷取。沒有 Tauri 角色視窗、沒有可信 overlay、沒有 ESP32／iPhone 真機。
// - 角色頁的「角色視窗未連線」是真話：瀏覽器 e2e 沒有角色視窗。
// - 「工作」的四種狀態（處理中／等你允許／對方說已完成／已由你確認）來自真 Runtime＋
//   fixture agent 子程序（global-setup 預設把 Codex／Claude Code 指向
//   crates/interaction-runtime/tests/fixtures/fake_*.sh；E2E_REAL_AGENTS=1 時該測試 skip）。
//   狀態機、mailbox、lease、人工 verify 全是真的；只有 agent 本體是模擬器。
// - 「角色如何接上系統」的外部 adapter 是 examples/character-adapters/text-adapter.mjs
//   （模擬 adapter，fixture）真的透過 WebSocket 接上 daemon。
// - waiting 狀態是真的 Knowledge Candidate；loading 是延遲真請求；error 是中斷一個真 transport
//   request；offline 指向沒人聽的 port；emergency 是真的 estop（放最後，因為會撤銷同意）。
// - 新安裝的空狀態即「空白／初次使用」證據：不硬編假資料。
//
// 檔名慣例：`desktop-<state>[-<page>].png`（1200×800）／`narrow-…`（390×844）。

import { test, expect, Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  api,
  apiBase,
  appUrl,
  confirmApply,
  DESKTOP,
  NARROW,
  navigateTo,
  openApp,
  openNarrow,
  PAGES,
  waitSessionState,
} from "./helpers";

const OUT = path.resolve(process.cwd(), "../../docs/assets/v05-evidence");
const REPO_ROOT = process.env.E2E_REPO_ROOT ?? path.resolve(process.cwd(), "../..");

async function shot(page: Page, name: string) {
  await page.waitForTimeout(150);
  await page.screenshot({ path: path.join(OUT, `${name}.png`), fullPage: false });
}

/** 把元素捲到視窗頂端（截圖以它為主角）。 */
/** M3 §4.1 的收合區塊：展開它，內容才在畫面上（收合 ≠ 刪功能）。 */
async function openDisclosure(page: Page, id: string) {
  const disclosure = page.locator(`details[data-disclosure="${id}"]`);
  await expect(disclosure).toBeVisible({ timeout: 20_000 });
  if (!(await disclosure.evaluate((el) => (el as HTMLDetailsElement).open))) {
    await disclosure.locator("summary").click();
  }
}

async function scrollTop(locator: import("@playwright/test").Locator) {
  await locator.first().evaluate((el) => el.scrollIntoView({ block: "start" }));
  await locator.first().page().waitForTimeout(120);
}

async function capturePageMatrix(page: Page, state: string) {
  for (const [viewport, size] of [
    ["desktop", DESKTOP],
    ["narrow", NARROW],
  ] as const) {
    await page.setViewportSize(size);
    for (const target of PAGES) {
      await navigateTo(page, target, viewport === "narrow");
      await page.waitForTimeout(120);
      await page.screenshot({
        path: path.join(OUT, `${viewport}-${state}-${target.id}.png`),
        fullPage: false,
      });
    }
  }
}

// 順序有意義（單一 worker、單一 daemon）：緊急停止放最後（會撤銷同意）。
// 故意不用 serial mode：一張圖失敗不該讓其餘證據跟著被 skip。

test("擷取：每個一級頁（桌面 1200px；現在三個回答、角色頁五區、工作空狀態、連接與權限五區）", async ({
  page,
}) => {
  test.setTimeout(90_000);
  fs.mkdirSync(OUT, { recursive: true });
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  for (const p of PAGES) {
    await navigateTo(page, p, false);
    await expect(page.getByText(p.marker).first()).toBeVisible({ timeout: 10_000 });
    if (p.id === "home") {
      // 第一屏只回答三件事＋五個快速操作。
      for (const id of ["now-character", "now-work", "now-decisions"]) {
        await expect(page.getByTestId(id)).toBeVisible();
      }
      await expect(page.getByText("角色離線，改用文字。")).toBeVisible();
      for (const name of ["交代一件事", "暫停主動互動", "加入裝置", "停止所有感測"]) {
        await expect(page.getByRole("button", { name })).toBeVisible();
      }
      // 首頁也有緊急停止（與頂部列同一條路徑）；選擇器要限定範圍避免撞名。
      await expect(
        page.locator(".home").getByRole("button", { name: "緊急停止", exact: true })
      ).toBeVisible();
    }
    if (p.id === "companion") {
      // M3 §4.1 之後的一般模式 IA：首屏三格＋按需展開的收合區塊（收合 ≠ 刪功能）。
      for (const heading of ["目前角色", "陪伴方式", "同步"]) {
        await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
      }
      for (const [id, title] of [
        ["appearance", "外觀與名字"],
        ["behavior", "調整陪伴方式"],
        ["library", "更換或加入角色"],
      ] as const) {
        const disclosure = page.locator(`details[data-disclosure="${id}"]`);
        await expect(disclosure).toBeVisible();
        await expect(disclosure.locator("summary")).toContainText(title);
      }
      await expect(page.getByText("角色視窗未連線")).toBeVisible();
    }
    if (p.id === "work") {
      // 全新 daemon：工作頁空狀態＝composer＋「目前沒有交代中的工作」。
      await expect(page.getByLabel(/幫你做什麼/)).toBeVisible();
      await expect(page.getByText("目前沒有交代中的工作。")).toBeVisible();
    }
    if (p.id === "connect") {
      // 裝置優先：五區固定順序，已連接的裝置排在能力兩區之前。
      const areaIds = [
        "connect-area-devices",
        "connect-area-see",
        "connect-area-respond",
        "connect-area-confirm",
        "connect-area-stop",
      ];
      for (const id of areaIds) {
        await expect(page.getByTestId(id)).toBeVisible();
      }
      const order = await page
        .locator("[data-testid^='connect-area-']")
        .evaluateAll((els) => els.map((el) => el.getAttribute("data-testid")));
      expect(order).toEqual(areaIds);
      for (const heading of [
        "已連接的裝置",
        "系統可以看見什麼",
        "系統可以做什麼",
        "目前需要確認的權限",
        "立即停止與撤銷",
      ]) {
        await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
      }
    }
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(OUT, `desktop-${p.id}.png`), fullPage: false });
    if (p.id === "connect") {
      await page.getByRole("tab", { name: "裝置與來源" }).click();
      await page.getByRole("button", { name: "重新掃描" }).click();
      await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
      await page.screenshot({ path: path.join(OUT, "desktop-hardware-scan.png"), fullPage: false });
    }
  }
});

test("擷取：每個一級頁（390px 窄視窗）", async ({ page }) => {
  test.setTimeout(90_000);
  await openNarrow(page);
  for (const p of PAGES) {
    // 主要四項直接點；「更多」走更多選單。
    await navigateTo(page, p, true);
    // 窄視窗以頁標題確認導覽成功（內容細節由桌面測試把關）。
    await expect(page.locator(".topbar-title")).toHaveText(p.label, { timeout: 10_000 });
    if (p.id === "home") {
      for (const id of ["now-character", "now-work", "now-decisions"]) {
        await expect(page.getByTestId(id)).toBeVisible();
      }
    }
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(OUT, `narrow-${p.id}.png`) });
    if (p.id === "connect") {
      await page.getByRole("tab", { name: "裝置與來源" }).click();
      await page.getByRole("button", { name: "重新掃描" }).click();
      await expect(page.getByText(/感測器啟動：否/)).toBeVisible();
      await page.screenshot({ path: path.join(OUT, "narrow-hardware-scan.png") });
    }
  }
});

test("擷取：全域搜尋（⌘K）", async ({ page }) => {
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.getByRole("dialog", { name: "全域搜尋" })).toBeVisible();
  await page.screenshot({ path: path.join(OUT, "desktop-global-search.png") });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "全域搜尋" })).toBeHidden();
});

test("擷取：角色頁細節（能力摘要／外觀與名字／陪伴方式／更換或加入角色／匯入對話框；桌面＋390px）", async ({
  page,
}) => {
  test.setTimeout(90_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, PAGES[1], false);
  // 角色能力摘要：manifest／registry 的轉述行（capabilitySummary），不是硬編文案。
  const summary = page.getByRole("list", { name: "角色能力摘要" });
  await expect(summary).toBeVisible();
  expect(await summary.getByRole("listitem").count()).toBeGreaterThan(0);
  await expect(summary.getByText(/可以接收：/)).toBeVisible();
  await scrollTop(page.getByRole("heading", { name: "目前角色", exact: true }));
  await shot(page, "desktop-companion-capabilities");

  // 外觀與名字／調整陪伴方式：M3 之後是收合區塊（展開才看得到內容）。桌面 prefs 只在 Tauri 存在；
  // 瀏覽器檢視必須誠實說明，但 36 表情預覽（與桌面角色同一套即時繪製）仍在。
  await openDisclosure(page, "appearance");
  await scrollTop(page.locator('details[data-disclosure="appearance"] summary'));
  await expect(
    page.getByText("桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。").first()
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "36 表情預覽" })).toBeVisible();
  await shot(page, "desktop-companion-appearance");

  // 陪伴方式：首屏一句話摘要＋三個檔位；瀏覽器模式沒有桌面 prefs，誠實說需要桌面版。
  await scrollTop(page.getByRole("heading", { name: "陪伴方式", exact: true }));
  await openDisclosure(page, "behavior");
  await expect(
    page.getByText("桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。").nth(1)
  ).toBeVisible();
  await shot(page, "desktop-companion-companionship");

  await openDisclosure(page, "library");
  await scrollTop(page.locator('details[data-disclosure="library"] summary'));
  // 內建目錄有多張同名「小樞」卡（女僕／黃昏／櫻）：以「使用中」那張為準。
  await expect(page.locator("article.character-card.active")).toHaveCount(1);
  await expect(page.locator("article.character-card.active")).toContainText("小樞");
  await expect(page.locator("article.character-card.active")).toContainText("使用中");
  await expect(page.getByRole("article", { name: /^角色 / }).first()).toBeVisible();
  await shot(page, "desktop-companion-library");

  // 匯入對話框：瀏覽器檢視必須誠實說匯入需要桌面版（仍可檢查角色檔）。
  // 一般模式只有選檔；貼上角色描述檔原文的輸入框在進階模式。
  await page.getByRole("button", { name: "匯入角色…" }).click();
  const importDialog = page.getByRole("dialog", { name: "匯入角色" });
  await expect(importDialog).toBeVisible();
  await expect(importDialog.getByText(/匯入需要桌面版控制中心/)).toBeVisible();
  await expect(importDialog.getByLabel("選擇角色描述檔")).toHaveCount(1);
  await expect(importDialog.getByLabel("角色描述檔內容")).toHaveCount(0);
  await shot(page, "desktop-companion-import");
  await page.keyboard.press("Escape");
  await expect(importDialog).toBeHidden();

  // 390px：更換角色。
  await page.setViewportSize(NARROW);
  await openDisclosure(page, "library");
  await scrollTop(page.locator('details[data-disclosure="library"] summary'));
  // 內建目錄有多張同名「小樞」卡（女僕／黃昏／櫻）：以「使用中」那張為準。
  await expect(page.locator("article.character-card.active")).toHaveCount(1);
  await expect(page.locator("article.character-card.active")).toContainText("小樞");
  await expect(page.locator("article.character-card.active")).toContainText("使用中");
  await shot(page, "narrow-companion-library");
  await scrollTop(page.getByRole("heading", { name: "目前角色", exact: true }));
  await shot(page, "narrow-companion-capabilities");
});

test("擷取：角色載入失敗 → 中立「角色」＋改用文字（真實中斷內建索引請求）", async ({ page }) => {
  test.setTimeout(60_000);
  // 瀏覽器模式沒有桌面 prefs，無法直接「選用純文字角色」；改為真的讓內建角色索引請求失敗
  // （connectionrefused），畫面必須退回中立名字與固定文字，而不是假造一個角色。
  await page.route("**/characters/index.json", (route) => route.abort("connectionrefused"));
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  const nav = page.getByRole("navigation", { name: "主要導覽" });
  await expect(nav.getByText("角色", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(nav.getByText("小樞", { exact: true })).toHaveCount(0);
  await nav.getByText("角色", { exact: true }).click();
  await expect(page.locator(".topbar-title")).toHaveText("角色");
  await expect(page.getByText(/內建角色索引無法載入/)).toBeVisible({ timeout: 10_000 });
  await expect(
    page.getByText("找不到目前設定的角色資料；桌面角色視窗會改用文字顯示。")
  ).toBeVisible();
  await expect(page.getByText("角色視窗未連線")).toBeVisible();
  await shot(page, "desktop-companion-fallback");
  await page.setViewportSize(NARROW);
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
  await shot(page, "narrow-companion-fallback");
  await page.unroute("**/characters/index.json");
});

test("擷取：工作 composer 填寫後的開始前預覽（桌面＋390px）", async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, PAGES[2], false);
  const task = page.getByLabel(/幫你做什麼/);
  await expect(task).toBeVisible();
  await expect(page.getByRole("button", { name: "開始", exact: true })).toBeDisabled();
  await task.fill("看一下這個資料夾的測試有沒有壞掉，跟我說結果就好，不要改任何檔案。");
  const preview = page.getByRole("group", { name: "開始前預覽" });
  for (const term of ["這次會讀取什麼", "會不會修改內容", "最多使用多少時間與費用"]) {
    await expect(preview.getByText(term, { exact: true })).toBeVisible();
  }
  await scrollTop(page.getByRole("heading", { name: "交代一件工作", exact: true }));
  await shot(page, "desktop-work-preview");
  // 展開「查看技術細節」：Agent／工具／沙箱／工作目錄／上限／取消／原始授權範圍。
  const techDetails = preview.locator("details.tech-details");
  await techDetails.getByText("查看技術細節").click();
  await expect(preview.getByText("原始授權範圍", { exact: true })).toBeVisible();
  await scrollTop(preview);
  await shot(page, "desktop-work-preview-details");
  await page.setViewportSize(NARROW);
  await scrollTop(preview);
  await shot(page, "narrow-work-preview");
  await page.getByRole("button", { name: "清空" }).click();
  await expect(task).toHaveValue("");
});

test("擷取：工作四種誠實狀態（fixture agent：處理中／等你允許／對方說已完成／已由你確認）", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  test.skip(
    process.env.E2E_FAKE_AGENTS !== "1",
    "需要 fixture agent（global-setup 預設啟用；E2E_REAL_AGENTS=1 時略過，不用真 CLI 花錢）"
  );
  // 每個 session 一個隔離 workdir：fixture 只在 cwd 讀 fake-mode／寫 fake-pid，不碰 repo。
  const workRoot = fs.mkdtempSync(path.join(os.tmpdir(), "interaction-e2e-work-"));
  const workdir = (name: string, mode?: string) => {
    const dir = path.join(workRoot, name);
    fs.mkdirSync(dir);
    if (mode) fs.writeFileSync(path.join(dir, "fake-mode"), mode);
    return dir;
  };
  const create = async (agentId: "codex" | "claude-code", label: string, dir: string) => {
    const record = (await api(request, "POST", "/v1/agent-sessions", {
      agentId,
      label,
      ttlMinutes: 30,
      workdir: dir,
      dataScope: [`workspace:${dir}`],
      toolScope: [],
      consentScope: [],
      allowWrite: false,
    })) as { sessionId: string };
    await api(request, "POST", `/v1/agent-sessions/${record.sessionId}/messages`, {
      kind: "task",
      body: { task: "畫面證據用的一句話任務（fixture）。" },
    });
    return record.sessionId;
  };

  // A 處理中：fake_codex `turns` 模式回 turn/started＋一則 agent 訊息後就不再說話。
  const labelA = "證據A：處理中（fixture Codex）";
  const idA = await create("codex", labelA, workdir("a-working", "turns"));
  // B 等你允許：fake_codex 預設在 thread/start 後丟一個 approval ServerRequest（沒人裁決）。
  const labelB = "證據B：等你允許（fixture Codex）";
  const idB = await create("codex", labelB, workdir("b-consent"));
  // C 對方說已完成：fake_claude 預設模式一輪就回 result（只是聲稱）。
  const labelC = "證據C：對方說已完成（fixture Claude）";
  const idC = await create("claude-code", labelC, workdir("c-claimed"));
  // D 已由你確認：同 C，再由人類 token POST /verify。
  const labelD = "證據D：已由你確認（fixture Claude＋人工驗證）";
  const idD = await create("claude-code", labelD, workdir("d-verified"));

  await waitSessionState(request, idA, ["active"]);
  await waitSessionState(request, idB, ["waiting-for-consent"]);
  await waitSessionState(request, idC, ["claimed-completed"]);
  await waitSessionState(request, idD, ["claimed-completed"]);
  const verified = (await api(request, "POST", `/v1/agent-sessions/${idD}/verify`, {
    note: "畫面證據：人工確認（fixture 任務）",
  })) as { state: string; humanVerified: unknown };
  // 誠實階梯：verify 不改 state（仍是 claimed-completed），只加 humanVerified 旗標。
  expect(verified.state).toBe("claimed-completed");
  expect(verified.humanVerified).toBeTruthy();

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, PAGES[2], false);
  const card = (label: string) => page.locator(".provider-card", { hasText: label });
  await expect(card(labelA)).toBeVisible({ timeout: 15_000 });

  // A：處理中。
  await expect(card(labelA).getByText("處理中", { exact: true })).toBeVisible();
  await scrollTop(card(labelA));
  await shot(page, "desktop-work-working");

  // B：等你允許；展開後是「等待你核可」＋核可／拒絕＋倒數（後端 TTL 到期即失效）。
  await expect(card(labelB).getByText("等你允許", { exact: true })).toBeVisible();
  await card(labelB).getByRole("button", { name: "查看結果／訊息" }).click();
  await expect(card(labelB).getByText("等待你核可")).toBeVisible({ timeout: 15_000 });
  await expect(card(labelB).getByRole("button", { name: "核可", exact: true })).toBeVisible();
  await expect(card(labelB).getByRole("button", { name: "拒絕", exact: true })).toBeVisible();
  await scrollTop(card(labelB));
  await shot(page, "desktop-work-consent");

  // C：對方說已完成——它的說法；沒有綠勾；有「標記為已驗證」按鈕。
  await expect(card(labelC).getByText("對方說已完成", { exact: true })).toBeVisible();
  await expect(card(labelC).getByText(/它的說法/)).toBeVisible();
  await expect(card(labelC).getByText(/✓/)).toHaveCount(0);
  await expect(
    card(labelC).getByRole("button", { name: "標記為已驗證（我確認過結果）" })
  ).toBeVisible();
  await scrollTop(card(labelC));
  await shot(page, "desktop-work-claimed");

  // D：已由你確認（綠勾唯一來源＝人工 verify）；不再顯示「標記為已驗證」。
  await expect(card(labelD).getByText(/✓ 已由你確認/)).toBeVisible();
  await expect(card(labelD).getByText(/由你親自確認/)).toBeVisible();
  await expect(
    card(labelD).getByRole("button", { name: "標記為已驗證（我確認過結果）" })
  ).toHaveCount(0);
  await scrollTop(card(labelD));
  await shot(page, "desktop-work-verified");

  // 390px：等待確認（claimed）／等你允許／處理中／已由你確認。
  await page.setViewportSize(NARROW);
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
  await scrollTop(card(labelC));
  await expect(card(labelC).getByText("對方說已完成", { exact: true })).toBeVisible();
  await shot(page, "narrow-work-claimed");
  await scrollTop(card(labelB));
  await shot(page, "narrow-work-consent");
  await scrollTop(card(labelA));
  await shot(page, "narrow-work-working");
  await scrollTop(card(labelD));
  await shot(page, "narrow-work-verified");
  // 這四個 session 留著：後面「現在」／通知中心／緊急停止的截圖會反映它們的真實狀態；
  // 緊急停止那一支測試最後統一關閉（子程序是 fixture，stdin 收 EOF 就會結束）。
});

test("擷取：連接與權限五區＋角色 adapter 詳細資料（模擬 adapter，fixture，真 WebSocket 連線）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const nodeMajor = Number(process.versions.node.split(".")[0]);
  const manifest = JSON.parse(
    fs.readFileSync(
      path.join(REPO_ROOT, "examples/character-adapters/text-adapter.manifest.json"),
      "utf8"
    )
  ) as Record<string, unknown>;
  const registered = (await api(request, "POST", "/v1/character/adapters", {
    displayName: "文字 adapter（模擬 adapter，fixture）",
    manifest,
  })) as { adapterId: string; token: string };
  expect(registered.adapterId).toMatch(/^adp-/);
  let child: ChildProcess | null = null;
  const connected = async () => {
    const view = (await api(request, "GET", "/v1/character/adapters")) as {
      adapters: { adapterId: string; connected: boolean }[];
    };
    return view.adapters.find((a) => a.adapterId === registered.adapterId)?.connected === true;
  };
  try {
    if (nodeMajor >= 22) {
      // 參考 adapter 只用 Node 內建 WebSocket；用剛拿到的 adapter token（不是 human token）。
      child = spawn("node", [path.join(REPO_ROOT, "examples/character-adapters/text-adapter.mjs")], {
        cwd: REPO_ROOT,
        env: {
          ...process.env,
          INTERACT_AI_API: apiBase(),
          INTERACT_AI_CHARACTER_TOKEN: registered.token,
          CHARACTER_FIXTURE_QUIET: "1",
        },
        stdio: ["ignore", "ignore", "pipe"],
      });
      child.stderr?.on("data", () => {});
      const deadline = Date.now() + 20_000;
      while (!(await connected())) {
        if (Date.now() > deadline) throw new Error("模擬 adapter 20 秒內沒有接上 daemon");
        await new Promise((r) => setTimeout(r, 250));
      }
    }

    await page.setViewportSize(DESKTOP);
    await openApp(page);
    await navigateTo(page, PAGES[3], false);
    const devices = page.getByTestId("connect-area-devices");
    await expect(devices.getByRole("heading", { name: "角色" })).toBeVisible();
    const row = devices.locator("[data-testid^='adapter-row-']", { hasText: "文字 adapter" });
    await expect(row.first()).toBeVisible({ timeout: 15_000 });
    await expect(row.first().getByText(child ? "已連線" : "未連線")).toBeVisible({ timeout: 15_000 });
    // 有可執行程式／外部：adapter 的安全旗標必須看得到。
    await expect(row.first().getByText(/外部|第三方/).first()).toBeVisible();
    await scrollTop(devices.getByRole("heading", { name: "角色" }));
    await shot(page, "desktop-connect-adapters");

    // 全部能力與裝置 → 裝置與來源：獨立的「角色如何接上系統」區。
    await page.getByRole("tab", { name: "裝置與來源" }).click();
    const hub = page.getByRole("heading", { name: "角色如何接上系統" });
    await expect(hub).toBeVisible();
    await scrollTop(hub);
    await shot(page, "desktop-connect-adapters-hub");

    await page.setViewportSize(NARROW);
    await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
    await scrollTop(devices.getByRole("heading", { name: "角色" }));
    await expect(row.first()).toBeVisible();
    await shot(page, "narrow-connect-adapters");
  } finally {
    if (child) {
      child.kill("SIGTERM");
      const deadline = Date.now() + 10_000;
      while ((await connected()) && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 250));
      }
    }
  }
});

test("擷取：每個一級頁的真實待確認狀態＋通知中心（桌面／390px）", async ({ page, request }) => {
  test.setTimeout(120_000);
  await api(request, "POST", "/v1/knowledge/nodes", {
    title: "畫面驗收候選",
    content: "這筆候選由實際 Runtime 保存，只用於驗證統一待辦入口。",
    domains: ["acceptance-evidence"],
    asAgent: "evidence-reviewer",
    evidence: [{ url: "https://example.invalid/acceptance", segment: "local-test" }],
  });

  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await expect(page.getByRole("button", { name: /通知中心，[1-9][0-9]* 項待決定/ })).toBeVisible({
    timeout: 15_000,
  });
  // 右上 Inbox：待決定入口的實際畫面。
  await page.getByRole("button", { name: /通知中心/ }).click();
  await expect(page.getByRole("dialog", { name: "通知中心" })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "通知中心" }).getByText("待你決定")).toBeVisible();
  await page.screenshot({ path: path.join(OUT, "desktop-inbox.png") });
  // 通知中心必須真的能用鍵盤關掉（以前這裡是 .catch 遷就實作缺陷）。
  await expect(page.getByRole("dialog", { name: "通知中心" })).toHaveAttribute(
    "aria-modal",
    "true"
  );
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "通知中心" })).toBeHidden();
  // 焦點回到觸發按鈕，鍵盤使用者不會被丟回頁首。
  await expect(page.getByRole("button", { name: /通知中心/ })).toBeFocused();

  // 390px 的通知中心。
  await page.setViewportSize(NARROW);
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
  await page.getByRole("button", { name: /通知中心/ }).click();
  await expect(page.getByRole("dialog", { name: "通知中心" })).toBeVisible();
  await page.screenshot({ path: path.join(OUT, "narrow-inbox.png") });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "通知中心" })).toBeHidden();

  await capturePageMatrix(page, "waiting");
});

test("擷取：每個一級頁的實際載入狀態", async ({ page }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });
  await page.setViewportSize(DESKTOP);
  await openApp(page);

  for (const [viewport, size] of [
    ["desktop", DESKTOP],
    ["narrow", NARROW],
  ] as const) {
    await page.setViewportSize(size);
    for (const target of PAGES) {
      const away = target.id === "home" ? PAGES[PAGES.length - 1] : PAGES[0];
      await navigateTo(page, away, viewport === "narrow");
      const delayed = async (route: import("@playwright/test").Route) => {
        if (route.request().url().includes("/events/stream")) return route.continue();
        await new Promise((resolve) => setTimeout(resolve, 700));
        // React can unmount a page while one of its delayed requests is still
        // pending. In that case Chromium has already cancelled the route; it
        // is not an acceptance failure and must not fail the screenshot run.
        try {
          await route.continue();
        } catch (error) {
          if (!String(error).includes("already handled")) throw error;
        }
      };
      await page.route("**/v1/**", delayed);
      await navigateTo(page, target, viewport === "narrow");
      await page.waitForTimeout(80);
      await page.screenshot({
        path: path.join(OUT, `${viewport}-loading-${target.id}.png`),
        fullPage: false,
      });
      await page.waitForTimeout(750);
      await page.unroute("**/v1/**", delayed);
    }
  }
});

test("擷取：每個一級頁的傳輸錯誤／未知狀態", async ({ page }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });
  for (const [viewport, size] of [
    ["desktop", DESKTOP],
    ["narrow", NARROW],
  ] as const) {
    await page.setViewportSize(size);
    if (viewport === "desktop") await openApp(page);
    else await page.goto(appUrl());
    await expect(
      viewport === "narrow"
        ? page.getByRole("navigation", { name: "主要導覽（窄視窗）" })
        : page.getByRole("navigation", { name: "主要導覽" })
    ).toBeVisible({ timeout: 15_000 });

    // Inject a real transport failure (no fake response payload), run an
    // actual permission-aware command, and preserve the resulting honest
    // global error banner while visiting every page.
    await page.route("**/v1/sensors/stop", (route) => route.abort("connectionrefused"));
    await page.keyboard.press("ControlOrMeta+k");
    const search = page.getByRole("dialog", { name: "全域搜尋" });
    await search.getByPlaceholder(/搜尋設定/).fill("停止所有感測");
    await search.getByRole("option", { name: /停止所有感測/ }).first().click();
    await expect(page.getByRole("alert")).toContainText("停止所有感測", { timeout: 10_000 });
    await page.unroute("**/v1/sensors/stop");

    for (const target of PAGES) {
      await navigateTo(page, target, viewport === "narrow");
      await expect(page.getByRole("alert")).toBeVisible();
      await page.screenshot({
        path: path.join(OUT, `${viewport}-error-unknown-${target.id}.png`),
        fullPage: false,
      });
    }
  }
});

test("擷取：外觀主題（淺色／深色；Runtime UI 偏好 appearance，presentation only）", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  try {
    for (const appearance of ["light", "dark"] as const) {
      await api(request, "PATCH", "/v1/ui/preferences", { appearance });
      await page.setViewportSize(DESKTOP);
      await openApp(page);
      await expect(page.locator("html")).toHaveAttribute("data-theme", appearance, {
        timeout: 10_000,
      });
      await expect(page.getByTestId("now-character")).toBeVisible();
      await shot(page, `desktop-theme-${appearance}-home`);
      await page.setViewportSize(NARROW);
      await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible();
      await shot(page, `narrow-theme-${appearance}-home`);
    }
  } finally {
    await api(request, "PATCH", "/v1/ui/preferences", { appearance: "system" });
  }
});

test("擷取：首次成功體驗（設定 → 重新執行首次設定 → 精靈套用 → FirstSuccess；桌面＋390px）", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  // app.spec 已把 firstSuccessSeen 記到 host；為了再看到這一屏，先誠實地把旗標放回 false。
  await api(request, "PATCH", "/v1/ui/preferences", { firstSuccessSeen: false });
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await navigateTo(page, PAGES[4], false);
  await page.getByRole("tablist", { name: "更多分類" }).getByRole("tab", { name: "外觀與語言" }).click();
  await page.getByRole("button", { name: "重新執行首次設定" }).click();
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  await expect(wizard).toBeVisible({ timeout: 15_000 });
  // 一步一步走（連按兩下會用到同一個 closure 的 step，第二下等於沒按）。
  await expect(wizard.getByRole("heading", { name: "選擇角色與陪伴方式" })).toBeVisible();
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: /幫忙工作嗎？/ })).toBeVisible();
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: "確認安全與權限預設" })).toBeVisible();
  await wizard.getByRole("button", { name: "完成設定" }).click();
  // 重新執行時多半「沒有任何變更」，但仍然要人按下「套用」才記錄完成。
  await confirmApply(page);
  const firstSuccess = page.getByRole("dialog", { name: "首次成功體驗" });
  await expect(firstSuccess).toBeVisible({ timeout: 15_000 });
  await expect(
    firstSuccess.getByRole("heading", { name: /準備好了。要不要先試一次？/ })
  ).toBeVisible();
  await expect(firstSuccess.getByText(/安全訊息永遠是固定文字/)).toBeVisible();
  for (const option of ["提醒我休息", "交代一件小工作", "先在桌面陪我", "更換角色"]) {
    await expect(firstSuccess.getByText(option, { exact: true })).toBeVisible();
  }
  await shot(page, "desktop-first-success");
  await page.setViewportSize(NARROW);
  await expect(firstSuccess).toBeVisible();
  await shot(page, "narrow-first-success");
  // 「先在桌面陪我」在瀏覽器檢視必須誠實：沒有桌面角色，只有文字。
  await firstSuccess.getByText("先在桌面陪我", { exact: true }).click();
  await expect(firstSuccess.getByText(/桌面角色需要桌面版控制中心/)).toBeVisible();
  await shot(page, "narrow-first-success-browser-honest");
  await firstSuccess.getByRole("button", { name: "完成", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "主要導覽（窄視窗）" })).toBeVisible({
    timeout: 15_000,
  });
  const prefs = (await api(request, "GET", "/v1/ui/preferences")) as { firstSuccessSeen?: boolean };
  expect(prefs.firstSuccessSeen).toBe(true);
});

test("擷取：Runtime 離線（誠實錯誤畫面）", async ({ page }) => {
  await page.setViewportSize(DESKTOP);
  await page.goto(`/?api=${encodeURIComponent("http://127.0.0.1:1")}&token=x`);
  await expect(page.getByText("系統無法啟動")).toBeVisible({ timeout: 20_000 });
  await page.screenshot({ path: path.join(OUT, "desktop-offline.png") });
  await page.setViewportSize(NARROW);
  await page.screenshot({ path: path.join(OUT, "narrow-offline.png") });
});

// 最後：緊急停止（真實觸發；會撤銷同意、停掉進行中的工作）→ 全頁矩陣 → 安全解除。
test("擷取：緊急停止狀態（放最後；真實觸發 → 擷取 → 安全流程解除 → 收尾關閉 fixture session）", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize(DESKTOP);
  await openApp(page);
  await page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true }).click();
  await page.getByRole("button", { name: "立即停止一切？" }).click();
  await expect(page.getByText("緊急停止已啟動").first()).toBeVisible();
  const status = (await api(request, "GET", "/v1/status")) as { emergencyStop?: boolean };
  expect(status.emergencyStop).toBe(true);
  await capturePageMatrix(page, "emergency");
  await page.setViewportSize(DESKTOP);
  // 解除（讓 suite 保持乾淨收尾）。
  await page.locator(".topbar").getByRole("button", { name: /緊急停止中 — 前往解除/ }).click();
  await page.getByRole("button", { name: /開始安全解除流程/ }).click();
  const dialog = page.getByRole("dialog", { name: "解除緊急停止" });
  await dialog.getByRole("button", { name: "我了解，解除緊急停止" }).click();
  await dialog.getByRole("button", { name: "確定解除？" }).click();
  await expect(
    page.locator(".topbar").getByRole("button", { name: "緊急停止", exact: true })
  ).toBeVisible({ timeout: 10_000 });
  // 收尾：關掉 evidence 建立的 fixture session（估計 estop 已把它們停了；關閉是冪等的收進歷史）。
  const sessions = (await api(request, "GET", "/v1/agent-sessions")) as { sessionId: string; state: string; label?: string | null }[];
  for (const s of sessions) {
    if (!String(s.label ?? "").startsWith("證據")) continue;
    if (["closed", "cancelled", "expired"].includes(s.state)) continue;
    await request
      .post(`${apiBase()}/v1/agent-sessions/${s.sessionId}/close`, {
        headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
        data: { reason: "evidence-cleanup" },
      })
      .catch(() => {});
  }
});
