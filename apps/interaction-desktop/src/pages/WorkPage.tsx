// 工作（v0.5 IA，一般模式 task-first）：交代一件工作（TaskComposer）→ 進行中與最近的
// 工作（AiPage 清單，人話狀態投影）→ 折疊的「工作設定」（Agent 偵測／登入／分工）。
// 自動互動是第二個分頁。舊 tab id（ai／automations）由 App 的相容路由導到對應分頁。

import React from "react";
import { useAppState } from "../appstate";
import { AiPage } from "./AiPage";
import { AutomationsPage } from "./AutomationsPage";
import { TaskComposer } from "./work/TaskComposer";

export type WorkTab = "sessions" | "automations";

const ROUTE_ROLES = ["conversation", "programming", "knowledge", "review"] as const;

function agentLabel(agent?: string): string {
  if (agent === "codex") return "Codex";
  if (agent === "claude-code") return "Claude Code";
  if (agent === "none") return "不交給 Agent";
  return "尚未選擇";
}

/**
 * 目前生效的分工，用人話講一次。
 * 只描述「現在是怎麼分的」，不宣稱來源：偏好一律帶著後端預設的四筆路由
 * （精靈選「稍後再說」不會寫入任何路由），所以畫面上看到的分工不一定是使用者選過的。
 */
export function agentRouteSummary(routes?: Record<string, string>): string {
  const map = routes ?? {};
  const values = ROUTE_ROLES.map((role) => map[role]).filter(Boolean);
  if (values.length === 0) return "尚未設定（使用預設分工）";
  const distinct = [...new Set(values)];
  if (distinct.length === 1) {
    return distinct[0] === "none" ? "全部不交給 Agent" : `全部交給 ${agentLabel(distinct[0])}`;
  }
  return `程式工作交給 ${agentLabel(map.programming)}；一般對話、知識整理與結果複審交給 ${agentLabel(
    map.conversation
  )}`;
}

export function WorkPage({
  refreshKey,
  advanced,
  onNavigate,
  initial = "sessions",
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  initial?: WorkTab;
}) {
  const [tab, setTab] = React.useState<WorkTab>(initial);
  // 相容路由：App 對「work／automations」這類舊 id 都渲染同一個元件，React 會沿用
  // 已掛載的實例，useState(initial) 只在首次掛載生效——route 改變時必須同步分頁，
  // 否則 tray／深連結／全域搜尋／Inbox 切到舊 id 只會高亮導覽、內容不動。
  React.useEffect(() => {
    setTab(initial);
  }, [initial]);
  const { prefs } = useAppState();
  // 交代成功後讓清單重新載入（refreshKey 與本地計數都只會遞增，和一定會變）。
  const [created, setCreated] = React.useState(0);
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const settingsRef = React.useRef<HTMLDivElement>(null);

  return (
    <div className="work-page">
      <div className="hub-tabs" role="tablist" aria-label="工作分類">
        {(
          [
            ["sessions", "工作"],
            ["automations", "自動互動"],
          ] as [WorkTab, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "hub-tab active" : "hub-tab"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === "sessions" && (
        <>
          <TaskComposer advanced={advanced} onCreated={() => setCreated((n) => n + 1)} />
          <p className="muted small">
            目前分工：{agentRouteSummary(prefs.agentRoutes)}{" "}
            <button
              onClick={() => {
                setSettingsOpen(true);
                settingsRef.current?.scrollIntoView?.({ block: "start" });
              }}
            >
              調整分工
            </button>
          </p>
          <div ref={settingsRef}>
            <AiPage
              refreshKey={refreshKey + created}
              advanced={advanced}
              onNavigate={onNavigate}
              layout={advanced ? "full" : "task-first"}
              settingsOpen={settingsOpen}
              onSettingsToggle={setSettingsOpen}
            />
          </div>
        </>
      )}
      {tab === "automations" && <AutomationsPage refreshKey={refreshKey} advanced={advanced} />}
    </div>
  );
}
