// 工作（spec §16-1.D／§9）：本機 Agent 連接、分工偏好、工作卡片（誠實狀態階梯：
// Agent 說已完成 ≠ 已確認完成）、核可裁決、再交代、暫停／中斷、關閉。
//
// 兩種版面：
// - "task-first"（一般模式，由 WorkPage 搭配 TaskComposer 使用）：工作清單在前，
//   Agent 管理（偵測／登入／分工）收進折疊的「工作設定」；不出現建立面板、狀態碼、
//   原始上限與訊息 JSON。
// - "full"（進階模式／獨立使用）：Agent 卡片＋完整建立面板（ConsentSheet 語意）＋清單。
// 兩條建立路徑共用 work/TaskComposer 的 buildSessionCreateInput，不重複權限邏輯。

import React from "react";
import { AgentSessionRecord, api } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";
import { Dialog } from "../components/Dialog";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import {
  BadgeKind,
  isOpenWorkState,
  projectWorkState,
  WORK_STATE_PROJECTION,
  WORK_STATES,
  WorkState,
} from "../statusProjection";
import {
  AgentDiscovery,
  agentAvailability,
  agentDisplayName,
  agentIdOfDiscovery,
  buildSessionCreateInput,
  DEFAULT_MAX_COST_USD,
  DEFAULT_TTL_MINUTES,
} from "./work/TaskComposer";

/** 工作階段狀態的人話對照：由共用狀態投影（statusProjection.ts）導出的
 *  相容檢視（text／kind），供外部介面沿用，免得一般模式又冒出
 *  `claimed-completed` 這種原始字串。新增狀態一律改 statusProjection.ts，
 *  這裡不另外維護文案。 */
export const SESSION_STATE_LABEL: Record<WorkState, { text: string; kind: BadgeKind }> =
  Object.fromEntries(
    WORK_STATES.map((state) => [
      state,
      { text: WORK_STATE_PROJECTION[state].label, kind: WORK_STATE_PROJECTION[state].badge },
    ])
  ) as Record<WorkState, { text: string; kind: BadgeKind }>;

/** 後端 budget 另外回報的原始時間上限（舊後端沒有；缺席時退回租期長度）。 */
type BudgetWithDuration = AgentSessionRecord["budget"] & { maxDurationMs?: unknown };

/**
 * 「接續上次」要沿用的上限：時間、費用、訊息則數。
 *
 * 誠實／最小權限：省略欄位**不是**沿用——後端省略就落到預設（時間 120 分鐘、
 * 沒有金額上限、訊息吃 policy），每一項都比上次寬。所以這裡一律把上次的
 * 實際值算出來帶過去。
 */
export function resumeLimits(record: AgentSessionRecord): {
  ttlMinutes: number;
  maxCost: number;
  maxMessages: number;
} {
  const budget = record.budget as BudgetWithDuration;
  const fromBudget =
    typeof budget.maxDurationMs === "number" && budget.maxDurationMs > 0
      ? Math.round(budget.maxDurationMs / 60000)
      : 0;
  const leaseSpan = Math.round(
    (Date.parse(record.lease.expiresAt) - Date.parse(record.lease.issuedAt)) / 60000
  );
  const ttlMinutes = Math.max(
    1,
    fromBudget || (Number.isFinite(leaseSpan) && leaseSpan > 0 ? leaseSpan : DEFAULT_TTL_MINUTES)
  );
  return {
    ttlMinutes,
    maxCost: typeof record.budget.maxCost === "number" ? record.budget.maxCost : 0,
    maxMessages: record.budget.maxMessages,
  };
}

/**
 * 接續要用的資料夾，以及「這個資料夾是不是後端確認過的事實」。
 *
 * 誠實階梯：後端記錄的 `resolvedWorkdir` 是上一次**真的**掛上子程序的目錄
 * （正規化後的絕對路徑）；`dataScope` 裡的 `workspace:` 只是呼叫端自己附加
 * 的人話標籤，兩者不一致時以後端的事實為準。只有標籤、沒有記錄時（升級前
 * 建立的舊 session）不得假裝確認過——後端也會據此保守拒絕。
 */
export function resumeWorkdir(record: AgentSessionRecord): {
  path?: string;
  confirmed: boolean;
} {
  const recorded = record.resolvedWorkdir;
  if (typeof recorded === "string" && recorded.length > 0) {
    return { path: recorded, confirmed: true };
  }
  const labelled = record.dataScope
    .find((s) => s.startsWith("workspace:"))
    ?.slice("workspace:".length);
  return { path: labelled, confirmed: false };
}

