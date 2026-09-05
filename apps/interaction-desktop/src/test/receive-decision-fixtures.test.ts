// @vitest-environment node
//
// AIP 1.0 **接收端決策表**（`docs/aip/character-session.md` §7.2）的桌面端對答案。
//
// 讀的是 Rust crate 底下**同一份** fixture（`crates/interaction-aip/tests/fixtures/manifest.json`
// 的 `receiveDecisions` 段，43 個具名案例），與 `canonical-hash.test.ts` 同一個讀法。
// Rust（`crates/interaction-session/tests/receive_decisions_from_json.rs`）與
// Swift（`InteractionCompanionTests`）逐筆對同一個 `expect`；三端得到同一個決策這件事
// 因此不是靠人對照註解維持的，而是三邊會同時紅。
//
// # hash 怎麼搬過來
//
// fixture 的 `hash`／`computedHash` 是不透明的 SHA-256 字串；桌面端的接收機**自己算**
// hash（`../aip/canonical.ts`），所以這裡搬的是「哪兩個 hash 相同、哪兩個不同」這個**關係**，
// 不是字串本身：每一個出現過的 hash 字串固定對應一個真的 state 物件（同字串＝同物件＝同 hash），
// snapshot 帶的 state 用 `computedHash` 那一份、宣告的 `hash` 用 `hash` 那一份；patch 則送一份
// 「把本地狀態改寫成 `computedHash` 那個物件」的 merge patch。於是
// 「宣告的 hash ＝ 自己算出來的 hash」在兩邊是同一個真假值。
//
// # 為什麼有些案例走 `reduce()`
//
// 決策表的規則 0（連線／請求世代）與有界 realign 預算是**呼叫端狀態**，不在
// `alignState()` 的輸入裡（它只看本地副本與一則訊息）。那些案例走完整的 `reduce()` 路徑：
// SSE 與 HTTP 回覆都帶 `arrivedOn` 世代進去。一個案例都不跳過。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { stateHash } from "../aip/canonical";
import {
  MAX_RESUME_PATCHES,
  REALIGN_STREAK_LIMIT,
  alignState,
  initialSession,
  reduce,
  type LocalSessionState,
  type SessionEffect,
  type SessionMachine,
  type StateMessage,
} from "../aip/sessionClient";

const FIXTURES = decodeURIComponent(
  new URL("../../../../crates/interaction-aip/tests/fixtures/", import.meta.url).pathname,
);

interface FixtureLocal {
  hasState: boolean;
  sessionId?: string;
  epoch: number;
  revision: number;
  hash?: string;
  connectionGeneration: number;
}

interface FixtureIncoming {
  kind: "snapshot" | "patch";
  sessionId?: string;
  epoch: number;
  revision: number;
  baseRevision?: number;
  reason?: string;
  hash?: string;
  computedHash?: string;
  statePresent?: boolean;
  arrivedOnGeneration: number;
  viaAuthoritativeReply?: boolean;
}

interface FixtureExpect {
  decision: string;
  reason?: string;
  revisionAfter: number;
  epochAfter: number;
  budget: "ok" | "unrecoverable";
  budgetAfter: number;
  applied?: number;
  skipped?: number;
  stoppedAt?: number;
}

interface FixtureCase {
  id: string;
  note: string;
  local: FixtureLocal;
  incoming?: FixtureIncoming;
  incomingBatch?: FixtureIncoming[];
  incomingBatchChain?: { kind: string; count: number };
  budgetBefore?: number;
  expect: FixtureExpect;
}

const manifest = JSON.parse(readFileSync(`${FIXTURES}manifest.json`, "utf8")) as {
  receiveDecisions: FixtureCase[];
};
const CASES = manifest.receiveDecisions;

// ------------------------------------------------------------ hash → 真的 state

/** 一個 hash 字串固定對應一份 state（同字串＝同物件＝同 canonical hash）。 */
const STATES = new Map<string, Record<string, unknown>>();

