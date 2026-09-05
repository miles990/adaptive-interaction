// @vitest-environment node
//
// AIP Character Session 的接收端狀態機（桌面端，純 reducer）。
//
// 權威決策表在 `docs/aip/character-session.md` §7.2，權威實作是 Rust 的
// `crates/interaction-session/src/receive.rs::decide_receive`。**逐筆對答案**的那一支是
// `receive-decision-fixtures.test.ts`（三端共用同一份 fixture）；這一支是桌面端自己的
// 行為契約：解析的嚴格度、reducer 的世代／預算、resume 的逐則規則、以及「零差異」棘輪。
//
//   * 缺 `revision`／`sessionEpoch` 不得被當成 0（那會把任何一則壞訊息變成
//     「host 回到最初」，把本地狀態整份沖掉）；
//   * 舊連線／舊請求世代的遲到品不算數（規則 0，**先於**一切 epoch 判斷）；
//   * 套用後 hash 對不上就不算套用（不猜、不硬套）。
//
// 每一條規則都可以只用純函式驗，沒有 React、沒有計時器。

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { stateHash } from "../aip/canonical";
import { AIP_LIMITS } from "../aip/generated";
import {
  DEDUPE_RING_CAP,
  MAX_RESUME_PATCHES,
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
    { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision, epoch }) },
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
    ["未知 kind", { kind: "delta", revision: 7, sessionEpoch: 2, state: { ...BASE_STATE } }],
  ])("%s → 讀不出來", (_label, payload) => {
    expect(readStateEnvelope(stateEnvelope(payload as Record<string, unknown>))).toBeNull();
  });

  // 這三種**讀得出來**，但不是一則能用的 state 訊息：那是決策表的規則 2
  // （`reject-invalid`），不是 boundary 的「讀不出來」。兩者的差別在有界 realign
  // 預算怎麼記：壞掉的**權威回覆**算一次對齊失敗，推播上的垃圾不算。
  it.each([
    ["snapshot 缺 hash", { kind: "snapshot", revision: 30, sessionEpoch: 3, state: { ...BASE_STATE } }],
    ["snapshot 缺 state", { kind: "snapshot", revision: 30, sessionEpoch: 3, hash: "f".repeat(64) }],
    [
      "patch 缺 baseRevision",
      { kind: "patch", revision: 30, sessionEpoch: 3, patch: { activity: "x" } },
    ],
  ])("%s → reject-invalid（規則 2），本地原封不動", (_label, payload) => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", arrivedOn: 0, envelope: stateEnvelope(payload as Record<string, unknown>) },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.rejectedInvalid).toBe(1);
    // 推播上的垃圾不花對齊預算（它不是我們要來的答案）。
    expect(machine.realignStreak).toBe(0);
    expect(effects).toEqual([]);
  });

  it("壞掉的**權威回覆**算一次對齊失敗：連續三次就停止自動重試", () => {
    let machine = aligned(20, 3);
    const effects: SessionEffect[] = [];
    for (let i = 0; i < REALIGN_STREAK_LIMIT; i += 1) {
      machine = reduce(machine, { kind: "fetch-issued", requestId: i + 1 }).next;
      const step = reduce(machine, {
        kind: "fetch-response",
        requestId: i + 1,
        arrivedOn: 0,
        // 讀得出來、但缺 hash：AIP 1.0 的 snapshot 必帶 hash（沒有 legacy profile）。
        envelope: snapshotEnvelope({ revision: 30 + i, epoch: 3, hash: null }),
      });
      machine = step.next;
      effects.push(...step.effects);
    }
    expect(machine.counters.rejectedInvalid).toBe(REALIGN_STREAK_LIMIT);
    expect(machine.realignStreak).toBe(REALIGN_STREAK_LIMIT);
    expect(effects[effects.length - 1]).toEqual({ kind: "unrecoverable" });
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
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 10, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(local(machine).epoch).toBe(3);
    expect(machine.counters.ignoredStale).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("(b) 世代：慢的回覆與舊連線的遲到品都不算數（決策表規則 0／7）", () => {
  it("新 SSE patch 先到、舊的初始 GET 後到 → GET 落後，忽略（ignore-stale）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      {
        kind: "sse",
        arrivedOn: 0,
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 3,
          patch: { activity: "reacting" },
        }),
      },
      // GET 在 SSE 之後才回來，帶的是更早的 revision 20。
      { kind: "fetch-response", requestId: 1, arrivedOn: 0, envelope: snapshotEnvelope({ revision: 20, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(local(machine).state["activity"]).toBe("reacting");
    expect(machine.counters.ignoredStale).toBe(1);
    expect(effects).toEqual([]);
  });

  it("比本地舊的**權威回覆**也一樣忽略：同一個 session 的回退要 host 明說（規則 7）", () => {
    // 以前這裡是 `allowRegression`／`hostRegressed`：「中間沒有 SSE 套用過的話，
    // 最新的 HTTP 回覆比本地舊也接受」。那等於讓「哪一則先回來」決定畫面。
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      {
        kind: "fetch-response",
        arrivedOn: 0,
        requestId: 1,
        envelope: snapshotEnvelope({
          revision: 12,
          epoch: 3,
          state: { ...BASE_STATE, activity: "resting" },
        }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.ignoredStale).toBe(1);
    expect(machine.counters.applied).toBe(1); // 只有起點那一則
    expect(effects).toEqual([]);
  });

  it("host 明說 `recovery` 時才退回它的 revision（規則 6）", () => {
    const start = aligned(20, 3);
    const state = { ...BASE_STATE, activity: "resting" };
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      {
        kind: "fetch-response",
        arrivedOn: 0,
        requestId: 1,
        envelope: snapshotEnvelope({ revision: 12, epoch: 3, reason: "recovery", state }),
      },
    ]);
    expect(local(machine).revision).toBe(12);
    expect(local(machine).state["activity"]).toBe("resting");
    expect(machine.counters.recovered).toBe(1);
    // 退回是 host 說了算的事實，不是對齊失敗：不要求重新對齊，也清掉 realign 預算。
    expect(effects).toEqual([]);
    expect(machine.realignStreak).toBe(0);
  });

  it("`recovery` 只在同一個 epoch 內有效；epoch 不同一律 realign（不猜）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 12, epoch: 4, reason: "recovery" }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("過期的 requestId 回覆一律忽略（上一輪的請求）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      { kind: "fetch-issued", requestId: 2 },
      { kind: "fetch-response", requestId: 1, arrivedOn: 0, envelope: snapshotEnvelope({ revision: 99, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(machine.counters.staleConnection).toBe(1);
    expect(effects).toEqual([]);
  });

  it("連線換了一條之後，舊連線的 SSE 遲到品不算數——即使它宣告 session-reset（規則 0）", () => {
    // 舊連線送出的 `session-reset` 宣告的 epoch 一定與本地不同，任何 epoch 判斷都會被
    // 它騙過去（會被當成「host 重建了 session」而整份採用）。世代檢查是唯一防線。
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "connection-changed", generation: 1 },
      {
        kind: "sse",
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 1, epoch: 9, reason: "session-reset" }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.staleConnection).toBe(1);
    expect(machine.counters.reset).toBe(0);
    expect(effects).toEqual([]);
  });

  it("連線換了一條之後，飛行中的舊 GET 回覆也不算數（請求世代）", () => {
    const start = aligned(20, 3);
    const { machine } = run(start, [
      { kind: "fetch-issued", requestId: 1 },
      { kind: "connection-changed", generation: 1 },
      { kind: "fetch-issued", requestId: 2 },
      // 世代 0 發出的那一次現在才回來。
      { kind: "fetch-response", requestId: 2, arrivedOn: 0, envelope: snapshotEnvelope({ revision: 99, epoch: 3 }) },
    ]);
    expect(local(machine).revision).toBe(20);
    expect(machine.counters.staleConnection).toBe(1);
  });
});

