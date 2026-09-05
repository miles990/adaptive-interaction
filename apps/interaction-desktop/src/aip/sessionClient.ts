// AIP Character Session 的接收端狀態機（桌面端）。
//
// 契約：`docs/aip/character-session.md` §7.2 的**接收端決策表**（AIP 1.0 接收端澄清，
// v0.7.0；wire 沒有變，只是把「收到一則 `state` 之後到底要做什麼」寫成一張三端共用的表）。
// 權威實作是 Rust 的 `crates/interaction-session/src/receive.rs::decide_receive`；
// iPhone 的對照實作是 `apps/interaction-ios/.../SessionClient.swift`。
//
// # 與 Rust／Swift 的差異：**零差異**
//
// v0.6.0 的這裡列著三條「桌面比較嚴」的已知分歧，理由是契約沒裁決。契約已經裁決了，
// 三端現在讀同一張表，而且**同一份跨語言 fixture 逐筆對答案**：
//
//   * 來源：`crates/interaction-aip/tests/fixtures/manifest.json` 的 `receiveDecisions` 段
//     （43 個具名案例，`docs/aip/conformance.md` §3 說明欄位與 `incomingBatchChain` 展開規則）
//   * Rust：`crates/interaction-session/tests/receive_decisions_from_json.rs`
//   * TypeScript（這一支）：`src/test/receive-decision-fixtures.test.ts`
//   * Swift：`apps/interaction-ios/InteractionCompanionTests`（`AIPFixtures.swift` 已內嵌同一段；
//     iPhone 端的接收機改讀這張表是另一支工作，**尚未完成**——差異目前只剩那一端）
//
// 差異一旦重新長出來，三邊會同時紅——不再靠「有沒有人記得更新註解」維持。
// 兩個上限（`maxResumePatches`／`maxRealignAttempts`）也不在這一端自己寫：從 codegen 產出的
// `AIP_LIMITS` 讀同一個數字（`./generated.ts`，由 golden schema 的 `limits` 表產生）。
//
// # 表（第一個命中即決定）
//
//   0. 訊息來自已失效的連線／請求世代 → `ignore-stale-connection`（**先於一切 epoch 判斷**）
//   1. incoming 有 sessionId、本地有狀態、且與本地不同 → `reject-identity`（不 realign）
//   2. snapshot 缺 `hash` 或缺 `state`（patch 缺 `baseRevision`）→ `reject-invalid`
//   3. snapshot、`reason == "session-reset"`、epoch 與本地不同（或本地無狀態）→ `reset`
//   4. snapshot、本地無狀態 → `apply`（bootstrap）
//   5. snapshot、epoch 不同、無 reset 宣告 → `realign(epoch-changed)`
//   6. snapshot、同 epoch、`reason == "recovery"`、revision 較舊 → `recover`
//   7. snapshot、同 epoch、revision 較舊 → `ignore-stale`
//   8. snapshot、同 epoch、revision 相同 → `already-applied`（宣告的 hash 與本地的不同 → realign）
//   9. snapshot、同 epoch、revision 較新 → `apply`（hash 不符 → realign）
//  10. patch、本地無狀態 → `realign(no-local)`
//  11. patch、epoch 不同 → `realign(epoch-changed)`
//  12. patch、revision ≤ 本地 → `ignore-stale`／`already-applied`
//  13. patch、`baseRevision` 接不上 → `realign(base-mismatch)`
//  14. merge 之後的 hash 與宣告的不同 → `realign(hash-mismatch)`
//  15. 其餘 → `apply`
//
// # 這個模組的邊界
//
// **純函式**：沒有 React、沒有 I/O、沒有計時器、沒有全域狀態。呼叫端
// （`../components/CharacterSyncCard.tsx`）只負責把 SSE 事件與 HTTP 回應（連同它們所屬的
// **連線世代** `arrivedOn`）餵進 `reduce()`，再照 `effects` 去發下一個請求。
// hash 由這裡自己算（`./canonical.ts` 依 codegen 產出的 double 路徑重印字面，跨語言 fixture
// 逐位元組核對過——`src/test/canonical-hash.test.ts`），算出來與宣告的不同就**不套用**。

