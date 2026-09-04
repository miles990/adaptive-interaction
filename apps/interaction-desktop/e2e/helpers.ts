// E2E 共用工具：所有 spec 共享同一份「怎麼開 App、怎麼打真 daemon、怎麼起
// fixture」的定義，避免每支 spec 各自抄一份選擇器與流程（抄久了就會各自過時）。
//
// 誠實邊界（每一支用到這裡的 spec 都適用）：
// - `api()` 打的是 global-setup 起的**真** daemon，非 2xx 直接讓測試失敗。
// - `spawnDaemon()` 起的是同一支 `interact-ai` 執行檔，只是換一個隔離的家與埠號。
// - `spawnFakeIphone()` 起的是【模擬 iPhone（fixture）】：程序外假手機，
//   不是 iPhone 真機。用到它的斷言與截圖一律要標示 fixture。

import { expect, Page, APIRequestContext } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

export const DESKTOP = { width: 1200, height: 800 } as const;
export const NARROW = { width: 390, height: 844 } as const;

/** 額外 daemon 的 pid 紀錄（global-teardown 的保險絲：spec 崩了也不留孤兒程序）。 */
export const EXTRA_DAEMONS_FILE = join(tmpdir(), "interaction-e2e-extra-daemons.json");

export function apiBase(): string {
  return process.env.E2E_API!;
}

export function repoRoot(): string {
  return process.env.E2E_REPO_ROOT ?? resolve(process.cwd(), "../..");
}

export function appUrl(api: string = apiBase(), token: string = process.env.E2E_TOKEN!): string {
  return `/?api=${encodeURIComponent(api)}&token=${encodeURIComponent(token)}`;
}

/** 以人類 token 打真 daemon 的 /v1 路由；非 2xx 直接讓測試失敗（不吞錯）。 */
export async function api(
  request: APIRequestContext,
  method: "GET" | "POST" | "PATCH" | "DELETE",
  route: string,
  data?: unknown,
  options?: { base?: string; token?: string }
): Promise<unknown> {
  const base = options?.base ?? apiBase();
  const token = options?.token ?? process.env.E2E_TOKEN!;
  const res = await request.fetch(`${base}${route}`, {
    method,
    headers: { Authorization: `Bearer ${token}` },
    data,
  });
  const text = await res.text();
  expect(res.ok(), `${method} ${route} → HTTP ${res.status()} ${text.slice(0, 300)}`).toBeTruthy();
  return text ? (JSON.parse(text) as unknown) : null;
}

// 角色頁的 label 是目前角色的名字：瀏覽器 e2e 沒有角色視窗，名字來自 bundled 索引的
// default（shu-maid → 小樞）。若索引載入失敗，導覽會顯示中立的「角色」。
export const PAGES: { id: string; label: string; marker: string | RegExp }[] = [
  { id: "home", label: "現在", marker: "快速操作" },
  { id: "companion", label: "小樞", marker: /36 表情預覽/ },
  { id: "work", label: "工作", marker: "本機 AI Agent" },
  { id: "connect", label: "連接與權限", marker: "系統時間" },
  { id: "more", label: "更多", marker: "關於我的記憶" },
];

/**
 * 打開控制中心並走完（或跳過）首次設定。
 *
 * 精靈流程：3 步 → 完成設定 → **套用前確認**對話框（按「套用」才真的動手）→
 * 首次成功體驗（只在 host 尚未記錄看過時出現）。
 */
export async function openApp(page: Page, url: string = appUrl()) {
  await page.goto(url);
  await settleShell(page, page.getByRole("navigation", { name: "主要導覽" }));
}

/**
 * 等控制中心真的可用：精靈是在 `GET /v1/status` 回來之後才決定要不要出現的
 * （剛開機的 daemon 上會晚幾百毫秒），所以這裡用有界輪詢，看到精靈就走完它，
 * 看到導覽就結束——不用「賭一次」的 race。
 */
