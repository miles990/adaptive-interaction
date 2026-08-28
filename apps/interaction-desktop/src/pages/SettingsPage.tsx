// 設定：一般／進階模式切換（後端持久化）、重新執行首次設定精靈。

import React from "react";
import { api } from "../api";
import { useAppState } from "../appstate";
import { Section, Toggle, useAsync } from "../ui";
import { desktop, DesktopPrefs, isTauri } from "../desktop";

export function SettingsPage({
  onRerunOnboarding,
  onNavigate,
}: {
  onRerunOnboarding: () => void;
  onNavigate: (tab: string) => void;
}) {
  const { prefs, setMode, setPreferences } = useAppState();
  const [error, setError] = React.useState<string | null>(null);
  const [runtime] = useAsync(() => api.status(), []);

  return (
    <div>
      <Section title="顯示模式">
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
          進階模式會多出技術頁面（原始受器／動器／工具／配方 YAML／政策 JSON／時間軸），
          並在各頁顯示技術 ID 與原始資料。兩種模式使用相同的後端狀態與安全規則。
        </p>
        {error && <p className="cap-card-error" role="alert">{error}</p>}
      </Section>

      <Section title="語言、外觀與可及性">
        <div className="settings-grid">
          <label className="field-label">
            介面語言
            <select
              value={prefs.locale}
              onChange={(event) => void setPreferences({ locale: event.target.value })}
            >
              <option value="zh-TW">繁體中文</option>
            </select>
          </label>
          <label className="field-label">
            外觀
            <select
              value={prefs.appearance ?? "system"}
              onChange={(event) =>
                void setPreferences({ appearance: event.target.value as "system" | "dark" | "light" })
              }
            >
              <option value="system">跟隨系統</option>
              <option value="dark">深色</option>
              <option value="light">淺色</option>
            </select>
          </label>
          <label className="field-label">
            介面縮放 {prefs.scalePercent ?? 100}%
            <input
              type="range"
              min={80}
              max={150}
              step={10}
              value={prefs.scalePercent ?? 100}
              onChange={(event) => void setPreferences({ scalePercent: Number(event.target.value) })}
            />
          </label>
        </div>
        <Toggle
          checked={prefs.reduceMotion === true}
          onChange={(on) => setPreferences({ reduceMotion: on })}
          label="減少非必要動畫（會與作業系統 Reduced Motion 一併生效）"
        />
        <p className="muted small">
          安全狀態、文字標籤與鍵盤焦點不會因減少動畫而消失。音效、語音、通知與勿擾使用「能力與裝置」及「隱私與安全」中的同一 Runtime 設定。
        </p>
      </Section>

      <Section title="首次設定">
        <p className="muted small">
          重新執行首次設定精靈，重新選擇感知來源、回應方式與互動偏好。已有的設定不會被清除，
          精靈套用時才會變更。
        </p>
        <button onClick={onRerunOnboarding}>重新執行首次設定</button>
      </Section>

      <Section title="小樞的設定">
        <p className="muted small">
          外觀、女僕裝、表現程度、主動對話與安靜時段都由「小樞」頁統一管理，
          這裡不再放第二份相同開關。
        </p>
        <button onClick={() => onNavigate("companion")}>前往小樞</button>
      </Section>

      <DesktopLifecycleSection />

      <Section title="資料備份、還原與重設">
        <p className="muted small">
          記憶頁提供可讀 JSON 備份、逐筆驗證還原、期限修改、匯出與刪除；原始素材及其衍生物會在刪除前顯示影響預覽。重新執行首次設定不會清除既有資料。
        </p>
        <div className="row wrap">
          <button onClick={() => onNavigate("memory")}>開啟匯出、還原與刪除</button>
        </div>
      </Section>

      <Section title="更新與版本">
        {runtime.loading ? (
          <p className="muted small">正在讀取 Runtime 版本…</p>
        ) : runtime.error ? (
          <p className="state-box state-error">目前無法確認版本：{runtime.error}</p>
        ) : (
          <p className="muted small">
            Runtime {String(runtime.data?.version ?? "未知")}・Schema {String(runtime.data?.schemaVersion ?? "未知")}
          </p>
        )}
        <p className="muted small">
          更新不會自動安裝或替換執行檔；正式發布仍由簽章 Release 流程處理，避免背景更新繞過驗證。
        </p>
      </Section>

      <Section title="關於名稱">
        <p className="muted small">
          你在各能力「詳情」中自訂的名稱只影響顯示，不影響行為或安全規則；
          目前有 {Object.keys(prefs.customNames).length} 個自訂名稱。
        </p>
      </Section>
    </div>
  );
}

/** 桌面 App 生命週期（只在 Tauri 環境顯示）：關閉行為、登入啟動。 */
function DesktopLifecycleSection() {
  const [dprefs, setDprefs] = React.useState<DesktopPrefs | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!isTauri) return;
    desktop
      .prefsGet()
      .then(setDprefs)
      .catch((e) => setError(String(e)));
  }, []);

  if (!isTauri) return null;
  if (error) {
    return (
      <Section title="視窗與啟動">
        <p className="state-box state-error">無法載入桌面設定：{error}</p>
      </Section>
    );
  }
  if (!dprefs) return null;

  const patch = async (p: Partial<DesktopPrefs>) => {
    try {
      setDprefs(await desktop.prefsPatch(p));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Section title="視窗與啟動">
      <p className="muted small">
        關閉控制中心視窗時：
      </p>
      <div role="radiogroup" aria-label="關閉視窗行為">
        {(
          [
            ["keep-running", "保持在背景運作（狀態列與桌面角色保留）"],
            ["hide-companion", "保持背景運作，但隱藏桌面角色"],
            ["quit", "完全結束（停止所有功能）"],
          ] as const
        ).map(([value, label]) => (
          <label key={value} className="toggle">
            <input
              type="radio"
              name="close-behavior"
              checked={dprefs.closeBehavior === value}
              onChange={() => patch({ closeBehavior: value })}
            />
            <span>{label}</span>
          </label>
        ))}
      </div>
      <Toggle
        checked={dprefs.askOnClose}
        onChange={(on) => patch({ askOnClose: on })}
        label="每次關閉時詢問"
      />
      <hr />
      <Toggle
        checked={dprefs.launchAtLogin}
        onChange={(on) => patch({ launchAtLogin: on })}
        label="登入電腦時啟動（預設關閉）"
      />
      <Toggle
        checked={dprefs.showCompanionOnStart}
        onChange={(on) => patch({ showCompanionOnStart: on })}
        label="啟動後顯示桌面角色"
      />
      <Toggle
        checked={dprefs.openControlCenterOnStart}
        onChange={(on) => patch({ openControlCenterOnStart: on })}
        label="啟動後打開控制中心"
      />
      <div className="row wrap" style={{ marginTop: 10 }}>
        <button
          className="danger"
          onClick={() => desktop.fullQuit().catch((e) => setError(String(e)))}
        >
          完全結束 Adaptive Interaction
        </button>
      </div>
    </Section>
  );
}