import { stateHash } from "./canonical";
import { applyMergePatch } from "./envelope";
import { AIP_LIMITS } from "./generated";

/** host 明確宣告「這個 session 被重建了」的理由字串（Rust `interaction_session::REASON_SESSION_RESET`）。 */
export const REASON_SESSION_RESET = "session-reset";
/** host 明確宣告「同一個 session 真的從較舊的快照還原了」（Rust `REASON_RECOVERY`）。 */
export const REASON_RECOVERY = "recovery";
/** 去重環的上限（有界，不隨連線時間成長）。權威值在 `AIP_LIMITS`。 */
export const DEDUPE_RING_CAP = AIP_LIMITS.dedupeRing;
/**
 * 連續幾次未能 apply 就是「無法恢復」。
 *
 * realign 會讓呼叫端再打一次 resume／GET；如果 host 送來的東西一直對不上
 * （例如 snapshot 自己的 hash 就錯），無上限的話就是一個打不完的請求迴圈。
 * 達上限改回報 `unrecoverable`：狀態是**未知**，畫面照實說，不再自動重試。
 * 權威值在 `AIP_LIMITS.maxRealignAttempts`（golden schema 的 `limits` 表），不在這裡自己寫。
 */
export const REALIGN_STREAK_LIMIT = AIP_LIMITS.maxRealignAttempts;
/**
 * 一次 resume 回應最多幾則補丁（＝host 事件日誌環大小）。
 *
 * 超過就是不正常的回應：**整批不處理**、改要求重新對齊。以前這裡截斷到上限再 realign——
 * 那會讓本地停在一個「從來沒有完整存在過」的中間狀態。權威值在
 * `AIP_LIMITS.maxResumePatches`，本地可以更嚴但不得靜默截斷。
 */
export const MAX_RESUME_PATCHES = AIP_LIMITS.maxResumePatches;

// ------------------------------------------------------------------ 型別

/** 本地保存的一份權威狀態副本（只給投影用；沒有任何權力）。 */
export interface LocalSessionState {
  /** host 的 session id（沒給就是 null，不編一個）。bootstrap 那一則記下來，之後用來擋別的 session。 */
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
  /** payload 宣告的 hash；缺就是 null（snapshot 缺它＝規則 2 的 `reject-invalid`）。 */
  hash: string | null;
  reason: string | null;
  sessionId: string | null;
  /** 去重用的鍵（messageId 優先；沒有就用 epoch+sequence）；都沒有就是 null。 */
  dedupeKey: string | null;
  /** snapshot 專用（缺＝規則 2）。 */
  state: Record<string, unknown> | null;
  /** patch 專用。 */
  patch: unknown;
  baseRevision: number | null;
}

/** realign 的原因（穩定字串；與 Rust `RealignReason` 及 fixture 同名）。 */
export type RealignReason =
  | "no-local"
  | "epoch-changed"
  | "base-mismatch"
  | "hash-mismatch"
  | "resume-too-long";

/** 決策的穩定名字（與 Rust `ReceiveDecision::as_str` 及 fixture 的 `expect.decision` 同名）。 */
export type ReceiveDecisionKind =
  | "ignore-stale-connection"
  | "reject-identity"
  | "reject-invalid"
  | "reset"
  | "apply"
  | "realign"
  | "recover"
  | "ignore-stale"
  | "already-applied";

/** 一則 state 訊息對本地副本的意義（逐條鏡射 Rust 的 `ReceiveDecision`）。 */
export type SessionAlignment =
  /** 套用。 */
  | { kind: "apply"; session: LocalSessionState }
  /** host 明說 session 被重建：丟棄本地副本，採用新的 epoch／revision。 */
  | { kind: "reset"; session: LocalSessionState }
  /** host 明說同一個 session 從較舊的快照還原了：套用並退回 host 的 revision。 */
  | { kind: "recover"; session: LocalSessionState }
  /** 接不上：必須重新對齊（送 resume 或 GET），**不**硬套、不猜。 */
  | { kind: "realign"; reason: RealignReason }
  /** 落後：忽略。 */
  | { kind: "ignore-stale" }
  /** 重播：已經套用過，什麼都不做。 */
  | { kind: "already-applied" }
  /** 別的 session 的狀態：不相干——不套用也不 realign。 */
  | { kind: "reject-identity" }
  /** 不是一則能用的 state 訊息。 */
  | { kind: "reject-invalid" }
  /** 舊連線／舊請求世代的遲到品。 */
  | { kind: "ignore-stale-connection" };

