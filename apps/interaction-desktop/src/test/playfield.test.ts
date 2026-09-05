// v0.5 Phase 3 遊戲互動 regression：玩具物理、投擲、追逐/撲抓/帶回/
// 拒絕歸還、凍結與 Reduced Motion 門檻、使魔、Roll Call、設定匯出匯入。

import { describe, expect, it } from "vitest";
import {
  clearToys,
  createWorld,
  grabToyAt,
  releaseToy,
  rollCall,
  rollCallKey,
  spawnToy,
  stepWorld,
  StepInputs,
  World,
} from "../companion/playfield";
// 註冊 builtin adapter meta：角色專屬欄位的驗證要問目標角色的 adapter 宣告了什麼。
import "../character/adapters";
import {
  exportCompanionSettings,
  parseCompanionSettingsImport,
} from "../companion/settingsTransfer";
import { DesktopPrefs } from "../desktop";

const baseInputs = (over?: Partial<StepInputs>): StepInputs => ({
  nowMs: 1_000_000,
  dtMs: 16,
  ambient: true,
  frozen: false,
  quiet: false,
  reducedMotion: false,
  playEnabled: true,
  cursorPlayEnabled: true,
  deskMoveEnabled: true,
  pointer: null,
  ...over,
});

function run(world: World, inputs: StepInputs, steps: number, rng: () => number) {
  let w = world;
  const events: ReturnType<typeof stepWorld>["events"] = [];
  for (let i = 0; i < steps; i++) {
    const r = stepWorld(w, { ...inputs, nowMs: inputs.nowMs + i * inputs.dtMs }, rng);
    w = r.world;
    events.push(...r.events);
  }
  return { world: w, events };
}

describe("玩具物理", () => {
  it("投擲後受重力、落地反彈、最後停在地面", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    const id = w.toys[0].id;
    const g = grabToyAt(w, w.toys[0].x, w.toys[0].y);
    expect(g.toyId).toBe(id);
    w = releaseToy(g.world, id, 200, -150, 1_000_000);
    const { world: after } = run(w, baseInputs({ playEnabled: false }), 400, () => 0.99);
    const toy = after.toys[0];
    expect(toy.y).toBeGreaterThan(140); // 已落地
    expect(Math.abs(toy.vx)).toBeLessThan(8); // 摩擦停下
    expect(toy.x).toBeGreaterThan(w.toys[0].x); // 有被丟出去
  });

  it("牆壁反彈：玩具不會離開遊玩場", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "paper", 1_000_000);
    const id = w.toys[0].id;
    const g = grabToyAt(w, w.toys[0].x, w.toys[0].y);
    w = releaseToy(g.world, id, -400, -50, 1_000_000);
    const { world: after } = run(w, baseInputs({ playEnabled: false }), 300, () => 0.99);
    expect(after.toys[0].x).toBeGreaterThanOrEqual(6);
    expect(after.toys[0].x).toBeLessThanOrEqual(314);
  });

  it("生命週期：過期玩具被收走並發出事件", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 0);
    const { world: after, events } = run(
      w,
      baseInputs({ nowMs: 200_000, playEnabled: false }),
      2,
      () => 0.99
    );
    expect(after.toys).toHaveLength(0);
    expect(events.some((e) => e.type === "toy-expired")).toBe(true);
  });

  it("frozen（緊急/離線/暫停）：一切凍結，位置不變", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    const g = grabToyAt(w, w.toys[0].x, w.toys[0].y);
    w = releaseToy(g.world, w.toys[0].id, 300, -100, 1_000_000);
    const before = w.toys[0];
    const { world: after } = run(w, baseInputs({ frozen: true }), 100, () => 0.5);
    expect(after.toys[0].x).toBe(before.x);
    expect(after.toys[0].y).toBe(before.y);
  });

  it("Reduced Motion：玩具不彈跳直接安放地面；角色不追", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    const { world: after } = run(w, baseInputs({ reducedMotion: true }), 60, () => 0.0);
    expect(after.toys[0].y).toBeCloseTo(after.ground - 6);
    expect(after.char.mode).toBe("free");
  });

  it("玩具上限 4、光點/逗貓棒單一實例", () => {
    let w = createWorld(320, 170);
    for (const k of ["yarn", "paper", "plane", "yarn", "paper"] as const) {
      w = spawnToy(w, k, 0);
    }
    expect(w.toys.length).toBe(4);
    w = clearToys(w);
    w = spawnToy(w, "light", 0);
    w = spawnToy(w, "light", 0);
    expect(w.toys.filter((t) => t.kind === "light")).toHaveLength(1);
  });
});

