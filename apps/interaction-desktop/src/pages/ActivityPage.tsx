// 活動紀錄：人類可讀的活動故事。嚴格區分規劃／授權／排入／收到／送達／
// 完成／驗證；技術細節（原始 event、UUID）收在「技術詳細資料」。

import React from "react";
import { api, Receipt, RuntimeEvent } from "../api";
import { actionStatusLabel, useAppState } from "../appstate";
import { Icon } from "../icons";
import { Badge, JsonView, Section, statusBadgeKind, useAsync } from "../ui";

export function ActivityPage({
  refreshKey,
  events,
  advanced,
  onNavigate,
}: {
  refreshKey: number;
  events: RuntimeEvent[];
  advanced: boolean;
  onNavigate?: (tab: string) => void;
}) {
  const [actions] = useAsync(() => api.actionsList(30), [refreshKey]);

  return (
    <div>
      <InboxSection refreshKey={refreshKey} onNavigate={onNavigate ?? (() => {})} />
      <p className="page-intro">
        每一次互動的完整歷程：系統感知到什麼、如何決定、安全規則說了什麼、實際結果到哪一層。
      </p>
      {actions.loading ? (
        <div className="state-box">載入中…</div>
      ) : actions.error ? (
        <div className="state-box state-error">{actions.error}</div>
      ) : (actions.data ?? []).length === 0 ? (
        <div className="state-box">還沒有任何互動紀錄。</div>
      ) : (
        (actions.data as Receipt[]).map((r) => (
          <ActivityStory key={r.actionId} receipt={r} advanced={advanced} />
        ))
      )}
      <SystemFeed events={events} advanced={advanced} />
    </div>
  );
}

function ActivityStory({ receipt, advanced }: { receipt: Receipt; advanced: boolean }) {
  const { findCard } = useAppState();
  const actuator = findCard("actuator", receipt.actuatorId);
  const time = receipt.timestamps?.[0]?.[1]
    ? new Date(receipt.timestamps[0][1]).toLocaleTimeString()
    : "";
  const blocked = receipt.currentStatus === "blocked";
  const blockReason = receipt.policyDecisions
    .map((d) => d as { outcome?: string; rule?: string; reason?: string })
    .find((d) => d.outcome === "blocked" || d.outcome === "approvalRequired");
  const clamped = receipt.policyDecisions
    .map((d) => d as { outcome?: string; field?: string })
    .filter((d) => d.outcome === "clamped");
  const verdict = receipt.verification?.verdict;
  const [cancelMsg, setCancelMsg] = React.useState<string | null>(null);
  const cancellable = ["accepted", "dispatched", "acknowledged"].includes(receipt.currentStatus);

  return (
    <Section
      title={`${time}　${receipt.intent}`}
      actions={
        <Badge kind={statusBadgeKind(receipt.currentStatus)}>
          {actionStatusLabel(receipt.currentStatus)}
        </Badge>
      }
    >
      <ol className="story-flow">
        <li>
          <span className="story-label">回應</span>
          <span>
            <Icon name={actuator.icon} size={14} /> {actuator.name}
          </span>
        </li>
        <li>
          <span className="story-label">安全</span>
          <span>
            {blocked
              ? `被安全規則阻止${blockReason?.reason ? `：${blockReason.reason}` : ""}`
              : clamped.length > 0
                ? `允許執行（${clamped.length} 個參數被安全上限收斂）`
                : "通過安全規則檢查"}
          </span>
        </li>
        <li>
          <span className="story-label">結果</span>
          <span>
            {actionStatusLabel(receipt.currentStatus)}
            {verdict === "observed" && <Badge kind="ok">驗證成功 — 已觀察到實際效果</Badge>}
            {verdict === "acknowledged-only" && (
              <Badge kind="warn">僅確認送出 — 尚未觀察到實際效果</Badge>
            )}
            {receipt.currentStatus === "uncertain" && (
              <span className="muted small">
                　系統無法確認這個動作是否產生了效果；不會自動重試，以免重複執行。
              </span>
            )}
          </span>
        </li>
        {receipt.errors.length > 0 && (
          <li>
            <span className="story-label">問題</span>
            <span>{receipt.errors.map((e) => e.message).join("；")}</span>
          </li>
        )}
      </ol>
      <div className="row wrap">
        {cancellable && (
          <button
            onClick={async () => {
              try {
                await api.cancelAction(receipt.actionId);
                setCancelMsg("已要求取消。");
              } catch (e) {
                setCancelMsg(`取消失敗：${e}`);
              }
            }}
          >
            取消
          </button>
        )}
        {["acknowledged", "uncertain", "completed"].includes(receipt.currentStatus) && (
          <button onClick={() => api.verifyAction(receipt.actionId)}>重新驗證</button>
        )}
      </div>
      {cancelMsg && <p className="muted small" role="status">{cancelMsg}</p>}
      {advanced && (
        <details className="tech-details">
          <summary>技術詳細資料</summary>
          <JsonView value={receipt} />
        </details>
      )}
    </Section>
  );
}

