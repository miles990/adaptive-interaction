// Desktop-app-only bridge (supervisor, lifecycle, tray) — always Tauri IPC,
// even when the runtime traffic itself flows over HTTP to an external daemon.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { configureHttp, isTauri } from "./transport";

export interface SupervisorInfo {
  mode: "embedded" | "external" | "undecided";
  state:
    | "starting"
    | "embedded-owned"
    | "connected-to-external"
    | "ready"
    | "degraded"
    | "disconnected"
    | "stopping"
    | "stopped";
  apiBase: string;
  token?: string;
  detail?: string;
}

export interface DesktopPrefs {
  closeBehavior: "keep-running" | "hide-companion" | "quit" | null;
  askOnClose: boolean;
  launchAtLogin: boolean;
  showCompanionOnStart: boolean;
  openControlCenterOnStart: boolean;
  companionVisible: boolean;
  companionPack: string;
  companionPersona: string;
  companionExpressiveness: "quiet" | "natural" | "lively" | string;
  companionAlwaysOnTop: boolean;
  storyProgress: Record<string, boolean>;
  schemaVersion: number;
}

export const desktop = {
  supervisorInfo: () => invoke<SupervisorInfo>("supervisor_info"),
  prefsGet: () => invoke<DesktopPrefs>("desktop_prefs_get"),
  prefsPatch: (patch: Partial<DesktopPrefs>) =>
    invoke<DesktopPrefs>("desktop_prefs_patch", { patch }),
  closeDecision: (behavior: string, remember: boolean) =>
    invoke("close_decision", { behavior, remember }),
  fullQuit: () => invoke("full_quit"),
  companionApplyPrefs: () => invoke("companion_apply_prefs"),
};

/** Poll the supervisor until it decides embedded vs external; switch the
 *  runtime transport to HTTP when an external daemon owns the runtime.
 *  Resolves with the final info (or null outside Tauri). */
export async function bootstrapSupervisor(): Promise<SupervisorInfo | null> {
  if (!isTauri) return null; // browser mode: transport already HTTP
  for (let i = 0; i < 240; i++) {
    try {
      const info = await desktop.supervisorInfo();
      if (info.mode === "external" && info.token) {
        configureHttp(info.apiBase, info.token);
        return info;
      }
      if (info.mode === "embedded" || info.state === "disconnected") {
        return info;
      }
    } catch {
      /* command not ready yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  return null;
}

export function onCloseRequested(handler: () => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen("close-requested", () => handler());
}

export function onNavigate(handler: (tab: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("navigate", (e) => handler(e.payload));
}

export function onSupervisorState(handler: (state: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("supervisor-state", (e) => handler(e.payload));
}

export function onTrayActionError(handler: (message: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("tray-action-error", (e) => handler(e.payload));
}

export { isTauri };
