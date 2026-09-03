// Playfield（spec §5）：玩具、輕量 2D 物理與遊戲決策。
//
// 純模組：stepWorld(world, inputs, rng) → { world, events }。
// - 不讀系統游標：pointer 是「遊玩場 canvas 內」的區域座標，由視窗內
//   pointer 事件餵入，永不送 runtime／AI、不持久化。
// - 玩具資料模型：位置/速度/重力/碰撞/抓取狀態/擁有者/興趣值/冷卻/生命週期。
// - 角色決策（追逐/撲抓/帶回/拒絕歸還）為 hazard 抽樣＋冷卻，非固定週期。
// - frozen（緊急/離線/暫停）：一切凍結；reduced motion：無自主移動與
//   物理彈跳（玩具仍可拖放，直接落地）。

export type ToyKind = "yarn" | "paper" | "plane" | "light" | "wand" | "trinket";

/** 第 6 種玩具「小物件」：可拖曳、有重力與碰撞，但角色不追不叼——
 *  她只會好奇地靠近嗅一嗅，偶爾用尾巴把它推一下。 */
export const TRINKET: ToyKind = "trinket";

export interface Toy {
  id: number;
  kind: ToyKind;
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** 抓取狀態＋擁有者。 */
  grabbed: "player" | "character" | null;
  /** 角色興趣 0..1（新丟出高、玩過衰減）。 */
  interest: number;
  /** 玩過後冷卻（不再追）。 */
  cooldownUntil: number;
  /** 生命週期（過期自動收走）。 */
  expiresAt: number;
  /** 靜止累計 ms（光點/逗貓棒不適用）。 */
  restMs: number;
}

export type CharPlayMode =
  | "free"
  | "stroll"
  | "chase"
  | "pounce"
  | "carry"
  | "return"
  | "refuse"
  /** 靠近小物件、好奇地看/嗅（不追、不叼）。 */
  | "sniff"
  /** 主動走向使魔打招呼（互相注意是雙向的，spec §5.1）。 */
  | "greet-familiar";

export interface CharPlay {
  x: number;
  vx: number;
  facing: 1 | -1;
  mode: CharPlayMode;
  modeUntil: number;
  targetToy: number | null;
  carryToy: number | null;
  /** 撲抓落點（pounce 開始時鎖定）。 */
  pounceX: number;
  /** 她主動要去打招呼的使魔 id（greet-familiar 模式；null＝沒有）。 */
  targetFamiliar: string | null;
  /** 正在回看的使魔 id（有使魔向她打招呼時；null＝沒有）。 */
  attendTo: string | null;
  /** 回看到什麼時候（ms）。 */
  attendUntil: number;
  /** 回一顆愛心到什麼時候（ms；0＝不回）。 */
  greetBackUntil: number;
}

export type FamiliarState = "idle" | "walk" | "sleep" | "greet" | "chase";

export interface Familiar {
  id: string;
  name: string;
  palette: string;
  x: number;
  vx: number;
  facing: 1 | -1;
  state: FamiliarState;
  stateUntil: number;
  /** greet 對象（另一隻使魔或 "char"）。 */
  greetWith: string | null;
}

export interface World {
  w: number;
  h: number;
  /** 地面 y（腳底）。 */
  ground: number;
  char: CharPlay;
  toys: Toy[];
  familiars: Familiar[];
  nextToyId: number;
}

export interface StepInputs {
  nowMs: number;
  dtMs: number;
  /** machine ambient（idle 且無 transient）——非 ambient 角色不玩。 */
  ambient: boolean;
  /** emergency/offline/paused：全場凍結。 */
  frozen: boolean;
  quiet: boolean;
  reducedMotion: boolean;
  playEnabled: boolean;
  cursorPlayEnabled: boolean;
  deskMoveEnabled: boolean;
  /** canvas 內指標（光點/逗貓棒目標；null=不在場內）。 */
  pointer: { x: number; y: number; active: boolean } | null;
  // ---- 個性 tuning（personality.ts；省略＝中性 1.0）----
  /** 散步速度倍率。 */
  speedScale?: number;
  /** 追逐速度倍率。 */
  chaseSpeedScale?: number;
  /** 靠近目標時停下的距離（邏輯 px）。 */
  approachDistance?: number;
  /** 慵懶：決定要動之後慢半拍才起身（ms）。 */
  riseDelayMs?: number;
}

