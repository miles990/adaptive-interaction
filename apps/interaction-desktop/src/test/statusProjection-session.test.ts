// 角色同步的一般模式投影（`docs/aip/character-session.md` §11 文案表的 UI 鏡射）。
//
// 這一支釘住三件事：
//   1. 九種同步狀態各有一句固定人話，而且**窮舉**（`satisfies Record<…>` 讓漏掉的
//      狀態在 typecheck 就爆，不會靜默退化成把技術值印到畫面上）。
//   2. 一般模式**不得**出現 revision／sequence／epoch／schema／token 之類的技術詞
//      （正反兩面都斷言：該有的人話有、不該有的技術詞一個都沒有）。
//   3. 空狀態不像成功、未知不冒充已同步：沒有裝置是中性的「尚未連接 iPhone」，
//      認不得的 presence 一律退回「同步尚未完成」並標 `known: false`。
//
// 模擬 iPhone（fixture）的名稱本身已經含標籤，投影只負責原樣顯示，不再加工。

import { describe, expect, it } from "vitest";
import {
  CHARACTER_SYNC_PROJECTION,
  CHARACTER_SYNC_STATES,
  characterSyncLastInteraction,
  characterSyncMembers,
  characterSyncSafetyNote,
  projectCharacterSession,
  type CharacterSyncMember,
  type CharacterSyncSignals,
  type CharacterSyncState,
} from "../statusProjection";

const FIXTURE_PHONE = "模擬 iPhone（fixture）";

/** 一般模式一個字都不能出現的技術詞（含中英文寫法）。 */
const FORBIDDEN = /revision|sequence|epoch|schema|token|provider|lease|transport|uuid|payload|envelope/i;

function signals(overrides: Partial<CharacterSyncSignals> = {}): CharacterSyncSignals {
  return {
    enabled: true,
    failedReads: 0,
    revokedDevice: false,
    connectedButNotSynced: false,
    ...overrides,
  };
}

function member(overrides: Partial<CharacterSyncMember> = {}): CharacterSyncMember {
  return {
    name: FIXTURE_PHONE,
    remote: true,
    presence: "online",
    canPresent: true,
    ...overrides,
  };
}

/** `GET /v1/character-session` 的 snapshot envelope（只留一般模式用得到的部分）。 */
function snapshot(state: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    specVersion: "aip/1.0",
    messageType: "state",
    name: "character.session.snapshot",
    payload: {
      kind: "snapshot",
      state: {
        characterId: "character",
        mood: { kind: "neutral", intensity: 0 },
        activity: "idle",
        truth: { state: "none" },
        members: [],
        reducedMotion: false,
        ...state,
      },
    },
  };
}

describe("角色同步投影：九種狀態的固定人話", () => {
  it("狀態表窮舉九種，且每一句都是人話（沒有技術詞）", () => {
    expect(CHARACTER_SYNC_STATES).toEqual([
      "synced",
      "reconnecting",
      "offline",
      "partial-capability",
      "syncing",
      "unrecoverable",
      "needs-reconfirmation",
      "no-device",
      "disabled",
    ]);
    for (const state of CHARACTER_SYNC_STATES) {
      const projection = CHARACTER_SYNC_PROJECTION[state];
      expect(projection.headline.length).toBeGreaterThan(0);
      expect(projection.detail.length).toBeGreaterThan(0);
      expect(`${projection.headline}${projection.detail}`).not.toMatch(FORBIDDEN);
    }
  });

  it("契約 §11 的文案一字不改", () => {
    const spec: Record<CharacterSyncState, string> = {
      synced: "iPhone 已連接，角色狀態已同步",
      reconnecting: "iPhone 正在重新連線",
      offline: "iPhone 暫時離線",
      "partial-capability": "部分能力目前不可用",
      syncing: "同步尚未完成",
      unrecoverable: "無法恢復，請重新連接",
      "needs-reconfirmation": "需要重新確認裝置",
      "no-device": "尚未連接 iPhone",
      disabled: "角色同步目前關閉",
    };
    for (const state of CHARACTER_SYNC_STATES) {
      expect(CHARACTER_SYNC_PROJECTION[state].headline).toBe(spec[state]);
    }
  });
});

