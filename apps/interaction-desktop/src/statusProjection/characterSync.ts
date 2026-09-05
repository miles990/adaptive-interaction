// 角色同步（AIP Character Session；契約 `docs/aip/character-session.md` §11）：
// 一般模式只看得到人話。權威狀態、revision／sequence／epoch、counters 全部留在
// 進階模式的「連接診斷」，這裡的輸出一個技術詞都不得出現。
//
// 誠實階梯：
// - 讀不到權威狀態 ≠ 已同步：一律「同步尚未完成」，不用上一次的樣子冒充現在。
// - 空狀態 ≠ 成功：一台裝置都沒有時是中性的「尚未連接 iPhone」（muted），不是綠勾。
// - 認不得的 presence 不猜：退回「同步尚未完成」並標 `known: false`。
// - 模擬 iPhone（fixture）的名稱本身已含標籤，投影原樣顯示、不再加工，也不會
//   把它寫成真機。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔（既有 import 路徑不變）。

import type { BadgeKind } from "./workState";

/** 十一種同步狀態（`satisfies Record<CharacterSyncState, …>` 窮舉）。 */
export type CharacterSyncState =
  | "synced"
  | "reconnecting"
  | "offline"
  | "partial-capability"
  | "capability-unknown"
  | "syncing"
  | "unrecoverable"
  | "needs-reconfirmation"
  | "no-device"
  | "disabled"
  | "store-reset";

export interface CharacterSyncProjection {
  /** 一般模式的主要句子（契約 §11 文案表，一字不改）。 */
  headline: string;
  /** 一句補充：說清楚「現在能相信什麼」，不是安慰話。 */
  detail: string;
  tone: BadgeKind;
}

/** §11 文案表。新增狀態而沒有文案，typecheck 會失敗。 */
export const CHARACTER_SYNC_PROJECTION = {
  synced: {
    headline: "iPhone 已連接，角色狀態已同步",
    detail: "手機上的角色和這台電腦看到的是同一個狀態。",
    tone: "ok",
  },
  reconnecting: {
    headline: "iPhone 正在重新連線",
    detail: "連線斷了一下，正在接回來；這段時間的互動不會補播。",
    tone: "pending",
  },
  offline: {
    headline: "iPhone 暫時離線",
    detail: "手機現在收不到角色狀態，也送不出互動；接回來之後才會重新對齊。",
    tone: "warn",
  },
  "partial-capability": {
    headline: "部分能力目前不可用",
    detail: "這台裝置接上了，但它做不到角色的部分表演；做不到的不會假裝做到。",
    tone: "warn",
  },
  // 裝置連上了，但這台電腦拿不到「它到底演得出哪些 intent」的協商結果。
  // 不猜：既不寫成已同步（綠勾只給真的），也不誣賴它做不到。
  "capability-unknown": {
    headline: "iPhone 已連接，能力核對中",
    detail: "狀態對齊了，但還沒確認這台裝置演得出哪些表演；在確認之前不要當成完全同步。",
    tone: "pending",
  },
  syncing: {
    headline: "同步尚未完成",
    detail: "還在對齊角色狀態；在這之前不要把畫面上的樣子當成最新的。",
    tone: "pending",
  },
  unrecoverable: {
    headline: "無法恢復，請重新連接",
    detail: "連續好幾次都對不齊角色狀態，需要你重新連一次裝置。",
    tone: "bad",
  },
  "needs-reconfirmation": {
    headline: "需要重新確認裝置",
    detail: "有裝置現在連著這台電腦，但還不是角色同步的成員（撤銷過的裝置重新連上來也算）；要再同步角色，必須在手機上重新確認一次。",
    tone: "warn",
  },
  "no-device": {
    headline: "尚未連接 iPhone",
    detail: "目前只有這台電腦在陪你；連上手機之後才會有東西可以同步。",
    tone: "muted",
  },
  disabled: {
    headline: "角色同步目前關閉",
    detail: "這台電腦沒有啟用角色同步；其他功能不受影響。",
    tone: "muted",
  },
  // 保存的同步紀錄壞掉、已被隔離並重新開始（Runtime 的 `storeNote`）。
  // 不靜默：這件事會讓已連接的裝置重新對齊一次，使用者有權知道；但它不是
  // 緊急狀況（不給紅色），也不是成功（不給綠色）。
  "store-reset": {
    headline: "角色同步紀錄曾損毀，已重新開始",
    detail: "已重新連接的裝置會重新同步；不影響角色本身。",
    tone: "warn",
  },
} satisfies Record<CharacterSyncState, CharacterSyncProjection>;

/** 判定順序也是宣告順序（測試釘住，避免有人偷偷把 synced 往後搬）。 */
export const CHARACTER_SYNC_STATES: readonly CharacterSyncState[] = Object.keys(
  CHARACTER_SYNC_PROJECTION
) as CharacterSyncState[];

