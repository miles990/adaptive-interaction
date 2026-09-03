// 工作頁 task-first 交代流程（一般模式第一屏）：
// 「想讓{角色}幫你做什麼？」→ 加入檔案或選擇資料夾 → 開始前預覽（使用哪個 Agent／
// 讀取範圍／是否寫入／工具／時間、訊息與費用上限／如何取消）→ 開始。
//
// 建立走 AiPage 同一條 api.agentSessionCreate 路徑：payload 由這裡的
// buildSessionCreateInput 產生，AiPage 的完整建立面板（進階／獨立）也用同一個函式，
// 權限語意（預設只讀取、寫入要第二次確認、scope 精確對應）只有一份，不在前端另寫政策。
// 誠實階梯：送出只宣稱「已送達」，不宣稱完成；做完後仍要人工檢查才有綠勾。

import React from "react";
import { AgentSessionRecord, api } from "../../api";
import { useAppState } from "../../appstate";
import { useCharacterName } from "../../characterName";
import type { BadgeKind } from "../../statusProjection";
import { isTauri } from "../../transport";
import { Badge, Section, useAsync } from "../../ui";

// ---------------------------------------------------------------------------
// 既有建立流程的預設值與純函式（AiPage 建立面板與 task-first 共用）
// ---------------------------------------------------------------------------

/** 既有建立面板的預設時間上限（分鐘）。 */
export const DEFAULT_TTL_MINUTES = 30;
/** 既有建立面板的預設費用上限（USD；Codex 依登入方案計費，不送上限）。 */
export const DEFAULT_MAX_COST_USD = 0.5;
/** 由任務描述自動產生名稱時的最長字數。 */
export const TASK_LABEL_MAX = 40;
/** 「現在」與首次成功體驗把要交代的內容放在這個 sessionStorage 鍵，工作頁讀完即清除。 */
export const WORK_PREFILL_KEY = "work.prefill";
/** 如何取消：一句話，固定文案。 */
export const CANCEL_SENTENCE =
  "開始後隨時可以按「暫停／中斷」或「關閉」；緊急停止會立刻終止；時間到自動結束。";

export type AgentId = "codex" | "claude-code";
export type AgentChoice = AgentId | "none";

/** 產品名可以留，但要有一句用途說明。 */
export const AGENT_PURPOSE: Record<AgentId, string> = {
  codex: "擅長程式實作、跑測試與修改檔案",
  "claude-code": "擅長長文件、歸納重點與規劃",
};

export function agentDisplayName(agentId: string): string {
  if (agentId === "codex") return "Codex";
  if (agentId === "claude-code") return "Claude Code";
  if (agentId === "none") return "不交給 Agent";
  return agentId;
}

export type WorkKind = "programming" | "conversation" | "knowledge" | "review";

/** 工作類型 → 精靈／工作設定裡的分工角色（agentRoutes 的 key）。 */
export const WORK_KIND_OPTIONS: { id: WorkKind; label: string; hint: string }[] = [
  { id: "conversation", label: "一般對話與文件", hint: "問問題、寫文件、做規劃" },
  { id: "programming", label: "程式工作", hint: "看程式碼、跑測試、修改檔案" },
  { id: "knowledge", label: "知識整理", hint: "整理資料、歸納重點" },
  { id: "review", label: "結果複審", hint: "檢查另一份結果對不對" },
];

export function isWorkKind(value: unknown): value is WorkKind {
  return WORK_KIND_OPTIONS.some((o) => o.id === value);
}

export function workKindLabel(kind: WorkKind): string {
  return WORK_KIND_OPTIONS.find((o) => o.id === kind)?.label ?? kind;
}

/** 與 appstate DEFAULT_PREFS／後端建議一致的預設分工。 */
const DEFAULT_ROUTES: Record<WorkKind, AgentChoice> = {
  conversation: "claude-code",
  programming: "codex",
  knowledge: "claude-code",
  review: "claude-code",
};

/** 依使用者的分工設定決定這種工作交給誰；設定缺席時用預設分工。 */
export function agentForKind(
  routes: Record<string, string> | undefined,
  kind: WorkKind
): AgentChoice {
  const value = routes?.[kind];
  if (value === "codex" || value === "claude-code" || value === "none") return value;
  return DEFAULT_ROUTES[kind];
}

/** 後端 agents_discoveries 的單筆結果（欄位缺席一律當作未知，不猜）。 */
export interface AgentDiscovery {
  kind?: unknown;
  found?: unknown;
  loggedIn?: unknown;
  detail?: unknown;
}

