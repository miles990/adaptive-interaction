// 更多（v0.5 IA）：記憶與知識、活動歷史、一般設定。
// Activity 的「待決定」主入口在右上角 Inbox；這裡保留完整歷史。

import React from "react";
import { RuntimeEvent } from "../api";
import { MemoryKnowledgePage } from "./MemoryKnowledgePage";
import { ActivityPage } from "./ActivityPage";
import { SettingsPage } from "./SettingsPage";

export type MoreTab = "memory" | "activity" | "settings";

export function MorePage({
  refreshKey,
  events,
  advanced,
  onNavigate,
  onRerunOnboarding,
  initial = "memory",
}: {
  refreshKey: number;
  events: RuntimeEvent[];
  advanced: boolean;
  onNavigate: (tab: string) => void;
  onRerunOnboarding: () => void;
  initial?: MoreTab;
}) {
  const [tab, setTab] = React.useState<MoreTab>(initial);
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="更多分類">
        {(
          [
            ["memory", "記憶與知識"],
            ["activity", "活動歷史"],
            ["settings", "設定"],
          ] as [MoreTab, string][]
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
      {tab === "memory" && (
        <MemoryKnowledgePage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
        />
      )}
      {tab === "activity" && (
        <ActivityPage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
        />
      )}
      {tab === "settings" && (
        <SettingsPage onRerunOnboarding={onRerunOnboarding} onNavigate={onNavigate} />
      )}
    </div>
  );
}
