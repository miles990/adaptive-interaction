// 更多（v0.5 IA）：記憶與知識、活動歷史、設定、角色與整合管理、進階功能。
// Activity 的「待決定」主入口在右上角 Inbox；這裡保留完整歷史。
// 「角色與整合管理」只是指路（角色頁／連接與權限的裝置區），不放第二份設定；
// 「進階功能」是顯示模式切換唯一的家（設定頁只指路）。

import React from "react";
import { RuntimeEvent } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Icon } from "../icons";
import { Badge, Section, Toggle } from "../ui";
import { MemoryKnowledgePage } from "./MemoryKnowledgePage";
import { ActivityPage } from "./ActivityPage";
import { SettingsPage } from "./SettingsPage";

export type MoreTab = "memory" | "activity" | "settings" | "manage" | "advanced-features";

export const MORE_TABS: [MoreTab, string][] = [
  ["memory", "記憶與知識"],
  ["activity", "活動歷史"],
  ["settings", "設定"],
  ["manage", "角色與整合管理"],
  ["advanced-features", "進階功能"],
];

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
  // 相容路由：App 對「work／automations」這類舊 id 都渲染同一個元件，React 會沿用
  // 已掛載的實例，useState(initial) 只在首次掛載生效——route 改變時必須同步分頁，
  // 否則 tray／深連結／全域搜尋／Inbox 切到舊 id 只會高亮導覽、內容不動。
  React.useEffect(() => {
    setTab(initial);
  }, [initial]);
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="更多分類">
        {MORE_TABS.map(([id, label]) => (
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
      {tab === "manage" && <ManageSection onNavigate={onNavigate} />}
      {tab === "advanced-features" && <AdvancedFeaturesSection />}
    </div>
  );
}

/** 角色與整合管理：指路到角色頁（外觀／名字／陪伴方式／更換或加入角色）與
 *  「連接與權限」的裝置區（iPhone、ESP32、角色的整合來源）。這裡不放第二份設定。 */
export function ManageSection({ onNavigate }: { onNavigate: (tab: string) => void }) {
  const character = useCharacterName();
  return (
    <div className="more-manage">
      <Section title="角色">
        <p className="muted small">
          目前角色：<strong>{character.name}</strong>
          {character.loaded ? (
            <Badge kind="ok">已載入</Badge>
          ) : (
            <Badge kind="warn">尚未載入，先用文字顯示</Badge>
          )}
        </p>
        <p className="muted small">
          外觀與名字、平常怎麼陪伴、安靜與勿擾、更換或加入角色，都在「{character.name}」頁。
        </p>
        <button onClick={() => onNavigate("companion")}>
          <Icon name={character.icon} size={14} /> 管理角色
        </button>
      </Section>
      <Section title="整合與裝置">
        <p className="muted small">
          iPhone、ESP32 這類裝置，以及角色使用的整合來源（內建或第三方、本機或外部、
          是否需要網路），都在「連接與權限」的裝置區管理。
        </p>
        <button onClick={() => onNavigate("connect")}>
          <Icon name="plug" size={14} /> 管理裝置與整合
        </button>
      </Section>
    </div>
  );
}

/** 進階功能：顯示模式切換唯一的家（後端持久化；CLI 也能讀寫同一份偏好）。 */
export function AdvancedFeaturesSection() {
  const { prefs, setMode } = useAppState();
  const [error, setError] = React.useState<string | null>(null);
  return (
    <Section title="進階功能">
      <Toggle
        checked={prefs.mode === "advanced"}
        onChange={async (on) => {
          try {
            await setMode(on ? "advanced" : "simple");
            setError(null);
          } catch (e) {
            setError(String(e));
          }
        }}
        label="顯示進階功能"
      />
      <p className="muted small">
        開啟後，側欄與「更多」選單會多出技術頁面（原始的感知來源、回應方式、工具清單、
        自動互動與安全規則的原始設定、時間軸、整合來源與知識圖），各頁也會顯示技術識別碼與原始資料。
        兩種模式使用相同的後端狀態與安全規則。
      </p>
      {error && (
        <p className="cap-card-error" role="alert">
          {error}
        </p>
      )}
    </Section>
  );
}
