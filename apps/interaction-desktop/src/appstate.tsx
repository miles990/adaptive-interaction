// App-wide state: UI preferences (simple/advanced mode — persisted in the
// backend, same store the CLI and API read), proactive pause, and the human
// capability projection. The frontend is never a truth source: everything
// here mirrors backend state.

import React from "react";
import { api, HumanCapabilities, PauseState, UiPreferences } from "./api";

export interface AppStateValue {
  prefs: UiPreferences;
  setMode: (mode: "simple" | "advanced") => Promise<void>;
  setCustomName: (key: string, name: string | null) => Promise<void>;
  pause: PauseState;
  refreshPause: () => Promise<void>;
  doPause: (durationMinutes?: number, reason?: string) => Promise<void>;
  doResume: () => Promise<void>;
  human?: HumanCapabilities;
  refreshHuman: () => Promise<void>;
  humanError?: string;
  findCard: (kind: "receptor" | "actuator" | "tool", id: string) => CardLookup;
}

export interface CardLookup {
  name: string;
  icon: string;
}

const DEFAULT_PREFS: UiPreferences = {
  mode: "simple",
  locale: "zh-TW",
  customNames: {},
  schemaVersion: "1.0",
};

const AppStateContext = React.createContext<AppStateValue | null>(null);

export function useAppState(): AppStateValue {
  const v = React.useContext(AppStateContext);
  if (!v) throw new Error("useAppState outside provider");
  return v;
}

export function AppStateProvider({
  ready,
  refreshKey,
  children,
}: {
  ready: boolean;
  refreshKey: number;
  children: React.ReactNode;
}) {
  const [prefs, setPrefs] = React.useState<UiPreferences>(DEFAULT_PREFS);
  const [pause, setPause] = React.useState<PauseState>({ paused: false });
  const [human, setHuman] = React.useState<HumanCapabilities | undefined>();
  const [humanError, setHumanError] = React.useState<string | undefined>();

  const refreshHuman = React.useCallback(async () => {
    try {
      setHuman(await api.capabilitiesHuman(undefined, true));
      setHumanError(undefined);
    } catch (e) {
      setHumanError(String(e));
    }
  }, []);

  const refreshPause = React.useCallback(async () => {
    try {
      setPause(await api.pauseGet());
    } catch {
      /* offline: keep last known */
    }
  }, []);

  // 上次真正重投影的時刻：trailing debounce 的餓死保險（見下）。
  const lastProjection = React.useRef(0);

  React.useEffect(() => {
    if (!ready) return;
    api.uiPrefsGet().then(setPrefs).catch(() => {});
    lastProjection.current = Date.now();
    refreshPause();
    refreshHuman();
  }, [ready]);

  // Capability/consent/policy events bump refreshKey upstream; re-project.
  // 純 trailing debounce 在持續事件流（例如受器連續觀測）下會不斷重置而
  // 永不觸發，導致撤回的同意一直顯示為「AI 可以做」。超過 1 秒未投影
  // 就立即執行，保證權限顯示最多落後約 1 秒。
  React.useEffect(() => {
    if (!ready) return;
    const overdue = Date.now() - lastProjection.current >= 1000;
    const t = setTimeout(
      () => {
        lastProjection.current = Date.now();
        refreshHuman();
        refreshPause();
      },
      overdue ? 0 : 250
    );
    return () => clearTimeout(t);
  }, [refreshKey, ready]);

  const value: AppStateValue = {
    prefs,
    setMode: async (mode) => {
      const updated = await api.uiPrefsPatch({ mode });
      setPrefs(updated);
    },
    setCustomName: async (key, name) => {
      const updated = await api.uiPrefsPatch({ customNames: { [key]: name } });
      setPrefs(updated);
      await refreshHuman();
    },
    pause,
    refreshPause,
    doPause: async (durationMinutes, reason) => {
      setPause(await api.pauseSet(durationMinutes, reason));
    },
    doResume: async () => {
      setPause(await api.pauseClear());
    },
    human,
    refreshHuman,
    humanError,
    findCard: (kind, id) => {
      const list =
        kind === "receptor"
          ? human?.receptors
          : kind === "actuator"
            ? human?.actuators
            : human?.toolOperations;
      const hit = list?.find((c) => c.id === id);
      return {
        name: hit?.displayName ?? id,
        icon: hit?.icon ?? (kind === "receptor" ? "scan-eye" : kind === "actuator" ? "send" : "wrench"),
      };
    },
  };

  return <AppStateContext.Provider value={value}>{children}</AppStateContext.Provider>;
}

// ---------------------------------------------------------------------------
// Shared human-display helpers (deterministic; no free-text invention).
// ---------------------------------------------------------------------------

export function triLabel(v: boolean | "unknown", yes: string, no: string, unknown: string): string {
  if (v === true) return yes;
  if (v === false) return no;
  return unknown;
}

export function availabilityLabel(a: string): string {
  switch (a) {
    case "available":
      return "可用";
    case "disabled":
      return "已停用";
    case "offline":
      return "離線";
    case "degraded":
      return "運作異常";
    case "revoked":
      return "授權已撤回";
    case "unknown":
      return "狀態未知";
    case "consent-required":
      return "需先取得同意";
    default:
      return a;
  }
}

export function confirmationLabel(level: string): { can: string; cannot: string } {
  switch (level) {
    case "requested":
      return { can: "已送出請求", cannot: "無法確認是否已被接受" };
    case "queued":
      return { can: "已排入佇列", cannot: "無法確認是否已實際執行" };
    case "acknowledged":
      return { can: "裝置或程式已確認收到", cannot: "無法確認實際效果" };
    case "delivered":
      return { can: "已送達（例如作業系統）", cannot: "無法確認你是否已經看見" };
    case "completed":
      return { can: "已實際完成", cannot: "" };
    case "verified":
      return { can: "已完成且經過驗證", cannot: "" };
    default:
      return { can: "", cannot: "無法確認執行到哪一層" };
  }
}

/** Map a receipt/action status to honest human wording. NEVER upgrades. */
export function actionStatusLabel(status: string): string {
  switch (status) {
    case "planned":
      return "已規劃";
    case "authorized":
      return "已授權";
    case "accepted":
      return "已排入（尚未執行）";
    case "dispatched":
      return "已送出（等待確認）";
    case "acknowledged":
      return "已收到（效果未確認）";
    case "observed":
      return "已觀察到效果";
    case "completed":
      return "已完成";
    case "blocked":
      return "被安全規則阻止";
    case "failed":
      return "失敗";
    case "uncertain":
      return "結果未知";
    case "cancelled":
      return "已取消";
    case "expired":
      return "已過期";
    case "stopped":
      return "已停止";
    default:
      return status;
  }
}
