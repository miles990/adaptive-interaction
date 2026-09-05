// 角色同步的一般模式投影（`docs/aip/character-session.md` §11 文案表的 UI 鏡射）。
//
// 這一支釘住四件事：
//   1. 十三種同步狀態各有一句人話，而且**窮舉**（`satisfies Record<…>` 讓漏掉的
//      狀態在 typecheck 就爆，不會靜默退化成把技術值印到畫面上）。
//   2. 一般模式**不得**出現 revision／sequence／epoch／schema／token 之類的技術詞
//      （正反兩面都斷言：該有的人話有、不該有的技術詞一個都沒有）。
//   3. 空狀態不像成功、未知不冒充已同步：沒有裝置是中性的「尚未連接 iPhone」，
//      認不得的 presence 一律退回「同步尚未完成」並標 `known: false`。
//   4. 每一態的「下一步」是穩定的 action id（M3 §4.2）——按鈕文案可以改寫，
//      機器語意不可以。文案測試因此只保護語意與安全句，不逐字釘死一般文案。
//
// 模擬 iPhone（fixture）的名稱本身已經含標籤，投影只負責原樣顯示，不再加工。

import { describe, expect, it } from "vitest";
import {
  CHARACTER_SYNC_EMERGENCY_TEXT,
  CHARACTER_SYNC_PROJECTION,
  CHARACTER_SYNC_RECOVERED_NOTE,
  CHARACTER_SYNC_STATES,
  characterSyncLastInteraction,
  characterSyncMembers,
  characterSyncProfileLabel,
  characterSyncProfileNote,
  characterSyncProfiles,
  characterSyncSafetyNote,
  characterSyncStoreSignals,
  characterSyncDeviceLine,
  projectCharacterSession,
  type CharacterSyncAction,
  type CharacterSyncActionId,
  type CharacterSyncMember,
  type CharacterSyncSignals,
  type CharacterSyncState,
  type CharacterSyncStoreSignals,
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

/** 保存層訊號：預設是「健康而且存得下來」。 */
function store(overrides: Partial<CharacterSyncStoreSignals> = {}): CharacterSyncStoreSignals {
  return {
    reset: false,
    parked: false,
    persistFailures: 0,
    lastPersistError: null,
    lastPersistedRevision: 42,
    migratedFrom: null,
    ...overrides,
  };
}

function member(overrides: Partial<CharacterSyncMember> = {}): CharacterSyncMember {
  return {
    name: FIXTURE_PHONE,
    remote: true,
    presence: "online",
    canPresent: true,
    // 協商結果齊全（每個 host intent 都演得出來）；拿不到協商結果是 null，不是 false。
    degraded: false,
    // 同步模式沒有回報（舊 Runtime／查不到出站通道）：維持既有語意，不憑空降級。
    syncProfile: null,
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

describe("角色同步投影：十三種狀態的語意契約", () => {
  it("狀態表窮舉十三種，且每一句都是人話（沒有技術詞）", () => {
    expect(CHARACTER_SYNC_STATES).toEqual([
      "synced",
      // v0.7.0：這條線送不到完整狀態的成員（`docs/aip/device-profile.md` §3.1）
      // ——只有 full-state 可以說「已同步」。
      "partial-sync",
      "reconnecting",
      "offline",
      "partial-capability",
      // 拿不到協商結果的那一態：不給綠勾也不誣賴裝置（對抗審查 general-mode-ux-022）。
      "capability-unknown",
      "syncing",
      "unrecoverable",
      "needs-reconfirmation",
      // M3 §4.3：撤銷／移除之後的**終態**——不是永遠亮著的「需要重新確認」。
      "local-only",
      "no-device",
      "disabled",
      // M3 §4.3b：保存層真的出問題（存不下來）才是狀態；曾經重建過只是歷史通知。
      "store-issue",
    ]);
    for (const state of CHARACTER_SYNC_STATES) {
      const projection = CHARACTER_SYNC_PROJECTION[state];
      expect(projection.headline.length).toBeGreaterThan(0);
      expect(projection.detail.length).toBeGreaterThan(0);
      expect(`${projection.headline}${projection.detail}`).not.toMatch(FORBIDDEN);
    }
  });

  // M3：文案表**不再逐字釘死**。逐字比對會把「把假警報改成人看得懂的話」變成一次
  // 破壞性改動，於是沒人敢改文案。這裡改成保護「不能鬆動的語意與安全句」：
  // 其餘一般文案（怎麼講、講幾個字）允許改寫。
  it("語意與安全句不得鬆動（其餘文案允許改寫）", () => {
    // 1. 緊急停止的固定安全句：一字不改（角色與 adapter 都不能覆寫）。
    expect(CHARACTER_SYNC_EMERGENCY_TEXT).toBe(
      "緊急停止中：角色已停止表演，解除前不會接受任何互動。"
    );

    // 2. 綠色只給真的已同步；其餘一律不是 ok。
    for (const state of CHARACTER_SYNC_STATES) {
      const tone = CHARACTER_SYNC_PROJECTION[state].tone;
      if (state === "synced") expect(tone).toBe("ok");
      else expect(tone, `${state} 不得給綠色`).not.toBe("ok");
    }

    // 3. needs-reconfirmation 必須講出「要在手機上重新確認」這件事。
    const reconfirm = CHARACTER_SYNC_PROJECTION["needs-reconfirmation"];
    expect(`${reconfirm.headline}${reconfirm.detail}`).toMatch(/重新確認/);

    // 4. local-only 是撤銷／移除的終態：必須說清楚「不會自動回來」與「要重新配對」，
    //    而且是中性的（撤銷成功不是故障，也不是成功）。
    const localOnly = CHARACTER_SYNC_PROJECTION["local-only"];
    expect(`${localOnly.headline}${localOnly.detail}`).toMatch(/不會自動/);
    expect(`${localOnly.headline}${localOnly.detail}`).toMatch(/重新配對/);
    expect(localOnly.tone).toBe("muted");

    // 5. 空狀態與關閉都不像故障（不給紅色）。
    for (const state of ["no-device", "local-only", "disabled"] as CharacterSyncState[]) {
      expect(CHARACTER_SYNC_PROJECTION[state].tone).toBe("muted");
    }

    // 6. partial-capability 不是故障：狀態已對齊，只是表演能力不完整——兩件事分開講。
    const partial = CHARACTER_SYNC_PROJECTION["partial-capability"];
    expect(partial.tone).not.toBe("bad");
    expect(`${partial.headline}${partial.detail}`).toMatch(/對齊/);
    expect(`${partial.headline}${partial.detail}`).toMatch(/表演/);
  });

  it("每一態的下一步是穩定的 action id（按鈕文案可改寫，機器語意不可）", () => {
    const expected: Record<CharacterSyncState, CharacterSyncActionId | null> = {
      synced: null,
      "partial-sync": "open-devices",
      syncing: null,
      disabled: null,
      reconnecting: "open-devices",
      offline: "open-devices",
      "partial-capability": "view-capabilities",
      "capability-unknown": "view-capabilities",
      unrecoverable: "safe-reconnect",
      "needs-reconfirmation": "reconfirm-device",
      "local-only": "connect-phone",
      "no-device": "connect-phone",
      "store-issue": "storage-help",
    };
    for (const state of CHARACTER_SYNC_STATES) {
      expect(CHARACTER_SYNC_PROJECTION[state].action.id, state).toBe(expected[state]);
    }
    // 有下一步就要有落點（storage-help 例外：它是說明，沒有可去的地方）。
    const target: Record<string, { tab: "connect"; hub?: "providers" | "devices" }> = {
      "connect-phone": { tab: "connect", hub: "providers" },
      "reconfirm-device": { tab: "connect", hub: "providers" },
      "safe-reconnect": { tab: "connect", hub: "providers" },
      "open-devices": { tab: "connect", hub: "devices" },
      "view-capabilities": { tab: "connect", hub: "devices" },
    };
    for (const state of CHARACTER_SYNC_STATES) {
      // `satisfies` 保留字面型別，所以要標成介面型別才看得到選填的 `target`。
      const action: CharacterSyncAction = CHARACTER_SYNC_PROJECTION[state].action;
      if (action.id === null) {
        expect(action.target, state).toBeUndefined();
        expect(action.label, state).toBeNull();
        continue;
      }
      expect(typeof action.label, state).toBe("string");
      expect(action.label ?? "", state).not.toMatch(FORBIDDEN);
      if (action.id === "storage-help") {
        expect(action.target, "storage-help 是說明，不是導覽").toBeUndefined();
      } else {
        expect(action.target, state).toEqual(target[action.id]);
      }
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
    // M3 §4.2：這不是故障（狀態已對齊，只是表演能力不完整），但也不是成功。
    expect(p.tone).toBe("info");
    expect(p.tone).not.toBe("ok");
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

  // M3 §4.3：撤銷／移除是一個**終態**。Runtime 的 provider 列永遠留著 revoked，
  // 舊投影因此在零裝置時永遠停在「需要重新確認裝置」——一個使用者做完該做的事
  // （移除手機）之後仍然亮著、而且做不了任何事的假警報。
  it("零裝置＋只剩歷史撤銷＝local-only（不是永遠亮著的「需要重新確認」）", () => {
    const p = projectCharacterSession(snapshot(), [], signals({ revokedDevice: true }));
    expect(p.state).toBe("local-only");
    expect(p.tone).toBe("muted");
    expect(`${p.headline}${p.detail}`).toMatch(/重新配對/);
    expect(p.action.id).toBe("connect-phone");
    // 這不改任何安全效果：文案必須明說被移除的手機**不會**自動回來。
    expect(`${p.headline}${p.detail}`).toMatch(/不會自動回來/);
  });

  it("手機連著但還不是同步成員＝需要重新確認裝置（不算已同步）", () => {
    const p = projectCharacterSession(snapshot(), [], signals({ connectedButNotSynced: true }));
    expect(p.state).toBe("needs-reconfirmation");
    expect(p.action.id).toBe("reconfirm-device");
  });

  it("撤銷過的裝置又連上來（裝置正嘗試回來）＝需要重新確認，不是 local-only", () => {
    const p = projectCharacterSession(
      snapshot(),
      [],
      signals({ revokedDevice: true, connectedButNotSynced: true })
    );
    expect(p.state).toBe("needs-reconfirmation");
  });

  it("需要重新確認時要指出是哪一台（只給名字，不給識別碼）", () => {
    const p = projectCharacterSession(
      snapshot(),
      [],
      signals({ connectedButNotSynced: true, pendingDeviceNames: [FIXTURE_PHONE] })
    );
    expect(p.detail).toContain(FIXTURE_PHONE);
    expect(p.detail).toMatch(/重新確認/);
    expect(p.detail).not.toMatch(FORBIDDEN);
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

  // --- M3 §4.3b：store 訊號分成「現在正在發生的問題」與「歷史通知」兩類 -------

  it("紀錄存不下來（parked）＝active issue，壓過「已同步」", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ parked: true }) })
    );
    expect(p.state).toBe("store-issue");
    expect(p.tone).toBe("warn");
    expect(p.tone).not.toBe("ok");
    expect(p.known).toBe(true);
    expect(p.action.id).toBe("storage-help");
    // 「這一輪存不下來，重新啟動之後會再試」——講的是紀錄，不是角色。
    expect(p.detail).toMatch(/重新啟動/);
    // 一般模式的人話：不得出現 storeNote／epoch／revision 之類的技術詞或後端原文。
    expect(`${p.headline}${p.detail}`).not.toMatch(FORBIDDEN);
    expect(`${p.headline}${p.detail}`).not.toMatch(/storeNote|corrupt|quarantine|parked/i);
  });

  it("持續寫入失敗（persistFailures>0 且有錯誤）＝active issue，說「目前寫不進去」", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({
        store: store({ persistFailures: 4, lastPersistError: "disk full: /var/state" }),
      })
    );
    expect(p.state).toBe("store-issue");
    expect(p.detail).toMatch(/寫不進去/);
    // 後端原文不得外洩。
    expect(`${p.headline}${p.detail}`).not.toContain("disk full");
    expect(`${p.headline}${p.detail}`).not.toMatch(FORBIDDEN);
  });

  it("只有計數、沒有錯誤原文＝不當成 active issue（不製造假警報）", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ persistFailures: 2, lastPersistError: null }) })
    );
    expect(p.state).toBe("synced");
  });

  it("曾經重建過但現在存得下來＝歷史通知，不再壓過「已同步」", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ reset: true, lastPersistedRevision: 7 }) })
    );
    expect(p.state).toBe("synced");
    expect(p.tone).toBe("ok");
    expect(p.note, "歷史通知要說出來，只是不當成警告").toBeTruthy();
    expect(p.note ?? "").not.toMatch(FORBIDDEN);
    expect(p.note ?? "").not.toMatch(/storeNote|quarantine/i);
  });

  // 重建之後還沒存過任何東西是**正常的**（新紀錄還沒寫第一筆），不是故障——
  // 但也不能說成「之後都正常存下來了」。通知照說，用詞照實。
  it("曾經重建、而且到現在一次都還沒存成功：仍是通知，但不宣稱已經存下來", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ reset: true, lastPersistedRevision: null }) })
    );
    expect(p.state).toBe("synced");
    expect(p.note ?? "").toMatch(/還沒/);
  });

  it("active issue 期間不再另外掛歷史通知（同一件事只講一次）", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ reset: true, parked: true }) })
    );
    expect(p.state).toBe("store-issue");
    expect(p.note).toBeNull();
  });

  // 決策表規則 6（`recover`）：host 明說它從較舊的權威狀態還原了，桌面照它說的退回去。
  // 那不是故障，但畫面上剛剛看得到的東西被換掉了——靜默處理會讓使用者以為自己記錯。
  it("host 從較舊的權威狀態還原過：掛一句人話的附註，不改 tone、不外洩 revision", () => {
    const p = projectCharacterSession(snapshot(), [member()], signals({ recovered: true }));
    expect(p.state).toBe("synced");
    expect(p.tone).toBe("ok");
    expect(p.note).toBe(CHARACTER_SYNC_RECOVERED_NOTE);
    expect(p.note ?? "").toContain("已依桌面的權威狀態重新對齊");
    // 一般模式一個數字都不給（更不給 revision）。
    expect(p.note ?? "").not.toMatch(FORBIDDEN);
    expect(p.note ?? "").not.toMatch(/[0-9]/);
  });

  it("還原的附註與保存層的歷史通知可以同時成立（兩件不同的事）", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ recovered: true, store: store({ reset: true, lastPersistedRevision: 7 }) })
    );
    expect(p.note ?? "").toContain("曾經重建過");
    expect(p.note ?? "").toContain(CHARACTER_SYNC_RECOVERED_NOTE);
  });

  it("沒有還原過就沒有那一句（不無中生有）", () => {
    expect(projectCharacterSession(snapshot(), [member()], signals()).note).toBeNull();
  });

  it("遷移（migratedFrom）不進一般模式：既不是狀態也不是通知", () => {
    const p = projectCharacterSession(
      snapshot(),
      [member()],
      signals({ store: store({ migratedFrom: 1 }) })
    );
    expect(p.state).toBe("synced");
    expect(p.note).toBeNull();
  });

  it("store 問題不得壓過「讀不到」與「關閉」（先擋不能相信的，再說紀錄）", () => {
    const parked = store({ parked: true });
    expect(
      projectCharacterSession(null, [], signals({ store: parked, enabled: false })).state
    ).toBe("disabled");
    expect(
      projectCharacterSession(null, [], signals({ store: parked, failedReads: 3 })).state
    ).toBe("unrecoverable");
    expect(projectCharacterSession(null, [], signals({ store: parked })).state).toBe("syncing");
  });

  it("characterSyncStoreSignals：讀不到診斷或舊 Runtime 沒有 store 欄位時不亂猜", () => {
    expect(characterSyncStoreSignals(null)).toBeNull();
    // 舊 Runtime：只有 storeNote，沒有 store 健康度。
    expect(characterSyncStoreSignals({ storeNote: "…", counters: {} })).toEqual({
      reset: true,
      parked: false,
      persistFailures: 0,
      lastPersistError: null,
      lastPersistedRevision: null,
      migratedFrom: null,
    });
    expect(
      characterSyncStoreSignals({
        storeNote: null,
        store: {
          format: 2,
          migratedFrom: 1,
          migrationNote: "…",
          lastPersistedRevision: 9,
          persistFailures: 3,
          skippedStale: 0,
          parked: true,
          lastPersistError: "boom",
          note: null,
        },
      })
    ).toEqual({
      reset: false,
      parked: true,
      persistFailures: 3,
      lastPersistError: "boom",
      lastPersistedRevision: 9,
      migratedFrom: 1,
    });
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
      [snapshot(), [member()], signals({ store: store({ parked: true }) })],
      [snapshot(), [member()], signals({ store: store({ reset: true }) })],
      [
        snapshot(),
        [],
        signals({ connectedButNotSynced: true, pendingDeviceNames: [FIXTURE_PHONE] }),
      ],
    ];
    for (const [snap, members, sig] of cases) {
      const p = projectCharacterSession(snap, members, sig);
      expect(`${p.headline}${p.detail}${p.note ?? ""}${p.action.label ?? ""}`).not.toMatch(
        FORBIDDEN
      );
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
      {
        name: FIXTURE_PHONE,
        remote: true,
        presence: "online",
        canPresent: true,
        degraded: null,
        // 沒有給 profiles：同步模式就是不知道（不猜成 full-state）。
        syncProfile: null,
      },
      {
        name: "這台電腦",
        remote: false,
        presence: "online",
        canPresent: true,
        degraded: null,
        syncProfile: null,
      },
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

// ---------------------------------------------------------------------------
// 成員同步模式（syncProfile；`docs/aip/device-profile.md` §3.1）
// ---------------------------------------------------------------------------
//
// Runtime 依「那條線的事實」（有沒有單則上限、會不會重組）＋已協商的 role 推導出
// 三種值。只有 `full-state` 拿得到完整的共享狀態，也只有它可以顯示「已同步」。
// 這一段釘住三件事：讀得出來、非 full-state 不給綠勾、一般模式看不到英文原始值。
describe("成員同步模式：只有 full-state 可以說「已同步」", () => {
  const DEVICE = "iphone-87b42264";

  /** 一個 online 的裝置成員（協商齊全）＝在沒有 syncProfile 時原本會拿到綠勾。 */
  function onlineSnapshot(id = DEVICE) {
    return snapshot({
      members: [
        {
          party: { kind: "device", id },
          role: "remote-renderer",
          presence: "online",
          negotiated: { intents: { idle: "exact" } },
        },
      ],
    });
  }

  it("status.characterSessionSync 與 diagnostics.members 都讀得出「裝置 → 同步模式」", () => {
    expect(
      characterSyncProfiles({
        characterSessionSync: [
          { deviceId: DEVICE, transport: "serial", syncProfile: "intent-only", presence: "online" },
        ],
      })
    ).toEqual({ [DEVICE]: "intent-only" });
    expect(
      characterSyncProfiles({
        members: [
          { party: { kind: "device", id: DEVICE }, syncProfile: "event-source" },
          // 這台電腦自己不是裝置成員，沒有同步模式可言。
          { party: { kind: "human-surface", id: "desktop" }, syncProfile: "full-state" },
        ],
      })
    ).toEqual({ [DEVICE]: "event-source" });
  });

  it("形狀不可信：不是物件／缺欄位／空字串都不會變成假的同步模式", () => {
    for (const input of [null, undefined, 3, "x", {}, { characterSessionSync: 5 }, { members: {} }]) {
      expect(characterSyncProfiles(input)).toEqual({});
    }
    expect(
      characterSyncProfiles({ characterSessionSync: [{ deviceId: "", syncProfile: "full-state" }] })
    ).toEqual({});
    expect(
      characterSyncProfiles({ characterSessionSync: [{ deviceId: DEVICE, syncProfile: "" }] })
    ).toEqual({});
  });

  it("人話標籤：full-state／沒有回報＝沒有附註；其餘一律說得出「拿不到完整狀態」", () => {
    // 沒有回報 ≠ 非 full-state（舊 Runtime 不送這個欄位）：不憑空降級。
    expect(characterSyncProfileLabel(undefined)).toBeNull();
    expect(characterSyncProfileLabel(null)).toBeNull();
    expect(characterSyncProfileLabel("")).toBeNull();
    expect(characterSyncProfileLabel("full-state")).toBeNull();
    expect(characterSyncProfileLabel("intent-only")).toBe("只接收指令");
    expect(characterSyncProfileLabel("event-source")).toBe("只回報事件");
    // 認不得的值不猜成 full-state，也不外洩原始字串。
    const unknown = characterSyncProfileLabel("something-new");
    expect(unknown).not.toBeNull();
    expect(unknown ?? "").not.toContain("something-new");
    for (const raw of ["intent-only", "event-source", "something-new"]) {
      expect(characterSyncProfileLabel(raw) ?? "").not.toMatch(/[a-z]/i);
      expect(characterSyncProfileNote(raw) ?? "").not.toMatch(/[a-z]/i);
      expect(characterSyncProfileNote(raw) ?? "").not.toMatch(FORBIDDEN);
    }
    expect(characterSyncProfileNote("full-state")).toBeNull();
  });

  it("成員帶著自己的同步模式（查不到就是 null，不猜）", () => {
    const members = characterSyncMembers(onlineSnapshot(), { [DEVICE]: FIXTURE_PHONE }, {
      [DEVICE]: "intent-only",
    });
    expect(members[0].syncProfile).toBe("intent-only");
    expect(characterSyncMembers(onlineSnapshot(), {}, {})[0].syncProfile).toBeNull();
  });

  it("online 但只接收指令／只回報事件：不是「已同步」，也不給綠勾", () => {
    for (const profile of ["intent-only", "event-source", "something-new"]) {
      const members = characterSyncMembers(onlineSnapshot(), { [DEVICE]: FIXTURE_PHONE }, {
        [DEVICE]: profile,
      });
      const p = projectCharacterSession(onlineSnapshot(), members, signals());
      expect(p.state, profile).toBe("partial-sync");
      expect(p.tone, profile).not.toBe("ok");
      expect(p.headline, profile).not.toContain("已同步");
      // 誠實：這一句必須自己說出「不算已同步」，不能只是不提。
      expect(p.detail, profile).toContain("不算已同步");
      expect(`${p.headline}${p.detail}${p.action.label ?? ""}`).not.toMatch(FORBIDDEN);
      expect(`${p.headline}${p.detail}`).not.toMatch(/[a-z]/i);
    }
  });

  it("full-state 與沒有回報維持既有語意（綠勾照舊）", () => {
    const cases: Record<string, string>[] = [{ [DEVICE]: "full-state" }, {}];
    for (const profiles of cases) {
      const members = characterSyncMembers(onlineSnapshot(), { [DEVICE]: FIXTURE_PHONE }, profiles);
      expect(projectCharacterSession(onlineSnapshot(), members, signals()).state).toBe("synced");
    }
  });

  it("同步模式的判定排在能力之前：狀態根本沒對齊時不得說「狀態已經對齊」", () => {
    const snap = snapshot({
      members: [
        {
          party: { kind: "device", id: DEVICE },
          role: "input-device",
          presence: "online",
        },
      ],
    });
    const members = characterSyncMembers(snap, { [DEVICE]: FIXTURE_PHONE }, {
      [DEVICE]: "event-source",
    });
    const p = projectCharacterSession(snap, members, signals());
    expect(p.state).toBe("partial-sync");
    expect(p.detail).not.toContain("對齊");
  });

  it("有裝置等著重新確認時，partial-sync 不得把它吞掉（要做的事排在前面）", () => {
    // `needs-reconfirmation` 是 warn ＋ 有落點（reconfirm-device）＋ 說得出是哪一台；
    // `partial-sync` 是 info、是一條線的長期性質。把 info 排在要人動手的那一態前面，
    // 等於整個吞掉「哪一台要重新確認」（對抗審查 general-mode-ux-025）。
    const members = characterSyncMembers(onlineSnapshot(), { [DEVICE]: FIXTURE_PHONE }, {
      [DEVICE]: "intent-only",
    });
    const p = projectCharacterSession(
      onlineSnapshot(),
      members,
      signals({ connectedButNotSynced: true, pendingDeviceNames: ["書桌 ESP32"] })
    );
    expect(p.state).toBe("needs-reconfirmation");
    expect(p.action.id).toBe("reconfirm-device");
    expect(p.detail).toContain("書桌 ESP32");
    // 綠勾一樣不得出現。
    expect(p.tone).not.toBe("ok");
  });

  it("沒有裝置等著重新確認時，非 full-state 仍然是 partial-sync（順序只影響那一種情況）", () => {
    const members = characterSyncMembers(onlineSnapshot(), { [DEVICE]: FIXTURE_PHONE }, {
      [DEVICE]: "intent-only",
    });
    expect(projectCharacterSession(onlineSnapshot(), members, signals()).state).toBe("partial-sync");
  });

  it("連接頁的裝置一行：非 full-state 不得寫成「已同步」", () => {
    const snap = onlineSnapshot();
    expect(characterSyncDeviceLine(snap, DEVICE)).toBe("角色同步：已同步");
    expect(characterSyncDeviceLine(snap, DEVICE, "full-state")).toBe("角色同步：已同步");
    const line = characterSyncDeviceLine(snap, DEVICE, "intent-only");
    expect(line).toContain("只接收指令");
    expect(line).not.toContain("已同步");
    expect(line).not.toMatch(/[a-z]/i);
    expect(characterSyncDeviceLine(snap, DEVICE, "event-source")).toContain("只回報事件");
  });
});
