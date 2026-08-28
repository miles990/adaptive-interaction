// AI 與工作階段（spec §16-1.D／§9）：本機 agent 連接、路由建議、
// session 卡片（誠實狀態階梯）、建立（ConsentSheet 預覽）、
// approval 裁決、任務送出、中斷、關閉。

import React from "react";
import { AgentSessionRecord, api } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";
import { Dialog } from "../components/Dialog";
import { useAppState } from "../appstate";

const STATE_LABEL: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  created: { text: "已建立", kind: "pending" },
  active: { text: "工作中", kind: "ok" },
  "waiting-for-input": { text: "等待輸入", kind: "warn" },
  "waiting-for-consent": { text: "等待你核可", kind: "warn" },
  "claimed-completed": { text: "回報完成（尚未驗證）", kind: "warn" },
  failed: { text: "失敗", kind: "bad" },
  "timed-out": { text: "逾時", kind: "bad" },
  cancelled: { text: "已取消", kind: "bad" },
  expired: { text: "已到期", kind: "bad" },
  closed: { text: "已關閉", kind: "pending" },
};

/** 工作階段狀態的人話對照（Global Search 等外部介面沿用同一份，
 *  免得一般模式又冒出 `claimed-completed` 這種原始字串）。 */
export const SESSION_STATE_LABEL = STATE_LABEL;

/** 訊息輪詢間隔：展開時才跑，收合／卸載即停（有界，不會長駐）。 */
export const MESSAGE_POLL_MS = 5000;
/** 後端 gateway 的 approval TTL（gateway.rs APPROVAL_TTL_SECS）。 */
export const APPROVAL_TTL_SECONDS = 300;

