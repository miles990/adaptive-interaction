// 「現在」頁（v0.5 一般模式）：第一屏只回答三件事——角色現在怎麼樣、正在做什麼、
// 有什麼需要處理——外加五個快速操作（交代一件事／暫停或恢復主動互動／加入裝置／
// 停止所有感測／緊急停止）。緊急停止是二段確認、只能觸發不能解除（解除走安全頁）；
// 「停止所有感測」送出後一定重讀狀態，只有真的沒有感測在用才敢說「已停止感測」。
// 系統狀態、自動互動／裝置數量、最近一次互動、記憶更新全部收進「詳細狀態」折疊區
// （工作階段只留一行摘要＋前往工作，完整清單的主人是工作頁）。全部來自後端真實狀態；
// 沒有驗證佐證絕不顯示「已確認完成」；狀態標籤一律走 statusProjection；
// 安全文字（緊急停止中／感測使用中）固定；機器字串一律翻成人話再上畫面。

import React from "react";
import { api, HumanCard, Receipt, RuntimeEvent, SensorUse } from "../api";
import { actionStatusLabel, useAppState } from "../appstate";
import { characterNameFallback, useCharacterName } from "../characterName";
import { displayNameOf } from "../character/manifest";
import { Icon } from "../icons";
import { Badge, Section, StateView, useAsync } from "../ui";
import {
  agentDisplayLabel,
  inboxItemTitle,
  isOpenWorkState,
  isPendingCountExact,
  knowledgeTriggerLabel,
  PENDING_INCOMPLETE_NOTE,
  pendingCountLabel,
  projectInboxStatus,
  projectSensorStop,
  projectWorkState,
  receiptIntentLabel,
  sensorKindLabel,
} from "../statusProjection";
import { ConfirmButton, Dialog } from "../components/Dialog";

/** 「交代一件事」把描述先放進 sessionStorage，工作頁掛載時讀取並預填。純文字。 */
export const WORK_PREFILL_KEY = "work.prefill";

/** 角色離線時第一屏的可信文字（固定文案，不由角色包決定）。 */
export const CHARACTER_OFFLINE_LINE = "角色離線，改用文字。";

