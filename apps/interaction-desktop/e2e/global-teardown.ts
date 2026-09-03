import { readFileSync, rmSync, existsSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const STATE_FILE = join(tmpdir(), "interaction-e2e-state.json");
/** spec 自己起的額外 daemon（helpers.spawnDaemon 逐行 append）。 */
const EXTRA_DAEMONS_FILE = join(tmpdir(), "interaction-e2e-extra-daemons.json");

function killExtraDaemons() {
  if (!existsSync(EXTRA_DAEMONS_FILE)) return;
  try {
    const lines = readFileSync(EXTRA_DAEMONS_FILE, "utf8").split("\n");
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      let record: { pid?: number; home?: string };
      try {
        record = JSON.parse(trimmed) as { pid?: number; home?: string };
      } catch {
        continue;
      }
      if (record.pid) {
        try {
          process.kill(record.pid, "SIGTERM");
        } catch {
          /* already gone */
        }
      }
      if (record.home && String(record.home).includes("interaction-e2e-")) {
        rmSync(record.home, { recursive: true, force: true });
      }
    }
  } finally {
    rmSync(EXTRA_DAEMONS_FILE, { force: true });
  }
}

export default async function globalTeardown() {
  killExtraDaemons();
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