export type WorldEvent =
  | { type: "expression"; id: string; durationMs: number }
  /** 有使魔向主角打招呼（主角回看／偶爾回一顆愛心）。 */
  | { type: "greeted-by"; id: string }
  /** 主角主動走過去跟某隻使魔打招呼（雙向注意的另一半）。 */
  | { type: "greeted-familiar"; id: string }
  /** 被追的使魔的反應（逃跑或回頭）。 */
  | { type: "familiar-fled"; id: string; by: string }
  | { type: "familiar-looked-back"; id: string; by: string }
  | { type: "toy-expired"; id: number }
  | { type: "toy-grabbed"; id: number }
  | { type: "toy-returned"; id: number }
  | { type: "toy-refused"; id: number }
  | { type: "toy-pushed"; id: number };

const GRAVITY = 560; // px/s²
const BOUNCE = 0.52;
const AIR_FRICTION = 0.999;
const GROUND_FRICTION = 0.86;
const CHAR_HALF = 20; // 角色碰撞半寬（邏輯 px）
const CHAR_SPEED = 46; // px/s 散步
const CHASE_SPEED = 120; // px/s 追逐
const TOY_TTL_MS = 150_000;
const MAX_TOYS = 4;
/** 小物件比毛球重：彈得少、停得快。 */
const TRINKET_BOUNCE = 0.18;
/** 尾巴推一下的力道（px/s）。 */
const TAIL_PUSH_VX = 78;
const TAIL_PUSH_VY = -48;
/** 主動打招呼的冷卻（從上一次冒愛心算起）：不會一直黏著同一隻。 */
const GREET_FAMILIAR_COOLDOWN_MS = 20_000;

export function createWorld(w: number, h: number): World {
  return {
    w,
    h,
    ground: h - 6,
    char: {
      x: w / 2,
      vx: 0,
      facing: 1,
      mode: "free",
      modeUntil: 0,
      targetToy: null,
      carryToy: null,
      pounceX: 0,
      targetFamiliar: null,
      attendTo: null,
      attendUntil: 0,
      greetBackUntil: 0,
    },
    toys: [],
    familiars: [],
    nextToyId: 1,
  };
}

/** 產生玩具（上限 MAX_TOYS；光點/逗貓棒單一實例）。 */
export function spawnToy(world: World, kind: ToyKind, nowMs: number): World {
  if (kind === "light" || kind === "wand") {
    // 單一實例：重生就替換。
    world = { ...world, toys: world.toys.filter((t) => t.kind !== kind) };
  }
  if (world.toys.length >= MAX_TOYS) return world;
  const toy: Toy = {
    id: world.nextToyId,
    kind,
    x: world.w * 0.25 + (world.nextToyId % 3) * 20,
    y: kind === "light" || kind === "wand" ? world.ground - 40 : world.ground - 60,
    vx: 0,
    vy: 0,
    grabbed: null,
    interest: 0.9,
    cooldownUntil: 0,
    expiresAt: nowMs + TOY_TTL_MS,
    restMs: 0,
  };
  return { ...world, toys: [...world.toys, toy], nextToyId: world.nextToyId + 1 };
}

export function clearToys(world: World): World {
  return {
    ...world,
    toys: [],
    char: { ...world.char, mode: "free", targetToy: null, carryToy: null },
  };
}

/** 跟著游標走的玩具（光點／逗貓棒）：不可被抓、也不算進互動框——它們永遠在游標底下。 */
export function isCursorToy(kind: ToyKind): boolean {
  return kind === "light" || kind === "wand";
}

/** 玩家抓住/拖曳/丟出玩具。光點／逗貓棒本來就跟著游標，抓它只會把它「抓停」——不可抓。 */
export function grabToyAt(world: World, x: number, y: number): { world: World; toyId: number | null } {
  for (const t of world.toys) {
    if (t.grabbed) continue;
    if (isCursorToy(t.kind)) continue;
    const r = t.kind === "wand" ? 16 : 12;
    if (Math.abs(t.x - x) <= r && Math.abs(t.y - y) <= r + 4) {
      return {
        world: updateToy(world, t.id, { grabbed: "player", vx: 0, vy: 0 }),
        toyId: t.id,
      };
    }
  }
  return { world, toyId: null };
}

