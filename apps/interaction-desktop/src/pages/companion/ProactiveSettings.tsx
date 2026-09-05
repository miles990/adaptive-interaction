// 主動式對話的完整設定（M3 §4.1：收合，摘要行仍看得到有效上限）。
//
// 這一層只有表單：狀態讀取與寫入（`api.proactiveDialoguePatch`）留在角色頁——
// 主動對話設定只有一個主人（見 regressions-v05 的守門測試）。
//
// 誠實：頻率上限與費用上限由後端強制執行；這裡顯示的數字就是有效值，
// 「今天已產生」與「本小時已發送」是後端回報的實際計數，不是估算。
// 寫入失敗的訊息刻意**不**畫在這裡：它的唯一的家在首屏的「陪伴方式」，
// 免得從收合區塊按下去失敗時連錯誤都被收起來。


export interface ProactiveConfig {
  mode: string;
  custom: Record<string, unknown>;
  maxPerHour: number;
  minIntervalMinutes: number;
  mergeWindowSeconds: number;
  noFollowUp: boolean;
  dndDefer: boolean;
  generativeAgent: string | null;
  dailyGenerativeSessions: number;
  dailyGenerativeCostUsd: number;
}

const CUSTOM_TRIGGERS: readonly [string, string][] = [
  ["taskProgress", "任務進度"],
  ["completion", "任務完成"],
  ["suggestion", "情境建議"],
  ["greeting", "問候"],
  ["companionship", "輕量陪伴"],
  ["worldEvent", "世界觀小事件"],
];

export function ProactiveSettings({
  name,
  advanced,
  config,
  agents,
  sentThisHour,
  generativeToday,
  onPatch,
  disabled = false,
}: {
  name: string;
  advanced: boolean;
  config: ProactiveConfig;
  /** 本機 AI 幫手的偵測結果（找不到就誠實顯示不可用，不會自動改送別家）。 */
  agents: { kind: string; label: string; detail: string }[];
  sentThisHour: number;
  generativeToday: { sessions: number; costUsd: number };
  onPatch: (value: Record<string, unknown>) => void;
  /**
   * 陪伴預設的兩段寫入正在進行（或正在補送）：整區鎖住（M4）。
   *
   * 檔位交易寫的就是這一區的「模式」。交易中途從這裡改同一個欄位，會讓補送用
   * 一份過時的意圖覆蓋掉使用者剛選的值，也會讓「使用者沒改過」的判斷失真
   *（`shouldResumePendingOp`）。首屏的表現程度與勿擾早就這樣鎖了，這一區是漏網的。
   */
  disabled?: boolean;
}) {
  return (
    <>
      {disabled && (
        <p className="muted small" role="status">
          正在套用陪伴預設，這一區暫時不能改；套用完成（或明確失敗）後會解鎖。
        </p>
      )}
      <p className="muted small">
        {name}什麼情況下可以主動說話。頻率限制（每小時最多 {config.maxPerHour} 則、最短間隔{" "}
        {config.minIntervalMinutes} 分鐘、沒有回覆不追問）由系統強制執行；
        安全與權限提示不受模式影響，一定會顯示。主動說話不代表可以主動做事——任何行動仍需授權。
      </p>
      <label className="field-label">
        模式
        <select
          value={config.mode}
          disabled={disabled}
          onChange={(e) => onPatch({ mode: e.target.value })}
        >
          <option value="off">關閉——不主動說話</option>
          <option value="necessary">必要——只有等待確認、失敗、結果不確定與感測提示</option>
          <option value="natural">自然（建議）——加上任務進度與低頻建議</option>
          <option value="lively">活潑——再加問候與輕量陪伴</option>
          <option value="custom">自訂——個別選擇訊息類型</option>
        </select>
      </label>
      {config.mode === "custom" && (
        <fieldset>
          <legend>自訂觸發類型</legend>
          {CUSTOM_TRIGGERS.map(([key, label]) => (
            <label className="row" key={key}>
              <input
                type="checkbox"
                checked={config.custom[key] === true}
                disabled={disabled}
                onChange={(event) => onPatch({ custom: { ...config.custom, [key]: event.target.checked } })}
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
            value={config.maxPerHour}
            disabled={disabled}
            onChange={(event) => onPatch({ maxPerHour: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          最短間隔（分鐘）
          <input
            type="number"
            min={1}
            max={60}
            value={config.minIntervalMinutes}
            disabled={disabled}
            onChange={(event) => onPatch({ minIntervalMinutes: Number(event.target.value) })}
          />
        </label>
        {/* 合併窗是調校參數，不是一般模式的選擇；只在進階模式出現。 */}
        {advanced && (
          <label className="field-label">
            事件合併窗（秒）
            <input
              type="number"
              min={5}
              max={300}
              value={config.mergeWindowSeconds}
              disabled={disabled}
              onChange={(event) => onPatch({ mergeWindowSeconds: Number(event.target.value) })}
            />
          </label>
        )}
      </div>
      <label className="row">
        <input
          type="checkbox"
          checked={config.noFollowUp}
          disabled={disabled}
          onChange={(e) => onPatch({ noFollowUp: e.target.checked })}
        />
        沒有回覆時不追問
      </label>
      <label className="row">
        <input
          type="checkbox"
          checked={config.dndDefer}
          disabled={disabled}
          onChange={(e) => onPatch({ dndDefer: e.target.checked })}
        />
        勿擾時段延後非必要訊息
      </label>
      <hr />
      <h4>由本機 AI 幫手產生的主動訊息</h4>
      <p className="muted small">
        沒有選擇 AI 幫手時只保留本機微反應與固定安全提示。選擇不會授予讀檔、工具、網路或行動權；
        每一則都是獨立、唯讀的一次性工作，不會留下長期工作。
      </p>
      <label className="field-label">
        指定 AI 幫手（不可用時不會自動改送另一家）
        <select
          value={config.generativeAgent ?? ""}
          disabled={disabled}
          onChange={(event) => onPatch({ generativeAgent: event.target.value || null })}
        >
          <option value="">不使用 AI 幫手產生主動訊息</option>
          <option value="codex" disabled>
            Codex（暫不支援：無法保證完全不用工具）
          </option>
          <option value="claude-code">Claude Code（對話、知識與審閱的本機 AI 幫手）</option>
        </select>
      </label>
      <div className="muted small">
        {agents.map((agent) => (
          <span key={agent.kind} className="character-agent-status">
            {agent.label}：{agent.detail}
          </span>
        ))}
      </div>
      <div className="settings-grid">
        <label className="field-label">
          每日產生次數上限
          <input
            type="number"
            min={0}
            max={50}
            value={config.dailyGenerativeSessions}
            disabled={disabled}
            onChange={(event) => onPatch({ dailyGenerativeSessions: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          每日費用上限（USD）
          <input
            type="number"
            min={0}
            max={100}
            step="0.1"
            value={config.dailyGenerativeCostUsd}
            disabled={disabled}
            onChange={(event) => onPatch({ dailyGenerativeCostUsd: Number(event.target.value) })}
          />
        </label>
      </div>
      <p className="muted small">
        今天已由 AI 幫手產生 {generativeToday.sessions} 則，費用回報 USD {generativeToday.costUsd}。
      </p>
      <p className="muted small">本小時已發送 {sentThisHour} 則。</p>
    </>
  );
}
