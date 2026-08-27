// AI 與工作階段（spec §16-1.D／§9）：本機 agent 連接、路由建議、
// session 卡片（誠實狀態階梯）、建立（ConsentSheet 預覽）、
// approval 裁決、任務送出、中斷、關閉。

import React from "react";
import { AgentSessionRecord, api } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";
import { Dialog } from "../components/Dialog";

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

export function AiPage({
  refreshKey,
  onNavigate,
}: {
  refreshKey: number;
  onNavigate: (tab: string) => void;
}) {
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
          登入由各 agent 自己管理。第一版所有工作階段皆為<strong>唯讀／計畫</strong>模式。
        </p>
        <StateView state={agents} empty="尚未偵測。">
          {(data) => (
            <div className="provider-list">
              {((data.agents as Record<string, unknown>[] | undefined) ?? []).map((a) => {
                const usable = a.found === true && a.loggedIn === true;
                return (
                  <div className="provider-card" key={String(a.kind)}>
                    <div className="row space-between">
                      <strong>{a.kind === "codex" ? "Codex" : "Claude Code"}</strong>
                      {usable ? (
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

      <Section title="工作階段">
        <StateView state={sessions} empty="目前沒有任何工作階段。">
          {(list) => (
            <div className="provider-list">
              {list.map((s) => (
                <SessionCard key={s.sessionId} record={s} onNotice={setNotice} onNavigate={onNavigate} />
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
  onNotice,
  onNavigate,
}: {
  record: AgentSessionRecord;
  onNotice: (m: string) => void;
  onNavigate: (tab: string) => void;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const [messages, setMessages] = React.useState<Record<string, unknown>[]>([]);
  const [task, setTask] = React.useState("");
  const label = STATE_LABEL[record.state] ?? { text: record.state, kind: "pending" as const };
  const open = !["closed", "cancelled", "expired"].includes(record.state);

  async function refreshMessages() {
    try {
      setMessages(await api.agentSessionMessages(record.sessionId, "from-session"));
    } catch {
      /* session gone */
    }
  }

  return (
    <div className="provider-card">
      <div className="row space-between">
        <strong>{record.label ?? record.agentId}</strong>
        <Badge kind={label.kind}>{label.text}</Badge>
      </div>
      <div className="muted small">
        {record.agentId}
        {record.providerSessionId ? `・provider session ${record.providerSessionId.slice(0, 8)}…` : ""}
        ・訊息 {record.budget.spentMessages}/{record.budget.maxMessages}
        {record.budget.maxCost > 0
          ? `・費用 $${record.budget.spentCost.toFixed(3)}/$${record.budget.maxCost.toFixed(2)}`
          : record.budget.spentCost > 0
            ? `・費用 $${record.budget.spentCost.toFixed(3)}`
            : ""}
      </div>
      {record.state === "claimed-completed" && (
        <p className="muted small">
          Agent 說做完了——這是<strong>它的說法</strong>，尚未經過驗證。請查看結果後自行確認。
        </p>
      )}
      <div className="row wrap">
        <button
          onClick={() => {
            setExpanded((v) => !v);
            if (!expanded) void refreshMessages();
          }}
        >
          {expanded ? "收合" : "查看結果／訊息"}
        </button>
        {open && (
          <>
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
              中斷目前工作
            </button>
            <button
              className="danger"
              onClick={async () => {
                try {
                  await api.agentSessionClose(record.sessionId, "closed");
                  onNotice("工作階段已關閉（子程序已終止）。");
                } catch (e) {
                  onNotice(`關閉失敗：${e}`);
                }
              }}
            >
              關閉
            </button>
          </>
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
                  <strong>{String(m.kind)}</strong>
                  {m.kind === "approval-request" && (
                    <ApprovalControls
                      sessionId={record.sessionId}
                      body={m.body as Record<string, unknown>}
                      onNotice={onNotice}
                    />
                  )}
                  <pre className="json-view small">
                    {JSON.stringify(m.body, null, 2)}
                  </pre>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
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

/** 建立工作階段（Consent Sheet 語意：誰／做什麼／資料／成本／時間／如何取消）。 */
function CreateSessionSheet({ onClose }: { onClose: () => void }) {
  const [agent, setAgent] = React.useState("claude-code");
  const [label, setLabel] = React.useState("");
  const [workdir, setWorkdir] = React.useState("");
  const [ttl, setTtl] = React.useState(30);
  const [maxCost, setMaxCost] = React.useState(0.5);
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
            <option value="claude-code">Claude Code（文件／歸納／規劃）</option>
            <option value="codex">Codex（程式／測試／Patch）</option>
          </select>
        </label>
        <label className="field-label">
          做什麼（名稱）
          <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="例：看一下這個 repo 的測試" />
        </label>
        <label className="field-label">
          可讀取的資料夾（工作目錄；唯讀）
          <input value={workdir} onChange={(e) => setWorkdir(e.target.value)} placeholder="/path/to/project（留空＝系統資料夾）" />
        </label>
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
            <li>資料範圍：只讀取上面指定的資料夾；不會傳送你的記憶（除非你之後明確提供 Context Bundle）。</li>
            <li>模式：唯讀／計畫——這個 agent 不能寫入檔案、不能執行未經你核可的指令。</li>
            <li>外部傳送：任務內容會送到該 agent 的模型服務（依你在該 agent 的登入方案計費）。</li>
            <li>取消方式：隨時「中斷」或「關閉」；緊急停止會立即終止子程序。時間到自動失效。</li>
          </ul>
          {routing && (
            <p className="muted small">路由提示：{String((routing as Record<string, unknown>).reason ?? "")}</p>
          )}
        </div>
        {error && (
          <p className="cap-card-error" role="alert">
            {error}
          </p>
        )}
        <div className="row wrap">
          <button
            className="primary"
            disabled={creating}
            onClick={async () => {
              setCreating(true);
              try {
                await api.agentSessionCreate({
                  agentId: agent,
                  label: label || null,
                  ttlMinutes: ttl,
                  maxCost: maxCost > 0 ? maxCost : null,
                  workdir: workdir || null,
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