async function settleShell(page: Page, nav: import("@playwright/test").Locator) {
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  const deadline = Date.now() + 60_000;
  // 導覽會先出現、之後才被精靈取代（`onboardingCompleted` 是另一個 status 回應
  // 才知道的），所以「看到導覽」要連續穩定一小段時間才算數，而且每一輪都先看精靈。
  const stableNeeded = 4;
  let stable = 0;
  for (;;) {
    if (await wizard.isVisible().catch(() => false)) {
      await finishWizard(page);
      stable = 0;
      continue;
    }
    if (await nav.isVisible().catch(() => false)) {
      stable += 1;
      if (stable >= stableNeeded) break;
    } else {
      stable = 0;
    }
    if (Date.now() > deadline) break;
    await page.waitForTimeout(150);
  }
  await expect(nav).toBeVisible({ timeout: 20_000 });
}

/** 從第一步走到套用完成（含套用前確認與可略過的首次成功體驗）。 */
export async function finishWizard(page: Page) {
  const wizard = page.getByRole("dialog", { name: "首次設定" });
  const desktopNav = page.getByRole("navigation", { name: "主要導覽" });
  // 一步一步走：先確認這一步真的到了才按「下一步」。連按兩下會用到同一個
  // closure 的 step，第二下等於沒按（剛開機的 daemon 上重現得到）。
  await expect(wizard.getByRole("heading", { name: "選擇角色與陪伴方式" })).toBeVisible({
    timeout: 20_000,
  });
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: /幫忙工作嗎？/ })).toBeVisible({
    timeout: 20_000,
  });
  await wizard.getByRole("button", { name: "下一步" }).click();
  await expect(wizard.getByRole("heading", { name: "確認安全與權限預設" })).toBeVisible({
    timeout: 20_000,
  });
  await wizard.getByRole("button", { name: "完成設定" }).click();
  await confirmApply(page);
  const firstSuccess = page.getByRole("dialog", { name: "首次成功體驗" });
  await Promise.race([
    firstSuccess.waitFor({ state: "visible", timeout: 20_000 }),
    desktopNav.waitFor({ state: "visible", timeout: 20_000 }),
  ]);
  if (await firstSuccess.isVisible().catch(() => false)) {
    await firstSuccess.getByRole("button", { name: "完成", exact: true }).click();
  }
}

/** 「完成設定」之後的套用前確認：按「套用」之前後端什麼都沒改。 */
export async function confirmApply(page: Page) {
  const confirm = page.getByRole("dialog", { name: "套用前確認" });
  await expect(confirm).toBeVisible({ timeout: 20_000 });
  await confirm.getByRole("button", { name: "套用", exact: true }).click();
  await expect(confirm).toBeHidden({ timeout: 20_000 });
}

export async function openNarrow(page: Page, url: string = appUrl()) {
  await page.setViewportSize(NARROW);
  await page.goto(url);
  await settleShell(page, page.getByRole("navigation", { name: "主要導覽（窄視窗）" }));
}

export async function clickNav(page: Page, label: string, narrow: boolean) {
  if (!narrow) {
    await page
      .getByRole("navigation", { name: "主要導覽" })
      .getByText(label, { exact: true })
      .click();
    return;
  }
  const bottomNav = page.getByRole("navigation", { name: "主要導覽（窄視窗）" });
  if (label === "更多") {
    // 窄視窗沒有獨立的「更多」頁——以更多選單抵達其中一個分頁。
    await bottomNav.getByRole("button", { name: "更多" }).click();
    await page
      .getByRole("dialog", { name: "更多功能" })
      .getByText("記憶與資料", { exact: true })
      .click();
    return;
  }
  await bottomNav.getByText(label, { exact: true }).click();
}

export async function navigateTo(page: Page, target: (typeof PAGES)[number], narrow: boolean) {
  await clickNav(page, target.label, narrow);
  await expect(page.locator(".topbar-title")).toHaveText(target.label, { timeout: 10_000 });
}