export function HomePage({
  refreshKey,
  events,
  onNavigate,
  estopped = false,
  onEstop,
}: {
  refreshKey: number;
  events: RuntimeEvent[];
  onNavigate: (tab: string) => void;
  /** Shell 已知的緊急停止狀態（沒傳＝未知，一律當作未停止並顯示觸發鈕）。 */
  estopped?: boolean;
  /** Shell 的緊急停止流程（含失敗時的重試警示列）；沒傳就直接呼叫後端。 */
  onEstop?: () => Promise<void>;
}) {
  const { pause, doPause, doResume, prefs } = useAppState();
  const character = useCharacterName({ locale: prefs.locale });
  const [status] = useAsync(() => api.status(), [refreshKey]);
  const [pauseDialog, setPauseDialog] = React.useState(false);
  const [pauseError, setPauseError] = React.useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = React.useState(false);
  const [task, setTask] = React.useState("");
  // 「停止所有感測」的結果：ok=true 才是確認停止，其餘一律以警示呈現（不得靜默）。
  const [sensorNotice, setSensorNotice] = React.useState<{ message: string; ok: boolean } | null>(
    null
  );
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

  // 誠實階梯：送出停止請求 ≠ 已停止。送出後一定重讀 status，只有 activeSensors 真的空了
  // 而且回報沒有「不確定」時才敢說「已停止感測」；讀不到狀態就說讀不到，不猜。
  const stopSensors = async () => {
    setSensorNotice(null);
    let report: unknown;
    try {
      report = await api.sensorsStop();
    } catch (e) {
      setSensorNotice({ message: `停止所有感測失敗：${String(e)}`, ok: false });
      return;
    }
    let remaining: SensorUse[] | null = null;
    try {
      remaining = ((await api.status())["activeSensors"] as SensorUse[] | undefined) ?? [];
    } catch {
      remaining = null;
    }
    setSensorNotice(projectSensorStop(report, remaining));
  };

  const delegate = (event: React.FormEvent) => {
    event.preventDefault();
    const text = task.trim();
    try {
      if (text) sessionStorage.setItem(WORK_PREFILL_KEY, text);
      else sessionStorage.removeItem(WORK_PREFILL_KEY);
    } catch {
      /* 私密模式等情況：沒有預填也能到工作頁 */
    }
    onNavigate("work");
  };

  return (
    <div className="home">
      <NowStrip
        refreshKey={refreshKey}
        status={status.data}
        statusError={status.error}
        paused={pause.paused}
        onNavigate={onNavigate}
      />

      <Section title="快速操作">
        <form className="home-delegate" onSubmit={delegate}>
          <label className="field-label">
            想讓{character.name}幫你做什麼？
            <input
              value={task}
              onChange={(e) => setTask(e.target.value)}
              placeholder="例如：整理這個資料夾裡的測試報告"
              maxLength={500}
            />
          </label>
          <button type="submit" className="primary">
            交代一件事
          </button>
        </form>
        <div className="row wrap">
          <span className="row wrap" role="group" aria-label="暫停或恢復主動互動">
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
          </span>
          <button onClick={() => onNavigate("connect")}>加入裝置</button>
          <button onClick={() => void stopSensors()}>停止所有感測</button>
          {/* 觸發是二段確認；「解除」刻意不在這裡——要走安全頁的恢復流程。 */}
          {estopped ? (
            <button className="estop-indicator" onClick={() => onNavigate("safety")}>
              <Icon name="octagon-x" size={16} /> 緊急停止中 — 前往解除
            </button>
          ) : (
            <ConfirmButton
              className="estop"
              label="緊急停止"
              confirmLabel="立即停止一切？"
              onConfirm={() => {
                if (onEstop) void onEstop();
                else
                  void api
                    .emergencyStop("home quick action")
                    .then(() => onNavigate("safety"))
                    .catch((e) => setPauseError(String(e)));
              }}
            />
          )}
        </div>
        {sensorNotice &&
          (sensorNotice.ok ? (
            <p className="muted small" role="status">
              {sensorNotice.message}
            </p>
          ) : (
            <p className="cap-card-error" role="alert">
              {sensorNotice.message}
            </p>
          ))}
        {pauseError && (
          <p className="cap-card-error" role="alert">
            操作失敗：{pauseError}
          </p>
        )}
      </Section>

      <details
        className="home-details"
        open={detailsOpen}
        onToggle={(e) => setDetailsOpen((e.currentTarget as HTMLDetailsElement).open)}
      >
        <summary>詳細狀態</summary>
        {detailsOpen && (
          <HomeDetails
            refreshKey={refreshKey}
            status={status}
            events={events}
            paused={pause.paused}
            pauseUntil={pause.until}
            onNavigate={onNavigate}
          />
        )}
      </details>

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

/** 詳細狀態（折疊區，展開才掛載、才查詢）：系統狀態、工作階段、自動互動、數量、
 *  交代中的工作摘要、最近一次互動、記憶與資料。 */
function HomeDetails({
  refreshKey,
  status,
  events,
  paused,
  pauseUntil,
  onNavigate,
}: {
  refreshKey: number;
  status: { loading: boolean; error?: string; data?: Record<string, unknown> };
  events: RuntimeEvent[];
  paused: boolean;
  pauseUntil?: string;
  onNavigate: (tab: string) => void;
}) {
  const [actions] = useAsync(() => api.actionsList(5), [refreshKey]);
  const [session, reloadSession] = useAsync(() => api.sessionGet(), [refreshKey]);
  // 開始／結束工作階段失敗不得靜默：畫面沒改變的話，使用者會以為狀態已經變了。
  const [sessionError, setSessionError] = React.useState<string | null>(null);
  const [providers] = useAsync(() => api.providersList(), [refreshKey]);
  const [receiptsData] = useAsync(() => api.knowledgeReceipts(), [refreshKey]);
  const estop = Boolean(status.data?.["emergencyStop"]);
  const recipes = (status.data?.["recipes"] as { loaded?: number } | undefined)?.loaded ?? 0;
  const pendingAi = Number(status.data?.["pendingAiAssists"] ?? 0);
  const cp = status.data?.["characterProtocol"] as Record<string, unknown> | undefined;
  const instances = Number(cp?.["instances"] ?? 0);
  const sensors = (status.data?.["activeSensors"] as SensorUse[] | undefined) ?? [];
  // 來源沒回報狀態（例如手機失聯）＝ 可能仍在使用，不得當作已停止。
  const sensorStateUnknown = sensors.some((s) => s.state !== undefined && s.state !== "active");
  const latestReceipt = (
    (receiptsData.data as Record<string, unknown> | undefined)?.receipts as
      | Record<string, unknown>[]
      | undefined
  )?.[0];

  return (
    <div className="home-details-body">
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
                  到「連接與權限 → 同意與安全」可以檢視原因並安全解除。
                </p>
              ) : paused ? (
                <p className="home-status-line warn">
                  <Icon name="pause" size={18} /> 主動互動已暫停
                  {pauseUntil ? `（至 ${new Date(pauseUntil).toLocaleTimeString()}）` : ""}。
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
              </p>
              {sensors.length > 0 && (
                <p className="home-status-line warn">
                  <Icon name="mic" size={16} /> 感測使用中：
                  {sensors.map((s) => sensorLabel(s.kind)).join("、")}
                  {sensorStateUnknown ? "（其中有來源沒回報狀態，視為仍在使用）" : ""}
                </p>
              )}
              {pendingAi > 0 && (
                <p className="home-status-line info">
                  <Icon name="bot" size={16} /> 有 {pendingAi} 個情境正在等 AI 協助判斷
                  （逾時會自動用本機規則處理）。
                </p>
              )}
              <div className="row wrap" style={{ marginTop: 8 }}>
                {session.loading ? null : !session.data ? (
                  <button
                    onClick={async () => {
                      try {
                        await api.sessionStart("desktop", []);
                        setSessionError(null);
                        reloadSession();
                        onNavigate("home");
                      } catch (e) {
                        setSessionError(
                          `開始工作階段失敗：${e}。工作階段沒有開始，自動互動仍然不會執行。`
                        );
                      }
                    }}
                  >
                    開始工作階段
                  </button>
                ) : (
                  <button
                    onClick={async () => {
                      try {
                        await api.sessionStop();
                        setSessionError(null);
                        reloadSession();
                      } catch (e) {
                        setSessionError(
                          `結束工作階段失敗：${e}。工作階段可能仍在進行，請重試或到工作頁確認。`
                        );
                      }
                    }}
                  >
                    結束工作階段
                  </button>
                )}
              </div>
              {sessionError && (
                <p className="cap-card-error" role="alert">
                  {sessionError}
                </p>
              )}
            </div>
          )}
        </Section>

        <Section title="數量">
          <ul className="plain-list home-counts">
            <li>已載入 {recipes} 個自動互動</li>
            <li>
              角色視窗：
              {cp ? `${instances} 個連線中` : "無法確認（系統未回報）"}
            </li>
            <li>
              裝置與整合來源：
              {providers.loading
                ? "讀取中…"
                : providers.error
                  ? "無法確認（查詢失敗）"
                  : `${(providers.data ?? []).length} 個`}
            </li>
          </ul>
        </Section>
      </div>

      <Section title="主動互動">
        <ProactiveSummary refreshKey={refreshKey} />
        <div className="row wrap" style={{ marginTop: 10 }}>
          <button onClick={() => onNavigate("automations")}>查看自動互動</button>
          <button onClick={() => onNavigate("automations")}>建立自動互動</button>
        </div>
      </Section>

      <AgentSessionsSection refreshKey={refreshKey} onNavigate={onNavigate} />

      <Section title="最近一次互動">
        <StateView state={actions} empty="還沒有任何互動。">
          {(list) => <LastInteraction receipt={(list as Receipt[])[0]} events={events} />}
        </StateView>
      </Section>

      <Section title="記憶與資料">
        {latestReceipt ? (
          <p className="muted small">
            最近更新：{knowledgeTriggerLabel(String(latestReceipt.triggeredBy ?? ""))}（
            {(latestReceipt.verification as Record<string, unknown> | undefined)?.humanReviewed ===
            true
              ? "已複審"
              : "未複審"}
            ）
          </p>
        ) : (
          <p className="muted small">尚無更新。</p>
        )}
        <button onClick={() => onNavigate("memory")}>前往記憶與資料</button>
      </Section>
    </div>
  );
}

