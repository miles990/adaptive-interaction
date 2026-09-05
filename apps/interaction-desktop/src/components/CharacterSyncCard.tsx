// 角色頁的「同步」卡：手機上的角色與這台電腦是不是同一個狀態。
//
// 契約：`docs/aip/character-session.md` §11（一般模式文案）＋
// `docs/aip/transport-bindings.md` §2／§3（四條路由與 SSE）。UI 只做投影：
//
//   * 每一句都來自 Runtime 的真實回應（權威快照＋已配對裝置清單＋來源狀態），
//     沒有示範資料、沒有樂觀預設。
//   * 讀不到就說讀不到；連續讀不到才升級成「無法恢復，請重新連接」（契約 §7.5）。
//   * 空狀態不像成功：一台裝置都沒有時是中性的「尚未連接 iPhone」。
//   * 保存層的兩類訊號分開：**現在**存不下來（parked／持續寫入失敗）是狀態，
//     「曾經重建過」（`storeNote`）只是一句 muted 的歷史通知——它在同一次 daemon
//     執行期間永遠不會清，當成警告會讓使用者從此再也看不到綠色（M3 §4.3b）。
//     兩者都只給人話，不給後端原文。
//   * 每一態的「下一步」是穩定的 action id；沒有 `onNavigate`（到不了那裡）就不給按鈕。
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
//     直接更新本地副本。**所有協定判斷都在 `../aip/sessionClient.ts` 的純 reducer 裡**
//     （`docs/aip/character-session.md` §7.2 的接收端決策表：世代 → 身分 → 格式 →
//     `session-reset` → epoch → `recovery` → revision → hash）；這個元件只負責發請求、
//     把回應（連同它所屬的連線世代）餵進去、把結果投影成人話。
//   * 已經有本地副本時，「重新檢查」／重新對齊／連線切換走
//     `POST /v1/character-session/resume`（帶 lastRevision／lastSequence／epoch），
//     **不**再 GET：resume 不消耗 sequence。只有「沒有本地副本」（首次載入、讀失敗
//     之後、卸載重掛）才 GET。
//   * `connectionKey` 是「這條連線換了一條」的訊號（App 在 supervisor 連線狀態變化時 +1），
//     不是「有新事件」——它一次連線只動一兩下，不會退回「每則事件三支 API」。
//     `refreshKey` 每一則 runtime 事件都 +1，所以 live 模式下它**不**驅動權威狀態的重取。
//     它同時是決策表規則 0 的**連線世代**：飛行中的 GET／resume 回覆帶著發出當下的世代
//     回到 reducer，連線在途中換過就是舊連線的遲到品，一律不算數。
//   * **桌面端現在會做接收端 hash 核對**（AIP §6）。過去不做的理由是「JS 的 number
//     留不住數字字面」；`../aip/canonical.ts` 依 codegen 從跨語言 fixture 產出的
//     double 路徑重印字面，逐位元組核對過（`src/test/canonical-hash.test.ts`），
//     所以那個理由沒了。對不上就**不套用**，改要求重新對齊——不硬套、不猜。
//   * 裝置清單／來源清單／診斷是另一組（它們不消耗 sequence，但一樣不必每則事件重打）：
//     節流成最小間隔 2 秒的 trailing 重取。