export function agentIdOfDiscovery(d: AgentDiscovery): AgentId {
  return d.kind === "codex" ? "codex" : "claude-code";
}

export function findDiscovery(data: unknown, agentId: AgentId): AgentDiscovery | undefined {
  const list = (data as { agents?: unknown } | undefined)?.agents;
  if (!Array.isArray(list)) return undefined;
  return (list as AgentDiscovery[]).find((a) => a && agentIdOfDiscovery(a) === agentId);
}

export interface AgentAvailability {
  label: string;
  badge: BadgeKind;
  /** true＝現在開始一定失敗（未安裝／未登入／已停用），介面先擋下並說原因。 */
  blocking: boolean;
  reason?: string;
}

/** 偵測結果 → 人話狀態。文案與 AiPage 的 Agent 卡片共用同一份。 */
export function agentAvailability(
  d: AgentDiscovery | undefined,
  disabled: boolean
): AgentAvailability {
  if (disabled) {
    return {
      label: "已停用",
      badge: "pending",
      blocking: true,
      reason: "你已停用這個 Agent；可以在「工作設定」重新啟用。",
    };
  }
  if (!d) return { label: "尚未偵測", badge: "muted", blocking: false };
  if (d.found !== true) {
    return {
      label: "未安裝",
      badge: "bad",
      blocking: true,
      reason: "電腦上找不到這個 Agent；請先安裝並登入，再按「重新偵測」。",
    };
  }
  if (d.loggedIn === true) return { label: "可用", badge: "ok", blocking: false };
  if (d.loggedIn === false) {
    return {
      label: "未登入",
      badge: "warn",
      blocking: true,
      reason: "這個 Agent 還沒登入；請先在它自己的程式裡登入，再按「重新偵測」。",
    };
  }
  return { label: "登入狀態未知", badge: "warn", blocking: false };
}

export interface SessionCreateDraft {
  agent: string;
  label: string;
  workdir: string;
  ttlMinutes: number;
  maxCost: number;
  allowWrite: boolean;
}

/** 唯一的建立 payload 來源：工作目錄→dataScope、寫入→tool／consent scope 精確對應。
 *  Codex 依登入方案計費，不送費用上限；費用 0 或負值＝不設上限（由後端政策決定）。 */
export function buildSessionCreateInput(draft: SessionCreateDraft): Record<string, unknown> {
  const workdir = draft.workdir.trim();
  return {
    agentId: draft.agent,
    label: draft.label || null,
    ttlMinutes: draft.ttlMinutes,
    maxCost: draft.agent === "codex" ? null : draft.maxCost > 0 ? draft.maxCost : null,
    workdir: workdir || null,
    allowWrite: draft.allowWrite,
    dataScope: workdir ? [`workspace:${workdir}`] : [],
    toolScope: draft.allowWrite ? ["workspace.write"] : [],
    consentScope: draft.allowWrite ? ["agent-session:workspace-write"] : [],
  };
}

/** 寫入的第二次確認：要有明確資料夾，而且使用者勾了「我已確認」。 */
export function writeConsentSatisfied(d: {
  allowWrite: boolean;
  workdir: string;
  writeConfirmed: boolean;
}): boolean {
  return !d.allowWrite || (d.workdir.trim().length > 0 && d.writeConfirmed);
}

/** 由任務描述取第一行當名稱（去多餘空白、限長）。 */
export function taskLabelFrom(task: string): string {
  const line =
    task
      .split(/\r?\n/)
      .map((s) => s.trim())
      .find(Boolean) ?? "";
  const collapsed = line.replace(/\s+/g, " ");
  const chars = Array.from(collapsed);
  return chars.length > TASK_LABEL_MAX ? `${chars.slice(0, TASK_LABEL_MAX).join("")}…` : collapsed;
}

export interface WorkPrefill {
  task: string;
  workdir: string;
  kind?: WorkKind;
}

/** 預填內容：接受純文字（＝要交代的話）或結構化 {task, workdir, kind}。 */
export function parseWorkPrefill(raw: string | null | undefined): WorkPrefill | null {
  if (typeof raw !== "string" || !raw.trim()) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === "object") {
      const obj = parsed as Record<string, unknown>;
      const task = typeof obj.task === "string" ? obj.task : "";
      const workdir = typeof obj.workdir === "string" ? obj.workdir : "";
      if (!task && !workdir) return null;
      return { task, workdir, kind: isWorkKind(obj.kind) ? obj.kind : undefined };
    }
    if (typeof parsed === "string") return parsed.trim() ? { task: parsed, workdir: "" } : null;
  } catch {
    /* 不是結構化內容：整段當作要交代的話 */
  }
  return { task: raw, workdir: "" };
}