/** 感測器種類的人話。未知種類不猜、也不外洩原始 id（`iphone.motion` 這種），
 *  一律走共用投影說「其他感測器」——「有東西在感測」這件事實仍然看得到。 */
export const sensorLabel = sensorKindLabel;

/** 知識更新來由與動作意圖的人話：定義搬到共用投影（statusProjection），
 *  活動紀錄與全域搜尋走同一份；這裡保留具名輸出給既有的引用點。 */
export { knowledgeTriggerLabel, receiptIntentLabel };

export interface CharacterSentenceInput {
  name: string;
  estop: boolean;
  paused: boolean;
  connected: boolean;
  visible: boolean;
  sensors: string[];
}

/**
 * 「角色現在怎麼樣」一句話。安全文字固定：緊急停止中／感測使用中；角色離線時用
 * 可信的固定文案（CHARACTER_OFFLINE_LINE），不由角色包決定。
 */
export function characterSentence(input: CharacterSentenceInput): string {
  const name = input.name || characterNameFallback;
  const sensing =
    input.sensors.length > 0 ? `感測使用中（${input.sensors.map(sensorLabel).join("、")}）。` : "";
  if (input.estop) return `緊急停止中：${name}已停止所有回應。`;
  if (!input.connected) return `${CHARACTER_OFFLINE_LINE}${sensing}`;
  if (!input.visible) return `${name}已連線，但目前隱藏中。${sensing}`;
  if (sensing) return `${name}在桌面上，${sensing}`;
  if (input.paused) return `${name}在桌面上，主動互動已暫停。`;
  return `${name}在桌面上，一切正常。`;
}