/**
 * 「接續上次（唯讀）」送出的建立內容。
 *
 * 不變量：接續**不得放寬**上次的範圍——
 * - 資料夾沿用上次那一個（同時寫回 `dataScope`，下一次接續才找得到，
 *   不會退回沒有資料夾、由後端自行決定的狀態）；
 * - 修改權限一律關閉，工具與使用授權不繼承（要修改檔案就重新授權一次）；
 * - 時間／費用／訊息上限沿用上次的實際值，不落到更寬的預設。
 */
export function buildResumeInput(record: AgentSessionRecord): Record<string, unknown> {
  const workspaceScope = record.dataScope.filter((s) => s.startsWith("workspace:"));
  const limits = resumeLimits(record);
  return {
    agentId: record.agentId,
    label: `接續：${record.label ?? agentDisplayName(record.agentId)}`,
    workdir: resumeWorkdir(record).path ?? null,
    dataScope: workspaceScope,
    toolScope: [],
    consentScope: [],
    allowWrite: false,
    ttlMinutes: limits.ttlMinutes,
    maxCost: limits.maxCost,
    maxMessages: limits.maxMessages,
    resumeProviderSessionId: record.providerSessionId,
  };
}

/**
 * 送出的訊息是否**真的**送到 Agent 了。
 *
 * 誠實階梯（dispatched ≠ acknowledged）：後端只有在訊息真的寫進 Agent
 * 子程序時才蓋 `deliveredAt`。輪詢型 Agent（尚未來取）、子程序已經不再
 * 接收的情況都會回一則沒有戳記的訊息——那是「已排進信箱」，不是「已送達」。
 */
export function deliveredToAgent(message: unknown): boolean {
  const at = (message as { deliveredAt?: unknown } | null | undefined)?.deliveredAt;
  return typeof at === "string" && at.length > 0;
}

/** 接續時實際沿用了什麼——說出來，不讓人以為只是「同一段對話」。 */
export function resumeLimitsText(record: AgentSessionRecord): string {
  const limits = resumeLimits(record);
  const folder = resumeWorkdir(record);
  const parts = [
    folder.path
      ? `原本的資料夾（${folder.path}${folder.confirmed ? "" : "；未確認"}）`
      : "不指定資料夾",
    `${limits.ttlMinutes} 分鐘`,
  ];
  if (limits.maxCost > 0) parts.push(`最多 US$${limits.maxCost}`);
  return `沿用${parts.join("、")}`;
}

/** 訊息輪詢間隔：展開時才跑，收合／卸載即停（有界，不會長駐）。 */
export const MESSAGE_POLL_MS = 5000;
/** 後端 gateway 的 approval TTL（gateway.rs APPROVAL_TTL_SECS）。 */
export const APPROVAL_TTL_SECONDS = 300;

export type AiPageLayout = "full" | "task-first";

/** 後端可能額外回報的 claim 對應欄位（v0.5 多輪工作階段；舊後端沒有）。 */
type ClaimScopedRecord = AgentSessionRecord & {
  /** 目前這一輪 claim 的 id（每次 task-started／claimed-completed 換新）。 */
  claimId?: unknown;
  /** 人工驗證對應的 claim id。 */
  humanVerifiedClaimId?: unknown;
  humanVerified?: AgentSessionRecord["humanVerified"] & { claimId?: unknown };
};

/**
 * 這一輪的 claim 是否已由人親自確認（綠勾的唯一來源）。
 * 誠實階梯：human_verified 是 session 層級旗標，但 session 可多輪——
 * - 只有 state 仍是 claimed-completed 時才可能是「已確認完成」；Active／等待中的
 *   第二輪不得沿用上一輪的綠勾；
 * - 後端若回報 claim id（`humanVerifiedClaimId`／`humanVerified.claimId` 對 `claimId`），
 *   兩者必須相同；
 * - 舊後端沒有 claim id：退回 state==claimed-completed && humanVerified。
 */
export function verifiedForCurrentClaim(record: AgentSessionRecord): boolean {
  if (record.state !== "claimed-completed" || !record.humanVerified) return false;
  const scoped = record as ClaimScopedRecord;
  const verifiedClaim =
    typeof scoped.humanVerifiedClaimId === "string"
      ? scoped.humanVerifiedClaimId
      : typeof scoped.humanVerified?.claimId === "string"
        ? scoped.humanVerified.claimId
        : null;
  const currentClaim = typeof scoped.claimId === "string" ? scoped.claimId : null;
  if (verifiedClaim !== null && currentClaim !== null) return verifiedClaim === currentClaim;
  return true;
}