function stateFor(key: string): Record<string, unknown> {
  const known = STATES.get(key);
  if (known) return known;
  // 每一份都是 host 真的會送的形狀，只有 `activity` 不同——所以兩份 state 的 hash
  // 不同 iff 對應的 fixture hash 字串不同。
  const state: Record<string, unknown> = {
    characterId: "character",
    mood: { kind: "neutral", intensity: 0 },
    activity: `state-${STATES.size}`,
    truth: { state: "none" },
    members: [],
    reducedMotion: false,
  };
  STATES.set(key, state);
  return state;
}

function localFrom(fixture: FixtureLocal): LocalSessionState | null {
  if (!fixture.hasState) return null;
  const state = stateFor(fixture.hash ?? "local-without-hash");
  return {
    sessionId: fixture.sessionId ?? null,
    epoch: fixture.epoch,
    revision: fixture.revision,
    sequence: null,
    state,
    hash: stateHash(state),
  };
}

/** patch ＝「把本地狀態改寫成 `computedHash` 指的那一份」；沒有 computedHash 就是不改變。 */
function patchFor(incoming: FixtureIncoming): Record<string, unknown> {
  return incoming.computedHash === undefined ? {} : { ...stateFor(incoming.computedHash) };
}

function messageFrom(incoming: FixtureIncoming): StateMessage {
  const common = {
    revision: incoming.revision,
    epoch: incoming.epoch,
    sequence: null,
    // 宣告的 hash：fixture 說有就給一個真的（對應那個字串的 state 的 hash）。
    hash: incoming.hash === undefined ? null : stateHash(stateFor(incoming.hash)),
    reason: incoming.reason ?? null,
    sessionId: incoming.sessionId ?? null,
    dedupeKey: null,
  };
  if (incoming.kind === "snapshot") {
    return {
      kind: "snapshot",
      ...common,
      // 收到的 state ＝ 接收端算出來會是 `computedHash` 的那一份。
      state: incoming.statePresent ? stateFor(incoming.computedHash ?? "unverified") : null,
      patch: undefined,
      baseRevision: null,
    };
  }
  return {
    kind: "patch",
    ...common,
    state: null,
    patch: patchFor(incoming),
    baseRevision: incoming.baseRevision ?? null,
  };
}

let envelopeCounter = 0;

/** 同一則訊息的 wire 形狀（走 `reduce()` 時用；messageId 每則都不同，不會撞去重環）。 */
function envelopeFrom(message: StateMessage): Record<string, unknown> {
  envelopeCounter += 1;
  const payload: Record<string, unknown> = {
    kind: message.kind,
    revision: message.revision,
    sessionEpoch: message.epoch,
  };
  if (message.reason !== null) payload["reason"] = message.reason;
  if (message.hash !== null) payload["hash"] = message.hash;
  if (message.kind === "snapshot") {
    if (message.state !== null) payload["state"] = message.state;
  } else {
    if (message.baseRevision !== null) payload["baseRevision"] = message.baseRevision;
    payload["patch"] = message.patch;
  }
  return {
    specVersion: "aip/1.0",
    messageId: `fixture-${envelopeCounter}`,
    messageType: "state",
    name: message.kind === "snapshot" ? "character.session.snapshot" : "character.session.patch",
    occurredAt: "2026-09-06T00:00:00Z",
    ...(message.sessionId === null ? {} : { sessionId: message.sessionId }),
    payload,
  };
}

/** resume 回覆裡的攤平補丁（`transport-bindings` §1.3）。 */
function resumePatchFrom(message: StateMessage): Record<string, unknown> {
  const entry: Record<string, unknown> = {
    revision: message.revision,
    sessionEpoch: message.epoch,
    patch: message.patch,
  };
  if (message.baseRevision !== null) entry["baseRevision"] = message.baseRevision;
  if (message.hash !== null) entry["hash"] = message.hash;
  if (message.sessionId !== null) entry["sessionId"] = message.sessionId;
  return entry;
}

