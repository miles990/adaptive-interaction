// @vitest-environment node
//
// AIP Character Session 的接收端狀態機（桌面端，純 reducer）。
//
// 權威決策在 Rust：`crates/interaction-session/src/patch.rs` 的 `accept_state_with_epoch`。
// 這一支釘住桌面端逐條鏡射它，外加 §6 要求的 hash 核對與「請求世代」——這兩件事
// 是 v0.6.0 的桌面端沒做、而 iOS（`SessionClient.swift`）做了的部分：
//
//   * 缺 `revision`／`sessionEpoch` 不得被當成 0（那會把任何一則壞訊息變成
//     「host 回到最初」，把本地狀態整份沖掉）；
//   * 慢的 GET 回應不得覆蓋在它之後才套用的 SSE（request generation）；
//   * 套用後 hash 對不上就不算套用（不猜、不硬套）。
//
// 每一條規則都可以只用純函式驗，沒有 React、沒有計時器。

import { describe, expect, it } from "vitest";

import { stateHash } from "../aip/canonical";
import {
  DEDUPE_RING_CAP,
  REALIGN_STREAK_LIMIT,
  initialSession,
  readStateEnvelope,
  reduce,
  type LocalSessionState,
  type SessionEffect,
  type SessionInput,
  type SessionMachine,
} from "../aip/sessionClient";

const BASE_STATE = {
  characterId: "ref-shape",
  mood: { kind: "neutral", intensity: 0 },
  activity: "idle",
  truth: { state: "none" },
  members: [],
  reducedMotion: false,
} as const;

let messageCounter = 0;
function messageId(): string {
  messageCounter += 1;
  return `msg_${messageCounter}`;
}

/** 一則 `state` envelope（SSE 與 GET 走的是同一個形狀）。 */
function stateEnvelope(payload: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return {
    specVersion: "aip/1.0",
    messageId: messageId(),
    messageType: "state",
    name: "character.session.snapshot",
    source: { kind: "session", id: "session.home" },
    occurredAt: "2026-09-05T00:00:00Z",
    ...extra,
    payload,
  };
}

function snapshotEnvelope(
  options: {
    revision: number;
    epoch: number;
    state?: Record<string, unknown>;
    reason?: string;
    hash?: string | null;
    sequence?: number;
  },
  extra: Record<string, unknown> = {},
) {
  const state = options.state ?? { ...BASE_STATE };
  const payload: Record<string, unknown> = {
    kind: "snapshot",
    revision: options.revision,
    sessionEpoch: options.epoch,
    state,
  };
  if (options.reason !== undefined) payload["reason"] = options.reason;
  if (options.sequence !== undefined) payload["sequence"] = options.sequence;
  if (options.hash !== null) payload["hash"] = options.hash ?? stateHash(state);
  return stateEnvelope(payload, extra);
}

function patchEnvelope(options: {
  revision: number;
  baseRevision: number;
  epoch: number;
  patch: Record<string, unknown>;
  hash?: string | null;
  sequence?: number;
}) {
  const payload: Record<string, unknown> = {
    kind: "patch",
    revision: options.revision,
    sessionEpoch: options.epoch,
    baseRevision: options.baseRevision,
    patch: options.patch,
  };
  if (options.sequence !== undefined) payload["sequence"] = options.sequence;
  if (options.hash !== undefined && options.hash !== null) payload["hash"] = options.hash;
  return stateEnvelope(payload, { name: "character.session.patch" });
}

/** 依序餵一串 input，回最後的機器與所有 effect。 */
function run(
  start: SessionMachine,
  inputs: readonly SessionInput[],
): { machine: SessionMachine; effects: SessionEffect[] } {
  let machine = start;
  const effects: SessionEffect[] = [];
  for (const input of inputs) {
    const step = reduce(machine, input);
    machine = step.next;
    effects.push(...step.effects);
  }
  return { machine, effects };
}

/** 一個「已經對齊到 epoch 3 / revision 20」的起點（用真的 snapshot 走進去，不硬塞）。 */
function aligned(revision = 20, epoch = 3): SessionMachine {
  const { machine } = run(initialSession(), [
    { kind: "sse", envelope: snapshotEnvelope({ revision, epoch }) },
  ]);
  if (!machine.local) throw new Error("fixture: 起點沒有對齊成功");
  return machine;
}

function local(machine: SessionMachine): LocalSessionState {
  if (!machine.local) throw new Error("expected a local session copy");
  return machine.local;
}

