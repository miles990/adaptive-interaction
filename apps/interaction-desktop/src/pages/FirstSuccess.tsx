// 首次成功體驗：精靈 commit 之後「可以略過」的一屏——不是第四個必要步驟。
// 「角色準備好了。要不要先試一次？」五個選項：
//   提醒我休息   → 本機提醒：走既有 plan 路徑（通知／角色氣泡本機回應），不建立任何 AI 工作；
//                  結果用 statusProjection 誠實投影（已送出≠已看見）。
//   交代一件小工作 → 預填 sessionStorage "work.prefill" 後前往「工作」（工作流程本身在工作頁）。
//   先在桌面陪我 → desktop.prefsPatch({ companionVisible: true }) ＋ companionApplyPrefs。
//   更換角色     → 前往角色頁。
//   稍後再說     → 關閉。
// 看過的旗標 firstSuccessSeen：先送 uiPrefsPatch，host 沒保存就退回 localStorage（不假裝已保存）。
// 角色不可用時整屏仍能運作（可信文字「角色」）；安全文字固定。

import React from "react";
import { api, type Receipt } from "../api";
import { characterNameFallback, useCharacterName } from "../characterName";
import { desktop, isTauri } from "../desktop";
import { projectInboxStatus, type ProjectedStatus } from "../statusProjection";
import { Badge } from "../ui";
import { sanitizeErrorText } from "./character/catalog";

export const FIRST_SUCCESS_STORAGE_KEY = "adaptive-interaction.firstSuccessSeen";
/** 與「現在」頁、工作頁共用的預填鍵（工作頁讀取後清除）。 */
export const WORK_PREFILL_KEY = "work.prefill";
export const FIRST_TASK_PREFILL = "幫我把今天要做的事整理成一份清單，先列出來讓我確認，不要直接動任何檔案。";

