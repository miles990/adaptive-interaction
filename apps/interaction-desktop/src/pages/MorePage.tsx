// 更多（v0.5 IA）：記憶與資料、活動紀錄、外觀與語言、備份與還原、進階模式。
// Activity 的「待決定」主入口在右上角 Inbox；這裡保留完整歷史。
// 「進階模式」是顯示模式切換唯一的家（設定頁只指路），開啟後才在同一頁展開第二層
// （版本、整合來源診斷、配方原始檔、開發者工具與授權檔位置）——一般模式看不到。
// 「角色與整合管理」不再是分頁按鈕，只保留隱藏的相容路由（舊書籤／深連結）。

import React from "react";
import { api, RuntimeEvent } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Icon } from "../icons";
import { Badge, Section, Toggle, useAsync } from "../ui";
import { MemoryKnowledgePage } from "./MemoryKnowledgePage";
import { ActivityPage } from "./ActivityPage";
import { SettingsPage } from "./SettingsPage";
import { BackupSection } from "./BackupSection";

export type MoreTab =
  | "memory"
  | "activity"
  | "settings"
  | "backup"
  | "manage"
  | "advanced-features";

/** 「更多」的五個一級入口。`manage` 刻意不在這裡：它是隱藏的相容路由。 */
export const MORE_TABS: [MoreTab, string][] = [
  ["memory", "記憶與資料"],
  ["activity", "活動紀錄"],
  ["settings", "外觀與語言"],
  ["backup", "備份與還原"],
  ["advanced-features", "進階模式"],
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
      {tab === "backup" && <BackupSection onNavigate={onNavigate} />}
      {tab === "manage" && <ManageSection onNavigate={onNavigate} />}
      {tab === "advanced-features" && <AdvancedFeaturesSection onNavigate={onNavigate} />}
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

/** 進階模式：顯示模式切換唯一的家（後端持久化；CLI 也能讀寫同一份偏好）。
 *  開啟後才在下方展開第二層技術入口——一般模式的「更多」不放任何技術字眼。 */
export function AdvancedFeaturesSection({ onNavigate }: { onNavigate?: (tab: string) => void }) {
  const { prefs, setMode } = useAppState();
  const [error, setError] = React.useState<string | null>(null);
  const advanced = prefs.mode === "advanced";
  return (
    <div>
      <Section title="進階模式">
        <Toggle
          checked={advanced}
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
      {advanced && <AdvancedSecondLevel onNavigate={onNavigate} />}
    </div>
  );
}

/** 進階模式的第二層：版本、Provider 診斷、配方 YAML、開發者工具與 token 檔位置。
 *  只在 `prefs.mode === "advanced"` 時渲染——這些字眼不得出現在一般模式的任何畫面。 */
function AdvancedSecondLevel({ onNavigate }: { onNavigate?: (tab: string) => void }) {
  const [runtime] = useAsync(() => api.status(), []);
  const version = String(runtime.data?.version ?? "未知");
  const schema = String(runtime.data?.schemaVersion ?? "未知");
  const go = (tab: string) => () => onNavigate?.(tab);
  return (
    <>
      <Section title="版本與 Runtime">
        {runtime.loading ? (
          <p className="muted small">正在讀取版本…</p>
        ) : runtime.error ? (
          <p className="state-box state-error">目前無法確認版本：{runtime.error}</p>
        ) : (
          <p className="muted small">
            Runtime {version}・Schema {schema}
          </p>
        )}
        <p className="muted small">
          更新不會自動安裝或替換執行檔；正式發布仍由簽章 Release 流程處理，避免背景更新繞過驗證。
        </p>
      </Section>
      <Section title="診斷與原始設定">
        <div className="row wrap">
          <button onClick={go("adv-providers")}>Provider 診斷</button>
          <button onClick={go("adv-recipes")}>配方 YAML</button>
          <button onClick={go("adv-policy")}>政策／同意原始設定</button>
        </div>
      </Section>
      <Section title="開發者工具">
        <div className="row wrap">
          <button onClick={go("adv-overview")}>總覽（原始）</button>
          <button onClick={go("adv-receptors")}>受器</button>
          <button onClick={go("adv-actuators")}>動器</button>
          <button onClick={go("adv-tools")}>工具</button>
          <button onClick={go("adv-timeline")}>時間軸</button>
          <button onClick={go("adv-knowledge")}>Knowledge Graph</button>
        </div>
        <p className="muted small">
          HTTP API 的授權 token 存在 <code>~/.adaptive-interaction/state/api-token</code>；
          CLI（<code>interact-ai</code>）與這個視窗共用同一個 Runtime 與同一份安全規則。
        </p>
      </Section>
    </>
  );
}