import React from "react";
import { api, type CharacterSessionDiagnostics, type RuntimeEvent } from "../api";
import {
  initialSession,
  isPatchEnvelope,
  reduce,
  type LocalSessionState,
  type SessionInput,
  type SessionMachine,
} from "../aip/sessionClient";
import {
  characterSyncLastInteraction,
  characterSyncMemberDeviceIds,
  characterSyncMembers,
  characterSyncPresenceLabel,
  characterSyncProfileLabel,
  characterSyncProfiles,
  characterSyncSafetyNote,
  characterSyncStoreSignals,
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

/**
 * 連續讀不到權威狀態幾次算「無法恢復」（契約 §7.5，與 `statusProjection` 的門檻一致）。
 */
const UNRECOVERABLE_READS = 3;

/**
 * 本地副本與對齊結果的型別都在 reducer 那一側。
 * 這裡 re-export 是為了讓既有的呼叫端／測試不必同時 import 兩個模組。
 */
export type { LocalSessionState, SessionAlignment } from "../aip/sessionClient";

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
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

/**
 * 有沒有裝置的授權被撤銷過。
 *
 * 這是**歷史事實**：Runtime 撤銷後會把 provider 條目留成 `revoked`（永遠留著）。
 * 零裝置時它代表「以前移除過手機」＝`local-only` 的終態；只有那台裝置**現在**又
 * 連上來（`connectedButNotSynced`）才需要人動手重新確認（M3 §4.3）。
 */
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
  connectionKey = 0,
  onNavigate,
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
  /**
   * 「這條連線換了一條」的訊號（App 在 supervisor 連線狀態變化／SSE 重連時 +1）。
   *
   * 刻意**不**重用 `refreshKey`：那個每一則 runtime 事件都會 +1，拿它當對齊訊號
   * 就等於退回「每則事件三支 API」。斷線期間漏掉的狀態要靠一次 resume 補回來，
   * 而不是靠事件流自己接上——所以這裡必須是一個獨立的、一次連線只動一兩下的數字。
   */
  connectionKey?: number;
  /**
   * 導覽（深連結）。**沒傳就不渲染主要動作按鈕**——一顆按了沒有反應的按鈕比沒有按鈕更糟。
   *
   * 落點由投影的 `action.target` 決定（機器語意穩定的 action id，文案可改寫）；
   * `deviceId` 只是給呼叫端比對用的參數，**不會**出現在任何畫面上。
   */
  onNavigate?: (tab: string, opts?: { hub?: string; deviceId?: string }) => void;
}) {
  const live = sessionEvents !== undefined;
  // 協定狀態機。ref 是「現在的真相」（dispatch 要同步讀它），state 只是讓畫面跟著更新。
  const machineRef = React.useRef<SessionMachine>(initialSession());
  const [machine, setMachine] = React.useState<SessionMachine>(machineRef.current);
  const session: LocalSessionState | null = machine.local;
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
  /**
   * reducer 說「接不上」時 +1：唯一會讓畫面主動再要一次對齊的訊號
   * （有本地副本走 resume，沒有才 GET）。有界——連續失敗達上限就換成
   * `unrecoverable`，不會變成打不完的請求迴圈。
   */
  const [realign, setRealign] = React.useState(0);

  /**
   * 把一件事餵進協定狀態機，並照它回的 effect 行動。
   *
   * `effects` 是「應該做的事」，不是「已經發生的事」：`realign` 去要一次對齊，
   * `unrecoverable` 代表連續對齊失敗達上限——狀態是**未知**的，本地副本作廢，
   * 畫面照契約 §7.5 升級成「無法恢復，請重新連接」，而不是繼續打一個打不完的迴圈。
   */
  const dispatch = React.useCallback((input: SessionInput) => {
    const step = reduce(machineRef.current, input);
    let next = step.next;
    if (step.effects.some((effect) => effect.kind === "unrecoverable")) {
      next = reduce(next, { kind: "reset-local" }).next;
      failedReadsRef.current = Math.max(failedReadsRef.current, UNRECOVERABLE_READS);
      setFailedReads(failedReadsRef.current);
    } else if (step.effects.some((effect) => effect.kind === "realign")) {
      setRealign((n) => n + 1);
    }
    machineRef.current = next;
    setMachine(next);
  }, []);

  // --- 0. 連線世代（決策表規則 0）。宣告順序就是執行順序：這個 effect 一定跑在下面
  //     發請求與餵 SSE 的 effect 之前，所以那兩邊看到的世代永遠是最新的。
  const connectionGenerationRef = React.useRef(connectionKey);
  React.useEffect(() => {
    connectionGenerationRef.current = connectionKey;
    dispatch({ kind: "connection-changed", generation: connectionKey });
  }, [connectionKey, dispatch]);

  // --- A. 權威狀態：首次載入／手動重新檢查／接不上時重新對齊／連線換了一條。
  //     `live` 時 refreshKey 不參與（那正是「每則事件三支 API」的來源）。
  const pollKey = live ? 0 : refreshKey;
  /** 讀取失敗後的退避重試計數（成功歸零）；只在 live 模式排程。 */
  const [retryTick, setRetryTick] = React.useState(0);
  const retryAttemptRef = React.useRef(0);
  /** 請求世代：只有「最近一次發出的請求」的回應算數（慢的回應不得蓋回舊狀態）。 */
  const requestIdRef = React.useRef(0);
  React.useEffect(() => {
    let alive = true;
    let timer: number | null = null;
    const requestId = (requestIdRef.current += 1);
    // 這次請求屬於「現在這條連線」；回覆帶著它回來，連線在途中換過就不算數（規則 0）。
    const arrivedOn = connectionKey;
    dispatch({ kind: "fetch-issued", requestId });
    // 有本地副本就走 resume（不消耗 sequence）；沒有才 GET 一份完整快照。
    const known = machineRef.current.local;
    void (async () => {
      try {
        if (known) {
          const payload = await api.characterSessionResume({
            lastRevision: known.revision,
            lastSequence: known.sequence ?? 0,
            epoch: known.epoch,
          });
          if (!alive) return;
          dispatch({ kind: "resume-response", requestId, payload, arrivedOn });
        } else {
          const envelope = await api.characterSessionSnapshot();
          if (!alive) return;
          dispatch({ kind: "fetch-response", requestId, envelope, arrivedOn });
        }
        failedReadsRef.current = 0;
        retryAttemptRef.current = 0;
        if (!alive) return;
        setEnabled(true);
        setFailedReads(0);
      } catch (e) {
        // 後端明說關閉是一個確定的事實，不是「讀失敗」（不計進連續失敗）。
        const disabled = isSessionDisabled(e);
        if (!disabled) failedReadsRef.current += 1;
        if (!alive) return;
        // 讀不到就是讀不到：本地副本作廢，不用上一次的樣子冒充現在。
        dispatch({ kind: "reset-local" });
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
  }, [pollKey, reload, realign, retryTick, connectionKey, live, dispatch]);

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

  // --- C. SSE：權威狀態的變更直接餵進 reducer（不重取）。
  //
  //     去重**不再**用 `RuntimeEvent.sequence`：daemon 重啟後那個序號從 1 重新開始
  //     （`crates/interaction-events` 的 AtomicU64 起始 1），舊的高水位會讓重啟後
  //     所有 state 事件被永久丟棄。改由 reducer 用 AIP 的 `messageId`
  //     （沒有就退回 sessionEpoch+sequence）做有界環去重；就算沒有任何去重鍵，
  //     revision 規則本身也會把重播判成 already-applied，不會重複套用。
  //
  //     下面那個游標是另一件事（不是去重）：它決定「這個陣列裡哪幾則是**新到的**」，
  //     免得每次陣列換參考就把整個保留視窗重放一遍。
  /**
   * 已經餵過的最後一則事件（身分＋序號）。
   *
   * `sessionEvents` 是 App 的**保留視窗**（`[...prev.slice(-299), event]`，重連時還會整批
   * 換成 `eventsRecent(200)`），不是「這一刻發生的事」。沒有游標就等於每次陣列換參考都把
   * 整個視窗重播一次——對 reducer 而言那是一批全新訊息，沒有本地副本時每一則 patch 都會
   * 花掉一次對齊預算，三則就誤報「無法恢復」（對抗審查 session-client-rollback-036）。
   */
  const feedCursorRef = React.useRef<{ event: RuntimeEvent | null; sequence: number | null }>({
    event: null,
    sequence: null,
  });
  const feedPrimedRef = React.useRef(false);

  React.useEffect(() => {
    if (!sessionEvents) return;
    const tail = sessionEvents.length > 0 ? sessionEvents[sessionEvents.length - 1] : null;
    const cursor = feedCursorRef.current;
    const advanceCursor = () => {
      feedCursorRef.current = {
        event: tail,
        sequence: tail ? tail.sequence : feedCursorRef.current.sequence,
      };
    };
    // 掛載當下已經在視窗裡的，是這張卡開始聽之前的歷史：不重播。這一刻正好有一個權威
    // 讀取在飛（effect A 宣告在前，一定已經送出），最新的狀態馬上就會到。
    if (!feedPrimedRef.current) {
      feedPrimedRef.current = true;
      advanceCursor();
      return;
    }
    const index = cursor.event ? sessionEvents.lastIndexOf(cursor.event) : -1;
    let fresh: readonly RuntimeEvent[];
    if (index >= 0) {
      fresh = sessionEvents.slice(index + 1);
    } else if (cursor.sequence === null) {
      // 還沒餵過任何一則（掛載時視窗是空的）：整個陣列都是新的。
      fresh = sessionEvents;
    } else if (tail !== null && tail.sequence > cursor.sequence) {
      // 整批換掉、但確實比看過的更新（重連回填）：只取真的比較新的那些。
      const seen = cursor.sequence;
      fresh = sessionEvents.filter((event) => event.sequence > seen);
    } else {
      // 整批換掉而且不比看過的新：可能是回填的歷史，也可能是 daemon 重啟後序號從 1 重來
      //（`crates/interaction-events` 的 AtomicU64 起始 1）。兩者都不重播歷史；真的漏掉的
      // 狀態由連線世代變化那一次 resume／GET 補回來（App 換整批事件時一定會 +1 connectionKey）。
      // 游標照樣往前，否則重啟之後的新事件會被永久當成「舊的」丟掉。
      fresh = [];
    }
    advanceCursor();
    // 沒有本地副本時，補丁只可能得到「沒有東西可以套上去」（規則 10）。那不是一次失敗的
    // 對齊往返，不該花掉契約 §7.5 的預算：正在飛的那個權威讀取就是在補這件事。所以這一批
    // 裡最多只讓一則補丁走進 reducer（由它要一次對齊），其餘略過；snapshot 不受影響
    // （它自己就能 bootstrap）。
    let alignmentPending = machineRef.current.pendingRequestId !== null;
    for (const event of fresh) {
      if (event.eventType !== SESSION_STATE_EVENT) continue;
      if (machineRef.current.local === null && isPatchEnvelope(event.payload)) {
        if (alignmentPending) continue;
        alignmentPending = true;
      }
      // 世代是「處理這一則時這條連線的編號」。誠實地說：事件流本身沒有帶世代，所以這裡
      // 只能標上當下的值——規則 0 在這條路徑上擋不住「換連線的那一瞬間才送達的舊事件」
      // （擋得住的是飛行中的 GET／resume 回覆，它們帶著發出當下的世代回來）。真正靠的是
      // 表的其餘規則：revision 落後就是 `ignore-stale`，接不上就是 realign。
      dispatch({ kind: "sse", envelope: event.payload, arrivedOn: connectionGenerationRef.current });
    }
  }, [sessionEvents, dispatch]);

  // --- 投影（純函式；所有句子都來自上面這些真實回應）。
  const snapshotView = React.useMemo(
    () => (session ? { payload: { kind: "snapshot", state: session.state } } : null),
    [session]
  );
  /**
   * 每一台裝置那條線送得到多少狀態（`docs/aip/device-profile.md` §3.1）。
   * 來源是診斷的 `members[].syncProfile`——Runtime 推導的，不是裝置自報的。
   * 讀不到診斷就是空的：不知道就不知道，既不降級也不升級。
   */
  const profiles = React.useMemo(() => characterSyncProfiles(diagnostics), [diagnostics]);
  const members = React.useMemo(
    () => characterSyncMembers(snapshotView, names, profiles),
    [snapshotView, names, profiles]
  );
  /** 連著、但不在成員名單裡的手機＝還沒重新確認過（送不出互動，也收不到狀態）。 */
  const pending = React.useMemo(() => {
    const synced = new Set(characterSyncMemberDeviceIds(snapshotView));
    return connected.filter((id) => !synced.has(id));
  }, [snapshotView, connected]);
  const projection = React.useMemo(() => {
    // 第一次請求還沒回來，但 SSE 已經送來一份權威狀態時，那份就是真的——
    // 沒有理由繼續說「正在讀取」。反過來，什麼都還沒有就照實說還在讀。
    if (!loaded && !session) return null;
    const signals: CharacterSyncSignals = {
      enabled,
      failedReads,
      revokedDevice: revoked,
      connectedButNotSynced: pending.length > 0,
      // 指出「是哪一台」用的是這台電腦上的顯示名稱；查不到名字用中性稱呼，
      // 裝置識別碼永遠不進畫面。
      pendingDeviceNames: pending.map((id) => names[id] ?? "一台裝置"),
      // 保存層：`parked`／持續寫入失敗是**現在**的問題；曾經重建過只是歷史通知。
      store: characterSyncStoreSignals(diagnostics),
      // host 明說 `recovery`、把這台電腦的副本帶回較舊的權威狀態（決策表規則 6）。
      // 那不是錯誤，但畫面上曾經看得到的東西被換掉了，不能靜默。
      recovered: machine.counters.recovered > 0,
    };
    return projectCharacterSession(snapshotView, members, signals);
  }, [
    loaded,
    session,
    snapshotView,
    members,
    enabled,
    failedReads,
    revoked,
    pending,
    names,
    diagnostics,
    machine.counters.recovered,
  ]);
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
  /** 這一態的下一步：有 id 又有落點才是一顆按得動的按鈕。 */
  const candidate = projection?.action;
  const action =
    candidate && candidate.id !== null && candidate.target
      ? { id: candidate.id, label: candidate.label, target: candidate.target }
      : null;
  /** 「去重新確認」要帶上是哪一台（只當參數用，不進畫面）。 */
  const actionDeviceId = action?.id === "reconfirm-device" ? (pending[0] ?? null) : null;

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
          {remote.map((m, index) => {
            // 非 full-state 的那一台要自己說得出「它拿得到什麼」——原始值只進進階模式。
            const profile = characterSyncProfileLabel(m.syncProfile);
            return (
              <li key={`${m.name}-${index}`}>
                <span>{m.name}</span>
                <span className="muted">：</span>
                <span>{characterSyncPresenceLabel(m.presence)}</span>
                {profile && <span className="muted">（{profile}）</span>}
              </li>
            );
          })}
        </ul>
      )}
      {lastInteraction && (
        <p className="small" role="status">
          最近互動：{lastInteraction}
        </p>
      )}
      {projection?.note && (
        <p className="muted small" data-testid="character-sync-note">
          {projection.note}
        </p>
      )}
      <div className="connect-area-actions">
        <button onClick={() => setReload((n) => n + 1)}>重新檢查</button>
        {/* 主要動作：只有「這一態真的有下一步」而且「這個畫面到得了那裡」時才出現。
            action id 是穩定的機器語意，按鈕文案可以改寫；storage-help 沒有落點
            （它是說明，不是一個可以去的地方），所以不會變成按鈕。 */}
        {action && onNavigate && (
          <button
            data-testid="character-sync-action"
            data-action={action.id ?? ""}
            onClick={() =>
              onNavigate(action.target.tab, {
                ...(action.target.hub ? { hub: action.target.hub } : {}),
                ...(actionDeviceId ? { deviceId: actionDeviceId } : {}),
              })
            }
          >
            {action.label}
          </button>
        )}
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
              {/* 保存層健康度：`migratedFrom` 只是歷史遷移通知，一般模式不顯示
                  （它既不是狀態也不是附註），進階模式才看得到。 */}
              {diagnostics.store && (
                <>
                  <li>store.parked {String(diagnostics.store.parked)}</li>
                  <li>store.persistFailures {diagnostics.store.persistFailures}</li>
                  <li>store.lastPersistedRevision {String(diagnostics.store.lastPersistedRevision)}</li>
                  {diagnostics.store.migratedFrom !== null && (
                    <li>store.migratedFrom {diagnostics.store.migratedFrom}</li>
                  )}
                  {diagnostics.store.lastPersistError && (
                    <li>store.lastPersistError {diagnostics.store.lastPersistError}</li>
                  )}
                </>
              )}
              {/* 成員身分怎麼來的（Runtime 依 Transport 決定；查不到通道就沒有這個欄位）。
                  只印原始值：`transport-hello+device-side-pairing` 弱於 `paired-token`，
                  不得翻成「已驗證身分」。 */}
              {diagnostics.members.map((m) => (
                <li key={`${m.party.kind}:${m.party.id}`}>
                  member {m.party.kind}:{m.party.id} {m.presence} identityStrength{" "}
                  {m.identityStrength ?? "—"} syncProfile {m.syncProfile ?? "—"}
                </li>
              ))}
              {/* 本地對齊的計數（reducer 的，不是後端的）：hash 不符、host 倒退、
                  被忽略的重播都必須看得見——安靜地丟掉一則狀態是最難察覺的錯。 */}
              {Object.entries(machine.counters).map(([key, value]) => (
                <li key={`alignment-${key}`}>
                  alignment.{key} {value}
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted small">目前讀不到連接診斷。</p>
          )}
        </details>
      )}
    </div>
  );
}