function localSeen(): boolean {
  try {
    return globalThis.localStorage?.getItem(FIRST_SUCCESS_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function setLocalSeen(): void {
  try {
    globalThis.localStorage?.setItem(FIRST_SUCCESS_STORAGE_KEY, "1");
  } catch {
    /* 私密模式／配額：略過 */
  }
}

/** 看過了嗎：host 偏好（若這個版本保存）＞本機旗標。 */
export async function isFirstSuccessSeen(): Promise<boolean> {
  try {
    const prefs = (await api.uiPrefsGet()) as unknown as Record<string, unknown>;
    if (prefs?.firstSuccessSeen === true) return true;
  } catch {
    /* 讀不到 host 偏好就看本機旗標 */
  }
  return localSeen();
}

/** 標記看過：先送 host；回傳沒帶回旗標就退回本機（回報 "local"，不宣稱已保存到 host）。 */
export async function markFirstSuccessSeen(): Promise<"host" | "local"> {
  try {
    const updated = (await api.uiPrefsPatch({ firstSuccessSeen: true })) as unknown as Record<string, unknown>;
    if (updated?.firstSuccessSeen === true) return "host";
  } catch {
    /* host 拒絕或離線：退回本機 */
  }
  setLocalSeen();
  return "local";
}

export interface ReminderOutcome {
  status: ProjectedStatus;
  /** 透過哪種本機回應（人話）。 */
  via: string;
}

function viaLabel(actuatorId: string | undefined): string {
  switch (actuatorId) {
    case "notification":
      return "透過系統通知";
    case "companion.bubble.show":
      return "透過桌面角色的氣泡";
    default:
      return "透過本機回應方式";
  }
}

/**
 * 本機休息提醒：確保有進行中的互動工作階段（純本機、沒有任何同意範圍），建立一個
 * 只用通知／角色氣泡的計畫並執行。**不會**建立 AI 工作。回傳的狀態來自 receipt 的
 * currentStatus 投影（已送出／已送達≠已看見）。
 */
export async function sendRestReminder(name: string): Promise<ReminderOutcome> {
  let session: { state?: string } | null = null;
  try {
    session = (await api.sessionGet()) as { state?: string } | null;
  } catch {
    session = null;
  }
  if (!session || (typeof session.state === "string" && session.state !== "active")) {
    await api.sessionStart("first-success", []);
  }
  const plan = await api.createPlan({
    intent: "rest-reminder",
    message: `${name}提醒：休息一下，喝口水、動一動。`,
    preferredChannels: ["notification", "desktop-pet"],
    candidates: ["notification", "companion.bubble.show"],
    minChannels: 1,
    maxChannels: 1,
    allowNoAction: false,
    metadata: { source: "first-success" },
  });
  const planId = String(plan?.planId ?? "");
  if (!planId) throw new Error("沒有拿到計畫編號");
  const receipts = (await api.executePlan(planId)) as Receipt[];
  const first = Array.isArray(receipts) ? receipts[0] : undefined;
  if (!first) {
    return { status: projectInboxStatus("uncertain"), via: "沒有收到任何回報" };
  }
  return { status: projectInboxStatus(String(first.currentStatus)), via: viaLabel(first.actuatorId) };
}

export function FirstSuccess({
  onDone,
  onNavigate,
}: {
  onDone: () => void;
  /** 沒提供時導覽選項只關閉此屏（App 會停在預設分頁）。 */
  onNavigate?: (tab: string) => void;
}) {
  const character = useCharacterName();
  const name = character.loaded ? character.name : characterNameFallback;
  const [reminder, setReminder] = React.useState<{ busy: boolean; outcome?: ReminderOutcome; error?: string }>({
    busy: false,
  });
  const [companion, setCompanion] = React.useState<{ busy: boolean; message?: string; error?: string }>({
    busy: false,
  });
  const finishing = React.useRef(false);

  const finish = async (next?: () => void) => {
    if (finishing.current) return;
    finishing.current = true;
    try {
      await markFirstSuccessSeen();
    } catch {
      /* 旗標寫不進去也不能卡住使用者 */
    }
    next?.();
    onDone();
  };

  const remind = async () => {
    setReminder({ busy: true });
    try {
      const outcome = await sendRestReminder(name);
      setReminder({ busy: false, outcome });
    } catch (e) {
      setReminder({ busy: false, error: sanitizeErrorText(e) });
    }
  };

  const showCompanion = async () => {
    if (!isTauri) {
      setCompanion({
        busy: false,
        message: "桌面角色需要桌面版控制中心；瀏覽器檢視只能看到文字，安全訊息仍會顯示。",
      });
      return;
    }
    setCompanion({ busy: true });
    try {
      await desktop.prefsPatch({ companionVisible: true });
      await desktop.companionApplyPrefs();
      setCompanion({ busy: false, message: `已請桌面角色視窗顯示${name}；若無法顯示會改用文字。` });
    } catch (e) {
      setCompanion({ busy: false, error: sanitizeErrorText(e) });
    }
  };

  const delegate = () => {
    try {
      globalThis.sessionStorage?.setItem(WORK_PREFILL_KEY, FIRST_TASK_PREFILL);
    } catch {
      /* 沒有 sessionStorage 就不預填 */
    }
    void finish(() => onNavigate?.("work"));
  };

  return (
    <div className="onboarding first-success" role="dialog" aria-label="首次成功體驗">
      <div className="onboarding-panel">
        <h1>{name}準備好了。要不要先試一次？</h1>
        <p className="muted">
          每一項都可以略過；之後隨時能在「現在」與「{name}」頁做同樣的事。安全訊息永遠是固定文字，
          不會因為角色而改變。
        </p>
        {!character.loaded && <p className="muted small">角色資料尚未載入；這裡先用可信的文字顯示。</p>}
        <div className="first-success-options">
          <button className="first-success-option" onClick={() => void remind()} disabled={reminder.busy}>
            <strong>提醒我休息</strong>
            <span className="muted small">
              {reminder.busy ? "送出中…" : "送一則本機提醒（不會啟動 AI，也不會建立長期工作）。"}
            </span>
          </button>
          {reminder.outcome && (
            <div className="first-success-result" role="status">
              <Badge kind={reminder.outcome.status.badge}>{reminder.outcome.status.label}</Badge>
              <span>{reminder.outcome.via}。</span>
              {reminder.outcome.status.honesty && <span className="muted small">{reminder.outcome.status.honesty}。</span>}
              <span className="muted small">已送出不等於你已經看見；{name}不會替你確認。</span>
            </div>
          )}
          {reminder.error && (
            <p className="cap-card-error" role="alert">
              提醒沒有送出：{reminder.error}
            </p>
          )}

          <button className="first-success-option" onClick={delegate}>
            <strong>交代一件小工作</strong>
            <span className="muted small">到「工作」頁先看一遍要用哪個 AI 幫手、讀哪些東西，再由你按開始。</span>
          </button>

          <button className="first-success-option" onClick={() => void showCompanion()} disabled={companion.busy}>
            <strong>先在桌面陪我</strong>
            <span className="muted small">{companion.busy ? "套用中…" : `把${name}顯示在桌面上（隨時可隱藏）。`}</span>
          </button>
          {companion.message && (
            <p className="muted small" role="status">
              {companion.message}
            </p>
          )}
          {companion.error && (
            <p className="cap-card-error" role="alert">
              無法顯示桌面角色：{companion.error}
            </p>
          )}

          <button className="first-success-option" onClick={() => void finish(() => onNavigate?.("companion"))}>
            <strong>更換角色</strong>
            <span className="muted small">到角色頁挑一個內建角色，或匯入第三方角色。</span>
          </button>
        </div>
        <footer className="onboarding-footer">
          <button className="ghost" onClick={() => void finish()}>
            稍後再說
          </button>
          <button className="primary" onClick={() => void finish()}>
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