describe("(b2) 身分：別的 session 的狀態不相干（規則 1）", () => {
  const identified = (id: string, revision: number, epoch = 3) =>
    snapshotEnvelope({ revision, epoch }, { sessionId: id });

  it("bootstrap 記下 host 的 sessionId，之後別的 session 一律 reject-identity", () => {
    const { machine: bootstrapped } = run(initialSession(), [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.home", 20) },
    ]);
    expect(local(bootstrapped).sessionId).toBe("session.home");
    const { machine, effects } = run(bootstrapped, [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.other-desktop", 21) },
    ]);
    expect(local(machine)).toEqual(local(bootstrapped));
    expect(machine.counters.rejectedIdentity).toBe(1);
    // **不** realign：realign 只會再要一次別人的 session。
    expect(effects).toEqual([]);
    expect(machine.counters.realign).toBe(0);
  });

  it("本地身分未知（bootstrap 的 snapshot 沒帶 sessionId）不算不符，套用時才記下來", () => {
    // 這是 fail-closed 的地雷：resume 回覆的 snapshot payload 不一定帶 sessionId，
    // 用它 bootstrap 之後本地有狀態卻沒有身分。舊寫法會把「未知」當成不符，於是之後
    // 每一則帶 sessionId 的 SSE 都 reject-identity——而 reject-identity 不 realign，
    // 那扇門永遠打不開。
    const { machine: bootstrapped } = run(initialSession(), [
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 20, epoch: 3 }) },
    ]);
    expect(local(bootstrapped).sessionId).toBeNull();

    const { machine, effects } = run(bootstrapped, [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.home", 21) },
    ]);
    expect(machine.counters.rejectedIdentity).toBe(0);
    expect(local(machine).revision).toBe(21);
    // 套用時把身分記下來，下一則就有得比。
    expect(local(machine).sessionId).toBe("session.home");
    expect(effects).toEqual([]);

    // 記下之後，別的 session 立刻被規則 1 擋下來。
    const { machine: guarded } = run(machine, [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.other-desktop", 22) },
    ]);
    expect(guarded.counters.rejectedIdentity).toBe(1);
    expect(local(guarded).revision).toBe(21);
  });

  it("放寬只發生在「本地不知道」那一格：知道就照樣 reject（連比較舊的也是）", () => {
    const { machine: bootstrapped } = run(initialSession(), [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.home", 20) },
    ]);
    const { machine } = run(bootstrapped, [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.other-desktop", 9) },
    ]);
    // 身分先於 revision：別的 session 的舊狀態不是 ignore-stale，是不相干。
    expect(machine.counters.rejectedIdentity).toBe(1);
    expect(machine.counters.ignoredStale).toBe(0);
    expect(local(machine)).toEqual(local(bootstrapped));
  });

  it("身分不符壓過 session-reset（不得讓別人的重建宣告接管本地）", () => {
    const { machine: bootstrapped } = run(initialSession(), [
      { kind: "sse", arrivedOn: 0, envelope: identified("session.home", 20) },
    ]);
    const { machine } = run(bootstrapped, [
      {
        kind: "sse",
        arrivedOn: 0,
        envelope: snapshotEnvelope(
          { revision: 1, epoch: 9, reason: "session-reset" },
          { sessionId: "session.other-desktop" }
        ),
      },
    ]);
    expect(local(machine)).toEqual(local(bootstrapped));
    expect(machine.counters.reset).toBe(0);
    expect(machine.counters.rejectedIdentity).toBe(1);
  });
});

