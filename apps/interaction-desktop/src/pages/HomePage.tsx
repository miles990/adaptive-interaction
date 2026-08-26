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
              <button onClick={() => doResume()}>恢復主動互動</button>
            ) : (
              <>
                <button onClick={() => doPause()}>暫停主動互動</button>
                <button onClick={() => setPauseDialog(true)}>暫停一段時間…</button>
              </>
            )}
            <button onClick={() => onNavigate("automations")}>查看自動互動</button>
          </div>
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
                  await doPause(m);
                  setPauseDialog(false);
                }}
              >
                {m >= 60 ? `${m / 60} 小時` : `${m} 分鐘`}
              </button>
            ))}
            <button
              onClick={async () => {
                await doPause();
                setPauseDialog(false);
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

function LastInteraction({ receipt, events }: { receipt: Receipt; events: RuntimeEvent[] }) {
  const { findCard } = useAppState();
  const actuator = findCard("actuator", receipt.actuatorId);
  const verified = receipt.verification?.verdict === "observed";
  const gate = findGate(events, receipt.planId);
  return (
    <ol className="story-flow" aria-label="最近一次互動的過程">
      <li>
        <span className="story-label">感知</span>
        <span>系統收到「{receipt.intent}」相關的訊號</span>
      </li>
      <li>
        <span className="story-label">理解</span>
        <span>{gate ?? "由本機規則判斷，這次不需要 AI"}</span>
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

function findGate(events: RuntimeEvent[], planId: string): string | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (e.eventType === "ai.assist.resolved" || e.eventType === "ai.assist.requested") {
      return "訊號模糊，曾請 AI 協助判斷";
    }
    if (e.eventType === "plan.created" && e.payload["planId"] === planId) break;
  }
  return null;
}