function SystemFeed({ events, advanced }: { events: RuntimeEvent[]; advanced: boolean }) {
  const interesting = events
    .filter((e) =>
      [
        "emergency.stop",
        "proactive.paused",
        "proactive.resumed",
        "consent.changed",
        "policy.changed",
        "session.started",
        "session.stopped",
        "ai.assist.requested",
        "ai.assist.resolved",
      ].includes(e.eventType)
    )
    .slice(-20)
    .reverse();
  if (interesting.length === 0) return null;
  return (
    <Section title="系統事件">
      <ul className="feed">
        {interesting.map((e) => (
          <li key={e.eventId}>
            <span className="muted small">{new Date(e.timestamp).toLocaleTimeString()}</span>{" "}
            {eventLabel(e)}
            {advanced && (
              <details className="tech-details inline">
                <summary>原始資料</summary>
                <JsonView value={e} />
              </details>
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}

function eventLabel(e: RuntimeEvent): string {
  switch (e.eventType) {
    case "emergency.stop":
      return e.payload["cleared"] === true ? "緊急停止已解除（人工確認）" : "緊急停止被觸發";
    case "proactive.paused":
      return "主動互動已暫停";
    case "proactive.resumed":
      return "主動互動已恢復";
    case "consent.changed":
      return "使用授權有變更";
    case "policy.changed":
      return "安全規則有變更";
    case "session.started":
      return "工作階段開始";
    case "session.stopped":
      return "工作階段結束";
    case "ai.assist.requested":
      return `訊號模糊，向 AI 請求協助判斷（${String(e.payload["reason"] ?? "")}）`;
    case "ai.assist.resolved":
      return "AI 協助請求已有結果";
    default:
      return e.eventType;
  }
}

/** 統一 Inbox／Timeline（spec §16-1.G）：由 Runtime application service
 *  正規化 Agent、Knowledge、Receipt 與 Safety；UI 不自行拼湊另一份真相。 */
export function InboxSection({
  refreshKey,
  onNavigate,
}: {
  refreshKey: number;
  onNavigate: (tab: string) => void;
}) {
  const [filters, setFilters] = React.useState({
    status: "",
    agent: "",
    device: "",
    task: "",
    domain: "",
    since: "",
  });
  const wireFilter = {
    ...Object.fromEntries(Object.entries(filters).filter(([, value]) => value.trim())),
    ...(filters.since ? { since: new Date(filters.since).toISOString() } : {}),
    limit: 200,
  };
  const [inbox] = useAsync(
    () => api.activityInbox(wireFilter),
    [refreshKey, JSON.stringify(wireFilter)]
  );
  const items = ((inbox.data?.items as Record<string, unknown>[] | undefined) ?? []);
  const pending = Number(inbox.data?.pendingCount ?? 0);
  const total = Number(inbox.data?.totalBeforeLimit ?? items.length);

  return (
    <Section title={`統一收件匣（待決定 ${pending}／共 ${total}）`}>
      <details className="inbox-filters" open>
        <summary>依時間、狀態、Agent、裝置、任務、知識 Domain 篩選</summary>
        <div className="inbox-filter-grid">
          {(["status", "agent", "device", "task", "domain"] as const).map((key) => (
            <label key={key}>
              {{ status: "狀態", agent: "Agent", device: "裝置", task: "任務", domain: "Domain" }[key]}
              <input
                value={filters[key]}
                onChange={(event) => setFilters((current) => ({ ...current, [key]: event.target.value }))}
              />
            </label>
          ))}
          <label>
            起始時間
            <input
              type="datetime-local"
              value={filters.since}
              onChange={(event) => setFilters((current) => ({ ...current, since: event.target.value }))}
            />
          </label>
          <button onClick={() => setFilters({ status: "", agent: "", device: "", task: "", domain: "", since: "" })}>
            清除篩選
          </button>
        </div>
      </details>
      {inbox.loading ? (
        <div className="state-box">載入中…</div>
      ) : inbox.error ? (
        <div className="state-box state-error" role="alert">收件匣無法載入：{inbox.error}</div>
      ) : items.length === 0 ? (
        <div className="state-box">目前沒有符合篩選條件的活動。</div>
      ) : (
        <div className="provider-list" data-testid="activity-inbox-results">
          {items.map((item) => (
            <div className="provider-card" key={`${String(item.kind)}-${String(item.itemId)}`}>
              <div className="row space-between">
                <strong>{String(item.title)}</strong>
                <Badge kind={item.needsDecision === true ? "warn" : statusBadgeKind(String(item.status))}>
                  {String(item.status)}
                </Badge>
              </div>
              <div className="muted small">
                {new Date(String(item.occurredAt)).toLocaleString("zh-TW")}・{String(item.kind)}
                {item.agentId ? `・Agent ${String(item.agentId)}` : ""}
                {item.deviceId ? `・裝置 ${String(item.deviceId)}` : ""}
                {Array.isArray(item.domains) && item.domains.length ? `・${item.domains.join(", ")}` : ""}
              </div>
              <button onClick={() => onNavigate(String(item.route))}>開啟對應頁面</button>
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}