const MESSAGE_KIND_LABEL: Record<string, string> = {
  task: "你送出的任務",
  "approval-request": "等待你核可",
  "approval-resolved": "核可結果",
  progress: "進度更新",
  result: "結果回報",
  error: "錯誤",
  note: "備註",
  text: "訊息",
  question: "Agent 的提問",
};

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
}: {
  refreshKey: number;
  advanced?: boolean;
  onNavigate: (tab: string) => void;
}) {
  const { prefs, setPreferences } = useAppState();
  const [agents, retryAgents] = useAsync(
    () => api.agentsDiscoveries() as Promise<Record<string, unknown>>,
    [refreshKey]
  );
  const [sessions] = useAsync(() => api.agentSessionsList(), [refreshKey]);
  const [createOpen, setCreateOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);

  return (
    <div>
      <Section title="本機 AI Agent">
        <p className="muted small">
          直接使用你電腦上已安裝、已登入的 Codex 與 Claude Code。系統不讀取、不保存它們的登入憑證；
          登入由各 agent 自己管理。工作階段預設為<strong>唯讀／計畫</strong>；只有你在建立預覽中
          明確同意時，才會建立限於單一工作目錄的寫入工作階段。
        </p>
        <StateView state={agents} empty="尚未偵測。">
          {(data) => (
            <div className="provider-list">
              {((data.agents as Record<string, unknown>[] | undefined) ?? []).map((a) => {
                const agentId = a.kind === "codex" ? "codex" : "claude-code";
                const disabled = (prefs.disabledAgents ?? []).includes(agentId);
                const usable = !disabled && a.found === true && a.loggedIn === true;
                return (
                  <div className="provider-card" key={String(a.kind)}>
                    <div className="row space-between">
                      <strong>{a.kind === "codex" ? "Codex" : "Claude Code"}</strong>
                      {disabled ? (
                        <Badge kind="pending">已停用</Badge>
                      ) : usable ? (
                        <Badge kind="ok">可用</Badge>
                      ) : a.found === true ? (
                        <Badge kind="warn">
                          {a.loggedIn === false ? "未登入" : "登入狀態未知"}
                        </Badge>
                      ) : (
                        <Badge kind="bad">未安裝</Badge>
                      )}
                    </div>
                    <div className="muted small">{String(a.detail ?? "")}</div>
                    <div className="row wrap" style={{ marginTop: 8 }}>
                      <button
                        onClick={async () => {
                          try {
                            await api.agentsRefresh();
                            retryAgents();
                            setNotice(
                              a.found === true && a.loggedIn === true
                                ? `${agentId === "codex" ? "Codex" : "Claude Code"} 連線前置檢查通過（已安裝且已登入；未建立付費 Session）。`
                                : "連線前置檢查未通過；請查看版本與登入狀態。"
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
                              ? `${agentId === "codex" ? "Codex" : "Claude Code"} 已啟用。`
                              : `${agentId === "codex" ? "Codex" : "Claude Code"} 已停用；Runtime 將拒絕新 Session。`
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
          <button className="primary" onClick={() => setCreateOpen(true)}>
            建立工作階段…
          </button>
        </div>
        <p className="muted small">
          建議路由：程式實作／測試／Patch → Codex；長文件／概念歸納／規劃 → Claude Code；
          模糊任務會顯示兩個選項讓你選。Agent 不可用時不會自動改送另一個。
        </p>
      </Section>

      <Section title="小樞主要 AI">
        <p className="muted small">
          各用途只設定建議路由；建立 Session 前仍會顯示資料、工具、費用與時間預覽。指定的 Agent 不可用或已停用時不會自動改送另一家。
        </p>
        <div className="settings-grid">
          {(
            [
              ["conversation", "一般對話"],
              ["programming", "程式工作"],
              ["knowledge", "知識整理"],
              ["review", "結果複審"],
            ] as const
          ).map(([role, label]) => (
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
      </Section>

      <Section title="工作階段">
        <StateView state={sessions} empty="目前沒有任何工作階段。">
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
  const [expanded, setExpanded] = React.useState(false);
  const [messages, setMessages] = React.useState<Record<string, unknown>[]>([]);
  const [task, setTask] = React.useState("");
  const resolutions = React.useMemo(() => approvalResolutions(messages), [messages]);
  const label = STATE_LABEL[record.state] ?? { text: record.state, kind: "pending" as const };
  const open = !["closed", "cancelled", "expired"].includes(record.state);

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
        <strong>{record.label ?? record.agentId}</strong>
        <Badge kind={label.kind}>{label.text}</Badge>
      </div>
      <div className="muted small">
        {agentDisplayName(record.agentId)}
        {record.allowWrite ? "・限工作目錄寫入" : "・唯讀／計畫"}
        {record.providerSessionId
          ? advanced
            ? `・provider session ${record.providerSessionId.slice(0, 8)}…`
            : "・沿用既有對話脈絡"
          : ""}
        ・訊息 {record.budget.spentMessages}/{record.budget.maxMessages}
        {record.budget.maxCost > 0
          ? `・費用 $${record.budget.spentCost.toFixed(3)}/$${record.budget.maxCost.toFixed(2)}`
          : record.budget.spentCost > 0
            ? `・費用 $${record.budget.spentCost.toFixed(3)}`
            : ""}
        ・有效至 {new Date(record.lease.expiresAt).toLocaleString("zh-TW")}
      </div>
      {record.state === "claimed-completed" && !record.humanVerified && (
        <p className="muted small">
          Agent 說做完了——這是<strong>它的說法</strong>，尚未經過驗證。
          請實際查看結果；確認無誤後按「標記為已驗證」，小樞才會播放正式成功演出。
        </p>
      )}
      {record.humanVerified && (
        <p className="muted small" role="status">
          <Badge kind="ok">已人工驗證</Badge>{" "}
          {new Date(record.humanVerified.at).toLocaleString("zh-TW")}
          {record.humanVerified.note ? `・${record.humanVerified.note}` : ""}
        </p>
      )}
      <div className="row wrap">
        {record.state === "claimed-completed" && !record.humanVerified && (
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
              <button
                onClick={async () => {
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
                續租 30 分鐘
              </button>
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
          </>
        )}
        {!open && record.providerSessionId && (
          <button
            onClick={async () => {
              try {
                const workdir = record.dataScope
                  .find((s) => s.startsWith("workspace:"))
                  ?.slice("workspace:".length);
                await api.agentSessionCreate({
                  agentId: record.agentId,
                  label: `接續：${record.label ?? record.agentId}`,
                  workdir,
                  resumeProviderSessionId: record.providerSessionId,
                });
                // 誠實：接續＝新的唯讀 session 沿用 provider 對話脈絡；
                // 寫入權限不繼承，需要時另建限權寫入 session。
                onNotice("已建立接續工作階段（唯讀；沿用 provider 對話脈絡）。");
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
                placeholder="送一個任務給這個 agent…"
                onChange={(e) => setTask(e.target.value)}
              />
              <button
                disabled={!task.trim()}
                onClick={async () => {
                  try {
                    await api.agentSessionSend(record.sessionId, "task", { task });
                    setTask("");
                    onNotice("任務已送出（送達 agent 子程序）。");
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
                  <details className="tech-details">
                    <summary className="muted small">技術詳情</summary>
                    <pre className="json-view small">{JSON.stringify(m.body, null, 2)}</pre>
                  </details>
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

function agentDisplayName(agentId: string): string {
  return agentId === "codex" ? "Codex" : agentId === "claude-code" ? "Claude Code" : agentId;
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

/** 建立工作階段（Consent Sheet 語意：誰／做什麼／資料／成本／時間／如何取消）。 */
function CreateSessionSheet({ onClose }: { onClose: () => void }) {
  const { prefs } = useAppState();
  const disabledAgents = prefs.disabledAgents ?? [];
  const [agent, setAgent] = React.useState(
    disabledAgents.includes("claude-code") ? "codex" : "claude-code"
  );
  const [label, setLabel] = React.useState("");
  const [workdir, setWorkdir] = React.useState("");
  const [ttl, setTtl] = React.useState(30);
  const [maxCost, setMaxCost] = React.useState(0.5);
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
                await api.agentSessionCreate({
                  agentId: agent,
                  label: label || null,
                  ttlMinutes: ttl,
                  maxCost: agent === "codex" ? null : maxCost > 0 ? maxCost : null,
                  workdir: workdir || null,
                  allowWrite,
                  dataScope: workdir ? [`workspace:${workdir}`] : [],
                  toolScope: allowWrite ? ["workspace.write"] : [],
                  consentScope: allowWrite ? ["agent-session:workspace-write"] : [],
                });
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
