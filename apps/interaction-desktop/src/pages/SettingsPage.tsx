// 外觀與語言（更多 → 外觀與語言）：語言／外觀／縮放／減少動畫、視窗與啟動、
// 重新執行首次設定的入口。
// 一般／進階模式切換的唯一主人是「更多 → 進階模式」，版本與技術資訊也住在那一層；
// 角色的設定在角色頁；匯出／還原的唯一主人是「更多 → 備份與還原」。這一頁只放
// 「這台電腦上這個視窗長什麼樣、怎麼開關」，不放第二份任何設定。

import React from "react";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Section, Toggle } from "../ui";
import { desktop, DesktopPrefs, isTauri } from "../desktop";

export function SettingsPage({
  onRerunOnboarding,
  onNavigate,
}: {
  onRerunOnboarding: () => void;
  onNavigate: (tab: string) => void;
}) {
  const { prefs, setPreferences } = useAppState();
  const advanced = prefs.mode === "advanced";
  const character = useCharacterName({ locale: prefs.locale });

  return (
    <div>
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
          安全狀態、文字標籤與鍵盤焦點不會因減少動畫而消失。音效、語音、主動說話與安靜時段在「{character.name}」頁；
          通知與各項能力開關在「連接與權限」頁 —— 都是同一份系統設定，這裡不放第二份。
        </p>
      </Section>

      <Section title="首次設定">
        <p className="muted small">
          三個步驟：選擇角色與陪伴方式、選擇 AI 工作方式、確認安全與權限預設。
          重新執行<strong>不會</strong>自動關掉你已經開啟的能力；按「完成設定」後會先列出每一項變更，
          你按下「套用」才會生效，沒被列出的設定完全不動。
        </p>
        <button onClick={onRerunOnboarding}>重新執行首次設定</button>
      </Section>

      <Section title={`${character.name}的設定`}>
        <p className="muted small">
          外觀與名字、表現程度、主動對話與安靜時段都由「{character.name}」頁統一管理，
          這裡不再放第二份相同開關。
        </p>
        <button onClick={() => onNavigate("companion")}>前往{character.name}</button>
      </Section>

      <Section title="進階模式">
        <p className="muted small">
          {advanced ? "目前顯示進階功能（技術頁面與原始資料）。" : "目前只顯示一般功能。"}
          切換、版本與技術資訊都在「更多 → 進階模式」。
        </p>
        <button onClick={() => onNavigate("advanced-features")}>前往進階模式</button>
      </Section>

      <DesktopLifecycleSection />
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