/** 輪詢 GET /v1/agent-sessions/{id} 直到 state 落在 states（回傳 record）。 */
export async function waitSessionState(
  request: APIRequestContext,
  sessionId: string,
  states: string[],
  timeoutMs = 30_000,
  options?: { base?: string; token?: string }
): Promise<Record<string, unknown>> {
  const deadline = Date.now() + timeoutMs;
  let last = "";
  for (;;) {
    const record = (await api(
      request,
      "GET",
      `/v1/agent-sessions/${sessionId}`,
      undefined,
      options
    )) as Record<string, unknown>;
    last = String(record.state);
    if (states.includes(last)) return record;
    if (Date.now() > deadline) {
      throw new Error(`session ${sessionId} 停在 ${last}，等不到 ${states.join("/")}`);
    }
    await new Promise((r) => setTimeout(r, 300));
  }
}

/** 每個 fixture session 一個隔離工作目錄：fixture 只在 cwd 讀 fake-mode，不碰 repo。 */
export function makeWorkRoot(prefix = "interaction-e2e-work-"): string {
  return mkdtempSync(join(tmpdir(), prefix));
}

export function makeWorkdir(root: string, name: string, mode?: string): string {
  const dir = join(root, name);
  mkdirSync(dir, { recursive: true });
  if (mode) writeFileSync(join(dir, "fake-mode"), mode);
  return dir;
}

/** 用真 daemon 的 API 建一個 fixture 工作階段（唯讀），並送出一句任務。 */
export async function createFixtureSession(
  request: APIRequestContext,
  input: {
    agentId: "codex" | "claude-code";
    label: string;
    workdir: string;
    allowWrite?: boolean;
    /** `null`＝不送任務（有些 fixture 一啟動就結束，mailbox 會直接關閉）。 */
    task?: string | null;
    ttlMinutes?: number;
  }
): Promise<string> {
  const record = (await api(request, "POST", "/v1/agent-sessions", {
    agentId: input.agentId,
    label: input.label,
    ttlMinutes: input.ttlMinutes ?? 30,
    workdir: input.workdir,
    dataScope: [`workspace:${input.workdir}`],
    toolScope: [],
    consentScope: [],
    allowWrite: input.allowWrite === true,
  })) as { sessionId: string };
  if (input.task !== null) {
    await api(request, "POST", `/v1/agent-sessions/${record.sessionId}/messages`, {
      kind: "task",
      body: { task: input.task ?? "E2E fixture 的一句話任務（模擬 agent）。" },
    });
  }
  return record.sessionId;
}

/** 收尾：把還開著的 fixture session 關掉（冪等；失敗不讓測試變紅）。
 *  用全域 fetch 而不是 `request` fixture，這樣 afterAll 裡也叫得動。 */
export async function closeSessions(sessionIds: string[]) {
  for (const id of sessionIds) {
    await fetch(`${apiBase()}/v1/agent-sessions/${id}/close`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${process.env.E2E_TOKEN!}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ reason: "e2e-cleanup" }),
    }).catch(() => undefined);
  }
}