export function dragToy(world: World, toyId: number, x: number, y: number, vx: number, vy: number): World {
  const t = world.toys.find((toy) => toy.id === toyId);
  if (!t || t.grabbed !== "player") return world;
  return updateToy(world, toyId, {
    x: clampN(x, 6, world.w - 6),
    y: clampN(y, 6, world.ground),
    vx,
    vy,
  });
}

/** 放開＝依拖曳速度投擲（方向/速度/落點由物理決定）。 */
export function releaseToy(world: World, toyId: number, vx: number, vy: number, nowMs: number): World {
  const t = world.toys.find((toy) => toy.id === toyId);
  if (!t || t.grabbed !== "player") return world;
  return updateToy(world, toyId, {
    grabbed: null,
    vx: clampN(vx, -420, 420),
    vy: clampN(vy, -420, 300),
    interest: 1,
    cooldownUntil: 0,
    expiresAt: nowMs + TOY_TTL_MS,
    restMs: 0,
  });
}

function updateToy(world: World, id: number, patch: Partial<Toy>): World {
  return { ...world, toys: world.toys.map((t) => (t.id === id ? { ...t, ...patch } : t)) };
}

const clampN = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));

/** 主步進。 */
export function stepWorld(
  world: World,
  inputs: StepInputs,
  rng: () => number
): { world: World; events: WorldEvent[] } {
  const events: WorldEvent[] = [];
  if (inputs.frozen) return { world, events };
  const dt = clampN(inputs.dtMs, 0, 100) / 1000;
  const now = inputs.nowMs;
  let w = world;

  // ---- 玩具物理 ----
  const toys: Toy[] = [];
  for (const t of w.toys) {
    if (now > t.expiresAt) {
      events.push({ type: "toy-expired", id: t.id });
      if (w.char.targetToy === t.id || w.char.carryToy === t.id) {
        w = { ...w, char: { ...w.char, mode: "free", targetToy: null, carryToy: null } };
      }
      continue;
    }
    let toy = { ...t };
    if (toy.grabbed === "player") {
      // 玩家手上：跟著 pointer（由 dragToy 更新座標）。
      toys.push(toy);
      continue;
    }
    if (toy.grabbed === "character") {
      // 角色叼著：貼在角色前爪。
      toy.x = w.char.x + w.char.facing * 14;
      toy.y = w.ground - 26;
      toy.vx = 0;
      toy.vy = 0;
      toys.push(toy);
      continue;
    }
    if (toy.kind === "light" || toy.kind === "wand") {
      // 游標玩具：跟隨 pointer（不在場內就熄滅/垂下）。
      if (inputs.pointer && inputs.cursorPlayEnabled) {
        const target = inputs.pointer;
        toy.x += (target.x - toy.x) * Math.min(1, dt * 14);
        toy.y += (target.y - toy.y) * Math.min(1, dt * 14);
        toy.interest = 1;
      } else {
        toy.interest = Math.max(0, toy.interest - dt * 0.4);
        if (toy.kind === "wand") {
          toy.y += (w.ground - 24 - toy.y) * Math.min(1, dt * 6);
        }
      }
      toys.push(toy);
      continue;
    }
    if (inputs.reducedMotion) {
      // Reduced Motion：不彈跳——直接安放地面。
      toy.y = w.ground - 6;
      toy.vx = 0;
      toy.vy = 0;
      toys.push(toy);
      continue;
    }
    // 一般物理。紙飛機滑翔：重力弱、水平阻力低；小物件較重、幾乎不彈。
    const g = toy.kind === "plane" ? GRAVITY * 0.25 : GRAVITY;
    const bounce = toy.kind === "trinket" ? TRINKET_BOUNCE : BOUNCE;
    toy.vy += g * dt;
    toy.x += toy.vx * dt;
    toy.y += toy.vy * dt;
    toy.vx *= toy.kind === "plane" ? 0.9995 : AIR_FRICTION;
    // 牆反彈。
    if (toy.x < 6) {
      toy.x = 6;
      toy.vx = Math.abs(toy.vx) * bounce;
    } else if (toy.x > w.w - 6) {
      toy.x = w.w - 6;
      toy.vx = -Math.abs(toy.vx) * bounce;
    }
    // 地面。
    const floor = w.ground - 6;
    if (toy.y >= floor) {
      toy.y = floor;
      if (Math.abs(toy.vy) > 30 && toy.kind !== "plane") {
        toy.vy = -Math.abs(toy.vy) * bounce;
      } else {
        toy.vy = 0;
      }
      toy.vx *= GROUND_FRICTION;
    }
    const moving = Math.abs(toy.vx) > 4 || Math.abs(toy.vy) > 4;
    toy.restMs = moving ? 0 : toy.restMs + inputs.dtMs;
    toy.interest = clampN(toy.interest - dt * (moving ? 0.01 : 0.06), 0, 1);
    toys.push(toy);
  }
  w = { ...w, toys };

  // ---- 角色遊玩決策 ----
  w = stepChar(w, inputs, rng, events, dt, now);

  // ---- 使魔 ----
  w = stepFamiliars(w, inputs, rng, dt, now, events);

  return { world: w, events };
}

