// 角色頁的「同步」卡：手機上的角色與這台電腦是不是同一個狀態。
//
// 契約：`docs/aip/character-session.md` §11（一般模式文案）＋
// `docs/aip/transport-bindings.md` §2／§3（四條路由與 SSE）。UI 只做投影：
//
//   * 每一句都來自 Runtime 的真實回應（權威快照＋已配對裝置清單＋來源狀態），
//     沒有示範資料、沒有樂觀預設。
//   * 讀不到就說讀不到；連續讀不到才升級成「無法恢復，請重新連接」（契約 §7.5）。
//   * 空狀態不像成功：一台裝置都沒有時是中性的「尚未連接 iPhone」。
//   * 保存的同步紀錄壞掉過也要說（`storeNote` → 「角色同步紀錄曾損毀，已重新開始」），
//     一般模式只給人話，不給後端原文。
//   * 一般模式看不到任何技術數字；進階模式才展開「連接診斷」。
//   * 模擬 iPhone（fixture）的名稱自帶標籤，原樣顯示，不得被寫成真機。
//   * 緊急停止中的固定安全句由這裡（可信 host 介面）顯示，角色無法覆寫。
//
// 對齊策略（為什麼不是每則事件都重問一次）：
//
//   * `GET /v1/character-session` **會消耗一個 session sequence**（它是一則真的
//     `state{kind:"snapshot"}` envelope）。Runtime 的每一則事件都重取一次，等於讓
//     一個唯讀的畫面推著權威 session 的 sequence 前進。
//   * 所以：權威狀態改由 SSE `character.session.state` 的 payload（完整 envelope）
//     直接更新本地副本——`snapshot` 整份取代，`patch` 用 RFC 7396 merge patch 套上去。
//     只有「首次載入」「patch 的 baseRevision／epoch 接不上」「使用者按重新檢查」
//     才會再 GET 一次。
//   * **桌面端刻意不做接收端 hash 核對。** 理由：JS 的 number 留不住數字字面
//     （Rust 端的 `0.0` 在 JS 重新序列化後是 `0`），重算出來的 canonical JSON 不會與
//     Rust 端逐位元組相同，hash 一定對不上——那會變成一個永遠亮著的假警報。
//     不一致時的處理是「重新 GET 一次 snapshot 對齊」，判斷依據是 revision 單調
//     遞增與 baseRevision 相符，不是 hash。
//   * 裝置清單／來源清單／診斷是另一組（它們不消耗 sequence，但一樣不必每則事件重打）：
//     節流成最小間隔 2 秒的 trailing 重取。

import React from "react";
import { api, type CharacterSessionDiagnostics, type RuntimeEvent } from "../api";
import { applyMergePatch } from "../aip/envelope";
import {
  characterSyncLastInteraction,
  characterSyncMemberDeviceIds,
  characterSyncMembers,
  characterSyncPresenceLabel,
  characterSyncSafetyNote,
  projectCharacterSession,
  type CharacterSyncSignals,
} from "../statusProjection";
import { Badge } from "../ui";

/** 後端把角色同步整個關掉時的穩定錯誤碼（HTTP 503，AIP §12）。 */
const SESSION_DISABLED = "session-disabled";
/** 同一件事在 Runtime 內嵌模式的固定訊息（`SESSION_DISABLED_MESSAGE`）。 */
const SESSION_DISABLED_MESSAGE = "character session is turned off";
/** 手機在來源清單裡的 id 前綴（Runtime `mobile.rs` 的 `provider.mobile.<id>`）。 */
const MOBILE_PROVIDER_PREFIX = "provider.mobile.";
/** SSE 事件型別（`docs/aip/transport-bindings.md` §3）。 */
const SESSION_STATE_EVENT = "character.session.state";
/** 裝置清單／來源清單／診斷的最小重取間隔（trailing）。 */
export const SYNC_SLOW_REFRESH_MIN_MS = 2_000;

