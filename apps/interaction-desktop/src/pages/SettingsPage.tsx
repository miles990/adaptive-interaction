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

      <ProactiveDialogueSection />

      <CompanionSection />

      <DesktopLifecycleSection />

      <Section title="資料備份、還原與重設">
        <p className="muted small">
          記憶頁提供可讀 JSON 備份、逐筆驗證還原、期限修改、匯出與刪除；原始素材及其衍生物會在刪除前顯示影響預覽。重新執行首次設定不會清除既有資料。
        </p>
        <div className="row wrap">
          <button onClick={() => onNavigate("memory")}>開啟匯出、還原與刪除</button>
          <button onClick={onRerunOnboarding}>重設互動設定精靈</button>
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

/** 主動式對話：模式與頻率由 Rust 確定性強制；此處只是設定介面。 */
function ProactiveDialogueSection() {
  const [status, setStatus] = React.useState<Record<string, unknown> | null>(null);
  const [agents, setAgents] = React.useState<Record<string, unknown>[]>([]);
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
    void api
      .agentsDiscoveries()
      .then((result) => setAgents((result.agents as Record<string, unknown>[] | undefined) ?? []))
      .catch(() => setAgents([]));
  }, [load]);

  const config = (status?.config as Record<string, unknown> | undefined) ?? {};
  const mode = String(config.mode ?? "natural");
  const custom = (config.custom as Record<string, unknown> | undefined) ?? {};
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
  const patch = async (value: Record<string, unknown>) => {
    try {
      setStatus(await api.proactiveDialoguePatch(value));
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
      {mode === "custom" && (
        <fieldset>
          <legend>自訂觸發類型</legend>
          {(
            [
              ["taskProgress", "任務進度"],
              ["completion", "任務完成"],
              ["suggestion", "情境建議"],
              ["greeting", "問候"],
              ["companionship", "輕量陪伴"],
              ["worldEvent", "世界觀小事件"],
            ] as const
          ).map(([key, label]) => (
            <label className="row" key={key}>
              <input
                type="checkbox"
                checked={custom[key] === true}
                onChange={(event) => void patch({ custom: { ...custom, [key]: event.target.checked } })}
              />
              {label}
            </label>
          ))}
        </fieldset>
      )}
      <div className="settings-grid">
        <label className="field-label">
          每小時最多則數
          <input
            type="number"
            min={1}
            max={12}
            value={Number(config.maxPerHour ?? 3)}
            onChange={(event) => void patch({ maxPerHour: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          最短間隔（分鐘）
          <input
            type="number"
            min={1}
            max={60}
            value={Number(config.minIntervalMinutes ?? 12)}
            onChange={(event) => void patch({ minIntervalMinutes: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          事件合併窗（秒）
          <input
            type="number"
            min={5}
            max={300}
            value={Number(config.mergeWindowSeconds ?? 30)}
            onChange={(event) => void patch({ mergeWindowSeconds: Number(event.target.value) })}
          />
        </label>
      </div>
      <label className="row">
        <input
          type="checkbox"
          checked={config.noFollowUp !== false}
          onChange={(event) => void patch({ noFollowUp: event.target.checked })}
        />
        沒有回覆時不追問
      </label>
      <label className="row">
        <input
          type="checkbox"
          checked={config.dndDefer !== false}
          onChange={(event) => void patch({ dndDefer: event.target.checked })}
        />
        勿擾時段延後非必要訊息
      </label>
      <hr />
      <h4>生成式主動訊息（本機 Agent）</h4>
      <p className="muted small">
        未選擇 Agent 時只保留本機微反應與固定安全提示。選擇不會授予讀檔、工具、網路或行動權；每次使用獨立唯讀 Session。
      </p>
      <label className="field-label">
        指定 Agent（不自動改送）
        <select
          value={String(config.generativeAgent ?? "")}
          onChange={(event) => void patch({ generativeAgent: event.target.value || null })}
        >
          <option value="">不使用生成式主動訊息</option>
          <option value="codex">Codex</option>
          <option value="claude-code">Claude Code</option>
        </select>
      </label>
      <div className="muted small">
        {agents.map((agent) => (
          <span key={String(agent.kind)} style={{ marginRight: 12 }}>
            {String(agent.kind)}：{agent.found === true && agent.loggedIn === true ? "可用" : String(agent.detail ?? "不可用")}
          </span>
        ))}
      </div>
      <div className="settings-grid">
        <label className="field-label">
          每日 Session 上限
          <input
            type="number"
            min={0}
            max={50}
            value={Number(config.dailyGenerativeSessions ?? 8)}
            onChange={(event) => void patch({ dailyGenerativeSessions: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          每日費用上限（USD）
          <input
            type="number"
            min={0}
            max={100}
            step="0.1"
            value={Number(config.dailyGenerativeCostUsd ?? 1)}
            onChange={(event) => void patch({ dailyGenerativeCostUsd: Number(event.target.value) })}
          />
        </label>
      </div>
      <p className="muted small">
        今日已建立 {String((status?.generativeToday as Record<string, unknown> | undefined)?.sessions ?? 0)} 個生成式 Session，費用回報 USD {String((status?.generativeToday as Record<string, unknown> | undefined)?.costUsd ?? 0)}。
      </p>
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
