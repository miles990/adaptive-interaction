// Boots a REAL interact-ai daemon in an isolated temp home for the E2E suite.
// The tests exercise the actual runtime + policy governor over HTTP — no mocks.

import { spawn, execSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const PORT = 18790;
const STATE_FILE = join(tmpdir(), "interaction-e2e-state.json");
const EXTRA_DAEMONS_FILE = join(tmpdir(), "interaction-e2e-extra-daemons.json");

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

export default async function globalSetup() {
  // ESM context: derive the repo root from cwd (playwright runs in the app dir).
  const repoRoot = resolve(process.cwd(), "../..");
  const bin = join(repoRoot, "target/debug/interact-ai");
  // Acceptance must exercise the current worktree, never a stale binary left
  // by an earlier run. Cargo's incremental build keeps this inexpensive when
  // nothing changed.
  execSync("cargo build -p interaction-cli", { cwd: repoRoot, stdio: "inherit" });
  // 【模擬 iPhone（fixture）】：程序外假手機，讓 iPhone／感測相關的驗收不需要真機
  // （也不需要在區網上出現）。它只是 mobile_loop.rs 那個程序內模擬手機的可執行版本。
  execSync("cargo build -p interaction-runtime --example fake_iphone", {
    cwd: repoRoot,
    stdio: "inherit",
  });
  const fakeIphoneBin = join(repoRoot, "target/debug/examples/fake_iphone");

  // 上一輪若沒收乾淨，這裡先清掉紀錄（pid 由 teardown 負責殺）。
  rmSync(EXTRA_DAEMONS_FILE, { force: true });

  const home = mkdtempSync(join(tmpdir(), "interaction-e2e-"));
  mkdirSync(join(home, "config"), { recursive: true });
  writeFileSync(
    join(home, "config", "interaction.yaml"),
    `apiHost: 127.0.0.1\napiPort: ${PORT}\n`
  );

  // Agent 幫手：預設把 Codex／Claude Code 指向 Rust 測試用的 fake fixture 子程序
  // （crates/interaction-runtime/tests/fixtures），讓 evidence.spec 能建立真實的
  // agent session 並走完誠實階梯（處理中／等你允許／對方說已完成／人工驗證）。
  // 這是 fixture（模擬 agent），不是真的 Codex／Claude Code；截圖與文件一律標示。
  // E2E_REAL_AGENTS=1 改用 PATH 上的真 CLI（session 證據那一支測試會被 skip）。
  const fixtures = join(repoRoot, "crates/interaction-runtime/tests/fixtures");
  const fakeAgents = process.env.E2E_REAL_AGENTS !== "1";
  const agentEnv = fakeAgents
    ? {
        INTERACT_AI_CLAUDE_BIN: process.env.INTERACT_AI_CLAUDE_BIN ?? join(fixtures, "fake_claude.sh"),
        INTERACT_AI_CODEX_BIN: process.env.INTERACT_AI_CODEX_BIN ?? join(fixtures, "fake_codex.sh"),
      }
    : {};

  const child = spawn(bin, ["serve"], {
    env: {
      ...process.env,
      ...agentEnv,
      // 模擬不得有外部副作用：iPhone 伺服器只綁 127.0.0.1，也不對區網廣播
      // Bonjour（GET /v1/mobile/status 的 bonjour.advertised 會誠實回 false）。
      INTERACT_AI_MOBILE_ADVERTISE: "0",
      INTERACT_AI_HOME: home,
    },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  child.stderr?.on("data", () => {});
  child.stdout?.on("data", () => {});

  await waitReady(`http://127.0.0.1:${PORT}/ready`, 30_000);
  const token = readFileSync(join(home, "state", "api-token"), "utf8").trim();

  writeFileSync(
    STATE_FILE,
    JSON.stringify({ pid: child.pid, home, port: PORT, token, fakeAgents, fakeIphoneBin })
  );
  process.env.E2E_API = `http://127.0.0.1:${PORT}`;
  process.env.E2E_TOKEN = token;
  process.env.E2E_FAKE_AGENTS = fakeAgents ? "1" : "0";
  process.env.E2E_REPO_ROOT = repoRoot;
  process.env.E2E_FAKE_IPHONE_BIN = fakeIphoneBin;
}