/** 「現在」第一屏的三個回答：角色現在怎麼樣／正在做什麼／有什麼需要處理。
 *  誠實計數：查詢失敗＝未知（不得顯示綠色 0 項）；狀態標籤走 statusProjection。 */
export function NowStrip({
  refreshKey,
  status,
  statusError,
  paused = false,
  onNavigate,
}: {
  refreshKey: number;
  status?: Record<string, unknown>;
  statusError?: string;
  paused?: boolean;
  onNavigate: (tab: string) => void;
}) {
  const character = useCharacterName();
  const [sessions] = useAsync(() => api.agentSessionsList(), [refreshKey]);
  // 「待我決定」與右上角 Inbox 徽章共用同一個 Runtime application service，
  // 不再在前端拼第二份真相（也不會因為分頁而少算）。
  const [inbox] = useAsync(() => api.activityInbox({ limit: 5 }), [refreshKey]);

  const sensors = ((status?.["activeSensors"] as { kind: string }[] | undefined) ?? []).map(
    (s) => s.kind
  );
  const presentation = status?.["presentation"] as Record<string, unknown> | undefined;
  const cp = status?.["characterProtocol"] as Record<string, unknown> | undefined;
  const active = cp?.["activeCharacter"] as
    | { characterId?: unknown; displayName?: unknown }
    | null
    | undefined;
  const activeName =
    active && typeof active.displayName === "object" && active.displayName
      ? displayNameOf({ displayName: active.displayName as Record<string, string> }, "zh-TW")
      : null;
  const activeId = active && typeof active.characterId === "string" ? active.characterId : null;
  const estop = Boolean(status?.["emergencyStop"]);
  // 「進行中」的判定與 Rust `AgentSessionState::is_open` 同義，交給共用投影
  // （含 fetched／working 這類角色 taxonomy 別名；介面不認得的狀態不算在跑）。
  const open = (sessions.data ?? []).filter((s) => isOpenWorkState(s.state));
  const pendingDegraded = Boolean(inbox.error);
  const inboxData = inbox.data as Record<string, unknown> | undefined;
  const pendingTotal = Number(inboxData?.pendingCount ?? 0);
  // 後端說 pendingCount 只是下限時，這裡的數字要說「至少」，而且 0 也不可以
  // 用綠色的「0 項」宣稱沒事——那會把「還沒撈完」講成「沒有待辦」。
  const pendingExact = isPendingCountExact(inboxData);
  const pendingItems = ((inboxData?.items as Record<string, unknown>[] | undefined) ?? [])
    .filter((item) => item.needsDecision === true)
    .slice(0, 3);

  const sentence = statusError
    ? "無法確認角色狀態（系統查詢失敗）。"
    : characterSentence({
        name: character.name,
        estop,
        paused,
        connected: presentation?.connected === true,
        visible: presentation?.visible === true,
        sensors,
      });

  return (
    <div className="now-strip now-answers">
      <div className="now-card now-answer" data-testid="now-character">
        <span className="now-title">{character.name}</span>
        <p className="now-sentence">{sentence}</p>
        {activeId && character.characterId && activeId !== character.characterId && activeName && (
          <p className="muted small">目前連線的是另一個角色：{activeName}。</p>
        )}
        <button onClick={() => onNavigate("companion")}>前往{character.name}</button>
      </div>
      <div className="now-card now-answer" data-testid="now-work">
        <span className="now-title">進行中的工作</span>
        {sessions.error ? (
          <Badge kind="warn">無法確認進行中的工作</Badge>
        ) : open.length > 0 ? (
          <Badge kind="pending">{open.length} 個工作階段</Badge>
        ) : (
          <Badge kind="ok">沒有進行中</Badge>
        )}
        {open.length > 0 && (
          <ul className="plain-list now-list">
            {open.slice(0, 3).map((s) => {
              const st = projectWorkState(s.state);
              return (
                <li key={s.sessionId}>
                  <span>{s.label ?? agentDisplayLabel(s.agentId)}</span>
                  <Badge kind={st.badge}>{st.label}</Badge>
                </li>
              );
            })}
            {open.length > 3 && <li className="muted small">…還有 {open.length - 3} 件</li>}
          </ul>
        )}
        <button onClick={() => onNavigate("work")}>查看工作</button>
      </div>
      <div className="now-card now-answer" data-testid="now-decisions">
        <span className="now-title">待我決定</span>
        {pendingDegraded ? (
          <Badge kind="warn">無法確認（查詢失敗）</Badge>
        ) : pendingTotal > 0 || !pendingExact ? (
          <Badge kind="warn">{pendingCountLabel(pendingTotal, pendingExact)}</Badge>
        ) : (
          <Badge kind="ok">0 項</Badge>
        )}
        {!pendingDegraded && !pendingExact && (
          <p className="muted small" role="status">
            {PENDING_INCOMPLETE_NOTE}。
          </p>
        )}
        {pendingItems.length > 0 && (
          <ul className="plain-list now-list">
            {pendingItems.map((item) => (
              <li key={`${String(item.kind)}-${String(item.itemId)}`}>
                <Badge kind="warn">{projectInboxStatus(String(item.status)).label}</Badge>
                {/* 安全事件在舊 daemon 的 title 是原始事件型別（`emergency.stop`）——
                    一般模式第一屏不得印它，走與通知中心同一份人話投影。 */}
                <span>{inboxItemTitle(item)}</span>
                <button onClick={() => onNavigate(String(item.route))}>前往</button>
              </li>
            ))}
          </ul>
        )}
        <button onClick={() => onNavigate("activity")}>查看全部</button>
      </div>
    </div>
  );
}

