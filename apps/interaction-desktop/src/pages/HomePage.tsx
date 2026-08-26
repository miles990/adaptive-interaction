// 首頁：系統是否正常、主動互動狀態、三區權限地圖、最近一次互動的故事、
// 快速操作。全部來自後端真實狀態；沒有 receipt 佐證絕不顯示「已完成」。

import React from "react";
import { api, HumanCard, Receipt, RuntimeEvent } from "../api";
import { actionStatusLabel, useAppState } from "../appstate";
import { Icon } from "../icons";
import { Badge, Section, StateView, useAsync } from "../ui";
import { Dialog } from "../components/Dialog";

export function HomePage({
  refreshKey,
  events,
  onNavigate,
}: {
  refreshKey: number;
  events: RuntimeEvent[];
  onNavigate: (tab: string) => void;
}) {
  const { human, pause, doPause, doResume } = useAppState();
  const [status] = useAsync(() => api.status(), [refreshKey]);
  const [actions] = useAsync(() => api.actionsList(5), [refreshKey]);
  const [session] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [pauseDialog, setPauseDialog] = React.useState(false);
  const [pauseError, setPauseError] = React.useState<string | null>(null);
  const tryPause = async (minutes?: number) => {
    try {
      await doPause(minutes);
      setPauseError(null);
      return true;
    } catch (e) {
      setPauseError(String(e));
      return false;
    }
  };

  const estop = Boolean(status.data?.["emergencyStop"]);
  const recipes = (status.data?.["recipes"] as { loaded?: number } | undefined)?.loaded ?? 0;
  const pendingAi = Number(status.data?.["pendingAiAssists"] ?? 0);

  return (
    <div className="home">
      <div className="grid-two">
        <Section title="系統狀態">
          {status.loading ? (
            <div className="state-box">載入中…</div>
          ) : status.error ? (
            <div className="state-box state-error">
              無法連到系統：{status.error}。請稍後再試，或重新啟動應用程式。
            </div>
          ) : (
            <div className="home-status">
              {estop ? (
                <p className="home-status-line bad">
                  <Icon name="octagon-x" size={18} /> 緊急停止中 — 所有回應已停止。
                  到「同意與安全」頁可以檢視原因並安全解除。
                </p>
              ) : pause.paused ? (
                <p className="home-status-line warn">
                  <Icon name="pause" size={18} /> 主動互動已暫停
                  {pause.until ? `（至 ${new Date(pause.until).toLocaleTimeString()}）` : ""}。
                  系統仍會回應你的直接要求。
                </p>
              ) : (
                <p className="home-status-line ok">
                  <Icon name="circle-check" size={18} /> 系統運作正常。
                </p>
              )}
              <p className="muted small">
                {session.data
                  ? "工作階段進行中 — 自動互動可以運作。"
                  : "目前沒有工作階段 — 自動互動不會執行，直到你開始一個。"}
                {`　已載入 ${recipes} 個自動互動。`}
              </p>
              {pendingAi > 0 && (
                <p className="home-status-line info">
                  <Icon name="bot" size={16} /> 有 {pendingAi} 個情境正在等 AI 協助判斷
                  （逾時會自動用本機規則處理）。
                </p>
              )}
            </div>
          )}
        </Section>

        <Section title="主動互動">
          <ProactiveSummary />
          <div className="row wrap" style={{ marginTop: 10 }}>
            {pause.paused ? (
              <button onClick={() => doResume().catch((e) => setPauseError(String(e)))}>
                恢復主動互動
              </button>
            ) : (
              <>
                <button onClick={() => tryPause()}>暫停主動互動</button>
                <button onClick={() => setPauseDialog(true)}>暫停一段時間…</button>
              </>
            )}
            <button onClick={() => onNavigate("automations")}>查看自動互動</button>
          </div>
          {pauseError && (
            <p className="cap-card-error" role="alert">
              操作失敗：{pauseError}
            </p>
          )}
        </Section>
      </div>

      <Section title="權限地圖 — AI 現在可以做什麼？">
        {human ? (
          <PermissionMap
            receptors={human.receptors}
            actuators={human.actuators}
            tools={human.toolOperations}
          />
        ) : (
          <div className="state-box">載入中…</div>
        )}
        <p className="muted small">
          這張地圖來自目前的啟用狀態、安全規則與同意設定。到「同意與安全」頁可以調整。
        </p>
      </Section>

      <AgentSessionsSection refreshKey={refreshKey} advancedHint />

      <Section title="最近一次互動">
        <StateView state={actions} empty="還沒有任何互動。">
          {(list) => <LastInteraction receipt={(list as Receipt[])[0]} events={events} />}
        </StateView>
      </Section>

      <Section title="快速操作">
        <div className="row wrap">
          {!session.data ? (
            <button
              onClick={async () => {
                await api.sessionStart("desktop", []);
                onNavigate("home");
              }}
            >
              開始工作階段
            </button>
          ) : (
            <button onClick={() => api.sessionStop()}>結束工作階段</button>
          )}
          <button onClick={() => onNavigate("automations")}>建立自動互動</button>
          <button onClick={() => onNavigate("safety")}>管理權限</button>
          <button onClick={() => onNavigate("responses")}>測試回應方式</button>
        </div>
      </Section>

      {pauseDialog && (
        <Dialog title="暫停主動互動" onClose={() => setPauseDialog(false)}>
          <p className="muted">暫停期間，自動互動不會打擾你；你的直接要求仍會執行。</p>
          <div className="row wrap">
            {[30, 60, 120, 480].map((m) => (
              <button
                key={m}
                onClick={async () => {
                  if (await tryPause(m)) setPauseDialog(false);
                }}
              >
                {m >= 60 ? `${m / 60} 小時` : `${m} 分鐘`}
              </button>
            ))}
            <button
              onClick={async () => {
                if (await tryPause()) setPauseDialog(false);
              }}
            >
              直到我恢復
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}

function ProactiveSummary() {
  const [recipes] = useAsync(() => api.recipesList(), []);
  const enabled = (recipes.data ?? []).filter((r) => Boolean(r.recipe["enabled"]));
  const anyAi = enabled.some((r) => {
    const ai = r.recipe["ai"] as { mode?: string } | undefined;
    return ai && ai.mode && ai.mode !== "never";
  });
  return (
    <div>
      <p>
        目前啟用 <strong>{enabled.length}</strong> 個自動互動。
      </p>
      <ul className="plain-list">
        <li>
          <Icon name="circle-check" size={14} /> 明確事件由本機規則處理，不呼叫 AI
        </li>
        <li>
          {anyAi ? (
            <>
              <Icon name="bot" size={14} /> 只有訊號模糊時才請 AI 協助；AI 沒回應時用本機規則
            </>
          ) : (
            <>
              <Icon name="circle-check" size={14} /> 所有自動互動都不使用 AI
            </>
          )}
        </li>
      </ul>
    </div>
  );
}

export function PermissionMap({
  receptors,
  actuators,
  tools,
}: {
  receptors: HumanCard[];
  actuators: HumanCard[];
  tools: HumanCard[];
}) {
  const canKnow = receptors.filter((r) => r.availability === "available");
  const canDo = actuators.filter((a) => a.availability === "available" && a.consent.required !== true);
  const mustAsk = [
    ...actuators.filter((a) => a.consent.required === true || a.availability === "disabled"),
    ...tools.filter((t) => t.requiresConsent),
  ];
  return (
    <div className="perm-map">
      <div className="perm-zone perm-know">
        <h3>
          <Icon name="scan-eye" size={16} /> AI 可以知道
        </h3>
        <NameList cards={canKnow} empty="目前沒有啟用任何感知來源。" />
      </div>
      <div className="perm-zone perm-do">
        <h3>
          <Icon name="send" size={16} /> AI 可以做
        </h3>
        <NameList cards={canDo} empty="目前沒有可直接使用的回應方式。" />
      </div>
      <div className="perm-zone perm-ask">
        <h3>
          <Icon name="hand" size={16} /> AI 必須先問
        </h3>
        <NameList cards={mustAsk} empty="沒有需要另外同意的能力。" />
      </div>
    </div>
  );
}

function NameList({ cards, empty }: { cards: HumanCard[]; empty: string }) {
  if (cards.length === 0) return <p className="muted small">{empty}</p>;
  const shown = cards.slice(0, 8);
  return (
    <ul className="perm-list">
      {shown.map((c) => (
        <li key={`${c.kind}-${c.id}`}>
          <Icon name={c.icon} size={14} /> {c.displayName}
        </li>
      ))}
      {cards.length > shown.length && <li className="muted">…還有 {cards.length - shown.length} 項</li>}
    </ul>
  );
}

function LastInteraction({ receipt }: { receipt: Receipt; events: RuntimeEvent[] }) {
  const { findCard } = useAppState();
  const actuator = findCard("actuator", receipt.actuatorId);
  const verified = receipt.verification?.verdict === "observed";
  // 理解階段的描述來自該 plan 的持久化 metadata（真實決策資料），
  // 不從事件流猜測 —— 猜測會把別的配方的 AI 介入誤掛到這次互動上。
  const [gate, setGate] = React.useState<string | null>(null);
  React.useEffect(() => {
    let alive = true;
    api
      .planGet(receipt.planId)
      .then((plan) => {
        if (!alive) return;
        const meta = (plan["metadata"] ?? {}) as Record<string, unknown>;
        const aiGate = meta["aiGate"] as Record<string, unknown> | undefined;
        if (aiGate) {
          const outcome = String(aiGate["outcome"] ?? "");
          if (outcome === "requested") setGate("訊號模糊，曾請 AI 協助判斷");
          else if (outcome === "deferred-then-deterministic")
            setGate("曾等待 AI 判斷，最後由本機規則處理");
          else if (outcome === "notNeeded") setGate("證據明確，由本機規則處理，不需要 AI");
          else setGate("由本機規則處理，這次不需要 AI");
        } else if (meta["recipeId"]) {
          setGate("由本機規則自動觸發，不需要 AI");
        } else {
          setGate("回應明確要求");
        }
      })
      .catch(() => alive && setGate(null));
    return () => {
      alive = false;
    };
  }, [receipt.planId]);
  return (
    <ol className="story-flow" aria-label="最近一次互動的過程">
      <li>
        <span className="story-label">感知</span>
        <span>系統收到「{receipt.intent}」相關的訊號</span>
      </li>
      <li>
        <span className="story-label">理解</span>
        <span>{gate ?? "（讀取決策資料中…）"}</span>
      </li>
      <li>
        <span className="story-label">計畫</span>
        <span>
          <Icon name={actuator.icon} size={14} /> 使用「{actuator.name}」回應
        </span>
      </li>
      <li>
        <span className="story-label">安全</span>
        <span>
          {receipt.currentStatus === "blocked"
            ? "安全規則阻止了這次回應"
            : "通過安全規則檢查"}
        </span>
      </li>
      <li>
        <span className="story-label">結果</span>
        <span>
          <Badge kind={receipt.currentStatus === "completed" ? "ok" : receipt.currentStatus === "blocked" ? "bad" : "warn"}>
            {actionStatusLabel(receipt.currentStatus)}
          </Badge>
          {verified && <Badge kind="ok">已驗證</Badge>}
        </span>
      </li>
    </ol>
  );
}



/** 目前 AI 工作階段（多 Session 一般模式視圖）：真實 Session、人話狀態、
 *  權限範圍；「聲稱完成」明確標示為聲稱，非驗證。 */
function AgentSessionsSection({ refreshKey }: { refreshKey: number; advancedHint?: boolean }) {
  const [sessions] = useAsync(() => api.agentSessionsList(), [refreshKey]);
  const open = (sessions.data ?? []).filter((s) => !s.closedAt);
  if (sessions.loading || open.length === 0) return null;

  const stateLabel = (st: string) =>
    ({
      created: "已建立，尚未開始",
      active: "工作中",
      "waiting-for-input": "等待你的輸入",
      "waiting-for-consent": "等待你的同意",
      "claimed-completed": "聲稱已完成（尚未驗證）",
      failed: "失敗",
      "timed-out": "逾時",
      cancelled: "已取消",
      expired: "租約已到期",
      closed: "已結束",
    })[st] ?? st;

  return (
    <Section title={`目前有 ${open.length} 個 AI 工作階段`}>
      {open.map((s) => (
        <div key={s.sessionId} className="agent-session-row">
          <div className="agent-session-head">
            <strong>{s.label ?? s.agentId}</strong>
            <Badge
              kind={
                s.state === "claimed-completed"
                  ? "warn"
                  : s.state === "failed"
                    ? "bad"
                    : "info"
              }
            >
              {stateLabel(s.state)}
            </Badge>
          </div>
          <div className="muted small">
            權限：
            {s.dataScope.length > 0 ? `可讀 ${s.dataScope.join("、")}` : "無資料範圍"}
            {s.toolScope.length > 0 ? `；工具 ${s.toolScope.join("、")}` : ""}
            　·　訊息 {s.budget.spentMessages}/{s.budget.maxMessages || "∞"}
            　·　租約至 {new Date(s.lease.expiresAt).toLocaleTimeString()}
          </div>
          <div className="row wrap" style={{ marginTop: 4 }}>
            <button
              onClick={() =>
                api.agentSessionClose(s.sessionId, "cancelled").catch(() => {})
              }
            >
              取消這個工作階段
            </button>
          </div>
        </div>
      ))}
      <p className="muted small">
        AI 工作階段是有期限、有預算的委派工作。它們的回報是「聲稱」，
        實際結果仍以收據與驗證為準。
      </p>
    </Section>
  );
}