function stepChar(
  w: World,
  inputs: StepInputs,
  rng: () => number,
  events: WorldEvent[],
  dt: number,
  now: number
): World {
  const c = { ...w.char };
  const canPlay =
    inputs.ambient && inputs.playEnabled && !inputs.quiet && !inputs.reducedMotion;
  // 有使魔向她打招呼且她沒在忙：回看那一側（純呈現，不影響遊玩決策）。
  // 只在真的可以玩的時候：被擋下／失敗／未知／勿擾時不轉身回看（§14 不把真相狀態演成賣萌）。
  if (canPlay && c.attendTo !== null && c.mode === "free" && now <= c.attendUntil) {
    const f = w.familiars.find((x) => x.id === c.attendTo);
    if (f) c.facing = f.x >= c.x ? 1 : -1;
  }
  // 個性 tuning（省略＝中性）。
  const speed = CHAR_SPEED * (inputs.speedScale ?? 1);
  const chaseSpeed = CHASE_SPEED * (inputs.chaseSpeedScale ?? 1);
  const approach = inputs.approachDistance ?? 20;
  const riseDelay = Math.max(0, inputs.riseDelayMs ?? 0);

  if (!canPlay) {
    // 不玩：叼著的玩具放下、回 free、站住。
    if (c.carryToy != null) {
      w = updateToy(w, c.carryToy, { grabbed: null });
      c.carryToy = null;
    }
    c.mode = "free";
    c.targetToy = null;
    c.targetFamiliar = null;
    c.vx = 0;
    return { ...w, char: c };
  }

  const chaseable = w.toys.filter(
    (t) =>
      !t.grabbed &&
      // 小物件不追不叼：只會被好奇地靠近（sniff）。
      t.kind !== "trinket" &&
      t.cooldownUntil <= now &&
      t.interest > 0.25 &&
      (t.kind !== "light" && t.kind !== "wand"
        ? true
        : inputs.pointer !== null && inputs.cursorPlayEnabled)
  );
  const sniffable = w.toys.filter(
    (t) => t.kind === "trinket" && !t.grabbed && t.cooldownUntil <= now && t.interest > 0.2
  );

  switch (c.mode) {
    case "free": {
      // 找最有趣的玩具（hazard 率以秒為單位、依 dt 縮放——每幀呼叫也不會爆量）。
      const best = chaseable.sort((a, b) => b.interest - a.interest)[0];
      if (best && rng() < Math.min(0.5, 1.5 * dt)) {
        c.mode = "chase";
        c.targetToy = best.id;
        // 慵懶：決定要動之後慢半拍才真的起身。
        c.modeUntil = now + riseDelay;
        break;
      }
      // 小物件：不追不叼，只是好奇地靠近看看。
      const curiousToy = sniffable.sort((a, b) => b.interest - a.interest)[0];
      if (curiousToy && rng() < Math.min(0.3, 0.6 * dt)) {
        c.mode = "sniff";
        c.targetToy = curiousToy.id;
        c.modeUntil = now + riseDelay + 4_500;
        events.push({ type: "expression", id: "curious", durationMs: 1_600 });
        break;
      }
      // 主動去跟使魔打招呼（spec §5.1「互相注意」的另一半：以前只有使魔會
      // 過來打招呼、她永遠只是原地回看，對抗審查 companion-gameplay-035）。
      // 睡著的不吵、剛打完招呼的有冷卻；平均約 25 秒才會起一次意。
      const awake = w.familiars.filter((f) => f.state !== "sleep" && f.id !== c.attendTo);
      const friend = nearestFamiliar(awake, c.x);
      if (friend && now > c.greetBackUntil + GREET_FAMILIAR_COOLDOWN_MS && rng() < 0.04 * dt) {
        c.mode = "greet-familiar";
        c.targetFamiliar = friend.id;
        c.modeUntil = now + riseDelay + 6_000;
        break;
      }
      // 偶爾散步（桌面移動開啟時；平均約 20 秒一次）。
      if (inputs.deskMoveEnabled && rng() < 0.05 * dt) {
        c.mode = "stroll";
        c.pounceX = 24 + rng() * (w.w - 48);
        c.modeUntil = now + 8_000;
      }
      c.vx = 0;
      break;
    }
    case "greet-familiar": {
      const friend = w.familiars.find((f) => f.id === c.targetFamiliar);
      if (!friend || now > c.modeUntil) {
        c.mode = "free";
        c.targetFamiliar = null;
        c.vx = 0;
        break;
      }
      const dx = friend.x - c.x;
      c.facing = dx >= 0 ? 1 : -1;
      if (now < c.modeUntil - 6_000) {
        // 起身延遲（慵懶）：決定了，但還沒動。
        c.vx = 0;
        break;
      }
      if (Math.abs(dx) > approach + 12) {
        c.vx = Math.sign(dx) * speed;
        c.x += c.vx * dt;
        break;
      }
      // 到了：停下、面向牠、冒一顆愛心，對方也回過頭來（雙向）。
      c.vx = 0;
      c.attendTo = friend.id;
      c.attendUntil = now + 2_500;
      c.greetBackUntil = now + 1_800;
      w = {
        ...w,
        familiars: w.familiars.map((f) =>
          f.id !== friend.id || f.state === "sleep"
            ? f
            : {
                ...f,
                state: "greet" as FamiliarState,
                greetWith: "char",
                facing: (c.x >= f.x ? 1 : -1) as 1 | -1,
                vx: 0,
                stateUntil: now + 2_500,
              }
        ),
      };
      events.push({ type: "greeted-familiar", id: friend.id });
      c.mode = "free";
      c.targetFamiliar = null;
      break;
    }
    case "stroll": {
      const dx = c.pounceX - c.x;
      if (Math.abs(dx) < 4 || now > c.modeUntil) {
        c.mode = "free";
        c.vx = 0;
      } else {
        c.vx = Math.sign(dx) * speed;
        c.facing = dx >= 0 ? 1 : -1;
        c.x += c.vx * dt;
      }
      break;
    }
    case "sniff": {
      // 靠近小物件、看一看／嗅一嗅，偶爾用尾巴推一下——絕不叼走。
      const toy = w.toys.find((t) => t.id === c.targetToy);
      if (!toy || toy.kind !== "trinket" || toy.grabbed === "player" || now > c.modeUntil) {
        c.mode = "free";
        c.targetToy = null;
        c.vx = 0;
        break;
      }
      const dx = toy.x - c.x;
      c.facing = dx >= 0 ? 1 : -1;
      if (now < c.modeUntil - 4_500) {
        // 起身延遲（慵懶）：還沒真的動。
        c.vx = 0;
        break;
      }
      if (Math.abs(dx) > approach) {
        c.vx = Math.sign(dx) * speed;
        c.x += c.vx * dt;
        break;
      }
      c.vx = 0;
      // 到了：偶爾用尾巴把它推開一點（hazard 抽樣，不是固定週期）。
      if (rng() < Math.min(0.4, 1.2 * dt)) {
        w = updateToy(w, toy.id, {
          vx: c.facing * TAIL_PUSH_VX,
          vy: TAIL_PUSH_VY,
          interest: Math.max(0, toy.interest - 0.25),
          cooldownUntil: now + 5_000,
        });
        events.push({ type: "toy-pushed", id: toy.id });
        c.mode = "free";
        c.targetToy = null;
      }
      break;
    }
    case "chase": {
      const toy = w.toys.find((t) => t.id === c.targetToy);
      if (!toy || toy.grabbed === "player" || toy.interest <= 0.15) {
        c.mode = "free";
        c.targetToy = null;
        break;
      }
      if (now < c.modeUntil) {
        // 起身延遲（慵懶）：決定追了，但還沒動。
        c.vx = 0;
        break;
      }
      const dx = toy.x - c.x;
      c.facing = dx >= 0 ? 1 : -1;
      // 地面玩具要夠近才撲；游標玩具（光點/逗貓棒）可躍撲（不受高度限制）。
      const pounceable =
        toy.kind === "light" || toy.kind === "wand" ? true : toy.y > w.ground - 40;
      if (Math.abs(dx) < approach && pounceable) {
        // 夠近：預備撲抓。
        c.mode = "pounce";
        c.modeUntil = now + 450; // anticipation + 撲
        c.pounceX = toy.x;
        c.vx = 0;
      } else {
        c.vx = Math.sign(dx) * chaseSpeed;
        c.x += c.vx * dt;
      }
      break;
    }
    case "pounce": {
      if (now < c.modeUntil) break; // 撲抓演出中
      const toy = w.toys.find((t) => t.id === c.targetToy);
      const stillThere = toy && !toy.grabbed && Math.abs(toy.x - c.x) < 26;
      const catchable = stillThere && toy!.kind !== "light"; // 光點永遠抓不到
      if (catchable && rng() < 0.75) {
        if (toy!.kind === "wand") {
          // 逗貓棒：拍一下（不叼走）。
          w = updateToy(w, toy!.id, { vx: c.facing * 90, vy: -60 });
          events.push({ type: "expression", id: "hold-ball", durationMs: 900 });
          c.mode = "free";
          c.targetToy = null;
        } else {
          w = updateToy(w, toy!.id, { grabbed: "character" });
          c.carryToy = toy!.id;
          c.targetToy = null;
          events.push({ type: "toy-grabbed", id: toy!.id });
          events.push({ type: "expression", id: "hold-ball", durationMs: 1_200 });
          // 帶回或想獨占。
          if (rng() < 0.3) {
            c.mode = "refuse";
            c.modeUntil = now + 5_000;
            events.push({ type: "expression", id: "keep-ball", durationMs: 4_000 });
          } else {
            c.mode = "return";
            c.pounceX = inputs.pointer ? clampN(inputs.pointer.x, 24, w.w - 24) : w.w / 2;
          }
        }
      } else {
        // 撲空。
        events.push({ type: "expression", id: "pounce-miss", durationMs: 2_200 });
        c.mode = "free";
        c.targetToy = null;
        if (toy) w = updateToy(w, toy.id, { cooldownUntil: now + 6_000 });
      }
      break;
    }
    case "return": {
      const dx = c.pounceX - c.x;
      c.facing = dx >= 0 ? 1 : -1;
      if (Math.abs(dx) < 10) {
        // 放下：交還玩家。
        if (c.carryToy != null) {
          w = updateToy(w, c.carryToy, {
            grabbed: null,
            interest: 0.2,
            cooldownUntil: now + 12_000,
          });
          events.push({ type: "toy-returned", id: c.carryToy });
        }
        c.carryToy = null;
        c.mode = "free";
        c.vx = 0;
      } else {
        c.vx = Math.sign(dx) * speed * 1.3;
        c.x += c.vx * dt;
      }
      break;
    }
    case "refuse": {
      // 抱著不還：慢慢走遠、時間到才放下。
      if (now > c.modeUntil) {
        if (c.carryToy != null) {
          w = updateToy(w, c.carryToy, {
            grabbed: null,
            interest: 0.1,
            cooldownUntil: now + 15_000,
          });
          events.push({ type: "toy-refused", id: c.carryToy });
        }
        c.carryToy = null;
        c.mode = "free";
        c.vx = 0;
      } else {
        const away = c.x < w.w / 2 ? 1 : -1;
        c.vx = away * speed * 0.6;
        c.facing = away as 1 | -1;
        c.x += c.vx * dt;
      }
      break;
    }
  }
  c.x = clampN(c.x, CHAR_HALF + 4, w.w - CHAR_HALF - 4);
  return { ...w, char: c };
}