/** 一個同步成員在畫面上的樣子（名稱由呼叫端從裝置清單補；查不到就中性稱呼）。 */
export interface CharacterSyncMember {
  /** 顯示名稱。模擬 iPhone（fixture）的名稱自帶標籤，原樣顯示。 */
  name: string;
  /** 遠端裝置才算「另一頭」；這台電腦自己的角色視窗不算。 */
  remote: boolean;
  /** Runtime 回報的 presence 原始值；不認得就是不認得（不猜）。 */
  presence: string;
  /** 這個成員演得出角色嗎（只有遠端呈現角色的裝置可以）。 */
  canPresent: boolean;
  /**
   * 協商後有沒有做不到的 intent。
   *
   * `true`＝有（部分能力不可用）；`false`＝一個都沒有；`null`＝**這台電腦拿不到協商結果**。
   * 契約 §11 的判定條件是協商結果，不是成員自報的 role——role 是裝置自己填的，
   * 拿它當能力結論等於讓 renderer capability spoofing 影響人類看到的結果
   * （對抗審查 capability-consent-052／general-mode-ux-022）。拿不到就誠實說拿不到。
   */
  degraded: boolean | null;
}

/** 投影需要、但不屬於權威狀態的訊號（全部由呼叫端從真實回應算出來）。 */
export interface CharacterSyncSignals {
  /** Runtime 有沒有啟用角色同步（關閉時要誠實說關閉，不說成沒有裝置）。 */
  enabled: boolean;
  /** 連續讀不到權威狀態的次數（達 3 次＝無法恢復，契約 §7.5）。 */
  failedReads: number;
  /** 有裝置的授權被撤銷過，還沒重新確認。 */
  revokedDevice: boolean;
  /** 有手機連著這台電腦，但還不是角色同步的成員（要重新確認才會同步）。 */
  connectedButNotSynced: boolean;
  /**
   * 保存的角色同步紀錄讀不回來、已被隔離並重新開始（Runtime diagnostics 的
   * `storeNote` 不是 null）。一般模式要翻成人話，不得靜默——但它講的是「紀錄」，
   * 不是角色本身，所以不能當成緊急狀況。
   */
  storeReset: boolean;
}

export interface ProjectedCharacterSync extends CharacterSyncProjection {
  state: CharacterSyncState;
  /** 介面認得目前每一台裝置回報的狀態嗎；false＝不猜，只說尚未完成。 */
  known: boolean;
}

/** 這台電腦自己（桌面角色視窗）在成員清單裡的稱呼——不印任何識別碼。 */
export const CHARACTER_SYNC_LOCAL_NAME = "這台電腦";
/** 名字查不到的裝置：中性稱呼，絕不退回裝置識別碼。 */
export const CHARACTER_SYNC_UNNAMED_DEVICE = "一台裝置";

const CHARACTER_SYNC_PRESENCE = {
  online: "已連接",
  reconnecting: "重新連線中",
  offline: "離線",
} satisfies Record<string, string>;

/** presence 原始值 → 人話；不認得就誠實說不確定。 */
export function characterSyncPresenceLabel(presence: string): string {
  return Object.prototype.hasOwnProperty.call(CHARACTER_SYNC_PRESENCE, presence)
    ? CHARACTER_SYNC_PRESENCE[presence as keyof typeof CHARACTER_SYNC_PRESENCE]
    : "狀態不確定";
}

