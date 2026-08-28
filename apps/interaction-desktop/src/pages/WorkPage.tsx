// 工作（v0.5 IA）：AI 工作階段＋自動互動集中在同一個入口。
// 舊 tab id（ai／automations）由 App 的相容路由導到對應分頁。

import React from "react";
import { AiPage } from "./AiPage";
import { AutomationsPage } from "./AutomationsPage";

export type WorkTab = "sessions" | "automations";

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
      {tab === "sessions" && <AiPage refreshKey={refreshKey} onNavigate={onNavigate} />}
      {tab === "automations" && <AutomationsPage refreshKey={refreshKey} advanced={advanced} />}
    </div>
  );
}