/** 清單中離 `x` 最近的一隻（空清單回 null）。 */
export function nearestFamiliar<T extends { x: number }>(list: T[], x: number): T | null {
  let best: T | null = null;
  let bestD = Number.POSITIVE_INFINITY;
  for (const f of list) {
    const d = Math.abs(f.x - x);
    if (d < bestD) {
      bestD = d;
      best = f;
    }
  }
  return best;
}

function stepFamiliars(
  w: World,
  inputs: StepInputs,
  rng: () => number,
  dt: number,
  now: number,
  events: WorldEvent[]
): World {
  let char = w.char;
  // 回看到期就收回（凍結/reduced 時也要能收，所以放在 early return 之前）。
  if (char.attendTo !== null && now > char.attendUntil) {
    char = { ...char, attendTo: null };
  }
  // 使魔跟主角遵守同一套閘門：不 ambient（真相狀態在台上）、勿擾／安靜時段、
  // 關掉「玩耍」——都不散步、不追逐、不打招呼、不冒愛心。正在動的收回原地待著；
  // 主角的回看與回愛心也一起收掉（被擋下／失敗／未知的畫面上不能有粉紅愛心）。
  const active = inputs.ambient && inputs.playEnabled && !inputs.quiet;
  // Reduced Motion 也走同一條「收斂到靜止」的路：以前它是在狀態機推進**之前**
  // 直接 early return，使魔就永遠卡在切換當下的 state／vx 上——愛心永遠掛在頭上、
  // Roll Call 還說「在散步」（對抗審查 companion-gameplay-033）。
  if (!active || inputs.reducedMotion) {
    if (char.attendTo !== null || char.greetBackUntil !== 0) {
      char = { ...char, attendTo: null, greetBackUntil: 0 };
    }
    const settled = w.familiars.map((f) =>
      f.state === "sleep" || f.state === "idle"
        ? f.greetWith === null
          ? f
          : { ...f, greetWith: null }
        : { ...f, state: "idle" as FamiliarState, vx: 0, greetWith: null, stateUntil: now }
    );
    const changed = settled.some((f, i) => f !== w.familiars[i]);
    if (!changed && char === w.char) return w;
    return { ...w, char, familiars: changed ? settled : w.familiars };
  }
  if (w.familiars.length === 0) {
    return char === w.char ? w : { ...w, char };
  }
  const list = w.familiars.map((f) => ({ ...f }));
  for (const f of list) {
    if (now < f.stateUntil) {
      if (f.state === "walk" || f.state === "chase") {
        f.x = clampN(f.x + f.vx * dt, 12, w.w - 12);
      }
      continue;
    }
    // 換狀態（hazard 抽樣）。
    const roll = rng();
    if (roll < 0.25) {
      f.state = "sleep";
      f.stateUntil = now + 6_000 + rng() * 8_000;
      f.vx = 0;
    } else if (roll < 0.6) {
      f.state = "walk";
      const dir = rng() < 0.5 ? -1 : 1;
      f.vx = dir * (20 + rng() * 25);
      f.facing = dir as 1 | -1;
      f.stateUntil = now + 2_000 + rng() * 3_000;
    } else if (roll < 0.72) {
      // 互相注意/打招呼：找**最近**的另一隻（不是清單第一隻）或主角。
      const nearest = nearestFamiliar(
        list.filter((o) => o.id !== f.id),
        f.x
      );
      const target =
        nearest && rng() < 0.6 ? { x: nearest.x, id: nearest.id } : { x: w.char.x, id: "char" };
      f.state = "greet";
      f.greetWith = target.id;
      f.facing = target.x >= f.x ? 1 : -1;
      f.stateUntil = now + 2_500;
      f.vx = 0;
      if (target.id === "char") {
        // 主角也回看（視線/耳朵朝向她），偶爾回一顆愛心。
        char = {
          ...char,
          attendTo: f.id,
          attendUntil: now + 2_500,
          greetBackUntil: rng() < 0.35 ? now + 1_800 : char.greetBackUntil,
        };
        events.push({ type: "greeted-by", id: f.id });
      } else {
        // 對方也回看。
        const other = list.find((o) => o.id === f.greetWith);
        if (other && other.state !== "sleep") {
          other.state = "greet";
          other.greetWith = f.id;
          other.facing = f.x >= other.x ? 1 : -1;
          other.stateUntil = now + 2_500;
        }
      }
    } else if (roll < 0.8 && list.length > 1) {
      // 追逐**最近**的另一隻。
      const other = nearestFamiliar(
        list.filter((o) => o.id !== f.id),
        f.x
      )!;
      f.state = "chase";
      const dir = other.x >= f.x ? 1 : -1;
      f.vx = dir * 55;
      f.facing = dir as 1 | -1;
      f.stateUntil = now + 1_800 + rng() * 1_500;
      // 被追的一方有反應：多半跑掉，偶爾停下來回頭看。
      if (other.state !== "sleep") {
        if (rng() < 0.7) {
          const away = (other.x >= f.x ? 1 : -1) as 1 | -1;
          other.state = "walk";
          other.vx = away * 62;
          other.facing = away;
          other.stateUntil = now + 1_500;
          events.push({ type: "familiar-fled", id: other.id, by: f.id });
        } else {
          other.state = "idle";
          other.vx = 0;
          other.facing = (f.x >= other.x ? 1 : -1) as 1 | -1;
          other.stateUntil = now + 900;
          events.push({ type: "familiar-looked-back", id: other.id, by: f.id });
        }
      }
    } else {
      f.state = "idle";
      f.stateUntil = now + 2_000 + rng() * 4_000;
      f.vx = 0;
    }
  }
  return { ...w, char, familiars: list };
}

