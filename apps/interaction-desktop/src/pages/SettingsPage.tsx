// 設定：一般／進階模式切換（後端持久化）、重新執行首次設定精靈。

import React from "react";
import { api } from "../api";
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

      <ProactiveDialogueSection />

      <CompanionSection />

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

/** 主動式對話：模式與頻率由 Rust 確定性強制；此處只是設定介面。 */
function ProactiveDialogueSection() {
  const [status, setStatus] = React.useState<Record<string, unknown> | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      setStatus(await api.proactiveDialogueGet());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);
  React.useEffect(() => {
    void load();
  }, [load]);

  const config = (status?.config as Record<string, unknown> | undefined) ?? {};
  const mode = String(config.mode ?? "natural");
  const quietUntil = status?.quietUntil ? new Date(String(status.quietUntil)) : null;
  const quietActive = quietUntil !== null && quietUntil.getTime() > Date.now();

  const setMode = async (m: string) => {
    try {
      setStatus(await api.proactiveDialoguePatch({ mode: m }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Section title="主動式對話">
      <p className="muted small">
        小樞什麼情況下可以主動說話。頻率限制（每小時最多 {String(config.maxPerHour ?? 3)} 則、
        最短間隔 {String(config.minIntervalMinutes ?? 12)} 分鐘、沒有回覆不追問）由系統強制執行；
        安全與權限提示不受模式影響，一定會顯示。主動說話不代表可以主動做事——任何行動仍需授權。
      </p>
      <label className="field-label">
        模式
        <select value={mode} onChange={(e) => void setMode(e.target.value)}>
          <option value="off">關閉——不主動說話</option>
          <option value="necessary">必要——只有等待確認、失敗、未知與感測提示</option>
          <option value="natural">自然（建議）——加上任務進度與低頻建議</option>
          <option value="lively">活潑——再加問候與輕量陪伴</option>
          <option value="custom">自訂——個別選擇訊息類型</option>
        </select>
      </label>
      <p className="muted small">
        本小時已發送 {String(status?.sentThisHour ?? 0)} 則。
        {quietActive && quietUntil
          ? ` 安靜中，至 ${quietUntil.toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" })}。`
          : ""}
      </p>
      <div className="row-gap">
        <button
          onClick={async () => {
            setStatus(await api.proactiveDialogueQuiet(60));
          }}
        >
          一小時內不要主動說話
        </button>
        <button
          onClick={async () => {
            setStatus(await api.proactiveDialogueQuiet(12 * 60));
          }}
        >
          今天安靜一點
        </button>
      </div>
      {error && (
        <p className="cap-card-error" role="alert">
          {error}
        </p>
      )}
    </Section>
  );
}

/** 桌面角色設定（只在 Tauri 環境顯示）。 */
function CompanionSection() {
  const [dprefs, setDprefs] = React.useState<DesktopPrefs | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!isTauri) return;
    desktop
      .prefsGet()
      .then(setDprefs)
      .catch((e) => setError(String(e)));
  }, []);

  if (!isTauri) return null;
  if (!dprefs) {
    return error ? (
      <Section title="桌面角色">
        <p className="state-box state-error">無法載入設定：{error}</p>
      </Section>
    ) : null;
  }

  const patch = async (p: Partial<DesktopPrefs>) => {
    try {
      const updated = await desktop.prefsPatch(p);
      setDprefs(updated);
      await desktop.companionApplyPrefs();
      setError(null);
      setNotice("已套用。");
      setTimeout(() => setNotice(null), 2500);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Section title="桌面角色">
      <Toggle
        checked={dprefs.companionVisible}
        onChange={(on) => patch({ companionVisible: on })}
        label="顯示桌面角色（小樞）"
      />
      <div className="row wrap" style={{ marginTop: 8, alignItems: "flex-start" }}>
        <label className="field-label">
          外觀
          <select
            value={dprefs.companionPack}
            onChange={(e) => patch({ companionPack: e.target.value })}
          >
            <option value="shu-agile">小樞・靈巧型（貓系 v2，預設）</option>
            <option value="shu-lazy">小樞・慵懶型（貓系 v2）</option>
            <option value="shu-lively">小樞・活潑型（貓系 v2）</option>
            <option value="shu-standard">小樞・標準型（v1 經典）</option>
            <option value="shu-minimal">小樞・極簡型（v1 經典）</option>
          </select>
        </label>
        <label className="field-label">
          說話風格（Persona）
          <select
            value={dprefs.companionPersona}
            onChange={(e) => patch({ companionPersona: e.target.value })}
          >
            <option value="persona-shu">小樞・預設</option>
            <option value="persona-navigator">導航員（世界觀範例）</option>
          </select>
        </label>
        <label className="field-label">
          表現程度
          <select
            value={dprefs.companionExpressiveness}
            onChange={(e) => patch({ companionExpressiveness: e.target.value })}
          >
            <option value="quiet">安靜（只顯示安全訊息）</option>
            <option value="natural">自然</option>
            <option value="lively">活潑</option>
          </select>
        </label>
      </div>
      <Toggle
        checked={dprefs.companionAlwaysOnTop}
        onChange={(on) => patch({ companionAlwaysOnTop: on })}
        label="保持在其他視窗上方"
      />
      <p className="muted small">
        世界觀與說話風格只改變表達方式；緊急停止、被阻擋、結果未知等安全訊息
        永遠使用固定的標準文字，任何角色包都無法覆寫或隱藏。
      </p>
      <div className="row wrap">
        <button
          onClick={() => patch({ storyProgress: {} })}
          title="重看初次見面等劇情段落"
        >
          清除角色記憶（劇情進度）
        </button>
      </div>
      {notice && <p className="muted small" role="status">{notice}</p>}
      {error && <p className="cap-card-error" role="alert">{error}</p>}
    </Section>
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