describe("角色遊玩決策", () => {
  it("追逐 → 撲抓 → 叼住/帶回或拒絕歸還（rng 驅動）", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    // 玩具放遠一點。
    w = { ...w, toys: [{ ...w.toys[0], x: 260, y: w.ground - 6, interest: 1 }] };
    // rng=0：free→chase 觸發（<0.5）、撲抓成功（<0.75）、想獨占（<0.3）。
    const { world: after, events } = run(w, baseInputs(), 900, () => 0);
    const grabbed = events.some((e) => e.type === "toy-grabbed");
    expect(grabbed).toBe(true);
    // rng=0 →撲抓成功後 refuse 分支。
    expect(events.some((e) => e.type === "expression" && e.id === "keep-ball")).toBe(true);
    // refuse 5 秒後放下。
    expect(after.char.mode === "refuse" || events.some((e) => e.type === "toy-refused")).toBe(
      true
    );
  });

  it("撲空：抓取失敗播 pounce-miss 並冷卻玩具", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    w = { ...w, toys: [{ ...w.toys[0], x: 200, y: w.ground - 6, interest: 1 }] };
    // rng 序列：追逐觸發（0）→ 撲抓判定 0.9（>0.75 失敗）。
    let calls = 0;
    const rng = () => {
      calls += 1;
      return calls <= 1 ? 0 : 0.9;
    };
    const { events } = run(w, baseInputs(), 900, rng);
    expect(events.some((e) => e.type === "expression" && e.id === "pounce-miss")).toBe(true);
    expect(events.some((e) => e.type === "toy-grabbed")).toBe(false);
  });

  it("不 ambient（machine 有真實事件）：不玩、叼著的也放下", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "yarn", 1_000_000);
    w = {
      ...w,
      toys: [{ ...w.toys[0], grabbed: "character" as const }],
      char: { ...w.char, mode: "return" as const, carryToy: w.toys[0].id },
    };
    const { world: after } = run(w, baseInputs({ ambient: false }), 2, () => 0);
    expect(after.char.mode).toBe("free");
    expect(after.toys[0].grabbed).toBeNull();
  });

  it("光點永遠抓不到（撲空）", () => {
    let w = createWorld(320, 170);
    w = spawnToy(w, "light", 1_000_000);
    const inputs = baseInputs({ pointer: { x: 200, y: 100, active: true } });
    const { events } = run(w, inputs, 1200, () => 0);
    expect(events.some((e) => e.type === "toy-grabbed")).toBe(false);
    expect(events.some((e) => e.type === "expression" && e.id === "pounce-miss")).toBe(true);
  });
});

describe("使魔與 Roll Call", () => {
  it("使魔會互相注意/打招呼；Roll Call 用人話", () => {
    let w = createWorld(320, 170);
    w = {
      ...w,
      familiars: [
        { id: "a", name: "小雪", palette: "maid-classic", x: 60, vx: 0, facing: 1, state: "idle", stateUntil: 0, greetWith: null },
        { id: "b", name: "小炭", palette: "maid-dusk", x: 90, vx: 0, facing: 1, state: "idle", stateUntil: 0, greetWith: null },
      ],
    };
    // rng 落在 greet 區間（0.6..0.72）。
    // run-2 companion-gameplay-002：這裡原本用 playEnabled:false 避開 stepChar 的散步抽樣——
    // 等於釘住「關掉玩耍使魔照樣打招呼」。使魔跟主角遵守同一套閘門；改用 deskMoveEnabled:false。
    const { world: after } = run(w, baseInputs({ deskMoveEnabled: false }), 2, () => 0.65);
    expect(after.familiars.some((f) => f.state === "greet")).toBe(true);
    const roll = rollCall(after, "小樞", null);
    expect(roll[0].name).toBe("小樞");
    expect(roll).toHaveLength(3);
    for (const r of roll) {
      expect(r.activity.length).toBeGreaterThan(0);
      expect(r.activity.length).toBeLessThanOrEqual(32);
    }
  });

  it("machine 狀態優先於遊玩描述", () => {
    const w = createWorld(320, 170);
    const roll = rollCall(w, "小樞", "在等你確認");
    expect(roll[0].activity).toBe("在等你確認");
  });
});

