// AIP Character Session 的接收端狀態機（桌面端）。
//
// 契約：`docs/aip/README.md` §6（state 的接收規則與 hash）、
// `docs/aip/transport-bindings.md` §1.3／§2（resume 的兩種回應形狀）。
// 權威決策是 Rust 的 `crates/interaction-session/src/patch.rs::accept_state_with_epoch`；
// iPhone 端的對照實作是 `apps/interaction-ios/.../SessionClient.swift`。這一份必須
// 對同一則訊息得到同一個結論——桌面端寬鬆一分，「手機顯示的角色」與「電腦顯示的角色」
// 就會在使用者看不見的地方分岔，而畫面兩邊都寫著「已同步」。
//
// 這個模組是**純函式**：沒有 React、沒有 I/O、沒有計時器、沒有全域狀態。
// 呼叫端（`../components/CharacterSyncCard.tsx`）只負責把 SSE 事件與 HTTP 回應
// 餵進 `reduce()`，再照 `effects` 去發下一個請求。所有協定判斷都在這裡。
//
// 三件 v0.6.0 桌面端沒做、而規格要求的事，都在這裡補上：
//
//   1. **嚴格解析**：缺 `revision`／`sessionEpoch` 不得被當成 0。舊版
//      `readSnapshotEnvelope` 的 `?? 0` 會把任何一則壞掉的 state 訊息變成
//      「host 回到 epoch 0 / revision 0」，於是下一則真的訊息永遠比它「新」，
//      本地副本就被一則垃圾訊息接管了。
//   2. **請求世代**：GET／resume 的回應只有在它仍是「最近一次發出的請求」時才算數，
//      而且在它飛行途中若已有 SSE 套用過更新的狀態，舊回應一律忽略。
//   3. **hash 核對**（AIP §6）：套用後本地重算的 canonical hash 必須等於 payload 的
//      `hash`，不符就**不套用**、改要求重新對齊。桌面端過去刻意不做這件事，理由是
//      「JS 留不住數字字面」——那個理由現在沒了：`./canonical.ts` 依 codegen 產出的
//      double 路徑重印字面，跨語言 fixture 逐位元組核對過（`src/test/canonical-hash.test.ts`）。

import { stateHash } from "./canonical";
import { applyMergePatch } from "./envelope";

/** host 明確宣告「這個 session 被重建了」的理由字串（Rust `interaction_session::REASON_SESSION_RESET`）。 */
export const REASON_SESSION_RESET = "session-reset";
/** 去重環的上限（有界，不隨連線時間成長）。 */
export const DEDUPE_RING_CAP = 256;
/**
 * 連續要求重新對齊的上限。
 *
 * realign 會讓呼叫端再打一次 resume／GET；如果 host 送來的東西一直對不上
 * （例如 snapshot 自己的 hash 就錯），無上限的話就是一個打不完的請求迴圈。
 * 達上限改回報 `unrecoverable`：狀態是**未知**，畫面照實說，不再自動重試。
 */
export const REALIGN_STREAK_LIMIT = 3;

// ------------------------------------------------------------------ 型別

/** 本地保存的一份權威狀態副本（只給投影用；沒有任何權力）。 */
export interface LocalSessionState {
  /** host 的 session id（沒給就是 null，不編一個）。 */
  sessionId: string | null;
  epoch: number;
  revision: number;
  /** host 的 session sequence（診斷與 resume 用；沒給就是 null）。 */
  sequence: number | null;
  state: Record<string, unknown>;
  /** 這份 state 的 canonical hash（本地重算的，不是照抄 payload）。 */
  hash: string | null;
}

/** 一則已經通過嚴格解析的 `state` 訊息。欄位都是真的讀到的，沒有預設值。 */
export interface StateMessage {
  kind: "snapshot" | "patch";
  revision: number;
  epoch: number;
  /** host 的 session sequence；缺就是 null（**不**拿它當去重或排序依據）。 */
  sequence: number | null;
  /** payload 宣告的 hash；缺就是 null（沒得核對就誠實地不核對，不假裝有）。 */
  hash: string | null;
  reason: string | null;
  sessionId: string | null;
  /** 去重用的鍵（messageId 優先；沒有就用 epoch+sequence）；都沒有就是 null。 */
  dedupeKey: string | null;
  /** snapshot 專用。 */
  state: Record<string, unknown> | null;
  /** patch 專用。 */
  patch: unknown;
  baseRevision: number | null;
}