describe("readStateEnvelope：嚴格解析（缺欄位絕不變成 0）", () => {
  it("完整的 snapshot 讀得出 revision／epoch／hash", () => {
    const message = readStateEnvelope(snapshotEnvelope({ revision: 7, epoch: 2 }));
    expect(message?.kind).toBe("snapshot");
    expect(message?.revision).toBe(7);
    expect(message?.epoch).toBe(2);
    expect(message?.hash).toBe(stateHash({ ...BASE_STATE }));
  });

  it.each([
    ["缺 sessionEpoch", { kind: "snapshot", revision: 7, state: { ...BASE_STATE } }],
    ["缺 revision", { kind: "snapshot", sessionEpoch: 2, state: { ...BASE_STATE } }],
    ["負的 revision", { kind: "snapshot", revision: -1, sessionEpoch: 2, state: { ...BASE_STATE } }],
    ["負的 epoch", { kind: "snapshot", revision: 7, sessionEpoch: -3, state: { ...BASE_STATE } }],
    ["小數 revision", { kind: "snapshot", revision: 7.5, sessionEpoch: 2, state: { ...BASE_STATE } }],
    [
      "超過安全整數的 revision",
      { kind: "snapshot", revision: 2 ** 53, sessionEpoch: 2, state: { ...BASE_STATE } },
    ],
    ["revision 是字串", { kind: "snapshot", revision: "7", sessionEpoch: 2, state: { ...BASE_STATE } }],
    ["snapshot 缺 state", { kind: "snapshot", revision: 7, sessionEpoch: 2 }],
    ["未知 kind", { kind: "delta", revision: 7, sessionEpoch: 2, state: { ...BASE_STATE } }],
    ["patch 缺 baseRevision", { kind: "patch", revision: 7, sessionEpoch: 2, patch: {} }],
  ])("%s → invalid", (_label, payload) => {
    expect(readStateEnvelope(stateEnvelope(payload as Record<string, unknown>))).toBeNull();
  });

  it("messageType 不是 state 一律 invalid（不從別種訊息猜狀態）", () => {
    const envelope = snapshotEnvelope({ revision: 7, epoch: 2 });
    expect(readStateEnvelope({ ...envelope, messageType: "event" })).toBeNull();
  });
});