describe("角色同步投影：狀態判定", () => {
  it("有 online 的遠端成員＝已同步（ok）", () => {
    const p = projectCharacterSession(snapshot(), [member()], signals());
    expect(p.state).toBe("synced");
    expect(p.headline).toBe("iPhone 已連接，角色狀態已同步");
    expect(p.tone).toBe("ok");
    expect(p.known).toBe(true);
  });

  it("online 但那台裝置演不出角色＝部分能力不可用（不謊稱已同步）", () => {
    const p = projectCharacterSession(snapshot(), [member({ canPresent: false })], signals());
    expect(p.state).toBe("partial-capability");
    expect(p.headline).toBe("部分能力目前不可用");
    expect(p.tone).toBe("warn");
  });

  it("presence=reconnecting＝正在重新連線", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member({ presence: "reconnecting" })],
      signals()
    );
    expect(p.state).toBe("reconnecting");
    expect(p.tone).toBe("pending");
  });

  it("成員全部 offline＝暫時離線（不是成功、也不是沒有裝置）", () => {
    const p = projectCharacterSession(snapshot(), [member({ presence: "offline" })], signals());
    expect(p.state).toBe("offline");
    expect(p.headline).toBe("iPhone 暫時離線");
    expect(p.tone).toBe("warn");
  });

  it("讀不到權威狀態＝同步尚未完成（不假裝已同步）", () => {
    const p = projectCharacterSession(null, [], signals());
    expect(p.state).toBe("syncing");
    expect(p.headline).toBe("同步尚未完成");
  });

  it("連續三次讀不到＝無法恢復，請重新連接", () => {
    const p = projectCharacterSession(null, [], signals({ failedReads: 3 }));
    expect(p.state).toBe("unrecoverable");
    expect(p.tone).toBe("bad");
  });

  it("裝置被撤銷＝需要重新確認裝置", () => {
    const p = projectCharacterSession(snapshot(), [], signals({ revokedDevice: true }));
    expect(p.state).toBe("needs-reconfirmation");
    expect(p.headline).toBe("需要重新確認裝置");
  });

  it("手機連著但還不是同步成員＝需要重新確認裝置（不算已同步）", () => {
    const p = projectCharacterSession(snapshot(), [], signals({ connectedButNotSynced: true }));
    expect(p.state).toBe("needs-reconfirmation");
  });

  it("沒有任何裝置＝中性的「尚未連接 iPhone」（空狀態不像成功）", () => {
    const p = projectCharacterSession(snapshot(), [], signals());
    expect(p.state).toBe("no-device");
    expect(p.headline).toBe("尚未連接 iPhone");
    expect(p.tone).toBe("muted");
    expect(p.tone).not.toBe("ok");
  });

  it("Runtime 沒有啟用角色同步＝誠實說關閉，不說成沒有裝置", () => {
    const p = projectCharacterSession(null, [], signals({ enabled: false }));
    expect(p.state).toBe("disabled");
    expect(p.tone).toBe("muted");
  });

  it("認不得的 presence 不猜：退回「同步尚未完成」並標 known=false", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member({ presence: "teleporting" })],
      signals()
    );
    expect(p.state).toBe("syncing");
    expect(p.known).toBe(false);
    expect(p.detail).not.toMatch(FORBIDDEN);
  });

  it("桌面自己的角色視窗不算「遠端裝置」（一台手機都沒有時仍是空狀態）", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member({ remote: false, name: "這台電腦" })],
      signals()
    );
    expect(p.state).toBe("no-device");
  });

  it("每一種輸入組合的輸出都沒有技術詞", () => {
    const cases: [Record<string, unknown> | null, CharacterSyncMember[], CharacterSyncSignals][] = [
      [snapshot(), [member()], signals()],
      [snapshot(), [member({ presence: "offline" })], signals()],
      [null, [], signals({ failedReads: 5 })],
      [snapshot(), [], signals({ revokedDevice: true })],
      [snapshot(), [member({ presence: "??" })], signals()],
      [null, [], signals({ enabled: false })],
    ];
    for (const [snap, members, sig] of cases) {
      const p = projectCharacterSession(snap, members, sig);
      expect(`${p.headline}${p.detail}`).not.toMatch(FORBIDDEN);
    }
  });
});