function prefillStore(
  storage?: Pick<Storage, "getItem" | "removeItem"> | null
): Pick<Storage, "getItem" | "removeItem"> | null {
  if (storage !== undefined) return storage;
  try {
    return typeof window !== "undefined" ? window.sessionStorage : null;
  } catch {
    return null;
  }
}

/**
 * 只讀不清（給 useState 初始化器用）：React StrictMode 會把初始化器呼叫兩次並採用第二次結果，
 * 若在這裡就 removeItem，第二次會讀到空值、預填被吃掉（對抗審查／e2e 抓到的缺陷）。
 */
export function peekWorkPrefill(
  storage?: Pick<Storage, "getItem" | "removeItem"> | null
): WorkPrefill | null {
  const store = prefillStore(storage);
  if (!store) return null;
  try {
    return parseWorkPrefill(store.getItem(WORK_PREFILL_KEY));
  } catch {
    return null;
  }
}

/** 清掉預填（mount 之後呼叫；冪等）。 */
export function clearWorkPrefill(storage?: Pick<Storage, "getItem" | "removeItem"> | null): void {
  const store = prefillStore(storage);
  if (!store) return;
  try {
    store.removeItem(WORK_PREFILL_KEY);
  } catch {
    /* 被瀏覽器擋掉：下次 mount 會再讀到一次，屬可接受的降級 */
  }
}

/** 讀取並清除預填（一次性；沒有 storage 或被瀏覽器擋掉時安靜回 null）。 */
export function readWorkPrefill(
  storage?: Pick<Storage, "getItem" | "removeItem"> | null
): WorkPrefill | null {
  let store = storage;
  if (store === undefined) {
    try {
      store = typeof window !== "undefined" ? window.sessionStorage : null;
    } catch {
      store = null;
    }
  }
  if (!store) return null;
  try {
    const raw = store.getItem(WORK_PREFILL_KEY);
    const parsed = parseWorkPrefill(raw);
    if (raw !== null) store.removeItem(WORK_PREFILL_KEY);
    return parsed;
  } catch {
    return null;
  }
}

export type PickDirectoryResult =
  | { kind: "picked"; path: string }
  | { kind: "cancelled" }
  | { kind: "unavailable" };

/** 資料夾選擇器：只在 Tauri 且 host 有提供對話框指令時可用；沒有就誠實說沒有，
 *  讓使用者直接貼路徑。不新增任何依賴。 */
export async function pickDirectory(): Promise<PickDirectoryResult> {
  if (!isTauri) return { kind: "unavailable" };
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const picked = await invoke<unknown>("plugin:dialog|open", {
      options: { directory: true, multiple: false, title: "選擇資料夾" },
    });
    if (typeof picked === "string" && picked) return { kind: "picked", path: picked };
    if (Array.isArray(picked) && typeof picked[0] === "string" && picked[0]) {
      return { kind: "picked", path: picked[0] };
    }
    return { kind: "cancelled" };
  } catch {
    return { kind: "unavailable" };
  }
}

// ---------------------------------------------------------------------------
// 元件
// ---------------------------------------------------------------------------

