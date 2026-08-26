// 設定：一般／進階模式切換（後端持久化）、重新執行首次設定精靈。

import React from "react";
import { useAppState } from "../appstate";
import { Section, Toggle } from "../ui";
import { desktop, DesktopPrefs, isTauri } from "../desktop";

export function SettingsPage({ onRerunOnboarding }: { onRerunOnboarding: () => void }) {
  const { prefs, setMode } = useAppState();
  const [error, setError] = React.useState<string | null>(null);

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

      <Section title="首次設定">
        <p className="muted small">
          重新執行首次設定精靈，重新選擇感知來源、回應方式與互動偏好。已有的設定不會被清除，
          精靈套用時才會變更。
        </p>
        <button onClick={onRerunOnboarding}>重新執行首次設定</button>
      </Section>

      <DesktopLifecycleSection />

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