/**
 * live 模式下讀不到權威狀態時的退避重試間隔（毫秒），有界且不無限成長。
 *
 * live 模式只有掛載／使用者按「重新檢查」／patch 接不上才會重新 GET，所以後端一時
 * 讀不到時 `failedReads` 會永遠停在 1，畫面永遠停在 pending 的「同步尚未完成」
 * ——那句話暗示「正在進行中」，但其實什麼都沒有在進行；契約 §7.5 的
 * 「連續 3 次失敗 → 無法恢復，請重新連接」在這條路徑上也永遠達不到
 * （對抗審查 general-mode-ux-024）。所以失敗才排一次重試，成功就停。
 */
export const SYNC_RETRY_BACKOFF_MS: readonly number[] = [1_000, 3_000, 10_000, 30_000];

/** 本地保存的一份權威狀態副本（只給投影用；沒有任何權力）。 */
export interface LocalSessionState {
  state: Record<string, unknown>;
  revision: number;
  epoch: number;
}

/** 一則 `character.session.state` 事件對本地副本的意義。 */
export type SessionAlignment =
  | { kind: "aligned"; session: LocalSessionState }
  /** 接不上：必須重新 GET 一次 snapshot（不硬套、不猜）。 */
  | { kind: "realign" }
  /** 落後或無關的訊息：忽略（rollback 防護）。 */
  | { kind: "ignored" };

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** `state{kind:"snapshot"}` envelope → 本地副本（讀不出來就是 null）。 */
export function readSnapshotEnvelope(envelope: unknown): LocalSessionState | null {
  const payload = record(record(envelope)?.["payload"]);
  const state = record(payload?.["state"]);
  if (!payload || !state) return null;
  return {
    state,
    revision: num(payload["revision"]) ?? 0,
    epoch: num(payload["sessionEpoch"]) ?? 0,
  };
}

/**
 * 把一則 SSE `character.session.state` 的 envelope 對到本地副本上（純函式）。
 *
 * `snapshot` 整份取代；`patch` 只有在 epoch 相同且 `baseRevision` 正好等於本地
 * revision 時才套用（AIP §6：接收端只認 revision，不認 sequence）。落後的重播一律
 * 忽略；接不上的一律回 `realign`，由呼叫端重新 GET —— 絕不半套半猜。
 */
export function alignSession(current: LocalSessionState | null, envelope: unknown): SessionAlignment {
  const outer = record(envelope);
  const payload = record(outer?.["payload"]);
  if (!payload) return { kind: "ignored" };
  const epoch = num(payload["sessionEpoch"]) ?? 0;
  if (payload["kind"] === "snapshot") {
    const next = readSnapshotEnvelope(outer);
    return next ? { kind: "aligned", session: next } : { kind: "ignored" };
  }
  if (payload["kind"] !== "patch") return { kind: "ignored" };
  const revision = num(payload["revision"]);
  if (revision === null) return { kind: "ignored" };
  if (!current) return { kind: "realign" };
  // host 重建過 session：本地副本整份作廢，只能重新對齊。
  if (epoch !== current.epoch) return { kind: "realign" };
  if (revision <= current.revision) return { kind: "ignored" };
  if (num(outer?.["baseRevision"]) !== current.revision) return { kind: "realign" };
  const state = record(applyMergePatch(current.state, payload["patch"]));
  if (!state) return { kind: "realign" };
  return { kind: "aligned", session: { state, revision, epoch } };
}

/** 已配對裝置清單 → 「裝置識別碼 → 這台電腦上的顯示名稱」。 */
export function deviceNames(mobile: unknown): Record<string, string> {
  const devices = record(mobile)?.["devices"];
  const names: Record<string, string> = {};
  if (!Array.isArray(devices)) return names;
  for (const entry of devices) {
    const device = record(entry);
    const id = typeof device?.["deviceId"] === "string" ? String(device["deviceId"]) : "";
    const name = typeof device?.["name"] === "string" ? String(device["name"]).trim() : "";
    if (id) names[id] = name.length > 0 ? name : "一台裝置";
  }
  return names;
}