/** 最近一則訊息的決策（診斷與測試用；`reason` 只有 realign 有）。 */
export interface SessionDecision {
  decision: ReceiveDecisionKind;
  reason: RealignReason | null;
}

/** 可觀測的計數（進階模式的診斷用；一般模式一個數字都不顯示）。 */
export interface SessionCounters {
  applied: number;
  reset: number;
  /** host 明說 `recovery` 而把本地帶回較舊的權威狀態（不是錯誤，但必須看得見）。 */
  recovered: number;
  /** 落後的訊息（rollback 防護）。 */
  ignoredStale: number;
  ignoredAlreadyApplied: number;
  /** 不是一則能用的 state 訊息。 */
  rejectedInvalid: number;
  /** 別的 session 的狀態。 */
  rejectedIdentity: number;
  duplicate: number;
  /** 舊連線／舊請求世代的遲到品。 */
  staleConnection: number;
  hashMismatch: number;
  realign: number;
}

export interface SessionMachine {
  readonly local: LocalSessionState | null;
  /**
   * 現行連線／請求世代。呼叫端在連線換掉時 +1；帶著別的世代到達的訊息是舊連線的遲到品
   * （決策表規則 0——它宣告的 epoch 一定與本地不同，任何 epoch 判斷都會被它騙過去）。
   */
  readonly connectionGeneration: number;
  /** 最近一次發出的請求世代；只有它的回應算數。 */
  readonly pendingRequestId: number | null;
  /** 去重環（有界；messageId 優先）。 */
  readonly seen: readonly string[];
  /** 連續未能 apply 的次數（有界 realign 預算；apply／reset／recover 清零）。 */
  readonly realignStreak: number;
  /** 最近一則訊息的決策（null＝還沒收過任何訊息）。 */
  readonly lastDecision: SessionDecision | null;
  readonly counters: SessionCounters;
}

export type SessionInput =
  /** 連線換了一條（呼叫端的 `connectionKey` 變了）：舊世代的遲到品從此不算數。 */
  | { kind: "connection-changed"; generation: number }
  /** 一則 SSE `character.session.state` 事件的 payload（完整 AIP envelope）。 */
  | { kind: "sse"; envelope: unknown; arrivedOn: number }
  /** 送出一次 GET／resume。 */
  | { kind: "fetch-issued"; requestId: number }
  /** `GET /v1/character-session` 的回應（一則完整的 state envelope）。 */
  | { kind: "fetch-response"; requestId: number; envelope: unknown; arrivedOn: number }
  /** `POST /v1/character-session/resume` 的回應（**payload**，不是 envelope）。 */
  | { kind: "resume-response"; requestId: number; payload: unknown; arrivedOn: number }
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

/** 內部用：多帶一個決策名字，讓 resume 的逐則規則不必去猜 effect 的意思。 */
interface DecidedStep extends SessionStep {
  decision: ReceiveDecisionKind | "duplicate";
}

const EMPTY_COUNTERS: SessionCounters = {
  applied: 0,
  reset: 0,
  recovered: 0,
  ignoredStale: 0,
  ignoredAlreadyApplied: 0,
  rejectedInvalid: 0,
  rejectedIdentity: 0,
  duplicate: 0,
  staleConnection: 0,
  hashMismatch: 0,
  realign: 0,
};