export function TaskComposer({
  advanced = false,
  onCreated,
}: {
  advanced?: boolean;
  /** 建立成功（工作已存在）後通知外層重新載入清單；不代表任務已完成。 */
  onCreated?: (record: AgentSessionRecord) => void;
}) {
  const { prefs } = useAppState();
  const { name } = useCharacterName();
  const [prefill] = React.useState(() => peekWorkPrefill());
  // 讀完才清：與 StrictMode 的雙重初始化相容（初始化器不做副作用）。
  React.useEffect(() => {
    clearWorkPrefill();
  }, []);
  const [task, setTask] = React.useState(prefill?.task ?? "");
  const [workdir, setWorkdir] = React.useState(prefill?.workdir ?? "");
  const [kind, setKind] = React.useState<WorkKind>(
    prefill?.kind ?? (prefill?.workdir ? "programming" : "conversation")
  );
  const [allowWrite, setAllowWrite] = React.useState(false);
  const [writeConfirmed, setWriteConfirmed] = React.useState(false);
  const [ttl, setTtl] = React.useState(DEFAULT_TTL_MINUTES);
  const [maxCost, setMaxCost] = React.useState(DEFAULT_MAX_COST_USD);
  const [starting, setStarting] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [pickerHint, setPickerHint] = React.useState<string | null>(null);
  const [agents] = useAsync(
    () => api.agentsDiscoveries() as Promise<Record<string, unknown>>,
    []
  );

  const agentId = agentForKind(prefs.agentRoutes, kind);
  const kindLabel = workKindLabel(kind);
  const disabled = agentId !== "none" && (prefs.disabledAgents ?? []).includes(agentId);
  const availability: AgentAvailability | null =
    agentId === "none"
      ? null
      : agents.loading
        ? { label: "偵測中…", badge: "muted", blocking: false }
        : agentAvailability(findDiscovery(agents.data, agentId), disabled);
  const trimmedWorkdir = workdir.trim();
  const consentOk = writeConsentSatisfied({ allowWrite, workdir, writeConfirmed });
  const blockReason =
    agentId === "none"
      ? `你把「${kindLabel}」設定為不交給 Agent；換一種工作類型，或在「工作設定」調整分工。`
      : availability?.blocking
        ? `${agentDisplayName(agentId)}${availability.label}：${availability.reason ?? ""}`
        : null;
  const canStart = task.trim().length > 0 && !blockReason && consentOk && !starting;

  const pick = async () => {
    setPickerHint(null);
    const result = await pickDirectory();
    if (result.kind === "picked") setWorkdir(result.path);
    else if (result.kind === "unavailable") {
      setPickerHint("這個版本沒有資料夾選擇器；請直接貼上資料夾路徑。");
    }
  };

  const start = async () => {
    if (!canStart || agentId === "none") return;
    setStarting(true);
    setError(null);
    setNotice(null);
    const label = taskLabelFrom(task);
    const content = task.trim();
    try {
      const record = await api.agentSessionCreate(
        buildSessionCreateInput({
          agent: agentId,
          label,
          workdir: trimmedWorkdir,
          ttlMinutes: ttl,
          maxCost,
          allowWrite,
        })
      );
      let delivered = true;
      try {
        await api.agentSessionSend(record.sessionId, "task", { task: content });
      } catch (reason) {
        delivered = false;
        // 工作已存在但內容沒送到：不清空文字，讓使用者能從下方卡片再送一次。
        setError(`工作已建立，但內容沒能送出：${reason}。可以在下方的工作卡片再送一次。`);
      }
      if (delivered) {
        setNotice(
          `已交給 ${agentDisplayName(agentId)}：「${label}」。已送達、尚未完成；做完後會請你檢查結果。`
        );
        setTask("");
        setAllowWrite(false);
        setWriteConfirmed(false);
      }
      onCreated?.(record);
    } catch (reason) {
      setError(`沒能開始：${reason}`);
    } finally {
      setStarting(false);
    }
  };

  return (
    <Section title="交代一件工作">
      <div className="work-composer">
        <label className="work-question" htmlFor="work-task">
          想讓{name}幫你做什麼？
        </label>
        <textarea
          id="work-task"
          value={task}
          rows={4}
          placeholder="例：看一下這個資料夾的測試有沒有壞掉，跟我說結果就好。"
          onChange={(e) => setTask(e.target.value)}
        />
        <label className="field-label" htmlFor="work-folder">
          加入檔案或選擇資料夾
        </label>
        <div className="work-folder-row">
          <input
            id="work-folder"
            value={workdir}
            placeholder="貼上資料夾路徑（留空＝只用系統資料夾）"
            onChange={(e) => setWorkdir(e.target.value)}
          />
          {isTauri && (
            <button type="button" onClick={() => void pick()}>
              選擇資料夾…
            </button>
          )}
        </div>
        {pickerHint && <p className="muted small">{pickerHint}</p>}
        <label className="field-label" htmlFor="work-kind">
          這是哪一種工作
        </label>
        <div className="work-folder-row">
          <select id="work-kind" value={kind} onChange={(e) => setKind(e.target.value as WorkKind)}>
            {WORK_KIND_OPTIONS.map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
          <span className="muted small">{WORK_KIND_OPTIONS.find((o) => o.id === kind)?.hint}</span>
        </div>

        <div className="work-preview" role="group" aria-label="開始前預覽">
          <strong>開始前先看一下</strong>
          <dl className="work-preview-list">
            <dt>使用哪個 Agent</dt>
            <dd>
              {agentId === "none" ? (
                <span>
                  {agentDisplayName("none")}：你把「{kindLabel}」設定為不交給 Agent。
                </span>
              ) : (
                <>
                  <span>
                    <strong>{agentDisplayName(agentId)}</strong>{" "}
                    {availability && <Badge kind={availability.badge}>{availability.label}</Badge>}
                  </span>
                  <span className="muted small">
                    {AGENT_PURPOSE[agentId]}・依你的分工設定（{kindLabel}）
                  </span>
                </>
              )}
            </dd>
            <dt>讀取範圍</dt>
            <dd>
              {trimmedWorkdir
                ? `資料夾 ${trimmedWorkdir}（只讀取這個資料夾）`
                : "沒有指定資料夾：只在系統資料夾裡工作，不會讀取你的其他檔案。"}
            </dd>
            <dt>是否寫入</dt>
            <dd>
              <span>
                {allowWrite
                  ? writeConfirmed && trimmedWorkdir
                    ? "可以修改上面資料夾裡的檔案（你已確認）"
                    : "可以修改上面資料夾裡的檔案——還需要你再確認一次"
                  : "不寫入：只讀取，不修改任何檔案（預設）"}
              </span>
              <label className="work-consent">
                <input
                  type="checkbox"
                  checked={allowWrite}
                  onChange={(e) => {
                    setAllowWrite(e.target.checked);
                    if (!e.target.checked) setWriteConfirmed(false);
                  }}
                />
                <span>
                  允許修改這個資料夾裡的檔案
                  <span className="muted small" style={{ display: "block" }}>
                    不授予其他位置或網路；工作結束、逾時或緊急停止時立即失效。
                  </span>
                </span>
              </label>
              {allowWrite && !trimmedWorkdir && (
                <span className="risk-note">要允許修改，必須先指定資料夾。</span>
              )}
              {allowWrite && trimmedWorkdir && (
                <label className="work-consent">
                  <input
                    type="checkbox"
                    checked={writeConfirmed}
                    onChange={(e) => setWriteConfirmed(e.target.checked)}
                  />
                  <span>我已確認上面的資料夾，同意這次工作可以在裡面修改檔案。</span>
                </label>
              )}
            </dd>
            <dt>工具</dt>
            <dd>
              {allowWrite
                ? "讀取與修改這個資料夾裡的檔案；不含其他位置或網路。"
                : "只讀取檔案；不修改、不碰資料夾以外的位置。"}
            </dd>
            <dt>時間、訊息與費用上限</dt>
            <dd>
              <span>
                時間最多 {ttl} 分鐘・訊息則數依安全設定（開始後在工作卡片看得到）・費用
                {agentId === "codex"
                  ? "依 Codex 的登入方案計費（這裡不另設上限）"
                  : `最多 $${maxCost.toFixed(2)}`}
              </span>
              <details className="tech-details" style={{ marginTop: 4 }}>
                <summary className="muted small">調整時間與費用上限</summary>
                <div className="row wrap">
                  <label className="field-label">
                    時間上限（分鐘）
                    <input
                      type="number"
                      min={1}
                      max={240}
                      value={ttl}
                      onChange={(e) => setTtl(Math.max(1, Math.min(240, Number(e.target.value) || 1)))}
                    />
                  </label>
                  {agentId !== "codex" && (
                    <label className="field-label">
                      費用上限（USD）
                      <input
                        type="number"
                        min={0}
                        step={0.1}
                        value={maxCost}
                        onChange={(e) => setMaxCost(Math.max(0, Number(e.target.value) || 0))}
                      />
                    </label>
                  )}
                </div>
              </details>
            </dd>
            <dt>如何取消</dt>
            <dd>{CANCEL_SENTENCE}</dd>
          </dl>
          <p className="muted small">
            內容會送到該 Agent 的模型服務（依你在該 Agent 的登入方案計費）。到期、關閉或緊急停止時，權限立即失效。
          </p>
        </div>

        {blockReason && (
          <p className="risk-note" role="status">
            {blockReason}
          </p>
        )}
        {error && (
          <p className="cap-card-error" role="alert">
            {error}
          </p>
        )}
        <div className="row wrap">
          <button className="primary" disabled={!canStart} onClick={() => void start()}>
            {starting ? "開始中…" : "開始"}
          </button>
          {task.trim() && !starting && (
            <button
              onClick={() => {
                setTask("");
                setNotice(null);
                setError(null);
              }}
            >
              清空
            </button>
          )}
          {advanced && <span className="muted small">進階：工作卡片會顯示狀態碼與原始上限。</span>}
        </div>
        {notice && (
          <p className="muted small" role="status">
            {notice}
          </p>
        )}
      </div>
    </Section>
  );
}