describe("(c) 重播", () => {
  it("同一份 snapshot 重播 → ignored already-applied，本地不動", () => {
    const start = aligned(20, 3);
    const replay = snapshotEnvelope({ revision: 20, epoch: 3 });
    const { machine } = run(start, [{ kind: "sse", arrivedOn: 0, envelope: replay }]);
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
      { kind: "sse", arrivedOn: 0, envelope },
      { kind: "sse", arrivedOn: 0, envelope },
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
        arrivedOn: 0,
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
      { kind: "sse", arrivedOn: 0, envelope: stateEnvelope(payload as Record<string, unknown>) },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(local(machine).revision).toBe(20);
    expect(local(machine).epoch).toBe(3);
    expect(machine.counters.rejectedInvalid).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("(e) epoch 變了", () => {
  it("`reason: session-reset` ＋ 不同 epoch → 接受重建（丟掉本地）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        arrivedOn: 0,
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
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 1, epoch: 1, reason: "session-reset" }),
      },
    ]);
    expect(local(machine).epoch).toBe(1);
    expect(local(machine).revision).toBe(1);
  });

  it("epoch 不同但沒有 reason → 不猜，回 realign", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 30, epoch: 4 }) },
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
        arrivedOn: 0,
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
        arrivedOn: 0,
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

  it("reset-local 不作廢仍然當令的請求：飛行中的權威回覆回來時照樣算數", () => {
    // unrecoverable 之後 `CharacterSyncCard` 會馬上 reset-local。那一刻通常正好有一個
    // GET／resume 在飛：把 pendingRequestId 清成 null 會讓它回來時被當成上一輪的遲到品
    // 丟掉，畫面就停在「同步尚未完成」而且**沒有任何請求在跑**
    //（對抗審查 session-client-rollback-035）。
    const issued = run(aligned(20, 3), [{ kind: "fetch-issued", requestId: 7 }]);
    const afterReset = run(issued.machine, [{ kind: "reset-local" }]);
    expect(afterReset.machine.local).toBeNull();
    expect(afterReset.machine.pendingRequestId).toBe(7);

    const { machine } = run(afterReset.machine, [
      {
        kind: "fetch-response",
        requestId: 7,
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 21, epoch: 3 }),
      },
    ]);
    expect(local(machine).revision).toBe(21);
    expect(machine.pendingRequestId).toBeNull();
  });

  it("reset-local 不清去重環：已經處理過的訊息重播不得再花掉對齊預算", () => {
    // 去重環是**防重播**用的，本地副本作廢是另一件事。清掉它，呼叫端只要把保留的事件
    // 陣列再餵一次（`CharacterSyncCard` 的 SSE effect），三則舊 patch 就把有界的
    // realign 預算燒光，誤報「無法恢復」——後端其實完全正常
    //（對抗審查 session-client-rollback-036）。
    const start = aligned(20, 3);
    const replayed: SessionInput[] = [21, 22, 23].map((revision) => ({
      kind: "sse",
      arrivedOn: 0,
      envelope: patchEnvelope({
        revision,
        baseRevision: revision - 1,
        epoch: 3,
        patch: { activity: `step-${revision}` },
      }),
    }));
    const applied = run(start, replayed);
    expect(local(applied.machine).revision).toBe(23);
    expect(applied.effects).toEqual([]);

    const afterReset = run(applied.machine, [{ kind: "reset-local" }]);
    expect(afterReset.machine.local).toBeNull();
    const again = run(afterReset.machine, replayed);
    expect(again.effects).toEqual([]);
    expect(again.machine.realignStreak).toBe(0);
    expect(again.machine.counters.duplicate).toBe(3);
  });

  it("沒有本地副本時收到 patch → realign（不硬套、不猜）", () => {
    const { machine, effects } = run(initialSession(), [
      {
        kind: "sse",
        arrivedOn: 0,
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
        arrivedOn: 0,
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
        arrivedOn: 0,
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
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 30, epoch: 3, hash: "f".repeat(64) }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.hashMismatch).toBe(1);
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("連續對齊失敗有上限：第 maxRealignAttempts 次改回報 unrecoverable，不再無限重試", () => {
    let machine = aligned(20, 3);
    const effects: SessionEffect[] = [];
    for (let i = 0; i < REALIGN_STREAK_LIMIT; i += 1) {
      const step = reduce(machine, {
        kind: "sse",
        arrivedOn: 0,
        envelope: snapshotEnvelope({ revision: 30 + i, epoch: 3, hash: "f".repeat(64) }),
      });
      machine = step.next;
      effects.push(...step.effects);
    }
    expect(effects.filter((e) => e.kind === "realign")).toHaveLength(REALIGN_STREAK_LIMIT - 1);
    expect(effects[effects.length - 1]).toEqual({ kind: "unrecoverable" });
    expect(machine.realignStreak).toBe(REALIGN_STREAK_LIMIT);
  });

  it("任何一次成功套用都把對齊預算清零（apply／reset／recover）", () => {
    let machine = aligned(20, 3);
    machine = reduce(machine, {
      kind: "sse",
      arrivedOn: 0,
      envelope: snapshotEnvelope({ revision: 30, epoch: 3, hash: "f".repeat(64) }),
    }).next;
    expect(machine.realignStreak).toBe(1);
    machine = reduce(machine, {
      kind: "sse",
      arrivedOn: 0,
      envelope: snapshotEnvelope({ revision: 31, epoch: 3 }),
    }).next;
    expect(machine.realignStreak).toBe(0);
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
        arrivedOn: 0,
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
        arrivedOn: 0,
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

  it("前段補丁已被別的管道追過（stale）時，後段真正新的補丁不得被丟棄", () => {
    // 對抗審查 session-client-rollback-020：resume 在飛行途中，本地被另一條管道
    // （例如既有成員重新協商時 host 主動送的完整快照）推進到 revision 45；
    // host 卻仍照「client 送出時記得的 lastRevision=20」回放 21..50。
    // 前 25 項是良性的舊項（本地已經走過），後 5 項是真正比本地新的補丁。
    const start = aligned(20, 3);
    const stateAt = (revision: number) => ({ ...BASE_STATE, activity: `replay-${revision}` });
    const patches = [];
    for (let revision = 21; revision <= 50; revision += 1) {
      patches.push({
        sequence: 100 + revision,
        baseRevision: revision - 1,
        revision,
        patch: { activity: `replay-${revision}` },
        hash: stateHash(stateAt(revision)),
        sessionEpoch: 3,
      });
    }
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      // 另一條管道先把本地推到 45（同 epoch、hash 正確的完整快照）。
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 45, epoch: 3, state: stateAt(45) }) },
      { kind: "resume-response", requestId: 7, arrivedOn: 0, payload: { kind: "patches", patches } },
    ]);
    // 尾段（46..50）確實比本地新，必須套下去；良性舊項只記 stale，不觸發 realign。
    expect(local(machine).revision).toBe(50);
    expect(local(machine).state["activity"]).toBe("replay-50");
    expect(effects).toEqual([]);
    expect(machine.counters.realign).toBe(0);
    // 21..44 落後（ignore-stale），45 是重播（already-applied）：25 則良性舊項都跳過。
    expect(machine.counters.ignoredStale).toBe(24);
    expect(machine.counters.ignoredAlreadyApplied).toBe(1);
  });

  it("良性舊項之後真的有缺口時，仍然停下來並要求 realign（不硬跳）", () => {
    const start = aligned(20, 3);
    const stateAt = (revision: number) => ({ ...BASE_STATE, activity: `replay-${revision}` });
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 30, epoch: 3, state: stateAt(30) }) },
      {
        kind: "resume-response",
        arrivedOn: 0,
        requestId: 7,
        payload: {
          kind: "patches",
          patches: [
            // 舊項（本地已在 30）：跳過。
            {
              sequence: 121,
              baseRevision: 20,
              revision: 21,
              patch: { activity: "replay-21" },
              hash: stateHash(stateAt(21)),
              sessionEpoch: 3,
            },
            // 真的接不上（baseRevision 40 ≠ 本地 30）：停下來要求重新對齊。
            {
              sequence: 141,
              baseRevision: 40,
              revision: 41,
              patch: { activity: "replay-41" },
              hash: stateHash(stateAt(41)),
              sessionEpoch: 3,
            },
            {
              sequence: 142,
              baseRevision: 41,
              revision: 42,
              patch: { activity: "replay-42" },
              hash: stateHash(stateAt(42)),
              sessionEpoch: 3,
            },
          ],
        },
      },
    ]);
    expect(local(machine).revision).toBe(30);
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("patches 有界：超過上限**整批不處理**，改要求重新對齊（不截斷成「我以為我追上了」）", () => {
    const start = aligned(20, 3);
    const stateAt = (revision: number) => ({ ...BASE_STATE, activity: `replay-${revision}` });
    const patches = [];
    for (let revision = 21; revision <= 21 + MAX_RESUME_PATCHES; revision += 1) {
      patches.push({
        sequence: 1000 + revision,
        baseRevision: revision - 1,
        revision,
        patch: { activity: `replay-${revision}` },
        hash: stateHash(stateAt(revision)),
        sessionEpoch: 3,
      });
    }
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, arrivedOn: 0, payload: { kind: "patches", patches } },
    ]);
    // 截斷到上限會讓本地停在一個「從來沒有完整存在過」的中間狀態：一則都不套。
    expect(local(machine)).toEqual(local(start));
    expect(machine.lastDecision).toEqual({ decision: "realign", reason: "resume-too-long" });
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("剛好等於上限的一批仍然全部套用（邊界不是「差不多」）", () => {
    const start = aligned(20, 3);
    const stateAt = (revision: number) => ({ ...BASE_STATE, activity: `replay-${revision}` });
    const patches = [];
    for (let i = 0; i < MAX_RESUME_PATCHES; i += 1) {
      const revision = 21 + i;
      patches.push({
        sequence: 2000 + revision,
        baseRevision: revision - 1,
        revision,
        patch: { activity: `replay-${revision}` },
        hash: stateHash(stateAt(revision)),
        sessionEpoch: 3,
      });
    }
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, arrivedOn: 0, payload: { kind: "patches", patches } },
    ]);
    expect(local(machine).revision).toBe(20 + MAX_RESUME_PATCHES);
    expect(effects).toEqual([]);
  });

  it("空的 patches ＝ 已經對齊：本地不動、不再要求對齊", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, arrivedOn: 0, payload: { kind: "patches", patches: [] } },
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
        arrivedOn: 0,
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
        arrivedOn: 0,
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
    expect(machine.counters.staleConnection).toBe(1);
  });

  it("看不懂的 resume payload → invalid，本地不動，也不自動再要一次（不做無界迴圈）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "fetch-issued", requestId: 7 },
      { kind: "resume-response", requestId: 7, arrivedOn: 0, payload: { kind: "who-knows" } },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(machine.counters.rejectedInvalid).toBe(1);
    expect(effects).toEqual([]);
  });
});