/** 一台「已經在這個狀態」的接收機（純 reducer，直接給起點比一路演進來清楚）。 */
function machineAt(
  local: LocalSessionState | null,
  connectionGeneration: number,
  realignStreak: number,
): SessionMachine {
  return { ...initialSession(), local, connectionGeneration, realignStreak };
}

/** `incomingBatchChain{kind, count}` ＝「從本地 revision 起連續 count 則 patch」。 */
function expandChain(local: FixtureLocal, spec: { kind: string; count: number }): FixtureIncoming[] {
  expect(spec.kind, "目前只支援 patch 鏈").toBe("patch");
  return Array.from({ length: spec.count }, (_unused, index) => ({
    kind: "patch" as const,
    sessionId: local.sessionId,
    epoch: local.epoch,
    revision: local.revision + index + 1,
    baseRevision: local.revision + index,
    statePresent: false,
    arrivedOnGeneration: local.connectionGeneration,
    viaAuthoritativeReply: false,
  }));
}

function decisionOf(machine: SessionMachine): { decision: string; reason: string | null } {
  const last = machine.lastDecision;
  if (!last) throw new Error("reducer 沒有記下任何決策");
  return { decision: last.decision, reason: last.reason };
}

function positionOf(machine: SessionMachine): { revision: number; epoch: number } {
  return machine.local === null
    ? { revision: 0, epoch: 0 }
    : { revision: machine.local.revision, epoch: machine.local.epoch };
}

function budgetOf(machine: SessionMachine): "ok" | "unrecoverable" {
  return machine.realignStreak >= REALIGN_STREAK_LIMIT ? "unrecoverable" : "ok";
}

const SINGLE = CASES.filter((entry) => entry.incoming !== undefined);
const BATCHES = CASES.filter((entry) => entry.incoming === undefined);