export function initialSession(): SessionMachine {
  return {
    local: null,
    connectionGeneration: 0,
    pendingRequestId: null,
    seen: [],
    realignStreak: 0,
    lastDecision: null,
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
 * 一則 `state` envelope → [`StateMessage`]（讀不出來就是 `null`）。
 *
 * 這一層是 typed boundary（`interaction_aip::Envelope::parse` 的對應物），**不是**決策表：
 * 缺 `revision`／`sessionEpoch`、負數、小數、超過安全整數，一律讀不出來，絕不變成 0。
 * 讀不出來的訊息在 `reduce()` 裡是決策表的 `reject-invalid`。
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
    // hash 在這一層是選填：snapshot 缺它是**決策表**的規則 2（`reject-invalid`），
    // 不是「讀不出來」——兩者的差別在有界 realign 預算怎麼記。
    hash: text(payload["hash"]),
    reason: text(payload["reason"]),
    sessionId: text(payload["sessionId"]) ?? outer.sessionId ?? null,
    dedupeKey: messageId ?? (sequence === null ? null : `${epoch}:${sequence}`),
  };
  if (kind === "snapshot") {
    return {
      kind,
      ...common,
      state: record(payload["state"]),
      patch: undefined,
      baseRevision: null,
    };
  }
  if (payload["patch"] === undefined) return null;
  return {
    kind,
    ...common,
    state: null,
    patch: payload["patch"],
    baseRevision: u64(payload["baseRevision"]) ?? u64(outer.baseRevision),
  };
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

// ---------------------------------------------------------------- 決策表

function commit(
  local: LocalSessionState | null,
  message: StateMessage,
  state: Record<string, unknown>,
  hash: string,
): LocalSessionState {
  return {
    // incoming 沒帶 sessionId 時留著本地記得的那一個（bootstrap 記下來的身分不會被沖掉）。
    sessionId: message.sessionId ?? local?.sessionId ?? null,
    epoch: message.epoch,
    revision: message.revision,
    sequence: message.sequence,
    state,
    // 本地重算，不照抄 payload：這份 hash 之後用來核對「同一個 revision 有沒有兩份狀態」。
    hash,
  };
}

/**
 * 接收端決策表的規則 1..15（規則 0 是呼叫端狀態，在 `reduce()` 裡）。
 *
 * 逐條鏡射 Rust `receive.rs::decide_receive`；跨語言 fixture
 * （`manifest.json` 的 `receiveDecisions`）逐筆釘住三端得到同一個決策。
 */
export function alignState(
  local: LocalSessionState | null,
  message: StateMessage,
): SessionAlignment {
  // 1. 身分：別的 session 的狀態不是「比較舊」，是**不相干**——不套用也不 realign
  //    （realign 只會再要一次別人的 session）。
  if (local && message.sessionId !== null && message.sessionId !== local.sessionId) {
    return { kind: "reject-identity" };
  }
  return message.kind === "snapshot" ? alignSnapshot(local, message) : alignPatch(local, message);
}

function alignSnapshot(local: LocalSessionState | null, message: StateMessage): SessionAlignment {
  const state = message.state;
  // 2. AIP 1.0 的 snapshot 必帶 hash 與 state；沒有 legacy profile。
  if (message.hash === null || state === null) return { kind: "reject-invalid" };
  // 套用之前一律核對（reset／bootstrap 也一樣）：算出來的與宣告的不同就不採用。
  const computed = stateHash(state);
  const adopt = (kind: "apply" | "reset" | "recover"): SessionAlignment =>
    computed === message.hash
      ? { kind, session: commit(local, message, state, computed) }
      : { kind: "realign", reason: "hash-mismatch" };
  // 3. host 明說重建了 session。epoch 相同的 `session-reset` **不算**：host 重灌後 epoch
  //    可能比本地記得的小，所以判定是「不同」不是「更大」。
  if (message.reason === REASON_SESSION_RESET && (!local || message.epoch !== local.epoch)) {
    return adopt("reset");
  }
  // 4. bootstrap：本地什麼都沒有，第一份權威狀態直接收下。
  if (!local) return adopt("apply");
  // 5. epoch 不同又沒有重建宣告：兩份狀態沒有可比的順序，不猜——重新對齊。
  //    host 對 epoch 不同的 resume 一律回 `session-reset` snapshot，所以一次就收斂。
  if (message.epoch !== local.epoch) return { kind: "realign", reason: "epoch-changed" };
  // 6. 同一個 session 真的倒退過：host 明說 `recovery` 才採納。
  //    （v0.6.0 的桌面端在這裡有一個特例：「最新的 HTTP 回覆比本地舊就接受」。那等於讓
  //    「哪一則先回來」決定畫面，而不是讓 host 說了算——決策表取消了它。）
  if (message.reason === REASON_RECOVERY && message.revision < local.revision) {
    return adopt("recover");
  }
  // 7. 落後：忽略。權威回覆也一樣——真的倒退過的 host 會說 `recovery`。
  if (message.revision < local.revision) return { kind: "ignore-stale" };
  // 8. 重播：什麼都不做。除非它宣告的 hash 與本地算出來的不同——那就是同一個 revision
  //    有兩份不同的狀態，只能重新對齊。
  if (message.revision === local.revision) {
    return local.hash !== null && message.hash !== local.hash
      ? { kind: "realign", reason: "hash-mismatch" }
      : { kind: "already-applied" };
  }
  // 9. 較新：核對過就套用。
  return adopt("apply");
}

function alignPatch(local: LocalSessionState | null, message: StateMessage): SessionAlignment {
  // 2（patch 版）：typed boundary 已經擋掉缺 baseRevision 的 patch，這裡是第二道。
  if (message.baseRevision === null) return { kind: "reject-invalid" };
  // 10. 補丁不是完整狀態：沒有本地副本就沒有東西可以套上去。
  if (!local) return { kind: "realign", reason: "no-local" };
  // 11. epoch 不同 → realign（不靠 `baseRevision` 恰巧不符去擋）。
  if (message.epoch !== local.epoch) return { kind: "realign", reason: "epoch-changed" };
  // 12. 落後／重播。
  if (message.revision < local.revision) return { kind: "ignore-stale" };
  if (message.revision === local.revision) return { kind: "already-applied" };
  // 13. 接不上前一個 revision。
  if (message.baseRevision !== local.revision) return { kind: "realign", reason: "base-mismatch" };
  const merged = record(applyMergePatch(local.state, message.patch));
  // 表之外的一條（呼叫端責任）：merge 產不出一個物件，這則訊息就不是一份能用的狀態。
  if (merged === null) return { kind: "reject-invalid" };
  const computed = stateHash(merged);
  // 14. merge 之後的 hash 與宣告的不同。（沒宣告 hash 就沒得核對，誠實地不核對。）
  if (message.hash !== null && computed !== message.hash) {
    return { kind: "realign", reason: "hash-mismatch" };
  }
  // 15. 其餘：套用。
  return { kind: "apply", session: commit(local, message, merged, computed) };
}

// ------------------------------------------------------------------ reducer

/** 有界的去重環：滿了就擠掉最舊的（不是無界成長的 Set）。 */
function remember(seen: readonly string[], key: string | null): readonly string[] {
  if (key === null) return seen;
  const next = [...seen, key];
  return next.length > DEDUPE_RING_CAP ? next.slice(next.length - DEDUPE_RING_CAP) : next;
}

function bump(counters: SessionCounters, key: keyof SessionCounters): SessionCounters {
  return { ...counters, [key]: counters[key] + 1 };
}

/**
 * 一則決策 → 新的機器狀態（不含 effect）。
 *
 * `authoritative` ＝這則訊息是我們自己要來的權威回覆（HTTP GET／resume response）。
 * 只有它的 `reject-invalid` 算一次對齊失敗：對方回答了、但答案沒用。推播（SSE）上的垃圾
 * 不算——它不是我們要來的答案，不會讓對齊卡住。
 */
function settle(
  machine: SessionMachine,
  alignment: SessionAlignment,
  authoritative: boolean,
): SessionMachine {
  const base: SessionMachine = {
    ...machine,
    lastDecision: {
      decision: alignment.kind,
      reason: alignment.kind === "realign" ? alignment.reason : null,
    },
  };
  switch (alignment.kind) {
    case "apply":
      return {
        ...base,
        local: alignment.session,
        realignStreak: 0,
        counters: bump(base.counters, "applied"),
      };
    case "reset":
      return {
        ...base,
        local: alignment.session,
        realignStreak: 0,
        counters: bump(base.counters, "reset"),
      };
    case "recover":
      return {
        ...base,
        local: alignment.session,
        realignStreak: 0,
        counters: bump(base.counters, "recovered"),
      };
    case "ignore-stale":
      return { ...base, counters: bump(base.counters, "ignoredStale") };
    case "already-applied":
      return { ...base, counters: bump(base.counters, "ignoredAlreadyApplied") };
    case "reject-identity":
      return { ...base, counters: bump(base.counters, "rejectedIdentity") };
    case "ignore-stale-connection":
      return { ...base, counters: bump(base.counters, "staleConnection") };
    case "reject-invalid":
      return {
        ...base,
        realignStreak: authoritative ? base.realignStreak + 1 : base.realignStreak,
        counters: bump(base.counters, "rejectedInvalid"),
      };
    case "realign": {
      const counters =
        alignment.reason === "hash-mismatch"
          ? bump(bump(base.counters, "hashMismatch"), "realign")
          : bump(base.counters, "realign");
      return { ...base, realignStreak: base.realignStreak + 1, counters };
    }
  }
}

/**
 * 一則決策 → 呼叫端**應該做的事**。
 *
 * 有界：這一步真的花掉一次對齊預算、而且已經到上限時，回 `unrecoverable`（狀態未知，
 * 停止自動重試），不再回 `realign`——那是打不完的請求迴圈。
 */
function effectsFor(
  before: SessionMachine,
  after: SessionMachine,
  alignment: SessionAlignment,
): readonly SessionEffect[] {
  const spent = after.realignStreak > before.realignStreak;
  if (spent && after.realignStreak >= REALIGN_STREAK_LIMIT) return [{ kind: "unrecoverable" }];
  return alignment.kind === "realign" ? [{ kind: "realign" }] : [];
}

function step(
  machine: SessionMachine,
  alignment: SessionAlignment,
  authoritative: boolean,
): DecidedStep {
  const next = settle(machine, alignment, authoritative);
  return { next, effects: effectsFor(machine, next, alignment), decision: alignment.kind };
}

/** 收下一則訊息（SSE 或請求回應）。 */
function ingest(
  machine: SessionMachine,
  envelope: unknown,
  authoritative: boolean,
  arrivedOn: number,
): DecidedStep {
  // 0. 舊連線／舊請求世代的遲到品——**先於**解析與一切 epoch 判斷。
  if (arrivedOn !== machine.connectionGeneration) {
    return step(machine, { kind: "ignore-stale-connection" }, authoritative);
  }
  const message = readStateEnvelope(envelope);
  if (!message) return step(machine, { kind: "reject-invalid" }, authoritative);
  return ingestMessage(machine, message, authoritative);
}

function ingestMessage(
  machine: SessionMachine,
  message: StateMessage,
  authoritative: boolean,
): DecidedStep {
  if (message.dedupeKey !== null && machine.seen.includes(message.dedupeKey)) {
    return {
      next: { ...machine, counters: bump(machine.counters, "duplicate") },
      effects: [],
      decision: "duplicate",
    };
  }
  const remembered: SessionMachine = { ...machine, seen: remember(machine.seen, message.dedupeKey) };
  return step(remembered, alignState(remembered.local, message), authoritative);
}

/** 一則 resume 回覆裡「跳過就好、不中止」的良性結果（host 回放的範圍與本地重疊是正常的）。 */
const BENIGN_IN_BATCH: ReadonlySet<string> = new Set([
  "apply",
  "reset",
  "recover",
  "already-applied",
  "ignore-stale",
  "duplicate",
]);

/** resume 的回應（payload，不是 envelope）。 */
function ingestResume(machine: SessionMachine, payload: unknown, arrivedOn: number): DecidedStep {
  if (arrivedOn !== machine.connectionGeneration) {
    return step(machine, { kind: "ignore-stale-connection" }, true);
  }
  const body = record(payload);
  const kind = body?.["kind"];
  if (body && kind === "snapshot") {
    const message = readStatePayload(body);
    if (!message) return step(machine, { kind: "reject-invalid" }, true);
    return ingestMessage(machine, message, true);
  }
  if (body && kind === "patches" && Array.isArray(body["patches"])) {
    const patches = body["patches"];
    // 有界：超過上限**整批不處理**（不靜默截斷成「我以為我追上了」）。
    if (patches.length > MAX_RESUME_PATCHES) {
      return step(machine, { kind: "realign", reason: "resume-too-long" }, true);
    }
    let current = machine;
    let last: DecidedStep | null = null;
    for (const item of patches) {
      const message = readResumePatch(item);
      if (!message) return step(current, { kind: "reject-invalid" }, true);
      const outcome = ingestMessage(current, message, true);
      current = outcome.next;
      last = outcome;
      // 良性的舊項（already-applied／ignore-stale／duplicate）跳過就好，不能中止：host 是照
      // 「client 送出 resume 時記得的 lastRevision」回放的，陣列前段可能整段都是本地已經走過的，
      // 後段才是真正新的補丁；中止會把後段靜默丟掉。真有缺口不會漏掉——下一則的 baseRevision
      // 對不上本地時就是 realign，下一行帶著它的 effect 中止（不依賴「逐項精確銜接」的假設）。
      if (!BENIGN_IN_BATCH.has(outcome.decision)) {
        return { next: current, effects: outcome.effects, decision: outcome.decision };
      }
    }
    return {
      next: current,
      effects: [],
      // 空批＝已經對齊（沒有任何一則訊息，也就沒有新的決策）。
      decision: last?.decision ?? "already-applied",
    };
  }
  // 看不懂的 resume 回應：記下來，但**不**自動再要一次（那會變成無界的請求迴圈）。
  return step(machine, { kind: "reject-invalid" }, true);
}

/**
 * 狀態機的唯一入口（純函式）。
 *
 * 回傳的 `effects` 是呼叫端**應該做的事**，不是已經發生的事：`realign` 代表
 * 「請再要一次對齊」，`unrecoverable` 代表「別再要了，狀態未知」。
 */
export function reduce(current: SessionMachine, input: SessionInput): SessionStep {
  switch (input.kind) {
    case "connection-changed":
      // 換了一條連線：舊連線送出的東西（含飛行中的請求回覆）從此不算數。
      return {
        next: { ...current, connectionGeneration: input.generation, pendingRequestId: null },
        effects: [],
      };
    case "sse": {
      const outcome = ingest(current, input.envelope, false, input.arrivedOn);
      return { next: outcome.next, effects: outcome.effects };
    }
    case "fetch-issued":
      return { next: { ...current, pendingRequestId: input.requestId }, effects: [] };
    case "fetch-response":
    case "resume-response": {
      if (current.pendingRequestId !== input.requestId) {
        // 上一輪的請求回來了：它的請求世代已經過期，一律不算數（規則 0）。
        const outcome = step(current, { kind: "ignore-stale-connection" }, true);
        return { next: outcome.next, effects: outcome.effects };
      }
      const settled: SessionMachine = { ...current, pendingRequestId: null };
      const outcome =
        input.kind === "fetch-response"
          ? ingest(settled, input.envelope, true, input.arrivedOn)
          : ingestResume(settled, input.payload, input.arrivedOn);
      return { next: outcome.next, effects: outcome.effects };
    }
    case "reset-local":
      // 讀不到就是讀不到：本地副本作廢，不用上一次的樣子冒充現在。
      // 計數保留（那是這條連線發生過的事實，不該被一次重掛抹掉）。
      return {
        next: {
          ...current,
          local: null,
          pendingRequestId: null,
          seen: [],
          realignStreak: 0,
        },
        effects: [],
      };
  }
}
