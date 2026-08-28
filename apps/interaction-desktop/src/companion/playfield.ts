// Playfield（spec §5）：玩具、輕量 2D 物理與遊戲決策。
//
// 純模組：stepWorld(world, inputs, rng) → { world, events }。
// - 不讀系統游標：pointer 是「遊玩場 canvas 內」的區域座標，由視窗內
//   pointer 事件餵入，永不送 runtime／AI、不持久化。
// - 玩具資料模型：位置/速度/重力/碰撞/抓取狀態/擁有者/興趣值/冷卻/生命週期。
// - 角色決策（追逐/撲抓/帶回/拒絕歸還）為 hazard 抽樣＋冷卻，非固定週期。
// - frozen（緊急/離線/暫停）：一切凍結；reduced motion：無自主移動與
//   物理彈跳（玩具仍可拖放，直接落地）。

export type ToyKind = "yarn" | "paper" | "plane" | "light" | "wand";

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

export type CharPlayMode = "free" | "stroll" | "chase" | "pounce" | "carry" | "return" | "refuse";

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
}

export type WorldEvent =
  | { type: "expression"; id: string; durationMs: number }
  | { type: "toy-expired"; id: number }
  | { type: "toy-grabbed"; id: number }
  | { type: "toy-returned"; id: number }
  | { type: "toy-refused"; id: number };

const GRAVITY = 560; // px/s²
const BOUNCE = 0.52;
const AIR_FRICTION = 0.999;
const GROUND_FRICTION = 0.86;
const CHAR_HALF = 20; // 角色碰撞半寬（邏輯 px）
const CHAR_SPEED = 46; // px/s 散步
const CHASE_SPEED = 120; // px/s 追逐
const TOY_TTL_MS = 150_000;
const MAX_TOYS = 4;

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

/** 玩家抓住/拖曳/丟出玩具。 */
export function grabToyAt(world: World, x: number, y: number): { world: World; toyId: number | null } {
  for (const t of world.toys) {
    if (t.grabbed) continue;
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
    // 一般物理。紙飛機滑翔：重力弱、水平阻力低。
    const g = toy.kind === "plane" ? GRAVITY * 0.25 : GRAVITY;
    toy.vy += g * dt;
    toy.x += toy.vx * dt;
    toy.y += toy.vy * dt;
    toy.vx *= toy.kind === "plane" ? 0.9995 : AIR_FRICTION;
    // 牆反彈。
    if (toy.x < 6) {
      toy.x = 6;
      toy.vx = Math.abs(toy.vx) * BOUNCE;
    } else if (toy.x > w.w - 6) {
      toy.x = w.w - 6;
      toy.vx = -Math.abs(toy.vx) * BOUNCE;
    }
    // 地面。
    const floor = w.ground - 6;
    if (toy.y >= floor) {
      toy.y = floor;
      if (Math.abs(toy.vy) > 30 && toy.kind !== "plane") {
        toy.vy = -Math.abs(toy.vy) * BOUNCE;
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
  w = stepFamiliars(w, inputs, rng, dt, now);

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

  if (!canPlay) {
    // 不玩：叼著的玩具放下、回 free、站住。
    if (c.carryToy != null) {
      w = updateToy(w, c.carryToy, { grabbed: null });
      c.carryToy = null;
    }
    c.mode = "free";
    c.targetToy = null;
    c.vx = 0;
    return { ...w, char: c };
  }

  const chaseable = w.toys.filter(
    (t) =>
      !t.grabbed &&
      t.cooldownUntil <= now &&
      t.interest > 0.25 &&
      (t.kind !== "light" && t.kind !== "wand"
        ? true
        : inputs.pointer !== null && inputs.cursorPlayEnabled)
  );

  switch (c.mode) {
    case "free": {
      // 找最有趣的玩具（hazard 率以秒為單位、依 dt 縮放——每幀呼叫也不會爆量）。
      const best = chaseable.sort((a, b) => b.interest - a.interest)[0];
      if (best && rng() < Math.min(0.5, 1.5 * dt)) {
        c.mode = "chase";
        c.targetToy = best.id;
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
    case "stroll": {
      const dx = c.pounceX - c.x;
      if (Math.abs(dx) < 4 || now > c.modeUntil) {
        c.mode = "free";
        c.vx = 0;
      } else {
        c.vx = Math.sign(dx) * CHAR_SPEED;
        c.facing = dx >= 0 ? 1 : -1;
        c.x += c.vx * dt;
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
      const dx = toy.x - c.x;
      c.facing = dx >= 0 ? 1 : -1;
      // 地面玩具要夠近才撲；游標玩具（光點/逗貓棒）可躍撲（不受高度限制）。
      const pounceable =
        toy.kind === "light" || toy.kind === "wand" ? true : toy.y > w.ground - 40;
      if (Math.abs(dx) < 20 && pounceable) {
        // 夠近：預備撲抓。
        c.mode = "pounce";
        c.modeUntil = now + 450; // anticipation + 撲
        c.pounceX = toy.x;
        c.vx = 0;
      } else {
        c.vx = Math.sign(dx) * CHASE_SPEED;
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
        c.vx = Math.sign(dx) * CHAR_SPEED * 1.3;
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
        c.vx = away * CHAR_SPEED * 0.6;
        c.facing = away as 1 | -1;
        c.x += c.vx * dt;
      }
      break;
    }
  }
  c.x = clampN(c.x, CHAR_HALF + 4, w.w - CHAR_HALF - 4);
  return { ...w, char: c };
}

function stepFamiliars(w: World, inputs: StepInputs, rng: () => number, dt: number, now: number): World {
  if (inputs.reducedMotion || w.familiars.length === 0) return w;
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
      // 互相注意/打招呼：找最近的另一隻或主角。
      const others = list.filter((o) => o.id !== f.id);
      const target =
        others.length > 0 && rng() < 0.6
          ? { x: others[0].x, id: others[0].id }
          : { x: w.char.x, id: "char" };
      f.state = "greet";
      f.greetWith = target.id;
      f.facing = target.x >= f.x ? 1 : -1;
      f.stateUntil = now + 2_500;
      f.vx = 0;
      // 對方也回看。
      const other = list.find((o) => o.id === f.greetWith);
      if (other && other.state !== "sleep") {
        other.state = "greet";
        other.greetWith = f.id;
        other.facing = f.x >= other.x ? 1 : -1;
        other.stateUntil = now + 2_500;
      }
    } else if (roll < 0.8 && list.length > 1) {
      // 追逐另一隻。
      const other = list.find((o) => o.id !== f.id)!;
      f.state = "chase";
      const dir = other.x >= f.x ? 1 : -1;
      f.vx = dir * 55;
      f.facing = dir as 1 | -1;
      f.stateUntil = now + 1_800 + rng() * 1_500;
    } else {
      f.state = "idle";
      f.stateUntil = now + 2_000 + rng() * 4_000;
      f.vx = 0;
    }
  }
  return { ...w, familiars: list };
}

/** Roll Call：現在大家在做什麼（人類語言，不用技術術語）。 */
export function rollCall(world: World, charName: string, machineLabel: string | null): { name: string; activity: string }[] {
  const charActivity =
    machineLabel ??
    (() => {
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
        default:
          return "在休息";
      }
    })();
  const out = [{ name: charName, activity: charActivity }];
  for (const f of world.familiars) {
    const label =
      f.state === "sleep"
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