function isKnownPresence(presence: string): boolean {
  return Object.prototype.hasOwnProperty.call(CHARACTER_SYNC_PRESENCE, presence);
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** snapshot envelope → 權威狀態（讀不到就是 null，不用空物件冒充）。 */
function sessionState(snapshot: unknown): Record<string, unknown> | null {
  const payload = record(record(snapshot)?.["payload"]);
  return record(payload?.["state"]);
}

/**
 * 權威狀態的成員清單 → 畫面上的成員。
 * `names` 是「裝置識別碼 → 這台電腦上的顯示名稱」（來自已配對裝置清單）；
 * 查不到就用中性稱呼，一般模式永遠看不到識別碼本身。
 */
export function characterSyncMembers(
  snapshot: unknown,
  names: Record<string, string>
): CharacterSyncMember[] {
  const raw = sessionState(snapshot)?.["members"];
  if (!Array.isArray(raw)) return [];
  const members: CharacterSyncMember[] = [];
  for (const entry of raw) {
    const item = record(entry);
    if (!item) continue;
    const party = record(item["party"]);
    const kind = String(party?.["kind"] ?? "");
    const id = String(party?.["id"] ?? "");
    const remote = kind === "device";
    members.push({
      name: remote ? (names[id] ?? CHARACTER_SYNC_UNNAMED_DEVICE) : CHARACTER_SYNC_LOCAL_NAME,
      remote,
      presence: String(item["presence"] ?? ""),
      // 只有「遠端呈現角色」的成員演得出角色；只送輸入或只旁觀的不算。
      canPresent: item["role"] === "remote-renderer" || item["role"] === "host-renderer",
      degraded: memberDegraded(item),
    });
  }
  return members;
}

/**
 * 一個成員的協商摘要 → 「有沒有做不到的 intent」。
 *
 * Runtime 目前**沒有**把協商結果投影到 `GET /v1/character-session` 或 diagnostics
 * （`MemberView` 只有 party／role／presence／lastSeenAt，`Member.negotiated` 是 host 私有），
 * 所以正式路徑上這裡幾乎一定回 `null` ＝「不知道」。等 Runtime 補上欄位之後，
 * 下面兩種寫法都認得，不必再動這一支：
 *   - `members[].unsupportedIntents`：數字或字串陣列；
 *   - `members[].negotiated.intents`：intent → `"exact"｜"unsupported"` 的對照表
 *     （或 `negotiated.unsupportedIntents` 陣列）。
 * 認不得的形狀一律回 `null`（不猜成「都做得到」）。
 */
function memberDegraded(item: Record<string, unknown>): boolean | null {
  const direct = item["unsupportedIntents"];
  if (typeof direct === "number" && Number.isFinite(direct)) return direct > 0;
  if (Array.isArray(direct)) return direct.length > 0;
  const negotiated = record(item["negotiated"]);
  if (negotiated) {
    const listed = negotiated["unsupportedIntents"];
    if (typeof listed === "number" && Number.isFinite(listed)) return listed > 0;
    if (Array.isArray(listed)) return listed.length > 0;
    const intents = record(negotiated["intents"]);
    if (intents) {
      const values = Object.values(intents);
      if (values.length === 0) return null;
      return values.some((v) => String(v).toLowerCase() === "unsupported");
    }
  }
  return null;
}

/** 互動種類 → 人話（未知種類不猜，用最中性的說法）。 */
const CHARACTER_TOUCH_VERB: Record<string, string> = {
  tap: "摸了摸角色",
  pat: "輕拍了角色",
  stroke: "撫摸了角色",
  longpress: "按著角色不放",
};

/** 誰做的（`"<kind>:<id>"`）→ 人話稱呼；裝置查不到名字就中性稱呼。 */
function interactionActor(source: string, names: Record<string, string>): string {
  const split = source.indexOf(":");
  const kind = split >= 0 ? source.slice(0, split) : source;
  const id = split >= 0 ? source.slice(split + 1) : "";
  if (kind === "device") return names[id] ?? CHARACTER_SYNC_UNNAMED_DEVICE;
  if (kind === "human-surface" || kind === "renderer") return "你在這台電腦上";
  return "有人";
}

/** 最近一次互動的一句人話；沒有互動過就是 `null`（不編一個出來）。 */
export function characterSyncLastInteraction(
  snapshot: unknown,
  names: Record<string, string>
): string | null {
  const last = record(sessionState(snapshot)?.["lastInteraction"]);
  if (!last) return null;
  const name = String(last["name"] ?? "");
  const kind = String(last["kind"] ?? "");
  const who = interactionActor(String(last["source"] ?? ""), names);
  if (name === "character.interaction.dismiss") return `${who}請角色休息一下`;
  if (name !== "character.interaction.touch") return null;
  return `${who}${CHARACTER_TOUCH_VERB[kind] ?? "和角色互動了一下"}`;
}

/** 緊急停止中的固定安全句（角色與 adapter 都不能覆寫）；其餘狀態沒有這一句。 */
export const CHARACTER_SYNC_EMERGENCY_TEXT =
  "緊急停止中：角色已停止表演，解除前不會接受任何互動。";

export function characterSyncSafetyNote(snapshot: unknown): string | null {
  const truth = record(sessionState(snapshot)?.["truth"]);
  return truth?.["state"] === "emergency" ? CHARACTER_SYNC_EMERGENCY_TEXT : null;
}

/**
 * 角色同步的一般模式投影。
 *
 * 判定順序（先擋住「不能相信」的情況，再談成功）：
 * 關閉 → 連續讀不到 → 讀不到這一次 → 認不得的回報 → 紀錄曾損毀 → online →
 * reconnecting → offline → 需要重新確認 → 沒有裝置。
 *
 * 「紀錄曾損毀」排在 online 之前：那一刻技術上也許真的同步著，但綠色徽章讀起來
 * 是「一切正常」，會把「你的裝置得重新對齊一次」這件事蓋掉。它仍然排在「讀不到」
 * 之後——連現在的狀態都讀不到時，先講讀不到。緊急停止的固定安全句由呼叫端
 * （可信 host 介面）另外顯示，永遠壓過這一句。
 */
export function projectCharacterSession(
  snapshot: unknown,
  members: readonly CharacterSyncMember[],
  signals: CharacterSyncSignals
): ProjectedCharacterSync {
  const project = (state: CharacterSyncState, known = true): ProjectedCharacterSync => ({
    state,
    known,
    ...CHARACTER_SYNC_PROJECTION[state],
  });
  if (!signals.enabled) return project("disabled");
  if (signals.failedReads >= 3) return project("unrecoverable");
  if (sessionState(snapshot) === null) return project("syncing");

  const remote = members.filter((m) => m.remote);
  // 認不得的回報一律不猜：不會被算成 online，也不會被寫成 offline。
  if (remote.some((m) => !isKnownPresence(m.presence))) {
    return {
      ...project("syncing", false),
      detail: "有裝置回報了這台電腦不認得的狀態；在弄清楚之前都當成尚未完成，不會當成已同步。",
    };
  }
  if (signals.storeReset) return project("store-reset");
  const online = remote.filter((m) => m.presence === "online");
  if (online.length > 0) {
    // 已知做不到（role 根本不呈現角色，或協商出 unsupported intent）→ 部分能力不可用。
    if (online.some((m) => !m.canPresent || m.degraded === true)) {
      return project("partial-capability");
    }
    // 有 online 成員不代表「每一台裝置都同步了」：另一台**現在就連著這台電腦**卻不是
    // session 成員的手機（送不出互動、也收不到狀態）是當下的事實，綠勾會把它蓋掉
    //（對抗審查 general-mode-ux-026）。
    //
    // 這裡只看 `connectedButNotSynced`，不看 `revokedDevice`：後者是「曾經有裝置被撤銷過」
    // 的歷史事實（provider 列會永遠留著 revoked），拿它壓過一台真的在線的裝置會變成
    // 一個永遠亮著的假警報（general-mode-ux.md §3）。真的需要重新確認的裝置只要連上來，
    // 就會以「連著但不是成員」的身分出現在 `connectedButNotSynced` 裡。
    if (signals.connectedButNotSynced) return project("needs-reconfirmation");
    // 協商結果拿不到 → 不給綠色，也不誣賴它做不到。
    if (online.some((m) => m.degraded === null)) return project("capability-unknown");
    return project("synced");
  }
  if (remote.some((m) => m.presence === "reconnecting")) return project("reconnecting");
  if (remote.length > 0) return project("offline");
  if (signals.revokedDevice || signals.connectedButNotSynced) {
    return project("needs-reconfirmation");
  }
  return project("no-device");
}

/**
 * 權威狀態裡「已經是同步成員」的裝置識別碼。
 *
 * 只給程式比對用（例如判斷某台連線中的手機還沒重新確認），**不得**印到畫面上：
 * 一般模式看得到的永遠是裝置名稱。
 */
export function characterSyncMemberDeviceIds(snapshot: unknown): string[] {
  const raw = sessionState(snapshot)?.["members"];
  if (!Array.isArray(raw)) return [];
  const ids: string[] = [];
  for (const entry of raw) {
    const party = record(record(entry)?.["party"]);
    if (party?.["kind"] !== "device") continue;
    const id = String(party["id"] ?? "");
    if (id.length > 0) ids.push(id);
  }
  return ids;
}

/**
 * 連接頁手機卡上的那一行同步狀態（一台手機一句人話）。
 *
 * 讀不到權威狀態就說讀不到；手機連著卻不在成員名單裡是「尚未同步」而不是「已同步」。
 */
export function characterSyncDeviceLine(snapshot: unknown, deviceId: string): string {
  const state = sessionState(snapshot);
  if (state === null) return "角色同步：目前讀不到狀態";
  const raw = state["members"];
  const members = Array.isArray(raw) ? raw : [];
  for (const entry of members) {
    const item = record(entry);
    const party = record(item?.["party"]);
    if (party?.["kind"] !== "device" || String(party["id"] ?? "") !== deviceId) continue;
    const presence = String(item?.["presence"] ?? "");
    if (presence === "online") {
      // 和角色頁的同步卡走同一套判定：同一份快照不得在兩個一般模式畫面得出
      // 互相矛盾的結論（對抗審查 general-mode-ux-025）。
      const canPresent = item?.["role"] === "remote-renderer" || item?.["role"] === "host-renderer";
      const degraded = item ? memberDegraded(item) : null;
      if (!canPresent || degraded === true) {
        return `角色同步：${CHARACTER_SYNC_PROJECTION["partial-capability"].headline}`;
      }
      if (degraded === null) return "角色同步：已連接，能力核對中";
      return "角色同步：已同步";
    }
    return `角色同步：${characterSyncPresenceLabel(presence)}`;
  }
  return "角色同步：尚未同步（需要在手機上重新確認）";
}