describe("角色設定匯出／匯入", () => {
  const prefs = {
    companionName: "小樞",
    companionPack: "shu-maid",
    companionPersona: "persona-shu",
    companionExpressiveness: "natural",
    companionScene: "nest",
    companionPlay: true,
    companionCursorPlay: false,
    companionApproach: true,
    companionDeskMove: true,
    companionFamiliars: [{ id: "fam-1", name: "小雪", palette: "maid-classic" }],
  } as unknown as DesktopPrefs;

  it("round-trip：匯出 → 匯入得到相同偏好；不含權限/位置/token", () => {
    const exported = exportCompanionSettings(prefs);
    expect(exported.kind).toBe("companion-settings");
    expect(JSON.stringify(exported)).not.toContain("token");
    expect(JSON.stringify(exported)).not.toContain("Position");
    const imported = parseCompanionSettingsImport(JSON.parse(JSON.stringify(exported)), { entrypointFor });
    expect(imported.companionScene).toBe("nest");
    expect(imported.companionCursorPlay).toBe(false);
    expect(imported.companionFamiliars).toHaveLength(1);
  });

  /** 角色 → entrypoint（正式路徑由角色頁的 catalog 提供）：角色專屬欄位只用它的 adapter 驗證。 */
  const entrypointFor = (id: string): string | null => (id === "shu-maid" ? "shu-rig" : null);

  it("匯入驗證：壞 kind／未知 pack／過多使魔／非法配色都拒絕", () => {
    expect(() => parseCompanionSettingsImport({ kind: "evil" })).toThrow();
    expect(() =>
      parseCompanionSettingsImport({
        kind: "companion-settings",
        schemaVersion: 1,
        companionPack: "evil-pack",
      })
    ).toThrow();
    expect(() =>
      parseCompanionSettingsImport({
        kind: "companion-settings",
        schemaVersion: 1,
        companionFamiliars: [{}, {}, {}, {}],
      })
    ).toThrow();
    expect(() =>
      parseCompanionSettingsImport({
        kind: "companion-settings",
        schemaVersion: 1,
        companionPack: "shu-maid",
        companionFamiliars: [{ id: "a", name: "x", palette: "neon" }],
      }, { entrypointFor })
    ).toThrow();
    // 未知欄位被丟棄而不是報錯。
    const ok = parseCompanionSettingsImport({
      kind: "companion-settings",
      schemaVersion: 1,
      companionScene: "night",
      totallyUnknown: 123,
    });
    expect(ok.companionScene).toBe("night");
    expect("totallyUnknown" in ok).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// v0.5 第 6 種玩具：可拖曳的小物件（trinket）。
// 角色只好奇地靠近看/嗅，不追、不叼，偶爾用尾巴推一下。
// ---------------------------------------------------------------------------

describe("玩具：小物件（trinket）", () => {
  const seat = (over?: Partial<StepInputs>) => baseInputs({ dtMs: 16, ...over });

  it("有重力與碰撞：放著會落到地面並停下", () => {
    let w = spawnToy(createWorld(320, 170), "trinket", 1_000_000);
    expect(w.toys[0].kind).toBe("trinket");
    const started = w.toys[0].y;
    const r = run(w, seat(), 120, () => 0.99);
    w = r.world;
    expect(w.toys[0].y).toBeGreaterThan(started);
    expect(w.toys[0].y).toBeCloseTo(w.ground - 6, 1);
    // 比毛球重：落地後幾乎不彈。
    expect(Math.abs(w.toys[0].vy)).toBeLessThan(30);
  });

  it("角色不追也不叼小物件（永遠不會進 chase/pounce/carry）", () => {
    let w = spawnToy(createWorld(320, 170), "trinket", 1_000_000);
    const modes = new Set<string>();
    const base = seat();
    for (let i = 0; i < 400; i++) {
      const r = stepWorld(w, { ...base, nowMs: base.nowMs + i * 16 }, () => 0);
      w = r.world;
      modes.add(w.char.mode);
    }
    expect(modes.has("chase")).toBe(false);
    expect(modes.has("pounce")).toBe(false);
    expect(w.char.carryToy).toBeNull();
    // 但她確實會好奇地靠近。
    expect(modes.has("sniff")).toBe(true);
  });

  it("靠近後用尾巴推一下（不帶走），並回到自由狀態", () => {
    let w = spawnToy(createWorld(320, 170), "trinket", 1_000_000);
    const base = seat();
    const events: ReturnType<typeof stepWorld>["events"] = [];
    for (let i = 0; i < 400; i++) {
      const r = stepWorld(w, { ...base, nowMs: base.nowMs + i * 16 }, () => 0);
      w = r.world;
      events.push(...r.events);
    }
    expect(events.some((e) => e.type === "toy-pushed")).toBe(true);
    expect(events.some((e) => e.type === "expression" && e.id === "curious")).toBe(true);
    expect(events.some((e) => e.type === "toy-grabbed")).toBe(false);
    expect(w.toys[0].grabbed).toBeNull();
  });

  it("Roll Call 的列表 key 帶序號：同名的成員不會撞在一起", () => {
    let w = createWorld(320, 170);
    w = {
      ...w,
      familiars: [
        { id: "a", name: "小灰", palette: "maid-classic", x: 20, vx: 0, facing: 1, state: "idle", stateUntil: 0, greetWith: null },
        { id: "b", name: "小灰", palette: "maid-dusk", x: 60, vx: 0, facing: 1, state: "idle", stateUntil: 0, greetWith: null },
      ],
    };
    const rows = rollCall(w, "小灰", null);
    expect(rows).toHaveLength(3);
    const keys = rows.map((r, i) => rollCallKey(i, r.name));
    expect(new Set(keys).size).toBe(3);
  });

  it("Roll Call 用人話描述「在研究一個小東西」", () => {
    const w = spawnToy(createWorld(320, 170), "trinket", 1_000_000);
    const sniffing = { ...w, char: { ...w.char, mode: "sniff" as const } };
    expect(rollCall(sniffing, "小樞", null)[0].activity).toBe("在研究一個小東西");
  });
});