/** 連著這台電腦的手機（不是「配對過」，是現在真的連著）。 */
function connectedDeviceIds(mobile: unknown): string[] {
  const devices = record(mobile)?.["devices"];
  if (!Array.isArray(devices)) return [];
  return devices
    .map((entry) => record(entry))
    .filter((device) => device?.["connected"] === true)
    .map((device) => String(device?.["deviceId"] ?? ""))
    .filter((id) => id.length > 0);
}

/** 有沒有裝置的授權被撤銷過（撤銷之後要重新確認才會再同步）。 */
export function hasRevokedDevice(providers: unknown): boolean {
  if (!Array.isArray(providers)) return false;
  return providers.some((entry) => {
    const provider = record(entry);
    const id = String(record(provider?.["identity"])?.["id"] ?? "");
    return id.startsWith(MOBILE_PROVIDER_PREFIX) && provider?.["state"] === "revoked";
  });
}

/** 後端說「角色同步關閉」了嗎（只認這兩個穩定字串，其他失敗一律當成「讀不到」）。 */
function isSessionDisabled(error: unknown): boolean {
  const text = String(error);
  return text.includes(SESSION_DISABLED) || text.includes(SESSION_DISABLED_MESSAGE);
}

export function CharacterSyncCard({
  refreshKey,
  advanced = false,
  sessionEvents,
}: {
  refreshKey: number;
  /** 進階模式才顯示「連接診斷」（revision／sequence／計數）。 */
  advanced?: boolean;
  /**
   * Runtime SSE 事件（由頁面傳入）。**有傳**就代表這個畫面收得到
   * `character.session.state`，權威狀態改由事件對齊，`refreshKey` 不再驅動重取。
   * 沒傳（例如只渲染這張卡的測試）就退回「每次 refreshKey 重問一次」的老路——
   * 收不到事件時不假裝自己是最新的。
   */
  sessionEvents?: readonly RuntimeEvent[];
}) {
  const live = sessionEvents !== undefined;
  const [session, setSession] = React.useState<LocalSessionState | null>(null);
  const sessionRef = React.useRef<LocalSessionState | null>(null);
  const [loaded, setLoaded] = React.useState(false);
  const [enabled, setEnabled] = React.useState(true);
  // 連續讀不到權威狀態的次數：達 3 次才升級成「無法恢復」（契約 §7.5）。
  // 計數在 ref（每一次真的失敗都算，即使那一輪的畫面已經被下一輪取代），
  // state 只是讓畫面跟著更新。
  const failedReadsRef = React.useRef(0);
  const [failedReads, setFailedReads] = React.useState(0);
  const [names, setNames] = React.useState<Record<string, string>>({});
  const [connected, setConnected] = React.useState<string[]>([]);
  const [revoked, setRevoked] = React.useState(false);
  const [diagnostics, setDiagnostics] = React.useState<CharacterSessionDiagnostics | null>(null);
  const [reload, setReload] = React.useState(0);
  /** patch 接不上時 +1：唯一會讓畫面主動再 GET 一次 snapshot 的訊號。 */
  const [realign, setRealign] = React.useState(0);

  const commitSession = React.useCallback((next: LocalSessionState | null) => {
    sessionRef.current = next;
    setSession(next);
  }, []);

  // --- A. 權威狀態：首次載入／手動重新檢查／接不上時重新對齊。
  //     `live` 時 refreshKey 不參與（那正是「每則事件三支 API」的來源）。
  const pollKey = live ? 0 : refreshKey;
  /** 讀取失敗後的退避重試計數（成功歸零）；只在 live 模式排程。 */
  const [retryTick, setRetryTick] = React.useState(0);
  const retryAttemptRef = React.useRef(0);
  React.useEffect(() => {
    let alive = true;
    let timer: number | null = null;
    void (async () => {
      try {
        const envelope = await api.characterSessionSnapshot();
        failedReadsRef.current = 0;
        retryAttemptRef.current = 0;
        if (!alive) return;
        commitSession(readSnapshotEnvelope(envelope));
        setEnabled(true);
        setFailedReads(0);
      } catch (e) {
        // 後端明說關閉是一個確定的事實，不是「讀失敗」（不計進連續失敗）。
        const disabled = isSessionDisabled(e);
        if (!disabled) failedReadsRef.current += 1;
        if (!alive) return;
        // 讀不到就是讀不到：本地副本作廢，不用上一次的樣子冒充現在。
        commitSession(null);
        setEnabled(!disabled);
        setFailedReads(failedReadsRef.current);
        // live 模式沒有輪詢，得自己排一次退避重試；關閉是確定的事實，不重試。
        if (live && !disabled) {
          const attempt = retryAttemptRef.current;
          retryAttemptRef.current = attempt + 1;
          const waitMs =
            SYNC_RETRY_BACKOFF_MS[Math.min(attempt, SYNC_RETRY_BACKOFF_MS.length - 1)] ??
            SYNC_RETRY_BACKOFF_MS[SYNC_RETRY_BACKOFF_MS.length - 1] ??
            30_000;
          timer = window.setTimeout(() => {
            if (alive) setRetryTick((n) => n + 1);
          }, waitMs);
        }
      } finally {
        if (alive) setLoaded(true);
      }
    })();
    return () => {
      alive = false;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [pollKey, reload, realign, retryTick, live, commitSession]);

  // --- B. 裝置清單／來源清單／診斷：節流（trailing，最小間隔 2 秒）。
  //     首次載入、手動重新檢查、切換進階模式一律立刻執行。
  const lastSlowAt = React.useRef(0);
  const slowShape = React.useRef({ reload, advanced, first: true });
  React.useEffect(() => {
    let alive = true;
    const run = async () => {
      lastSlowAt.current = Date.now();
      let mobile: unknown = null;
      let providers: unknown = null;
      try {
        mobile = await api.mobileStatus();
      } catch {
        /* 裝置清單讀不到：名字就查不到，投影會用中性稱呼 */
      }
      try {
        providers = await api.providersList();
      } catch {
        /* 來源清單讀不到：不假設沒有被撤銷過的裝置，也不假設有 */
      }
      if (!alive) return;
      setNames(deviceNames(mobile));
      setConnected(connectedDeviceIds(mobile));
      setRevoked(hasRevokedDevice(providers));
      try {
        // 診斷一般模式也要讀：`storeNote` 是「紀錄曾損毀」唯一的來源，不得靜默。
        // 讀到的數字**不會**進一般模式的畫面（只有進階模式的收合區塊會顯示）。
        const value = await api.characterSessionDiagnostics();
        if (alive) setDiagnostics(value);
      } catch {
        if (alive) setDiagnostics(null);
      }
    };
    const previous = slowShape.current;
    const immediate =
      previous.first || previous.reload !== reload || previous.advanced !== advanced;
    slowShape.current = { reload, advanced, first: false };
    if (immediate) {
      void run();
      return () => {
        alive = false;
      };
    }
    const wait = Math.max(0, SYNC_SLOW_REFRESH_MIN_MS - (Date.now() - lastSlowAt.current));
    const timer = window.setTimeout(() => void run(), wait);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [refreshKey, reload, advanced]);

  // --- C. SSE：權威狀態的變更直接套在本地副本上（不重取、不做 hash 核對）。
  const lastEventSequence = React.useRef(-1);
  React.useEffect(() => {
    if (!sessionEvents) return;
    for (const event of sessionEvents) {
      if (event.eventType !== SESSION_STATE_EVENT) continue;
      if (typeof event.sequence === "number" && event.sequence <= lastEventSequence.current) {
        continue;
      }
      if (typeof event.sequence === "number") lastEventSequence.current = event.sequence;
      const alignment = alignSession(sessionRef.current, event.payload);
      if (alignment.kind === "aligned") commitSession(alignment.session);
      // 接不上：重新 GET 一次（效果 A），不硬套一個對不上的補丁。
      else if (alignment.kind === "realign") setRealign((n) => n + 1);
    }
  }, [sessionEvents, commitSession]);

  // --- 投影（純函式；所有句子都來自上面這些真實回應）。
  const snapshotView = React.useMemo(
    () => (session ? { payload: { kind: "snapshot", state: session.state } } : null),
    [session]
  );
  const members = React.useMemo(
    () => characterSyncMembers(snapshotView, names),
    [snapshotView, names]
  );
  const projection = React.useMemo(() => {
    if (!loaded) return null;
    const synced = new Set(characterSyncMemberDeviceIds(snapshotView));
    const signals: CharacterSyncSignals = {
      enabled,
      failedReads,
      revokedDevice: revoked,
      // 連著、但不在成員名單裡的手機＝還沒重新確認過（送不出互動，也收不到狀態）。
      connectedButNotSynced: connected.some((id) => !synced.has(id)),
      storeReset: diagnostics?.storeNote != null,
    };
    return projectCharacterSession(snapshotView, members, signals);
  }, [loaded, snapshotView, members, enabled, failedReads, revoked, connected, diagnostics]);
  const lastInteraction = React.useMemo(
    () => characterSyncLastInteraction(snapshotView, names),
    [snapshotView, names]
  );
  const safetyNote = React.useMemo(() => characterSyncSafetyNote(snapshotView), [snapshotView]);

  const remote = members.filter((m) => m.remote);
  // 安全狀態壓過同步狀態：緊急停止中，同步「技術上」也許還好好的，但綠色徽章
  // 讀起來就是「一切正常」，會和正下方的固定安全句互相矛盾。有安全訊息時一律
  // 不給綠色（句子本身照實不改：已同步就是已同步）。
  const tone = safetyNote ? "bad" : (projection?.tone ?? "muted");

  return (
    <div className="character-sync" data-testid="character-sync" role="region" aria-label="角色同步">
      <div className="row space-between wrap">
        <strong>同步</strong>
        {projection ? (
          <Badge kind={tone}>{projection.headline}</Badge>
        ) : (
          <Badge kind="muted">正在讀取</Badge>
        )}
      </div>
      <p className="muted small">
        {projection ? projection.detail : "正在讀取角色目前的同步狀態…"}
      </p>
      {safetyNote && (
        <p className="cap-card-error" role="alert">
          {safetyNote}
        </p>
      )}
      {remote.length > 0 && (
        <ul className="plain-list small" aria-label="同步中的裝置">
          {remote.map((m, index) => (
            <li key={`${m.name}-${index}`}>
              <span>{m.name}</span>
              <span className="muted">：</span>
              <span>{characterSyncPresenceLabel(m.presence)}</span>
            </li>
          ))}
        </ul>
      )}
      {lastInteraction && (
        <p className="small" role="status">
          最近互動：{lastInteraction}
        </p>
      )}
      <div className="connect-area-actions">
        <button onClick={() => setReload((n) => n + 1)}>重新檢查</button>
      </div>
      {advanced && (
        <details className="tech-details">
          <summary>連接診斷</summary>
          {diagnostics ? (
            <ul className="plain-list small">
              <li>revision {diagnostics.revision}</li>
              <li>sequence {diagnostics.sequence}</li>
              <li>sessionEpoch {diagnostics.sessionEpoch}</li>
              <li>
                eventLog {diagnostics.eventLog.len}/{diagnostics.eventLog.cap}
              </li>
              {Object.entries(diagnostics.counters).map(([key, value]) => (
                <li key={key}>
                  {key} {value}
                </li>
              ))}
              {diagnostics.storeNote && <li>storeNote {diagnostics.storeNote}</li>}
            </ul>
          ) : (
            <p className="muted small">目前讀不到連接診斷。</p>
          )}
        </details>
      )}
    </div>
  );
}
