import { readFileSync, rmSync, existsSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const STATE_FILE = join(tmpdir(), "interaction-e2e-state.json");

export default async function globalTeardown() {
  if (!existsSync(STATE_FILE)) return;
  try {
    const state = JSON.parse(readFileSync(STATE_FILE, "utf8"));
    if (state.pid) {
      try {
        process.kill(state.pid, "SIGTERM");
      } catch {
        /* already gone */
      }
    }
    // Give the daemon a moment for graceful shutdown, then clean the temp home.
    await new Promise((r) => setTimeout(r, 500));
    if (state.home && String(state.home).includes("interaction-e2e-")) {
      rmSync(state.home, { recursive: true, force: true });
    }
  } finally {
    rmSync(STATE_FILE, { force: true });
  }
}