describe("(a) rollback 防護：同 epoch 的舊 snapshot 不得覆蓋本地", () => {
  it("local epoch3/rev20 收到 epoch3/rev10 的 snapshot → ignored rollback", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", envelope: snapshotEnvelope({ revision: 10, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(local(machine).epoch).toBe(3);
    expect(machine.counters.ignoredRollback).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("(b) 請求世代：慢的 GET 不得蓋回舊狀態", () => {
  it("新 SSE patch 先到、舊的初始 GET 後到 → GET 被忽略（stale）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      {
        kind: "sse",
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 3,
          patch: { activity: "reacting" },
        }),
      },
      // GET 在 SSE 之後才回來，帶的是更早的 revision 20。
      { kind: "fetch-response", requestId: 1, envelope: snapshotEnvelope({ revision: 20, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(local(machine).state["activity"]).toBe("reacting");
    expect(machine.counters.stale).toBe(1);
    expect(machine.counters.hostRegressed).toBe(0);
    expect(effects).toEqual([]);
  });

  it("中間沒有 SSE 套用時，同 epoch 的較舊 GET 是 host 的權威事實：接受並計 hostRegressed", () => {
    const start = aligned(20, 3);
    const { machine } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      {
        kind: "fetch-response",
        requestId: 1,
        envelope: snapshotEnvelope({
          revision: 12,
          epoch: 3,
          state: { ...BASE_STATE, activity: "resting" },
        }),
      },
    ]);
    expect(local(machine).revision).toBe(12);
    expect(local(machine).state["activity"]).toBe("resting");
    expect(machine.counters.hostRegressed).toBe(1);
  });

  it("過期的 requestId 回覆一律忽略（上一輪的請求）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      { kind: "fetch-issued", requestId: 2 },
      { kind: "fetch-response", requestId: 1, envelope: snapshotEnvelope({ revision: 99, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(machine.counters.stale).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("(c) 重播", () => {
  it("同一份 snapshot 重播 → ignored already-applied，本地不動", () => {
    const start = aligned(20, 3);
    const replay = snapshotEnvelope({ revision: 20, epoch: 3 });
    const { machine } = run(start, [{ kind: "sse", envelope: replay }]);
    expect(local(machine).revision).toBe(20);
    expect(machine.counters.ignoredAlreadyApplied).toBe(1);
  });

  it("同一則 messageId 再送一次 → duplicate（有界環，不重算）", () => {
    const start = aligned(20, 3);
    const envelope = patchEnvelope({
      revision: 21,
      baseRevision: 20,
      epoch: 3,
      patch: { activity: "reacting" },
    });
    const { machine } = run(start, [
      { kind: "sse", envelope },
      { kind: "sse", envelope },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(machine.counters.applied).toBe(2); // 起點的 snapshot ＋ 這一則 patch
    expect(machine.counters.duplicate).toBe(1);
  });

  it("去重環有界：超過上限的舊 messageId 會被擠出去（不是無界成長）", () => {
    let machine = aligned(20, 3);
    for (let i = 0; i < DEDUPE_RING_CAP + 10; i += 1) {
      machine = reduce(machine, {
        kind: "sse",
        envelope: snapshotEnvelope({ revision: 10, epoch: 3 }),
      }).next;
    }
    expect(machine.seen.length).toBeLessThanOrEqual(DEDUPE_RING_CAP);
  });
});

describe("(d) 壞掉的欄位不得被當成 0", () => {
  it.each([
    ["缺 sessionEpoch", { kind: "snapshot", revision: 30, state: { ...BASE_STATE } }],
    ["缺 revision", { kind: "snapshot", sessionEpoch: 3, state: { ...BASE_STATE } }],
    ["負的 revision", { kind: "snapshot", revision: -1, sessionEpoch: 3, state: { ...BASE_STATE } }],
    ["小數 epoch", { kind: "snapshot", revision: 30, sessionEpoch: 3.5, state: { ...BASE_STATE } }],
    [
      "超過安全整數",
      { kind: "snapshot", revision: Number.MAX_SAFE_INTEGER + 2, sessionEpoch: 3, state: { ...BASE_STATE } },
    ],
  ])("%s → invalid，本地原封不動且不會出現 revision 0", (_label, payload) => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", envelope: stateEnvelope(payload as Record<string, unknown>) },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(local(machine).revision).toBe(20);
    expect(local(machine).epoch).toBe(3);
    expect(machine.counters.invalid).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("(e) epoch 變了", () => {
  it("`reason: session-reset` ＋ 不同 epoch → 接受重建（丟掉本地）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        envelope: snapshotEnvelope({
          revision: 1,
          epoch: 4,
          reason: "session-reset",
          state: { ...BASE_STATE, activity: "resting" },
        }),
      },
    ]);
    expect(local(machine).revision).toBe(1);
    expect(local(machine).epoch).toBe(4);
    expect(local(machine).state["activity"]).toBe("resting");
    expect(machine.counters.reset).toBe(1);
    expect(effects).toEqual([]);
  });

  it("epoch 比本地小、但有 session-reset：一樣接受（host 重灌不是 rollback）", () => {
    const start = aligned(20, 3);
    const { machine } = run(start, [
      {
        kind: "sse",
        envelope: snapshotEnvelope({ revision: 1, epoch: 1, reason: "session-reset" }),
      },
    ]);
    expect(local(machine).epoch).toBe(1);
    expect(local(machine).revision).toBe(1);
  });

  it("epoch 不同但沒有 reason → 不猜，回 realign", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", envelope: snapshotEnvelope({ revision: 30, epoch: 4 }) },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(effects).toEqual([{ kind: "realign" }]);
    expect(machine.counters.realign).toBe(1);
  });
});

describe("(f) daemon 重啟／重新掛載", () => {
  it("RuntimeEvent.sequence 重來不影響判斷：payload 的 revision 才算數", () => {
    // 同一個 (epoch, payload.sequence) 也可能重複出現（重啟後從 1 重新編號）；
    // 只要 messageId 不同、revision 前進，就必須套用。
    const start = aligned(20, 3);
    const { machine } = run(start, [
      {
        kind: "sse",
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 3,
          sequence: 1,
          patch: { activity: "reacting" },
        }),
      },
      {
        kind: "sse",
        envelope: patchEnvelope({
          revision: 22,
          baseRevision: 21,
          epoch: 3,
          sequence: 1,
          patch: { activity: "resting" },
        }),
      },
    ]);
    expect(local(machine).revision).toBe(22);
    expect(local(machine).state["activity"]).toBe("resting");
  });

  it("reset-local：本地副本作廢，等待重新 GET（不留著上一次的樣子冒充現在）", () => {
    const start = aligned(20, 3);
    const { machine } = run(start, [{ kind: "reset-local" }]);
    expect(machine.local).toBeNull();
    expect(machine.pendingRequestId).toBeNull();
  });

  it("沒有本地副本時收到 patch → realign（不硬套、不猜）", () => {
    const { machine, effects } = run(initialSession(), [
      {
        kind: "sse",
        envelope: patchEnvelope({ revision: 5, baseRevision: 4, epoch: 1, patch: { activity: "x" } }),
      },
    ]);
    expect(machine.local).toBeNull();
    expect(effects).toEqual([{ kind: "realign" }]);
  });
});

describe("hash 核對（AIP §6）", () => {
  it("patch 套用後 hash 對不上 → 不套用、realign、計 hashMismatch", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 3,
          patch: { activity: "reacting" },
          hash: "0".repeat(64),
        }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.hashMismatch).toBe(1);
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("patch 帶對的 hash → 套用", () => {
    const start = aligned(20, 3);
    const next = { ...BASE_STATE, activity: "reacting" };
    const { machine } = run(start, [
      {
        kind: "sse",
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 3,
          patch: { activity: "reacting" },
          hash: stateHash(next),
        }),
      },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(local(machine).hash).toBe(stateHash(next));
  });

  it("snapshot 自己就對不上自己的 hash → 不套用、realign", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        envelope: snapshotEnvelope({ revision: 30, epoch: 3, hash: "f".repeat(64) }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.hashMismatch).toBe(1);
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("連續對齊失敗有上限：達上限後改回報 unrecoverable，不再無限重試", () => {
    let machine = aligned(20, 3);
    const effects: SessionEffect[] = [];
    for (let i = 0; i < REALIGN_STREAK_LIMIT + 1; i += 1) {
      const step = reduce(machine, {
        kind: "sse",
        envelope: snapshotEnvelope({ revision: 30 + i, epoch: 3, hash: "f".repeat(64) }),
      });
      machine = step.next;
      effects.push(...step.effects);
    }
    expect(effects.filter((e) => e.kind === "realign")).toHaveLength(REALIGN_STREAK_LIMIT);
    expect(effects[effects.length - 1]).toEqual({ kind: "unrecoverable" });
  });
});

describe("resume 回應", () => {
  it("patches 連續套用（每一則都走同一套 revision／hash 規則）", () => {
    const start = aligned(20, 3);
    const afterFirst = { ...BASE_STATE, activity: "reacting" };
    const afterSecond = { ...afterFirst, reducedMotion: true };
    const { machine } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      {
        kind: "resume-response",
        requestId: 7,
        payload: {
          kind: "patches",
          patches: [
            {
              sequence: 30,
              baseRevision: 20,
              revision: 21,
              patch: { activity: "reacting" },
              hash: stateHash(afterFirst),
              sessionEpoch: 3,
            },
            {
              sequence: 31,
              baseRevision: 21,
              revision: 22,
              patch: { reducedMotion: true },
              hash: stateHash(afterSecond),
              sessionEpoch: 3,
            },
          ],
        },
      },
    ]);
    expect(local(machine).revision).toBe(22);
    expect(local(machine).state).toEqual(afterSecond);
  });

  it("patches 中間有一則接不上就停在那裡並 realign（不跳過、不猜）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      {
        kind: "resume-response",
        requestId: 7,
        payload: {
          kind: "patches",
          patches: [
            { sequence: 30, baseRevision: 20, revision: 21, patch: { activity: "reacting" }, sessionEpoch: 3 },
            { sequence: 32, baseRevision: 30, revision: 31, patch: { activity: "resting" }, sessionEpoch: 3 },
          ],
        },
      },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("空的 patches ＝ 已經對齊：本地不動、不再要求對齊", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, payload: { kind: "patches", patches: [] } },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(effects).toEqual([]);
  });

  it("resume 回 session-reset snapshot（payload 直接是 state payload，沒有外層 envelope）→ 接受", () => {
    const start = aligned(20, 3);
    const state = { ...BASE_STATE, activity: "resting" };
    const { machine } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      {
        kind: "resume-response",
        requestId: 7,
        payload: {
          kind: "snapshot",
          revision: 1,
          sequence: 1,
          sessionEpoch: 9,
          reason: "session-reset",
          hash: stateHash(state),
          state,
        },
      },
    ]);
    expect(local(machine).epoch).toBe(9);
    expect(local(machine).revision).toBe(1);
    expect(machine.counters.reset).toBe(1);
  });

  it("過期 requestId 的 resume 回應忽略", () => {
    const start = aligned(20, 3);
    const { machine } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "fetch-issued", requestId: 8 },
      {
        kind: "resume-response",
        requestId: 7,
        payload: {
          kind: "snapshot",
          revision: 50,
          sessionEpoch: 3,
          state: { ...BASE_STATE },
          hash: stateHash({ ...BASE_STATE }),
        },
      },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(machine.counters.stale).toBe(1);
  });

  it("看不懂的 resume payload → invalid，本地不動，也不自動再要一次（不做無界迴圈）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, payload: { kind: "who-knows" } },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.invalid).toBe(1);
    expect(effects).toEqual([]);
  });
});