/** 一則 state 訊息對本地副本的意義（鏡射 Rust 的 `StateDecision`）。 */
export type SessionAlignment =
  /** 可以套用。 */
  | { kind: "applied"; session: LocalSessionState }
  /** host 明確重建了 session：丟棄本地副本，接受這一份。 */
  | { kind: "reset"; session: LocalSessionState }
  /**
   * host 的權威讀取比本地舊，而且中間沒有任何 SSE 套用過。
   * 這是 host 說了算的事實（不是 rollback 攻擊），接受，但要可觀測。
   */
  | { kind: "regressed"; session: LocalSessionState }
  /** 落後或重播：忽略。 */
  | { kind: "ignored"; reason: "rollback" | "already-applied" }
  /** 接不上：必須重新對齊（送 resume 或 GET），**不**硬套、不猜。 */
  | { kind: "realign"; reason: "no-local" | "epoch-changed" | "base-mismatch" | "hash-mismatch" }
  /** 不是一則能讀懂的 state 訊息。 */
  | { kind: "invalid" };

/** 可觀測的計數（進階模式的診斷用；一般模式一個數字都不顯示）。 */
export interface SessionCounters {
  applied: number;
  reset: number;
  ignoredRollback: number;
  ignoredAlreadyApplied: number;
  invalid: number;
  duplicate: number;
  /** 過期的請求回應（世代不符，或飛行途中已被更新的 SSE 超車）。 */
  stale: number;
  hashMismatch: number;
  /** host 的權威讀取比本地舊：接受了，但這件事必須留下痕跡。 */
  hostRegressed: number;
  realign: number;
}

export interface SessionMachine {
  readonly local: LocalSessionState | null;
  /** 最近一次發出的請求世代；只有它的回應算數。 */
  readonly pendingRequestId: number | null;
  /** 這個請求發出之後，有沒有 SSE 已經套用過新狀態。 */
  readonly appliedSinceIssue: boolean;
  /** 去重環（有界；messageId 優先）。 */
  readonly seen: readonly string[];
  readonly realignStreak: number;
  readonly counters: SessionCounters;
}

export type SessionInput =
  /** 一則 SSE `character.session.state` 事件的 payload（完整 AIP envelope）。 */
  | { kind: "sse"; envelope: unknown }
  /** 送出一次 GET／resume。 */
  | { kind: "fetch-issued"; requestId: number }
  /** `GET /v1/character-session` 的回應（一則完整的 state envelope）。 */
  | { kind: "fetch-response"; requestId: number; envelope: unknown }
  /** `POST /v1/character-session/resume` 的回應（**payload**，不是 envelope）。 */
  | { kind: "resume-response"; requestId: number; payload: unknown }
  /** 本地副本作廢（讀不到、卸載重掛）。 */
  | { kind: "reset-local" };

export type SessionEffect =
  /** 請重新對齊：有本地副本走 resume，沒有就 GET。 */
  | { kind: "realign" }
  /** 連續對齊失敗達上限：狀態未知，停止自動重試（畫面照實說）。 */
  | { kind: "unrecoverable" };

export interface SessionStep {
  next: SessionMachine;
  effects: readonly SessionEffect[];
}

const EMPTY_COUNTERS: SessionCounters = {
  applied: 0,
  reset: 0,
  ignoredRollback: 0,
  ignoredAlreadyApplied: 0,
  invalid: 0,
  duplicate: 0,
  stale: 0,
  hashMismatch: 0,
  hostRegressed: 0,
  realign: 0,
};

export function initialSession(): SessionMachine {
  return {
    local: null,
    pendingRequestId: null,
    appliedSinceIssue: false,
    seen: [],
    realignStreak: 0,
    counters: EMPTY_COUNTERS,
  };
}

// -------------------------------------------------------------- 嚴格解析

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * u64 欄位：非負的安全整數才算數。
 *
 * `Number.isSafeInteger` 擋掉小數、`Infinity`、`NaN` 與超過 2^53 的值——超過安全整數
 * 之後 JS 的比較會失真（`2**53 === 2**53 + 1`），那就不是「大一點的 revision」，
 * 而是「這個數字這一端讀不出來」。讀不出來就是 invalid，不猜。
 */