describe("接收端決策表：跨語言 fixture（receiveDecisions）", () => {
  it("索引裡的案例一個都不少（43 個具名案例）", () => {
    expect(CASES.length).toBeGreaterThanOrEqual(43);
    expect(SINGLE.length + BATCHES.length).toBe(CASES.length);
    expect(new Set(CASES.map((entry) => entry.id)).size).toBe(CASES.length);
  });

  it.each(SINGLE.map((entry) => [entry.id, entry] as const))(
    "%s",
    (_id, entry) => {
      const incoming = entry.incoming as FixtureIncoming;
      const local = localFrom(entry.local);
      const message = messageFrom(incoming);
      const generation = entry.local.connectionGeneration;
      const staleGeneration = incoming.arrivedOnGeneration !== generation;

      // 規則 1..15 是純函式：直接問 `alignState`。規則 0（世代）不在它的輸入裡，
      // 所以世代案例改由 `reduce()` 裁決——而且要證明「不是 alignState 剛好也擋掉」。
      const aligned = alignState(local, message);
      if (staleGeneration) {
        expect(aligned.kind, `${entry.id}：世代案例必須是 reducer 擋的`).not.toBe(
          "ignore-stale-connection",
        );
      } else {
        expect(aligned.kind, `${entry.id}：決策不同`).toBe(entry.expect.decision);
        expect(
          aligned.kind === "realign" ? aligned.reason : null,
          `${entry.id}：realign 原因不同`,
        ).toBe(entry.expect.reason ?? null);
      }

      // 完整路徑：世代 → 身分 → 格式 → epoch → revision → hash，外加有界 realign 預算。
      const authoritative = incoming.viaAuthoritativeReply === true;
      const start = machineAt(local, generation, entry.budgetBefore ?? 0);
      const envelope = envelopeFrom(message);
      const effects: SessionEffect[] = [];
      let machine = start;
      if (authoritative) {
        const issued = reduce(machine, { kind: "fetch-issued", requestId: 1 });
        machine = issued.next;
        const step = reduce(machine, {
          kind: "fetch-response",
          requestId: 1,
          envelope,
          arrivedOn: incoming.arrivedOnGeneration,
        });
        machine = step.next;
        effects.push(...step.effects);
      } else {
        const step = reduce(machine, {
          kind: "sse",
          envelope,
          arrivedOn: incoming.arrivedOnGeneration,
        });
        machine = step.next;
        effects.push(...step.effects);
      }

      expect(decisionOf(machine), `${entry.id}：reducer 的決策不同`).toEqual({
        decision: entry.expect.decision,
        reason: entry.expect.reason ?? null,
      });
      expect(positionOf(machine), `${entry.id}：套用後的位置不同`).toEqual({
        revision: entry.expect.revisionAfter,
        epoch: entry.expect.epochAfter,
      });
      expect(machine.realignStreak, `${entry.id}：realign 預算不同`).toBe(entry.expect.budgetAfter);
      expect(budgetOf(machine), `${entry.id}：realign 預算的結論不同`).toBe(entry.expect.budget);
      if (entry.expect.budget === "unrecoverable") {
        expect(effects, `${entry.id}：達上限要停止自動重試`).toContainEqual({
          kind: "unrecoverable",
        });
      }
      // 不採用狀態的決策**不得**動到本地副本。
      if (!["apply", "reset", "recover"].includes(entry.expect.decision)) {
        expect(machine.local, `${entry.id}：不採用的決策不得改變本地狀態`).toEqual(start.local);
      }
    },
  );

  it.each(BATCHES.map((entry) => [entry.id, entry] as const))(
    "%s",
    (_id, entry) => {
      const items =
        entry.incomingBatch ??
        expandChain(entry.local, entry.incomingBatchChain as { kind: string; count: number });
      const local = localFrom(entry.local);
      const generation = entry.local.connectionGeneration;
      const start = machineAt(local, generation, entry.budgetBefore ?? 0);
      const patches = items.map((item) => resumePatchFrom(messageFrom(item)));

      const issued = reduce(start, { kind: "fetch-issued", requestId: 1 });
      const step = reduce(issued.next, {
        kind: "resume-response",
        requestId: 1,
        payload: { kind: "patches", patches },
        arrivedOn: generation,
      });
      const machine = step.next;

      expect(decisionOf(machine), `${entry.id}：整批的結論不同`).toEqual({
        decision: entry.expect.decision,
        reason: entry.expect.reason ?? null,
      });
      expect(positionOf(machine), `${entry.id}：套用後的位置不同`).toEqual({
        revision: entry.expect.revisionAfter,
        epoch: entry.expect.epochAfter,
      });
      const applied =
        machine.counters.applied + machine.counters.reset + machine.counters.recovered;
      const skipped = machine.counters.ignoredStale + machine.counters.ignoredAlreadyApplied;
      expect(applied, `${entry.id}：套用筆數不同`).toBe(entry.expect.applied);
      expect(skipped, `${entry.id}：跳過筆數不同`).toBe(entry.expect.skipped);
      // 中止的位置 ＝ 它前面已經處理完的筆數（套用的＋良性跳過的）。
      if (entry.expect.stoppedAt === undefined) {
        expect(step.effects, `${entry.id}：整批走完就不該再要求對齊`).toEqual([]);
      } else {
        expect(applied + skipped, `${entry.id}：中止位置不同`).toBe(entry.expect.stoppedAt);
      }
      expect(machine.realignStreak, `${entry.id}：realign 預算不同`).toBe(entry.expect.budgetAfter);
      expect(budgetOf(machine), `${entry.id}：realign 預算的結論不同`).toBe(entry.expect.budget);
    },
  );

  it("上限就是 codegen 產出的那個數字（不得在這一端自己寫寬鬆一點）", () => {
    // `resume-reply-at-the-bound-applies-every-patch` 的 count 就是權威上限。
    const bound = CASES.find((entry) => entry.id === "resume-reply-at-the-bound-applies-every-patch");
    expect(bound?.incomingBatchChain?.count).toBe(MAX_RESUME_PATCHES);
    // 有界 realign：連續 3 次未能 apply → unrecoverable。
    const exhausted = CASES.find(
      (entry) => entry.id === "realign-budget-exhausted-after-three-attempts",
    );
    expect(exhausted?.expect.budgetAfter).toBe(REALIGN_STREAK_LIMIT);
  });
});