describe("角色同步投影：成員清單與最近互動", () => {
  it("成員清單只列遠端裝置，名稱原樣顯示（模擬 iPhone（fixture）自帶標籤）", () => {
    const state = snapshot({
      members: [
        { party: { kind: "device", id: "iphone-87b42264" }, role: "remote-renderer", presence: "online" },
        { party: { kind: "human-surface", id: "desktop" }, role: "host-renderer", presence: "online" },
      ],
    });
    const members = characterSyncMembers(state, { "iphone-87b42264": FIXTURE_PHONE });
    expect(members).toEqual([
      { name: FIXTURE_PHONE, remote: true, presence: "online", canPresent: true },
      { name: "這台電腦", remote: false, presence: "online", canPresent: true },
    ]);
  });

  it("查不到名字的裝置用中性稱呼，絕不把裝置識別碼印到一般模式", () => {
    const state = snapshot({
      members: [
        { party: { kind: "device", id: "iphone-87b42264" }, role: "remote-renderer", presence: "offline" },
      ],
    });
    const members = characterSyncMembers(state, {});
    expect(members[0].name).toBe("一台裝置");
    expect(JSON.stringify(members)).not.toContain("87b42264");
  });

  it("只能收發輸入的裝置標成「演不出角色」（partial-capability 的來源）", () => {
    const state = snapshot({
      members: [
        { party: { kind: "device", id: "d1" }, role: "input-device", presence: "online" },
      ],
    });
    expect(characterSyncMembers(state, { d1: FIXTURE_PHONE })[0].canPresent).toBe(false);
  });

  it("最近互動翻成人話，並掛上互動的那台裝置名稱", () => {
    const state = snapshot({
      lastInteraction: {
        name: "character.interaction.touch",
        kind: "tap",
        source: "device:iphone-87b42264",
        at: "2026-09-04T12:30:00.000Z",
      },
    });
    const line = characterSyncLastInteraction(state, { "iphone-87b42264": FIXTURE_PHONE });
    expect(line).toBe(`${FIXTURE_PHONE}摸了摸角色`);
    expect(line).not.toMatch(FORBIDDEN);
  });

  it("每一種互動都有人話，未知種類不猜", () => {
    const table: [string, string, string][] = [
      ["character.interaction.touch", "tap", "摸了摸角色"],
      ["character.interaction.touch", "pat", "輕拍了角色"],
      ["character.interaction.touch", "stroke", "撫摸了角色"],
      ["character.interaction.touch", "longpress", "按著角色不放"],
      ["character.interaction.touch", "wiggle", "和角色互動了一下"],
      ["character.interaction.dismiss", "", "請角色休息一下"],
    ];
    for (const [name, kind, expected] of table) {
      const line = characterSyncLastInteraction(
        snapshot({ lastInteraction: { name, kind, source: "human-surface:desktop" } }),
        {}
      );
      expect(line).toBe(`你在這台電腦上${expected}`);
    }
  });

  it("沒有互動過就是 null（不編一個出來）", () => {
    expect(characterSyncLastInteraction(snapshot(), {})).toBeNull();
    expect(characterSyncLastInteraction(null, {})).toBeNull();
  });

  it("緊急停止中一定看得到固定安全句；其餘狀態沒有這一句", () => {
    expect(characterSyncSafetyNote(snapshot({ truth: { state: "emergency" } }))).toBe(
      "緊急停止中：角色已停止表演，解除前不會接受任何互動。"
    );
    expect(characterSyncSafetyNote(snapshot({ truth: { state: "working" } }))).toBeNull();
    expect(characterSyncSafetyNote(null)).toBeNull();
  });
});
