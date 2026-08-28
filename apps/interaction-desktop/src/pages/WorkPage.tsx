// 工作（v0.5 IA）：AI 工作階段＋自動互動集中在同一個入口。
// 舊 tab id（ai／automations）由 App 的相容路由導到對應分頁。

import React from "react";
import { useAppState } from "../appstate";
import { AiPage } from "./AiPage";
import { AutomationsPage } from "./AutomationsPage";

export type WorkTab = "sessions" | "automations";

const ROUTE_ROLES = ["conversation", "programming", "knowledge", "review"] as const;

function agentLabel(agent?: string): string {
  if (agent === "codex") return "Codex";
  if (agent === "claude-code") return "Claude Code";
  if (agent === "none") return "不交給 Agent";
  return "尚未選擇";
}

/** 首次設定精靈步驟二寫進來的路由偏好，用人話講一次。 */
export function agentRouteSummary(routes?: Record<string, string>): string {
  const map = routes ?? {};
  const values = ROUTE_ROLES.map((role) => map[role]).filter(Boolean);
  if (values.length === 0) return "尚未選擇（稍後再說）";
  const distinct = [...new Set(values)];
  if (distinct.length === 1) return `全部交給 ${agentLabel(distinct[0])}`;
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
  const { prefs } = useAppState();
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="工作分類">
        {(
          [
            ["sessions", "AI 工作階段"],
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
      <p className="muted small">
        精靈選擇：{agentRouteSummary(prefs.agentRoutes)}{" "}
        <button onClick={() => setTab("sessions")}>前往工作頁調整</button>
      </p>
      {tab === "sessions" && (
        <AiPage refreshKey={refreshKey} advanced={advanced} onNavigate={onNavigate} />
      )}
      {tab === "automations" && <AutomationsPage refreshKey={refreshKey} advanced={advanced} />}
    </div>
  );
}