function u64(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * 一則 `state` envelope → [`StateMessage`]（讀不出來就是 `null`＝invalid）。
 *
 * 與舊版 `readSnapshotEnvelope` 的差別是**沒有預設值**：缺 `revision`／`sessionEpoch`、
 * 負數、小數、超過安全整數，一律 invalid，絕不變成 0。
 */
export function readStateEnvelope(envelope: unknown): StateMessage | null {
  const outer = record(envelope);
  if (!outer) return null;
  if (outer["messageType"] !== "state") return null;
  const payload = record(outer["payload"]);
  if (!payload) return null;
  return readStatePayload(payload, {
    messageId: text(outer["messageId"]),
    sessionId: text(outer["sessionId"]),
    // patch 的 `baseRevision` 在 envelope 頂層（Rust 的 `with_base_revision`）；
    // 有些產生端也寫在 payload 裡，兩處都認，payload 優先。
    baseRevision: outer["baseRevision"],
    sequence: outer["sequence"],
  });
}

/**
 * `state` 的 payload → [`StateMessage`]。
 *
 * resume 的 snapshot 回應**就是這個 payload**（transport-bindings §1.3：少一層巢狀，
 * 因為 AIP §11 的深度上限是 8，再包一層完整 envelope 會超），所以解析要能單獨用。
 */
export function readStatePayload(
  payload: Record<string, unknown>,
  outer: {
    messageId?: string | null;
    sessionId?: string | null;
    baseRevision?: unknown;
    sequence?: unknown;
  } = {},
): StateMessage | null {
  const kind = payload["kind"];
  if (kind !== "snapshot" && kind !== "patch") return null;
  const revision = u64(payload["revision"]);
  const epoch = u64(payload["sessionEpoch"]);
  if (revision === null || epoch === null) return null;
  const sequence = u64(payload["sequence"]) ?? u64(outer.sequence);
  const messageId = outer.messageId ?? null;
  const common = {
    revision,
    epoch,
    sequence,
    hash: text(payload["hash"]),
    reason: text(payload["reason"]),
    sessionId: text(payload["sessionId"]) ?? outer.sessionId ?? null,
    dedupeKey: messageId ?? (sequence === null ? null : `${epoch}:${sequence}`),
  };
  if (kind === "snapshot") {
    const state = record(payload["state"]);
    if (!state) return null;
    return { kind, ...common, state, patch: undefined, baseRevision: null };
  }
  const baseRevision = u64(payload["baseRevision"]) ?? u64(outer.baseRevision);
  if (baseRevision === null) return null;
  if (payload["patch"] === undefined) return null;
  return { kind, ...common, state: null, patch: payload["patch"], baseRevision };
}

/**
 * resume 回應裡的一則補丁（`transport-bindings` §1.3 的 `patches[]`）。
 *
 * 形狀是攤平的（`{sequence, baseRevision, revision, patch, hash, sessionEpoch}`），
 * 沒有 envelope 外殼，也沒有 `kind`——但規則與 `state{kind:"patch"}` 完全一樣。
 */
export function readResumePatch(item: unknown): StateMessage | null {
  const entry = record(item);
  if (!entry) return null;
  return readStatePayload({ ...entry, kind: "patch" });
}

// ---------------------------------------------------------------- 對齊規則

/** 套用一份 state 前的 hash 核對：payload 沒給 hash 就沒得核對（不假裝有）。 */
function hashMatches(state: Record<string, unknown>, declared: string | null): boolean {
  return declared === null || stateHash(state) === declared;
}

function commit(message: StateMessage, state: Record<string, unknown>): LocalSessionState {
  return {
    sessionId: message.sessionId,
    epoch: message.epoch,
    revision: message.revision,
    sequence: message.sequence,
    state,
    // 本地重算，不照抄 payload：這份 hash 之後只用來說明「我算出來的是什麼」。
    hash: stateHash(state),
  };
}

/**
 * AIP §6 的完整接收規則，逐條鏡射 Rust `accept_state_with_epoch`：
 * rollback 防護 → `session-reset` 例外 → patch 續接 → hash 核對。
 *
 * `allowRegression` 只有「這是我們自己剛要來的權威讀取，而且要來的路上沒有任何
 * SSE 套用過新狀態」時才是 true：那時候比本地舊的 snapshot 是 host 說了算的事實
 * （host 從快照還原、或被重置過），不是重播攻擊。
 *
 * 與 Rust 的兩處刻意差異，都是**更嚴**的方向：
 *   * patch 的 `sessionEpoch` 這裡是必填（Rust 的 patch 分支完全不看 epoch）。
 *     Runtime 送出的 patch 一定帶它（`session.rs` 的 `patch_envelope`／`replay_envelope`）。
 *   * patch 的 epoch 與本地不同時回 realign，而不是靠 `baseRevision` 恰巧不符去擋。
 */
export function alignState(
  local: LocalSessionState | null,
  message: StateMessage,
  allowRegression = false,
): SessionAlignment {
  if (message.kind === "snapshot") {
    const state = message.state;
    if (!state) return { kind: "invalid" };
    const fresh = (kind: "applied" | "reset" | "regressed"): SessionAlignment =>
      hashMatches(state, message.hash)
        ? ({ kind, session: commit(message, state) } as SessionAlignment)
        : { kind: "realign", reason: "hash-mismatch" };
    if (!local) return fresh("applied");
    // §7 第 4 步：「epoch **不同** → 丟棄本地狀態、套用 snapshot」，不是「更大」。
    // host 重灌（epoch 從 1 重新起跳）時新 epoch 可能比本地記得的小；用 `>` 會把那份
    // 權威快照當成 rollback 丟掉，畫面就永遠停在舊狀態。`reason: session-reset` 是
    // host 明確的重建宣告，才享有這個例外。
    if (message.reason === REASON_SESSION_RESET && message.epoch !== local.epoch) {
      return fresh("reset");
    }
    // epoch 不同又沒有重建宣告：兩份狀態沒有可比的順序，不猜——重新對齊。
    if (message.epoch !== local.epoch) return { kind: "realign", reason: "epoch-changed" };
    if (message.revision < local.revision) {
      return allowRegression ? fresh("regressed") : { kind: "ignored", reason: "rollback" };
    }
    if (message.revision === local.revision) return { kind: "ignored", reason: "already-applied" };
    return fresh("applied");
  }
  if (message.revision < (local?.revision ?? -1)) return { kind: "ignored", reason: "rollback" };
  if (local && message.revision === local.revision) {
    return { kind: "ignored", reason: "already-applied" };
  }
  // 沒有本地副本就沒有東西可以套補丁上去（AIP §6：補丁不是完整狀態）。
  if (!local) return { kind: "realign", reason: "no-local" };
  if (message.epoch !== local.epoch) return { kind: "realign", reason: "epoch-changed" };
  if (message.baseRevision !== local.revision) return { kind: "realign", reason: "base-mismatch" };
  const merged = record(applyMergePatch(local.state, message.patch));
  if (!merged) return { kind: "realign", reason: "base-mismatch" };
  if (!hashMatches(merged, message.hash)) return { kind: "realign", reason: "hash-mismatch" };
  return { kind: "applied", session: commit(message, merged) };
}

// ------------------------------------------------------------------ reducer

/** 有界的去重環：滿了就擠掉最舊的（不是無界成長的 Set）。 */
function remember(seen: readonly string[], key: string | null): readonly string[] {
  if (key === null) return seen;
  const next = [...seen, key];
  return next.length > DEDUPE_RING_CAP ? next.slice(next.length - DEDUPE_RING_CAP) : next;
}

function bump(counters: SessionCounters, key: keyof SessionCounters, by = 1): SessionCounters {
  return { ...counters, [key]: counters[key] + by };
}

/** 一則 alignment 的結果 → 新的機器狀態（不含 effect）。 */
function settle(machine: SessionMachine, alignment: SessionAlignment): SessionMachine {
  switch (alignment.kind) {
    case "applied":
      return {
        ...machine,
        local: alignment.session,
        appliedSinceIssue: true,
        realignStreak: 0,
        counters: bump(machine.counters, "applied"),
      };
    case "reset":
      return {
        ...machine,
        local: alignment.session,
        appliedSinceIssue: true,
        realignStreak: 0,
        counters: bump(bump(machine.counters, "applied"), "reset"),
      };
    case "regressed":
      return {
        ...machine,
        local: alignment.session,
        appliedSinceIssue: true,
        realignStreak: 0,
        counters: bump(bump(machine.counters, "applied"), "hostRegressed"),
      };
    case "ignored":
      return {
        ...machine,
        counters: bump(
          machine.counters,
          alignment.reason === "rollback" ? "ignoredRollback" : "ignoredAlreadyApplied",
        ),
      };
    case "realign": {
      const counters =
        alignment.reason === "hash-mismatch"
          ? bump(bump(machine.counters, "hashMismatch"), "realign")
          : bump(machine.counters, "realign");
      return { ...machine, realignStreak: machine.realignStreak + 1, counters };
    }
    case "invalid":
      return { ...machine, counters: bump(machine.counters, "invalid") };
  }
}

/** realign 的 effect：連續失敗達上限就改成 `unrecoverable`（有界，不無限重試）。 */
function realignEffects(machine: SessionMachine): readonly SessionEffect[] {
  return machine.realignStreak > REALIGN_STREAK_LIMIT
    ? [{ kind: "unrecoverable" }]
    : [{ kind: "realign" }];
}

function step(machine: SessionMachine, alignment: SessionAlignment): SessionStep {
  const next = settle(machine, alignment);
  return { next, effects: alignment.kind === "realign" ? realignEffects(next) : [] };
}

/** 這則訊息是不是「不比本地新」——請求飛行途中已被 SSE 超車時用來判定過期。 */
function notNewerThanLocal(local: LocalSessionState | null, message: StateMessage): boolean {
  return local !== null && message.epoch === local.epoch && message.revision <= local.revision;
}

/**
 * 收下一則訊息（SSE 或請求回應）。
 *
 * `authoritative` 代表它來自「最近一次發出的請求」的回應：這時 host 的讀取比本地舊
 * 也要接受（並記 `hostRegressed`），除非飛行途中已經有 SSE 套用過更新的狀態。
 */
function ingest(machine: SessionMachine, envelope: unknown, authoritative: boolean): SessionStep {
  const message = readStateEnvelope(envelope);
  if (!message) return { next: settle(machine, { kind: "invalid" }), effects: [] };
  return ingestMessage(machine, message, authoritative);
}

function ingestMessage(
  machine: SessionMachine,
  message: StateMessage,
  authoritative: boolean,
): SessionStep {
  if (message.dedupeKey !== null && machine.seen.includes(message.dedupeKey)) {
    return { next: { ...machine, counters: bump(machine.counters, "duplicate") }, effects: [] };
  }
  const remembered: SessionMachine = { ...machine, seen: remember(machine.seen, message.dedupeKey) };
  if (authoritative && machine.appliedSinceIssue && notNewerThanLocal(machine.local, message)) {
    // 慢的回應被 SSE 超車了：它帶的是我們早就走過的狀態，套下去就是倒退。
    return { next: { ...remembered, counters: bump(remembered.counters, "stale") }, effects: [] };
  }
  const allowRegression = authoritative && !machine.appliedSinceIssue;
  return step(remembered, alignState(remembered.local, message, allowRegression));
}

/** resume 的回應（payload，不是 envelope）。 */
function ingestResume(machine: SessionMachine, payload: unknown): SessionStep {
  const body = record(payload);
  const kind = body?.["kind"];
  if (body && kind === "snapshot") {
    const message = readStatePayload(body);
    if (!message) return { next: settle(machine, { kind: "invalid" }), effects: [] };
    return ingestMessage(machine, message, true);
  }
  if (body && kind === "patches" && Array.isArray(body["patches"])) {
    let current = machine;
    for (const item of body["patches"]) {
      const message = readResumePatch(item);
      if (!message) return { next: settle(current, { kind: "invalid" }), effects: [] };
      const outcome = ingestMessage(current, message, true);
      current = outcome.next;
      // 中間一則接不上就停在那裡：後面的補丁都建立在沒套上的那一份之上，
      // 硬跳過去就是拿一個從來不存在的狀態當真（AIP §6）。
      if (outcome.effects.length > 0) return { next: current, effects: outcome.effects };
      const applied = current.local?.revision === message.revision;
      if (!applied) break;
    }
    return { next: current, effects: [] };
  }
  // 看不懂的 resume 回應：記下來，但**不**自動再要一次（那會變成無界的請求迴圈）。
  return { next: settle(machine, { kind: "invalid" }), effects: [] };
}

/**
 * 狀態機的唯一入口（純函式）。
 *
 * 回傳的 `effects` 是呼叫端**應該做的事**，不是已經發生的事：`realign` 代表
 * 「請再要一次對齊」，`unrecoverable` 代表「別再要了，狀態未知」。
 */
export function reduce(current: SessionMachine, input: SessionInput): SessionStep {
  switch (input.kind) {
    case "sse":
      return ingest(current, input.envelope, false);
    case "fetch-issued":
      return {
        next: { ...current, pendingRequestId: input.requestId, appliedSinceIssue: false },
        effects: [],
      };
    case "fetch-response":
    case "resume-response": {
      if (current.pendingRequestId !== input.requestId) {
        // 上一輪的請求回來了：它的世代已經過期，一律不算數。
        return { next: { ...current, counters: bump(current.counters, "stale") }, effects: [] };
      }
      const settled: SessionMachine = { ...current, pendingRequestId: null };
      return input.kind === "fetch-response"
        ? ingest(settled, input.envelope, true)
        : ingestResume(settled, input.payload);
    }
    case "reset-local":
      // 讀不到就是讀不到：本地副本作廢，不用上一次的樣子冒充現在。
      // 計數保留（那是這條連線發生過的事實，不該被一次重掛抹掉）。
      return {
        next: {
          ...current,
          local: null,
          pendingRequestId: null,
          appliedSinceIssue: false,
          seen: [],
          realignStreak: 0,
        },
        effects: [],
      };
  }
}