export function ProactiveSummary({ refreshKey }: { refreshKey: number }) {
  // 與本頁其他查詢一致吃 refreshKey：CLI／HTTP／tray 改配方時
  // 「啟用 N 個」與「都不使用 AI」的誠實宣稱才會跟著更新。
  const [recipes] = useAsync(() => api.recipesList(), [refreshKey]);
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
        <span>系統收到「{receiptIntentLabel(receipt.intent)}」相關的訊號</span>
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

/** 資料範圍的人話：一般模式不該看到 `workspace:/path` 這種原始 scope 字串。 */
export function dataScopeLabel(scope: string): string {
  if (scope.startsWith("workspace:")) return `資料夾 ${scope.slice("workspace:".length)}`;
  if (scope.startsWith("domain:")) return `知識領域「${scope.slice("domain:".length)}」`;
  if (scope.startsWith("memory:")) return `記憶「${scope.slice("memory:".length)}」`;
  if (scope.startsWith("device:")) return `裝置 ${scope.slice("device:".length)}`;
  return scope;
}

const TOOL_SCOPE_LABEL: Record<string, string> = {
  "workspace.write": "可以修改這個資料夾裡的檔案",
  "workspace.read": "只能讀取這個資料夾",
  "network.fetch": "可以連外部網路",
};

/** 工具範圍的人話；未知的 scope 不美化、原樣顯示（不假裝理解）。 */
export function toolScopeLabel(scope: string): string {
  return TOOL_SCOPE_LABEL[scope] ?? scope;
}

/** 交代中的工作（首頁只給一行摘要）：完整清單、權限範圍、期限與取消都住在工作頁，
 *  首頁不放第二份（規格 §12.1「不要在首頁重複全部工作階段」）。 */
function AgentSessionsSection({
  refreshKey,
  onNavigate,
}: {
  refreshKey: number;
  onNavigate: (tab: string) => void;
}) {
  const [sessions] = useAsync(() => api.agentSessionsList(), [refreshKey]);
  if (sessions.loading) return null;
  // 查詢失敗＝未知，不得顯示綠色的「沒有進行中」。
  if (sessions.error) {
    return (
      <Section title="交代中的工作">
        <p className="muted small" role="status">
          無法確認目前交代中的工作（查詢失敗）。
        </p>
        <button onClick={() => onNavigate("work")}>前往工作</button>
      </Section>
    );
  }
  const open = (sessions.data ?? []).filter((s) => !s.closedAt);
  if (open.length === 0) return null;

  return (
    <Section title="交代中的工作">
      <p className="muted small">
        目前有 {open.length} 件交代中的工作。它們的回報是「聲稱」，實際結果要等你檢查確認
        之後才算數；詳細狀態、可用期限與取消都在工作頁。
      </p>
      <button onClick={() => onNavigate("work")}>前往工作</button>
    </Section>
  );
}
