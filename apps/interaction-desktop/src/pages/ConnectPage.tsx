// 連接與權限（v0.5 IA）：裝置與能力＋同意與安全集中在同一個入口。
// 權限地圖從首頁移到這裡 — 這裡是「AI 可以讀取或操作什麼」唯一的主人。

import React from "react";
import { useAppState } from "../appstate";
import { Section } from "../ui";
import { CapabilitiesHub } from "./CapabilitiesHub";
import { SafetyPage } from "./SafetyPage";
import { PermissionMap } from "./HomePage";

export type ConnectTab = "devices" | "safety";

export function ConnectPage({
  refreshKey,
  advanced,
  onNavigate,
  initial = "devices",
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  initial?: ConnectTab;
}) {
  const [tab, setTab] = React.useState<ConnectTab>(initial);
  const { human } = useAppState();
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="連接與權限分類">
        {(
          [
            ["devices", "裝置與能力"],
            ["safety", "同意與安全"],
          ] as [ConnectTab, string][]
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
      {tab === "devices" && <CapabilitiesHub refreshKey={refreshKey} advanced={advanced} />}
      {tab === "safety" && (
        <div>
          <Section title="權限地圖 — AI 現在可以做什麼？">
            {human ? (
              <PermissionMap
                receptors={human.receptors}
                actuators={human.actuators}
                tools={human.toolOperations}
              />
            ) : (
              <div className="state-box">載入中…</div>
            )}
            <p className="muted small">
              這張地圖來自目前的啟用狀態、安全規則與同意設定，是全 App 唯一的完整權限總覽。
            </p>
          </Section>
          <SafetyPage refreshKey={refreshKey} onNavigate={onNavigate} />
        </div>
      )}
    </div>
  );
}