describe("與 Rust／Swift 零差異（同一份 receiveDecisions fixtures 裁決）", () => {
  // 以前這裡釘的是「三處刻意差異」的清單：契約沒裁決時，桌面端選比較嚴的一邊，
  // 差異只能靠註解說明。契約已經裁決（`docs/aip/character-session.md` §7.2），
  // 三端改讀同一張表、對同一份跨語言 fixture 交答案——這一組把「零差異」釘死：
  // 檔頭必須指得出那份共同來源，決策名字與 realign 原因必須就是表上那些。
  const SOURCE = readFileSync(join(__dirname, "..", "aip", "sessionClient.ts"), "utf8");

  it("檔頭寫的是零差異，而且指得出三端共同的來源（receiveDecisions fixtures）", () => {
    expect(SOURCE).toContain("零差異");
    expect(SOURCE).toContain("receiveDecisions");
    expect(SOURCE).toContain("crates/interaction-aip/tests/fixtures/manifest.json");
    // 三個消費者都要指名，少一個就沒有人會發現那一端漂走了。
    expect(SOURCE).toContain("receive_decisions_from_json.rs");
    expect(SOURCE).toContain("src/test/receive-decision-fixtures.test.ts");
    expect(SOURCE).toContain("InteractionCompanionTests");
  });

  it("已取消的桌面端特例不得以任何形式留在原始碼裡", () => {
    // `allowRegression`／`hostRegressed` 是「最新的 HTTP 回覆比本地舊就接受」，
    // 決策表規則 6／7 取消了它：同一個 incarnation 的回退要 host 明說 `recovery`。
    for (const gone of ["allowRegression", "hostRegressed", "刻意差異"]) {
      expect(SOURCE, `${gone} 應該已經消失`).not.toContain(gone);
    }
  });

  it("兩個上限讀 codegen 的 AIP_LIMITS，不在這一端自己寫數字", () => {
    expect(SOURCE).toContain("AIP_LIMITS.maxResumePatches");
    expect(SOURCE).toContain("AIP_LIMITS.maxRealignAttempts");
    expect(MAX_RESUME_PATCHES).toBe(AIP_LIMITS.maxResumePatches);
    expect(REALIGN_STREAK_LIMIT).toBe(AIP_LIMITS.maxRealignAttempts);
  });

  it("決策名字就是表上那九個（多一個就代表某一端自己長出了新結論）", () => {
    const body = SOURCE.slice(
      SOURCE.indexOf("export function alignState"),
      SOURCE.indexOf("// ------------------------------------------------------------------ reducer")
    );
    // 決策可能長成 `{ kind: "x" }`，也可能經由 `adopt("x")`（hash 核對過才採用）。
    const kinds = [...body.matchAll(/(?:kind: |adopt\()"([a-z-]+)"/g)].map((match) => match[1]);
    expect([...new Set(kinds)].sort()).toEqual([
      "already-applied",
      "apply",
      "ignore-stale",
      "realign",
      "recover",
      "reject-identity",
      "reject-invalid",
      "reset",
    ]);
    const reasons = [...body.matchAll(/reason: "([a-z-]+)"/g)].map((match) => match[1]);
    // `ignore-stale-connection` 與 `resume-too-long` 是 reducer 那一層的
    //（世代與整批上限），不在 `alignState` 的輸入裡。
    expect([...new Set(reasons)].sort()).toEqual([
      "base-mismatch",
      "epoch-changed",
      "hash-mismatch",
      "no-local",
    ]);
  });

  it("patch 缺 sessionEpoch → 讀不出來（boundary），不會被當成 epoch 0", () => {
    const envelope = stateEnvelope(
      { kind: "patch", revision: 21, baseRevision: 20, patch: { activity: "reacting" } },
      { name: "character.session.patch" }
    );
    expect(readStateEnvelope(envelope)).toBeNull();
  });

  it("patch 的 epoch 與本地不同 → realign（規則 11，三端相同）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      {
        kind: "sse",
        arrivedOn: 0,
        envelope: patchEnvelope({
          revision: 21,
          baseRevision: 20,
          epoch: 4,
          patch: { activity: "reacting" },
        }),
      },
    ]);
    expect(local(machine)).toEqual(local(start));
    expect(effects).toEqual([{ kind: "realign" }]);
  });

  it("snapshot 的 epoch 不同又沒有 session-reset → realign，本地 epoch 不被改寫（規則 5）", () => {
    const start = aligned(20, 3);
    const { machine, effects } = run(start, [
      { kind: "sse", arrivedOn: 0, envelope: snapshotEnvelope({ revision: 30, epoch: 4 }) },
    ]);
    expect(local(machine).epoch).toBe(3);
    expect(local(machine).revision).toBe(20);
    expect(effects).toEqual([{ kind: "realign" }]);
  });
});
