// Boots a REAL interact-ai daemon in an isolated temp home for the E2E suite.
// The tests exercise the actual runtime + policy governor over HTTP — no mocks.

import { spawn, execSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const PORT = 18790;
const STATE_FILE = join(tmpdir(), "interaction-e2e-state.json");

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

  const home = mkdtempSync(join(tmpdir(), "interaction-e2e-"));
  mkdirSync(join(home, "config"), { recursive: true });
  writeFileSync(
    join(home, "config", "interaction.yaml"),
    `apiHost: 127.0.0.1\napiPort: ${PORT}\n`
  );

  // Agent 幫手：預設把 Codex／Claude Code 指向 Rust 測試用的 fake fixture 子程序
  // （crates/interaction-runtime/tests/fixtures），讓 evidence.spec 能建立真實的
  // agent session 並走完誠實階梯（處理中／等你同意／Agent 說已完成／人工驗證）。
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
    env: { ...process.env, ...agentEnv, INTERACT_AI_HOME: home },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  child.stderr?.on("data", () => {});
  child.stdout?.on("data", () => {});

  await waitReady(`http://127.0.0.1:${PORT}/ready`, 30_000);
  const token = readFileSync(join(home, "state", "api-token"), "utf8").trim();

  writeFileSync(
    STATE_FILE,
    JSON.stringify({ pid: child.pid, home, port: PORT, token, fakeAgents })
  );
  process.env.E2E_API = `http://127.0.0.1:${PORT}`;
  process.env.E2E_TOKEN = token;
  process.env.E2E_FAKE_AGENTS = fakeAgents ? "1" : "0";
  process.env.E2E_REPO_ROOT = repoRoot;
}