const MESSAGE_KIND_LABEL: Record<string, string> = {
  task: "你交代的內容",
  "approval-request": "等待你核可",
  "approval-resolved": "核可結果",
  progress: "進度更新",
  result: "結果回報",
  error: "錯誤",
  note: "備註",
  text: "訊息",
  question: "Agent 的提問",
};

const ROUTE_ROLES: [string, string][] = [
  ["conversation", "一般對話與文件"],
  ["programming", "程式工作"],
  ["knowledge", "知識整理"],
  ["review", "結果複審"],
];

/** 一則已裁決的核可請求（後端 `approval-resolved` 訊息）。 */
export interface ApprovalResolution {
  decision: string;
  by: string;
  deliveredToAgent: boolean;
}

/** 把信箱裡的 `approval-resolved` 收成 requestId → 裁決結果。
 *  沒有這張表，已經失效的請求會繼續顯示可按的核可／拒絕按鈕。 */
export function approvalResolutions(
  messages: Record<string, unknown>[]
): Map<string, ApprovalResolution> {
  const out = new Map<string, ApprovalResolution>();
  for (const m of messages) {
    if (m.kind !== "approval-resolved") continue;
    const body = (m.body as Record<string, unknown> | undefined) ?? {};
    const requestId = String(body.requestId ?? "");
    if (!requestId) continue;
    out.set(requestId, {
      decision: String(body.decision ?? (body.approved === true ? "approved" : "denied")),
      by: String(body.by ?? ""),
      deliveredToAgent: body.deliveredToAgent !== false,
    });
  }
  return out;
}

/** 已裁決請求的人話說明。誰決定的要說清楚：逾時自動拒絕不是「你拒絕了」。 */
export function approvalResolutionText(r: ApprovalResolution): string {
  const base =
    r.by === "watchdog"
      ? `已由看門狗自動拒絕（${APPROVAL_TTL_SECONDS} 秒無人回應）`
      : r.by === "human"
        ? r.decision === "approved"
          ? "你已核可"
          : "你已拒絕"
        : r.decision === "approved"
          ? "已核可"
          : "已拒絕";
  // 裁決成立 ≠ 裁決送到 agent：送不到就照實說，不假裝已經生效。
  return r.deliveredToAgent ? base : `${base}（沒能送到 agent，實際結果未知）`;
}