/**
 * Roll Call 列表的 React key：名字可以重複（兩隻同名使魔、使魔跟主角同名），
 * 所以 key 必須帶序號，否則 React 會把兩列當成同一列。
 */
export function rollCallKey(index: number, name: string): string {
  return `${index}-${name}`;
}

/** Roll Call：現在大家在做什麼（人類語言，不用技術術語）。 */
export function rollCall(
  world: World,
  charName: string,
  machineLabel: string | null,
  nowMs = 0,
  opts: { frozen?: boolean; reducedMotion?: boolean } = {}
): { name: string; activity: string }[] {
  // 凍結與 Reduced Motion 都是「世界沒有在動」：不能拿殘影報「在散步」。
  const still = opts.frozen === true || opts.reducedMotion === true;
  const charActivity =
    machineLabel ??
    (() => {
      // 世界沒有在步進時，char.mode／attendTo 也是殘影。
      if (still) return "停下來了";
      switch (world.char.mode) {
        case "chase":
          return "在追玩具";
        case "pounce":
          return "正要撲";
        case "carry":
        case "return":
          return "叼著玩具走回來";
        case "refuse":
          return "抱著玩具不想還";
        case "stroll":
          return "在散步";
        case "sniff":
          return "在研究一個小東西";
        case "greet-familiar":
          return "正要去跟朋友打招呼";
        default:
          return world.char.attendTo !== null && nowMs <= world.char.attendUntil
            ? "在跟使魔打招呼"
            : "在休息";
      }
    })();
  const out = [{ name: charName, activity: charActivity }];
  for (const f of world.familiars) {
    // 凍結（緊急停止／離線／暫停）：世界沒有在步進，使魔的 state 是凍結前的殘影——
    // 不能拿它報「在散步」。Reduced Motion 同理：牠們真的不動了
    // （對抗審查 companion-gameplay-033）。誠實說：跟著停下來了。
    const label = still
      ? f.state === "sleep"
        ? "在睡覺"
        : "停下來了"
      : f.state === "sleep"
        ? "在睡覺"
        : f.state === "walk"
          ? "在散步"
          : f.state === "greet"
            ? f.greetWith === "char"
              ? "在跟大家打招呼"
              : "在打招呼"
            : f.state === "chase"
              ? "在追朋友"
              : "在發呆";
    out.push({ name: f.name, activity: label });
  }
  return out;
}
