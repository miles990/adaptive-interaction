// Boots a REAL interact-ai daemon in an isolated temp home for the E2E suite.
// The tests exercise the actual runtime + policy governor over HTTP — no mocks.

import { spawn, execSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
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
  if (!existsSync(bin)) {
    execSync("cargo build -p interaction-cli", { cwd: repoRoot, stdio: "inherit" });
  }

  const home = mkdtempSync(join(tmpdir(), "interaction-e2e-"));
  mkdirSync(join(home, "config"), { recursive: true });
  writeFileSync(
    join(home, "config", "interaction.yaml"),
    `apiHost: 127.0.0.1\napiPort: ${PORT}\n`
  );

  const child = spawn(bin, ["serve"], {
    env: { ...process.env, INTERACT_AI_HOME: home },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  child.stderr?.on("data", () => {});
  child.stdout?.on("data", () => {});

  await waitReady(`http://127.0.0.1:${PORT}/ready`, 30_000);
  const token = readFileSync(join(home, "state", "api-token"), "utf8").trim();

  writeFileSync(STATE_FILE, JSON.stringify({ pid: child.pid, home, port: PORT, token }));
  process.env.E2E_API = `http://127.0.0.1:${PORT}`;
  process.env.E2E_TOKEN = token;
}