/** 收尾：撤銷所有已配對的模擬手機（同樣用 fetch，afterAll 可用）。 */
export async function revokePairedPhones() {
  try {
    const res = await fetch(`${apiBase()}/v1/mobile/status`, {
      headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
    });
    if (!res.ok) return;
    const status = (await res.json()) as { devices?: { deviceId?: string }[] };
    for (const device of status.devices ?? []) {
      if (!device.deviceId) continue;
      await fetch(`${apiBase()}/v1/mobile/devices/${device.deviceId}`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${process.env.E2E_TOKEN!}` },
      }).catch(() => undefined);
    }
  } catch {
    /* 收尾失敗不讓測試變紅；下一支 spec 會自己確認狀態 */
  }
}

// ---------------------------------------------------------------------------
// 額外的真 daemon（例如「Agent 未安裝」需要另一個 discovery 結果）
// ---------------------------------------------------------------------------

export interface SpawnedDaemon {
  api: string;
  token: string;
  home: string;
  pid: number;
  kill: () => void;
}

async function waitReady(url: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error(`daemon not ready at ${url}`);
    await new Promise((r) => setTimeout(r, 250));
  }
}

/**
 * 起另一支真 daemon（同一個執行檔、隔離的家與埠號）。
 * 用途：需要不同啟動環境的驗收，例如 `INTERACT_AI_CODEX_BIN` 指向不存在的路徑
 * （agent discovery 只在 daemon 啟動／重新偵測時讀環境變數）。
 */
export async function spawnDaemon(options: {
  port: number;
  env?: Record<string, string>;
  label?: string;
}): Promise<SpawnedDaemon> {
  const bin = join(repoRoot(), "target/debug/interact-ai");
  const home = mkdtempSync(join(tmpdir(), `interaction-e2e-${options.label ?? "extra"}-`));
  mkdirSync(join(home, "config"), { recursive: true });
  writeFileSync(
    join(home, "config", "interaction.yaml"),
    `apiHost: 127.0.0.1\napiPort: ${options.port}\n`
  );
  const child: ChildProcess = spawn(bin, ["serve"], {
    env: {
      ...process.env,
      // 模擬不得有區網副作用：這支 daemon 也不廣播 Bonjour、只綁 127.0.0.1。
      INTERACT_AI_MOBILE_ADVERTISE: "0",
      ...(options.env ?? {}),
      INTERACT_AI_HOME: home,
    },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  child.stderr?.on("data", () => {});
  child.stdout?.on("data", () => {});
  try {
    appendFileSync(EXTRA_DAEMONS_FILE, `${JSON.stringify({ pid: child.pid, home })}\n`);
  } catch {
    /* teardown 的保險絲寫不進去不影響測試本身 */
  }
  await waitReady(`http://127.0.0.1:${options.port}/ready`, 60_000);
  const token = readFileSync(join(home, "state", "api-token"), "utf8").trim();
  // `/ready` 只代表 HTTP 起來了。控制中心是等 `GET /v1/status` 才決定要不要出精靈，
  // 剛開機的 daemon（agent 探測、能力健康檢查）可能還要好幾秒——先等它真的會回答，
  // 免得畫面卡在「正在啟動系統…」。
  const statusDeadline = Date.now() + 60_000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${options.port}/v1/status`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.ok) break;
    } catch {
      /* not answering yet */
    }
    if (Date.now() > statusDeadline) {
      throw new Error(`daemon on ${options.port} never answered /v1/status`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  return {
    api: `http://127.0.0.1:${options.port}`,
    token,
    home,
    pid: child.pid ?? -1,
    kill: () => {
      try {
        child.kill("SIGTERM");
      } catch {
        /* already gone */
      }
    },
  };
}

// ---------------------------------------------------------------------------
// 【模擬 iPhone（fixture）】——crates/interaction-runtime/examples/fake_iphone.rs
// ---------------------------------------------------------------------------

/** 每一段跟模擬手機有關的文案／截圖都要帶這個標籤（CLAUDE.md：模擬不得冒充真機）。 */
export const FAKE_IPHONE_LABEL = "模擬 iPhone（fixture）";

export interface FakeIphone {
  deviceId: string;
  deviceToken: string;
  /** 送一則 stdin 指令（status／disconnect／reconnect／ack-stop-all／quit）。 */
  send: (op: Record<string, unknown>) => void;
  /** 已收到的事件（JSON Lines）。 */
  events: Record<string, unknown>[];
  /** 等一個事件出現（從第 `from` 筆之後開始找；回傳它的索引）。 */
  waitForEvent: (
    predicate: (event: Record<string, unknown>) => boolean,
    timeoutMs?: number,
    from?: number
  ) => Promise<number>;
  kill: () => void;
}

/**
 * 起一台模擬 iPhone 並完成配對。
 * `code` 從 UI 的配對通知讀（人類流程），`port`／`fingerprint` 從
 * GET /v1/mobile/status 讀（UI 在一般模式只顯示指紋前 6 碼）。
 */
export async function spawnFakeIphone(options: {
  port: number;
  fingerprint: string;
  code: string;
  name?: string;
  model?: string;
  /** true＝收到 stop-all 立刻回 ack（測「確認停止」那條路徑）。 */
  autoAckStopAll?: boolean;
}): Promise<FakeIphone> {
  const bin =
    process.env.E2E_FAKE_IPHONE_BIN ?? join(repoRoot(), "target/debug/examples/fake_iphone");
  const args = [
    "--port",
    String(options.port),
    "--fingerprint",
    options.fingerprint,
    "--code",
    options.code,
    "--name",
    options.name ?? FAKE_IPHONE_LABEL,
    "--model",
    options.model ?? "iPhone12,1",
  ];
  if (options.autoAckStopAll) args.push("--auto-ack-stop-all");
  const child = spawn(bin, args, { stdio: ["pipe", "pipe", "pipe"] });
  const events: Record<string, unknown>[] = [];
  let identity: { deviceId: string; deviceToken: string } | null = null;
  let stderr = "";
  child.stderr?.on("data", (chunk: unknown) => {
    stderr += String(chunk);
  });
  let buffer = "";
  child.stdout?.on("data", (chunk: unknown) => {
    buffer += String(chunk);
    for (;;) {
      const nl = buffer.indexOf("\n");
      if (nl < 0) break;
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      if (!line) continue;
      let value: Record<string, unknown>;
      try {
        value = JSON.parse(line) as Record<string, unknown>;
      } catch {
        continue;
      }
      if (identity === null && typeof value.deviceId === "string") {
        identity = {
          deviceId: String(value.deviceId),
          deviceToken: String(value.deviceToken ?? ""),
        };
        continue;
      }
      events.push(value);
    }
  });

  const deadline = Date.now() + 30_000;
  while (identity === null) {
    if (child.exitCode !== null) {
      throw new Error(`模擬 iPhone 啟動失敗（exit ${child.exitCode}）：${stderr.slice(0, 300)}`);
    }
    if (Date.now() > deadline) {
      child.kill("SIGTERM");
      throw new Error(`模擬 iPhone 30 秒內沒有完成配對：${stderr.slice(0, 300)}`);
    }
    await new Promise((r) => setTimeout(r, 100));
  }

  const phone: FakeIphone = {
    deviceId: (identity as { deviceId: string }).deviceId,
    deviceToken: (identity as { deviceToken: string }).deviceToken,
    events,
    send: (op) => {
      child.stdin?.write(`${JSON.stringify(op)}\n`);
    },
    waitForEvent: async (predicate, timeoutMs = 15_000, from = 0) => {
      const until = Date.now() + timeoutMs;
      for (;;) {
        for (let i = from; i < events.length; i += 1) {
          if (predicate(events[i])) return i;
        }
        if (Date.now() > until) {
          throw new Error(
            `模擬 iPhone 在 ${timeoutMs}ms 內沒有出現預期事件；已收到：${JSON.stringify(
              events.slice(from)
            )}`
          );
        }
        await new Promise((r) => setTimeout(r, 100));
      }
    },
    kill: () => {
      try {
        child.stdin?.write('{"op":"quit"}\n');
      } catch {
        /* already gone */
      }
      child.kill("SIGTERM");
    },
  };
  // 連上（配對之後 fixture 會自報一次 status）。
  await phone.waitForEvent((e) => e.event === "connected");
  return phone;
}

/** 直接用 API 開一段配對期（不經 UI；回應本身就帶 code／port／fingerprint）。 */
export async function beginPairing(
  request: APIRequestContext,
  options?: { base?: string; token?: string }
): Promise<{ code: string; port: number; fingerprint: string }> {
  const session = (await api(
    request,
    "POST",
    "/v1/mobile/pairing-session",
    undefined,
    options
  )) as {
    code: string;
    port: number;
    fingerprint: string;
  };
  return { code: session.code, port: session.port, fingerprint: session.fingerprint };
}

/** 開始一段配對期並回傳 UI 上看得到的配對碼＋連線用的 port／fingerprint。 */
export async function beginPairingFromUi(
  page: Page,
  request: APIRequestContext,
  options?: { base?: string; token?: string }
): Promise<{ code: string; port: number; fingerprint: string }> {
  await page.getByRole("button", { name: "開始配對（5 分鐘內有效）" }).click();
  const notice = page.locator(".notice-box", { hasText: "輸入配對碼" });
  await expect(notice).toBeVisible({ timeout: 15_000 });
  const text = await notice.innerText();
  const match = text.match(/輸入配對碼：\s*(\d{6})/);
  expect(match, `配對通知裡沒有 6 位數配對碼：${text}`).not.toBeNull();
  const status = (await api(request, "GET", "/v1/mobile/status", undefined, options)) as {
    port: number;
    fingerprint: string;
  };
  return { code: match![1], port: status.port, fingerprint: status.fingerprint };
}

/** 等 GET /v1/status 的 activeSensors 變成期望的樣子（有界輪詢）。 */
export async function waitActiveSensors(
  request: APIRequestContext,
  predicate: (sensors: Record<string, unknown>[]) => boolean,
  timeoutMs = 15_000
): Promise<Record<string, unknown>[]> {
  const deadline = Date.now() + timeoutMs;
  let last: Record<string, unknown>[] = [];
  for (;;) {
    const status = (await api(request, "GET", "/v1/status")) as Record<string, unknown>;
    last = (status.activeSensors as Record<string, unknown>[] | undefined) ?? [];
    if (predicate(last)) return last;
    if (Date.now() > deadline) {
      throw new Error(`activeSensors 等不到預期狀態，最後一次是 ${JSON.stringify(last)}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

// ---------------------------------------------------------------------------
// AIP Character Session（`docs/aip/transport-bindings.md` §6 的 fixture op）
//
// 這一段全部是【模擬 iPhone（fixture）】的包裝：程序外假手機送 AIP frame，
// 不是 iPhone 真機。用到它的斷言、截圖與文件一律標示 fixture。
// ---------------------------------------------------------------------------

/** fixture 收到的 AIP 信封（stdout 的 `{"event":"aip","envelope":…}`）。 */
export function aipEnvelopes(phone: FakeIphone, from = 0): Record<string, unknown>[] {
  return phone.events
    .slice(from)
    .filter((e) => e.event === "aip")
    .map((e) => (e.envelope ?? {}) as Record<string, unknown>);
}

/** 等 fixture 收到一則符合條件的 AIP 信封（回傳那一則）。 */
export async function waitAip(
  phone: FakeIphone,
  predicate: (envelope: Record<string, unknown>) => boolean,
  timeoutMs = 15_000,
  from = 0
): Promise<Record<string, unknown>> {
  const index = await phone.waitForEvent(
    (e) => e.event === "aip" && predicate((e.envelope ?? {}) as Record<string, unknown>),
    timeoutMs,
    from
  );
  return (phone.events[index].envelope ?? {}) as Record<string, unknown>;
}

/** state 信封的 payload（snapshot／patch 共用）。 */
export function aipPayload(envelope: Record<string, unknown>): Record<string, unknown> {
  const payload = envelope.payload;
  return payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
}

/**
 * 模擬 iPhone（fixture）送 `capability`（第一次＝加入 session，重連後＝重新協商）。
 * host 會回 negotiated capability ＋ 一則完整 snapshot；回傳那份 snapshot 的 payload。
 */
export async function aipCapability(
  phone: FakeIphone,
  timeoutMs = 20_000
): Promise<Record<string, unknown>> {
  const from = phone.events.length;
  phone.send({ op: "aip-capability" });
  await waitAip(phone, (e) => e.messageType === "capability", timeoutMs, from);
  const snapshot = await waitAip(
    phone,
    (e) => e.messageType === "state" && aipPayload(e).kind === "snapshot",
    timeoutMs,
    from
  );
  return aipPayload(snapshot);
}

/** 模擬 iPhone（fixture）摸一下角色；回傳 host 回的 `result` payload（不預設成功）。 */
export async function aipTouch(
  phone: FakeIphone,
  kind: "tap" | "longpress" | "pat" | "stroke" = "tap",
  timeoutMs = 20_000
): Promise<Record<string, unknown>> {
  const from = phone.events.length;
  phone.send({ op: "aip-touch", kind, expiresInMs: 5000 });
  const result = await waitAip(phone, (e) => e.messageType === "result", timeoutMs, from);
  return aipPayload(result);
}

/** 模擬 iPhone（fixture）重連之後的對齊；回傳 `response` 的 payload。 */
export async function aipResume(
  phone: FakeIphone,
  cursor: { lastRevision: number; lastSequence?: number; epoch?: number },
  timeoutMs = 20_000
): Promise<Record<string, unknown>> {
  const from = phone.events.length;
  phone.send({
    op: "aip-resume",
    lastRevision: cursor.lastRevision,
    lastSequence: cursor.lastSequence ?? 0,
    epoch: cursor.epoch ?? 0,
  });
  const response = await waitAip(phone, (e) => e.messageType === "response", timeoutMs, from);
  return aipPayload(response);
}

/** 後端真相：`GET /v1/character-session` 的 snapshot payload（revision／state 都在這裡）。 */
export async function characterSessionSnapshot(
  request: APIRequestContext,
  options?: { base?: string; token?: string }
): Promise<Record<string, unknown>> {
  const envelope = (await api(
    request,
    "GET",
    "/v1/character-session",
    undefined,
    options
  )) as Record<string, unknown>;
  return aipPayload(envelope);
}

/** 有界輪詢後端真相直到符合條件（回傳最後一次讀到的 snapshot payload）。 */
export async function waitCharacterSession(
  request: APIRequestContext,
  predicate: (payload: Record<string, unknown>) => boolean,
  timeoutMs = 20_000,
  options?: { base?: string; token?: string }
): Promise<Record<string, unknown>> {
  const deadline = Date.now() + timeoutMs;
  let last: Record<string, unknown> = {};
  for (;;) {
    last = await characterSessionSnapshot(request, options);
    if (predicate(last)) return last;
    if (Date.now() > deadline) {
      throw new Error(`角色同步狀態等不到預期值，最後一次是 ${JSON.stringify(last).slice(0, 400)}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** snapshot payload 裡的權威狀態（成員、最近互動、真相都在這裡）。 */
export function sessionState(payload: Record<string, unknown>): Record<string, unknown> {
  const state = payload.state;
  return state && typeof state === "object" ? (state as Record<string, unknown>) : {};
}

/** 這個裝置現在的 presence（不是成員就回 null——不猜）。 */
export function memberPresence(
  payload: Record<string, unknown>,
  deviceId: string
): string | null {
  const members = sessionState(payload).members;
  if (!Array.isArray(members)) return null;
  for (const entry of members) {
    const member = entry as Record<string, unknown>;
    const party = (member.party ?? {}) as Record<string, unknown>;
    if (party.kind === "device" && String(party.id) === deviceId) {
      return String(member.presence ?? "");
    }
  }
  return null;
}