/** 從結構化 body 取一句人話摘要；沒有文字就誠實說沒有。 */
export function messageSummary(body: unknown): string {
  if (!body || typeof body !== "object") return "（沒有文字內容）";
  const record = body as Record<string, unknown>;
  for (const key of ["summary", "message", "text", "task", "detail", "reason", "result"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  const tool = record["tool"] as Record<string, unknown> | undefined;
  if (tool && typeof tool["name"] === "string") {
    return `使用工具 ${String(tool["name"])}${tool["phase"] ? `（${String(tool["phase"])}）` : ""}`;
  }
  if (typeof record["artifact"] === "string") return `產生檔案 ${String(record["artifact"])}`;
  return "（沒有文字內容）";
}

export function AiPage({
  refreshKey,
  advanced = false,
  onNavigate,
  layout = "full",
  settingsOpen,
  onSettingsToggle,
}: {
  refreshKey: number;
  advanced?: boolean;
  onNavigate: (tab: string) => void;
  layout?: AiPageLayout;
  /** task-first 版面的「工作設定」折疊區是否展開（外層可控；缺席時自管）。 */
  settingsOpen?: boolean;
  onSettingsToggle?: (open: boolean) => void;
}) {
  const { prefs, setPreferences } = useAppState();
  const [agents, retryAgents] = useAsync(
    () => api.agentsDiscoveries() as Promise<Record<string, unknown>>,
    [refreshKey]
  );
  const [sessions] = useAsync(() => api.agentSessionsList(), [refreshKey]);
  const [createOpen, setCreateOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [localSettingsOpen, setLocalSettingsOpen] = React.useState(false);
  const settingsIsOpen = settingsOpen ?? localSettingsOpen;
  const setSettingsIsOpen = onSettingsToggle ?? setLocalSettingsOpen;

  const agentPanel = (
    <>
      <p className="muted small">
        直接使用你電腦上已安裝、已登入的 Codex 與 Claude Code。系統不讀取、不保存它們的登入憑證；
        登入由各 Agent 自己管理。工作預設<strong>只讀取、不修改</strong>；只有你在開始前的預覽中
        明確同意時，才會允許修改指定資料夾裡的檔案。
      </p>
      <StateView state={agents} empty="尚未偵測。">
        {(data) => (
          <div className="provider-list">
            {((data.agents as AgentDiscovery[] | undefined) ?? []).map((a) => {
              const agentId = agentIdOfDiscovery(a);
              const disabled = (prefs.disabledAgents ?? []).includes(agentId);
              const availability = agentAvailability(a, disabled);
              const usable = !disabled && a.found === true && a.loggedIn === true;
              return (
                <div className="provider-card" key={agentId}>
                  <div className="row space-between">
                    <strong>{agentDisplayName(agentId)}</strong>
                    <Badge kind={availability.badge}>{availability.label}</Badge>
                  </div>
                  <div className="muted small">{String(a.detail ?? "")}</div>
                  <div className="row wrap" style={{ marginTop: 8 }}>
                    <button
                      onClick={async () => {
                        try {
                          await api.agentsRefresh();
                          retryAgents();
                          setNotice(
                            usable
                              ? `${agentDisplayName(agentId)} 前置檢查通過（已安裝且已登入；還沒開始任何會計費的工作）。`
                              : "前置檢查未通過；請查看版本與登入狀態。"
                          );
                        } catch (reason) {
                          setNotice(`連線測試失敗：${reason}`);
                        }
                      }}
                    >
                      測試連線
                    </button>
                    <button
                      onClick={async () => {
                        const current = new Set(prefs.disabledAgents ?? []);
                        if (disabled) current.delete(agentId);
                        else current.add(agentId);
                        await setPreferences({ disabledAgents: [...current] });
                        setNotice(
                          disabled
                            ? `${agentDisplayName(agentId)} 已啟用。`
                            : `${agentDisplayName(agentId)} 已停用；之後不會再把新工作交給它。`
                        );
                      }}
                    >
                      {disabled ? "啟用" : "停用"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </StateView>
      <div className="row wrap">
        <button
          onClick={async () => {
            await api.agentsRefresh().catch(() => {});
            retryAgents();
          }}
        >
          重新偵測
        </button>
        {layout === "full" && (
          <button className="primary" onClick={() => setCreateOpen(true)}>
            建立工作階段…
          </button>
        )}
      </div>
      <p className="muted small">
        分工建議：程式實作、測試與修改檔案 → Codex；長文件、歸納與規劃 → Claude Code。
        指定的 Agent 不可用時不會自動改交給另一個。
      </p>
    </>
  );

  const routingPanel = (
    <>
      <p className="muted small">
        每種工作交給誰。開始前仍會顯示讀取範圍、工具、費用與時間上限；指定的 Agent 不可用或已停用時不會自動改交給另一個。
      </p>
      <div className="settings-grid">
        {ROUTE_ROLES.map(([role, label]) => (
          <label className="field-label" key={role}>
            {label}
            <select
              value={prefs.agentRoutes?.[role] ?? (role === "programming" ? "codex" : "claude-code")}
              onChange={(event) =>
                void setPreferences({
                  agentRoutes: {
                    ...(prefs.agentRoutes ?? {}),
                    [role]: event.target.value as "codex" | "claude-code" | "none",
                  },
                })
              }
            >
              <option value="codex">Codex</option>
              <option value="claude-code">Claude Code</option>
              <option value="none">不交給 Agent</option>
            </select>
          </label>
        ))}
      </div>
    </>
  );

  const sessionsSection = (
    <Section title="進行中與最近的工作">
      <StateView state={sessions} empty="目前沒有交代中的工作。">
        {(list) => (
          <div className="provider-list">
            {list.map((s) => (
              <SessionCard
                key={s.sessionId}
                record={s}
                advanced={advanced}
                onNotice={setNotice}
                onNavigate={onNavigate}
              />
            ))}
          </div>
        )}
      </StateView>
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
    </Section>
  );

  if (layout === "task-first") {
    return (
      <div>
        {sessionsSection}
        <details
          className="work-settings"
          open={settingsIsOpen}
          onToggle={(e) => setSettingsIsOpen((e.currentTarget as HTMLDetailsElement).open)}
        >
          <summary>工作設定：本機 AI Agent 與分工</summary>
          <div className="work-settings-body">
            <h3>本機 AI Agent</h3>
            {agentPanel}
            <h3>每種工作交給誰</h3>
            {routingPanel}
          </div>
        </details>
      </div>
    );
  }

  return (
    <div>
      <Section title="本機 AI Agent">{agentPanel}</Section>
      <Section title="每種工作交給誰">{routingPanel}</Section>
      {sessionsSection}
      {createOpen && <CreateSessionSheet onClose={() => setCreateOpen(false)} />}
    </div>
  );
}

function SessionCard({
  record,
  advanced,
  onNotice,
  onNavigate,
}: {
  record: AgentSessionRecord;
  advanced: boolean;
  onNotice: (m: string) => void;
  onNavigate: (tab: string) => void;
}) {
  const { name: characterName } = useCharacterName();
  const [expanded, setExpanded] = React.useState(false);
  const [messages, setMessages] = React.useState<Record<string, unknown>[]>([]);
  const [task, setTask] = React.useState("");
  // 延長有效期＝連同「可修改資料夾」的權限一起延長，所以可寫入的工作要再問一次。
  const [renewConfirm, setRenewConfirm] = React.useState(false);
  const resolutions = React.useMemo(() => approvalResolutions(messages), [messages]);
  // 一般模式永遠是人話；介面不認得的狀態投影成「結果不確定」，
  // 原始狀態碼只在進階模式的次要行出現。
  // 人工驗證是後端的獨立欄位（state 仍是 claimed-completed）：只有它對應目前這一輪
  // claim 時才投影成「已確認完成」；沒有它，Agent 的說法永遠只是說法。
  const verified = verifiedForCurrentClaim(record);
  const status = verified ? projectWorkState("verified") : projectWorkState(record.state);
  const claimed = record.state === "claimed-completed" && !verified;
  // 「進行中」與 Rust `AgentSessionState::is_open` 對齊（statusProjection）：
  // failed／unknown／timed-out 都是終局，續租／中斷／再交代必定失敗，不能顯示；
  // 反過來，終局但帶 providerSessionId 的才可「接續上次」。
  const open = isOpenWorkState(record.state);
  // 失敗／結果不確定／逾時是終局但後端仍接受「關閉」（收進歷史）；已關閉／取消／到期則沒得關。
  const closable = open || ["failed", "unknown", "timed-out"].includes(record.state);

  // 展開時持續輪詢：Agent 是非同步在跑的，只抓一次會讓等待核可、
  // 進度與結果永遠停在打開的那一瞬間。收合或離開頁面立即停止。
  React.useEffect(() => {
    if (!expanded) return;
    let alive = true;
    const load = () => {
      api
        .agentSessionMessages(record.sessionId, "from-session")
        .then((list) => {
          if (alive) setMessages(list);
        })
        .catch(() => {
          /* session gone or transient failure: keep last known list */
        });
    };
    load();
    const timer = setInterval(load, MESSAGE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [expanded, record.sessionId]);

  return (
    <div className="provider-card">
      <div className="row space-between">
        <strong>{record.label ?? agentDisplayName(record.agentId)}</strong>
        <Badge kind={status.badge}>
          {verified ? "✓ " : ""}
          {status.label}
        </Badge>
      </div>
      <div className="muted small">
        {agentDisplayName(record.agentId)}
        {advanced ? `・狀態碼 ${record.state}` : ""}
        {record.allowWrite ? "・可修改資料夾裡的檔案" : "・只讀取，不修改"}
        {record.providerSessionId
          ? advanced
            ? `・provider session ${record.providerSessionId.slice(0, 8)}…`
            : "・沿用既有對話脈絡"
          : ""}
        {advanced
          ? `・訊息 ${record.budget.spentMessages}/${record.budget.maxMessages}${
              record.budget.maxCost > 0
                ? `・費用 $${record.budget.spentCost.toFixed(3)}/$${record.budget.maxCost.toFixed(2)}`
                : record.budget.spentCost > 0
                  ? `・費用 $${record.budget.spentCost.toFixed(3)}`
                  : ""
            }`
          : record.budget.maxCost > 0
            ? `・費用上限 $${record.budget.maxCost.toFixed(2)}`
            : ""}
        ・有效至 {new Date(record.lease.expiresAt).toLocaleString("zh-TW")}
        {advanced ? `・id ${record.sessionId}` : ""}
      </div>
      {claimed && (
        <p className="muted small">
          Agent 說做完了——這是<strong>它的說法</strong>，尚未經過檢查。
          請實際查看結果；確認無誤後按「標記為已驗證」，{characterName}才會顯示綠色勾勾。
        </p>
      )}
      {record.humanVerified && verified && (
        <p className="muted small" role="status">
          {status.honesty ?? "由你親自確認"}・
          {new Date(record.humanVerified.at).toLocaleString("zh-TW")}
          {record.humanVerified.note ? `・${record.humanVerified.note}` : ""}
        </p>
      )}
      {record.humanVerified && !verified && (
        <p className="muted small">
          先前一輪的結果你在 {new Date(record.humanVerified.at).toLocaleString("zh-TW")} 確認過；
          這一輪尚未檢查，不沿用綠色勾勾。
        </p>
      )}
      <div className="row wrap">
        {claimed && (
          <button
            onClick={async () => {
              try {
                await api.agentSessionVerify(record.sessionId);
                onNotice("已標記為已驗證（由你人工確認）。");
              } catch (e) {
                onNotice(`驗證失敗：${e}。狀態未變更。`);
              }
            }}
          >
            標記為已驗證（我確認過結果）
          </button>
        )}
        <button onClick={() => setExpanded((v) => !v)}>
          {expanded ? "收合" : "查看結果／訊息"}
        </button>
        {open && (
          <>
            {record.lease.renewable && (
              <>
                <button
                  onClick={async () => {
                    // 可寫入的工作：第一次點擊只是提出要求，要再確認一次才真的延長。
                    if (record.allowWrite && !renewConfirm) {
                      setRenewConfirm(true);
                      return;
                    }
                    setRenewConfirm(false);
                    try {
                      const renewed = await api.agentSessionRenew(record.sessionId, 30);
                      onNotice(
                        `已續租至 ${new Date(renewed.lease.expiresAt).toLocaleString("zh-TW")}。`
                      );
                    } catch (e) {
                      onNotice(`續租失敗：${e}。有效期間未變更。`);
                    }
                  }}
                >
                  {record.allowWrite && renewConfirm
                    ? "確認延長（含修改權限）"
                    : "續租 30 分鐘"}
                </button>
                {record.allowWrite && renewConfirm && (
                  <>
                    <button onClick={() => setRenewConfirm(false)}>不延長</button>
                    <span className="risk-note" role="status">
                      延長 30 分鐘會連同「可修改{" "}
                      {record.dataScope
                        .find((s) => s.startsWith("workspace:"))
                        ?.slice("workspace:".length) ?? "系統資料夾"}{" "}
                      裡的檔案」一起延長。
                    </span>
                  </>
                )}
              </>
            )}
            <button
              onClick={async () => {
                try {
                  await api.agentSessionInterrupt(record.sessionId);
                  onNotice("已送出中斷指令。");
                } catch (e) {
                  onNotice(`中斷失敗：${e}`);
                }
              }}
            >
              暫停／中斷目前工作
            </button>
          </>
        )}
        {closable && (
          <button
            className="danger"
            onClick={async () => {
              try {
                await api.agentSessionClose(record.sessionId, "closed");
                // 誠實階梯：關閉是 receipt-backed 事實；子程序終止在後端是
                // 非同步背景工作，此刻只能宣稱「已要求」，不得宣稱「已終止」。
                onNotice("工作階段已關閉（已要求終止子程序）。");
              } catch (e) {
                onNotice(`關閉失敗：${e}`);
              }
            }}
          >
            關閉
          </button>
        )}
        {!open && record.providerSessionId && (
          <button
            onClick={async () => {
              try {
                await api.agentSessionCreate(buildResumeInput(record));
                // 誠實：接續＝新的唯讀工作沿用先前的對話脈絡與**上次的上限**；
                // 寫入權限不繼承，需要時另外在開始前同意。
                onNotice(
                  `已接續上次的工作（只讀取；沿用先前的對話脈絡、${resumeLimitsText(record)}）。`
                );
              } catch (e) {
                onNotice(`接續失敗：${e}`);
              }
            }}
          >
            接續上次（唯讀）
          </button>
        )}
        <button onClick={() => onNavigate("activity")}>查看紀錄</button>
      </div>
      {expanded && (
        <div className="session-detail">
          {open && (
            <div className="row wrap">
              <input
                value={task}
                placeholder="再交代一句給這個 Agent…"
                onChange={(e) => setTask(e.target.value)}
              />
              <button
                disabled={!task.trim()}
                onClick={async () => {
                  try {
                    const sent = await api.agentSessionSend(record.sessionId, "task", { task });
                    setTask("");
                    // 誠實階梯：只有後端蓋了送達戳記才算「已送達」。沒有戳記
                    // ＝訊息還在信箱裡（輪詢型 Agent 尚未取走，或子程序已經
                    // 不再接收），不得一律宣稱送達。
                    onNotice(
                      deliveredToAgent(sent)
                        ? "已送出（已送達 Agent，尚未完成）。"
                        : "已放進信箱，尚未送達 Agent（Agent 取走後才會開始）。"
                    );
                  } catch (e) {
                    onNotice(`送出失敗：${e}`);
                  }
                }}
              >
                送出
              </button>
            </div>
          )}
          {messages.length === 0 ? (
            <div className="state-box">Agent 尚未回報任何結果。</div>
          ) : (
            <ul className="plain-list">
              {messages.map((m) => (
                <li key={String(m.messageId)}>
                  <strong>
                    {MESSAGE_KIND_LABEL[String(m.kind)] ?? (advanced ? String(m.kind) : "訊息")}
                  </strong>
                  {m.kind === "approval-request" &&
                    (() => {
                      const requestId = String(
                        (m.body as Record<string, unknown> | undefined)?.requestId ?? ""
                      );
                      const decided = resolutions.get(requestId);
                      // 已裁決（人類或看門狗）的請求後端已經不認：按鈕留著
                      // 只會讓人按到 NotFound，畫面必須說出實際結果。
                      return decided ? (
                        <span className="muted small">　{approvalResolutionText(decided)}</span>
                      ) : (
                        <>
                          <ApprovalControls
                            sessionId={record.sessionId}
                            body={m.body as Record<string, unknown>}
                            onNotice={onNotice}
                          />
                          <ApprovalCountdown createdAt={String(m.createdAt ?? "")} />
                        </>
                      );
                    })()}
                  <div className="muted small">{messageSummary(m.body)}</div>
                  {advanced && (
                    <details className="tech-details">
                      <summary className="muted small">技術詳情</summary>
                      <pre className="json-view small">{JSON.stringify(m.body, null, 2)}</pre>
                    </details>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

/** 核可請求的剩餘時間：後端 TTL 到期後即失效，介面不得假裝還能按。 */
export function ApprovalCountdown({ createdAt }: { createdAt: string }) {
  const deadline = React.useMemo(() => {
    const started = new Date(createdAt).getTime();
    return Number.isFinite(started) ? started + APPROVAL_TTL_SECONDS * 1000 : NaN;
  }, [createdAt]);
  const remaining = React.useCallback(
    () => Math.max(0, Math.round((deadline - Date.now()) / 1000)),
    [deadline]
  );
  const [secs, setSecs] = React.useState(remaining);
  React.useEffect(() => {
    if (!Number.isFinite(deadline)) return;
    setSecs(remaining());
    const timer = setInterval(() => setSecs(remaining()), 1000);
    return () => clearInterval(timer);
  }, [deadline, remaining]);
  if (!Number.isFinite(deadline)) {
    return <span className="muted small">　（無法確認剩餘時間）</span>;
  }
  return (
    <span className="muted small">
      　{secs > 0 ? `還有 ${secs} 秒可以決定` : "已超過決定時間，這個請求已失效"}
    </span>
  );
}

function ApprovalControls({
  sessionId,
  body,
  onNotice,
}: {
  sessionId: string;
  body: Record<string, unknown>;
  onNotice: (m: string) => void;
}) {
  const requestId = String(body?.requestId ?? "");
  if (!requestId) return null;
  return (
    <span className="row" style={{ display: "inline-flex", gap: 6, marginLeft: 8 }}>
      <button
        className="primary"
        onClick={async () => {
          try {
            await api.agentSessionApprove(sessionId, requestId, true);
            onNotice("已核可該請求。");
          } catch (e) {
            onNotice(`核可失敗：${e}`);
          }
        }}
      >
        核可
      </button>
      <button
        onClick={async () => {
          try {
            await api.agentSessionApprove(sessionId, requestId, false);
            onNotice("已拒絕該請求。");
          } catch (e) {
            onNotice(`拒絕失敗：${e}`);
          }
        }}
      >
        拒絕
      </button>
    </span>
  );
}

/** 完整建立面板（進階／獨立使用；Consent Sheet 語意：誰／做什麼／資料／成本／時間／如何取消）。
 *  一般模式的 task-first 流程改用 work/TaskComposer；兩者的 payload 同一個函式產生。 */
function CreateSessionSheet({ onClose }: { onClose: () => void }) {
  const { prefs } = useAppState();
  const disabledAgents = prefs.disabledAgents ?? [];
  const [agent, setAgent] = React.useState(
    disabledAgents.includes("claude-code") ? "codex" : "claude-code"
  );
  const [label, setLabel] = React.useState("");
  const [workdir, setWorkdir] = React.useState("");
  const [ttl, setTtl] = React.useState(DEFAULT_TTL_MINUTES);
  const [maxCost, setMaxCost] = React.useState(DEFAULT_MAX_COST_USD);
  const [allowWrite, setAllowWrite] = React.useState(false);
  const [writeConfirmed, setWriteConfirmed] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [creating, setCreating] = React.useState(false);
  const [routing, setRouting] = React.useState<Record<string, unknown> | null>(null);

  React.useEffect(() => {
    void api.agentsRouting().then(setRouting).catch(() => {});
  }, []);

  return (
    <Dialog title="建立 AI 工作階段" onClose={onClose}>
      <div className="consent-sheet">
        <label className="field-label">
          誰來做（Agent）
          <select value={agent} onChange={(e) => setAgent(e.target.value)}>
            <option value="claude-code" disabled={disabledAgents.includes("claude-code")}>
              Claude Code（文件／歸納／規劃）
            </option>
            <option value="codex" disabled={disabledAgents.includes("codex")}>
              Codex（程式／測試／Patch）
            </option>
          </select>
        </label>
        <label className="field-label">
          做什麼（名稱）
          <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="例：看一下這個 repo 的測試" />
        </label>
        <label className="field-label">
          工作目錄{allowWrite ? "（可寫範圍）" : "（唯讀範圍）"}
          <input value={workdir} onChange={(e) => setWorkdir(e.target.value)} placeholder="/path/to/project（留空＝系統資料夾）" />
        </label>
        <label className="row" style={{ alignItems: "flex-start" }}>
          <input
            type="checkbox"
            checked={allowWrite}
            onChange={(e) => {
              setAllowWrite(e.target.checked);
              if (!e.target.checked) setWriteConfirmed(false);
            }}
          />
          <span>
            允許 Agent 修改這個工作目錄內的檔案
            <span className="muted small" style={{ display: "block" }}>
              不授予系統其他位置或網路；工作階段結束、逾時或緊急停止時立即失效。
            </span>
          </span>
        </label>
        {allowWrite && (
          <label className="row" style={{ alignItems: "flex-start" }}>
            <input
              type="checkbox"
              checked={writeConfirmed}
              onChange={(e) => setWriteConfirmed(e.target.checked)}
            />
            <span>我已確認上方工作目錄，並同意這個工作階段可在其中修改檔案。</span>
          </label>
        )}
        <div className="row wrap">
          <label className="field-label">
            時間上限（分鐘）
            <input
              type="number"
              min={1}
              max={240}
              value={ttl}
              onChange={(e) => setTtl(Number(e.target.value))}
            />
          </label>
          <label className="field-label">
            費用上限（USD）
            <input
              type="number"
              min={0}
              step={0.1}
              value={maxCost}
              onChange={(e) => setMaxCost(Number(e.target.value))}
            />
          </label>
        </div>
        <div className="state-box">
          <strong>授權預覽</strong>
          <ul className="plain-list muted small">
            <li>
              資料範圍：{allowWrite ? "讀取與修改" : "只讀取"}上面指定的資料夾；不會傳送你的記憶
              （除非你之後明確提供 Context Bundle）。
            </li>
            <li>
              模式：{allowWrite ? "工作目錄限權寫入；不含額外目錄或網路權限" : "唯讀／計畫；不能寫入檔案"}。
            </li>
            <li>外部傳送：任務內容會送到該 agent 的模型服務（依你在該 agent 的登入方案計費）。</li>
            <li>取消方式：隨時「中斷」或「關閉」；緊急停止會立即終止子程序。時間到自動失效。</li>
          </ul>
          {routing && (
            <p className="muted small">路由提示：{String((routing as Record<string, unknown>).reason ?? "")}</p>
          )}
        </div>
        {disabledAgents.includes(agent) && (
          <p className="cap-card-error" role="alert">
            這個 Agent 已由使用者停用；請先在 Agent 連接卡片中啟用。
          </p>
        )}
        {error && (
          <p className="cap-card-error" role="alert">
            {error}
          </p>
        )}
        <div className="row wrap">
          <button
            className="primary"
            disabled={
              creating ||
              disabledAgents.includes(agent) ||
              (allowWrite && (!workdir.trim() || !writeConfirmed))
            }
            onClick={async () => {
              setCreating(true);
              try {
                await api.agentSessionCreate(
                  buildSessionCreateInput({
                    agent,
                    label,
                    workdir,
                    ttlMinutes: ttl,
                    maxCost,
                    allowWrite,
                  })
                );
                onClose();
              } catch (e) {
                setError(String(e));
              } finally {
                setCreating(false);
              }
            }}
          >
            {creating ? "建立中…" : "同意並建立"}
          </button>
          <button onClick={onClose}>取消</button>
        </div>
      </div>
    </Dialog>
  );
}
