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

/** 十三種同步狀態（`satisfies Record<CharacterSyncState, …>` 窮舉）。 */
export type CharacterSyncState =
  | "synced"
  /**
   * 有裝置**連著、也對得上**，但它那條線送不到完整的共享狀態
   * （`syncProfile` 是 `intent-only`／`event-source`；`docs/aip/device-profile.md` §3.1）。
   * 不是故障、也不是已同步：它拿到的根本不是同一份狀態，綠勾會是謊。
   */
  | "partial-sync"
  | "reconnecting"
  | "offline"
  | "partial-capability"
  | "capability-unknown"
  | "syncing"
  | "unrecoverable"
  | "needs-reconfirmation"
  /**
   * 撤銷／移除之後的**終態**（M3 §4.3）：一台裝置都沒有，而且只剩下「以前移除過」
   * 這個歷史事實。舊投影在這裡停在「需要重新確認裝置」——使用者已經做完該做的事
   * （移除手機），畫面卻永遠亮著一個他做不了任何事的警告。這一態把它講成事實：
   * 目前只在這台電腦使用，要再用手機就重新配對。**安全效果完全不變**：被移除的
   * 裝置一樣不會自動回來，一樣要重新配對＋重新確認。
   */
  | "local-only"
  | "no-device"
  | "disabled"
  /** 保存層現在真的出問題（存不下來）。曾經重建過只是歷史通知，見 `note`。 */
  | "store-issue";

/**
 * 同步卡的「下一步」。
 *
 * `id` 是**機器語意**（穩定；測試與導覽都認它），按鈕上的文案（`label`）可以隨時
 * 改寫而不影響任何接線。`null` ＝這一態沒有下一步（已同步／正在同步／已關閉——
 * 給一顆按鈕只會催促使用者去修一個根本沒壞的東西）。
 */
export type CharacterSyncActionId =
  | "connect-phone"
  | "open-devices"
  | "reconfirm-device"
  | "view-capabilities"
  | "safe-reconnect"
  | "storage-help";

export interface CharacterSyncAction {
  id: CharacterSyncActionId | null;
  /** 按鈕文案；沒有下一步就是 `null`。一般文案，允許改寫。 */
  label: string | null;
  /**
   * 落點（深連結）。`storage-help` 沒有落點：它是「你的資料現在怎麼了」的說明，
   * 不是一個可以去的地方——給它一個假的落點才是不誠實。
   */
  target?: { tab: "connect"; hub?: "providers" | "devices" };
}

export interface CharacterSyncProjection {
  /** 一般模式的主要句子。 */
  headline: string;
  /** 一句補充：說清楚「現在能相信什麼」，不是安慰話。 */
  detail: string;
  tone: BadgeKind;
  /** 這一態的下一步（機器語意穩定，文案可改寫）。 */
  action: CharacterSyncAction;
}

const NO_ACTION: CharacterSyncAction = { id: null, label: null };

/**
 * §11 文案表。新增狀態而沒有文案，typecheck 會失敗。
 *
 * **文案不再逐字釘死**（M3）：測試保護的是語意與安全句（緊急停止句、綠色只給真的
 * 已同步、needs-reconfirmation 必須提到重新確認、local-only 必須說不會自動回來），
 * 其餘怎麼講可以改。逐字比對只會讓「把假警報改成人話」變成破壞性改動。
 */
export const CHARACTER_SYNC_PROJECTION = {
  synced: {
    headline: "iPhone 已連接，角色狀態已同步",
    detail: "手機上的角色和這台電腦看到的是同一個狀態。",
    tone: "ok",
    action: NO_ACTION,
  },
  // 這條線本身送不到完整狀態（單則有上限又不會重組）。和 partial-capability 不同：
  // 那一態是「狀態對齊了、只是演不出全部」，這一態連狀態都沒有完整送到，
  // 所以文案裡**不得**出現「已經對齊」。它也不是故障——線就是那條線。
  "partial-sync": {
    headline: "有裝置收不到完整狀態",
    detail: "這台裝置的連線只送得進一部分內容；它看到的角色不一定和這台電腦一樣，不算已同步。",
    tone: "info",
    action: { id: "open-devices", label: "查看裝置", target: { tab: "connect", hub: "devices" } },
  },
  reconnecting: {
    headline: "iPhone 正在重新連線",
    detail: "連線斷了一下，正在接回來；這段時間的互動不會補播。",
    tone: "pending",
    // 可選的下一步，不是催促：接回來通常不需要人做什麼。
    action: { id: "open-devices", label: "查看裝置", target: { tab: "connect", hub: "devices" } },
  },
  offline: {
    headline: "iPhone 暫時離線",
    detail: "手機現在收不到角色狀態，也送不出互動；接回來之後才會重新對齊。",
    tone: "warn",
    action: { id: "open-devices", label: "查看裝置", target: { tab: "connect", hub: "devices" } },
  },
  // 「接上了、狀態也對齊了，只是演不出全部」不是故障：兩件事分開講，
  // 免得使用者以為同步壞了而去重連一個根本沒壞的東西（M3 §4.2）。
  "partial-capability": {
    headline: "部分能力目前不可用",
    detail: "狀態已經對齊了；只是這台裝置演不出角色的部分表演——做不到的不會假裝做到。",
    tone: "info",
    action: {
      id: "view-capabilities",
      label: "看看少了什麼",
      target: { tab: "connect", hub: "devices" },
    },
  },
  // 裝置連上了，但這台電腦拿不到「它到底演得出哪些 intent」的協商結果。
  // 不猜：既不寫成已同步（綠勾只給真的），也不誣賴它做不到。
  "capability-unknown": {
    headline: "iPhone 已連接，能力核對中",
    detail: "狀態已經對齊了，但還沒確認這台裝置演得出哪些表演；在確認之前不算完全同步。",
    tone: "pending",
    action: {
      id: "view-capabilities",
      label: "看看少了什麼",
      target: { tab: "connect", hub: "devices" },
    },
  },
  syncing: {
    headline: "同步尚未完成",
    detail: "還在對齊角色狀態；在這之前不要把畫面上的樣子當成最新的。",
    tone: "pending",
    action: NO_ACTION,
  },
  unrecoverable: {
    headline: "無法恢復，請重新連接",
    detail: "連續好幾次都對不齊角色狀態，需要你重新連一次裝置。",
    tone: "bad",
    action: {
      id: "safe-reconnect",
      label: "重新連接手機",
      target: { tab: "connect", hub: "providers" },
    },
  },
  "needs-reconfirmation": {
    headline: "需要重新確認裝置",
    detail:
      "有裝置現在連著這台電腦，但還不是角色同步的成員（移除過的裝置重新連上來也算）；要再同步角色，必須在手機上重新確認一次。",
    tone: "warn",
    action: {
      id: "reconfirm-device",
      label: "去重新確認",
      target: { tab: "connect", hub: "providers" },
    },
  },
  "local-only": {
    headline: "目前只在這台電腦使用",
    detail: "之前移除過的手機不會自動回來；要再用手機時重新配對一次就好。",
    tone: "muted",
    action: { id: "connect-phone", label: "連接手機", target: { tab: "connect", hub: "providers" } },
  },
  "no-device": {
    headline: "尚未連接 iPhone",
    detail: "目前只有這台電腦在陪你；連上手機之後才會有東西可以同步。",
    tone: "muted",
    action: { id: "connect-phone", label: "連接手機", target: { tab: "connect", hub: "providers" } },
  },
  disabled: {
    headline: "角色同步目前關閉",
    detail: "這台電腦沒有啟用角色同步；其他功能不受影響。",
    tone: "muted",
    action: NO_ACTION,
  },
  // 保存層**現在**存不下來（parked，或連續寫入失敗且有錯誤）。不靜默：使用者有權
  // 知道「這一輪的同步紀錄留不住」；但它講的是紀錄，不是角色，所以不給紅色，
  // 也不給綠色。曾經重建過（storeNote）是歷史，不是這一態——見 `note`。
  "store-issue": {
    headline: "同步紀錄暫時存不下來",
    detail: "這一輪的同步紀錄存不下來，重新啟動之後會再試一次；角色和裝置的連線不受影響。",
    tone: "warn",
    action: { id: "storage-help", label: "這代表什麼？" },
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
  /**
   * Runtime 推導的同步模式原始值（`full-state`／`intent-only`／`event-source`），
   * 查不到就是 `null`。**不得**直接渲染：畫面一律走
   * [`characterSyncProfileLabel`]／[`characterSyncProfileNote`]。
   */
  syncProfile: string | null;
}

// ---------------------------------------------------------------------------
// 成員同步模式（syncProfile；`docs/aip/device-profile.md` §3.1）
// ---------------------------------------------------------------------------
//
// 這個成員**實際上**拿得到多少共享狀態，由 Runtime 依「那條線的事實」
//（`DeviceOutbound::max_line_bytes`／`supports_fragmentation`）＋已協商的 role 推導
//（`crates/interaction-runtime/src/character_session.rs::derive_sync_profile`），
// 不是裝置自己宣稱的。只有 `full-state` 拿得到完整狀態，也只有它可以說「已同步」。

/** 唯一可以說「已同步」的模式。 */
export const CHARACTER_SYNC_PROFILE_FULL_STATE = "full-state";

/** 非 full-state 的人話。一般模式一個英文原始值都不得出現。 */
const CHARACTER_SYNC_PROFILE_LABEL: Record<string, string> = {
  "intent-only": "只接收指令",
  "event-source": "只回報事件",
};

/** 認不得的模式：不猜成 full-state（那會把「不知道」畫成綠勾），也不外洩原始字串。 */
export const CHARACTER_SYNC_PROFILE_UNKNOWN_LABEL = "拿不到完整狀態";

/** 一次最多認幾台裝置的同步模式（有界；後端本身也有成員上限）。 */
const MAX_SYNC_PROFILES = 64;

/**
 * 同步模式 → 畫面上的短標籤；`null` ＝這一台沒有話要多說。
 *
 * 兩種情況回 `null`：
 *   * `full-state`——就是既有語意（已同步該長什麼樣就長什麼樣）；
 *   * **沒有回報**（欄位缺席、空字串）——舊 Runtime 不送這個欄位，Runtime 查不到
 *     出站通道時也會省略。沒有回報 ≠ 非 full-state，所以不憑空降級，也不憑空升級。
 */
export function characterSyncProfileLabel(profile: unknown): string | null {
  const raw = typeof profile === "string" ? profile.trim() : "";
  if (raw.length === 0 || raw === CHARACTER_SYNC_PROFILE_FULL_STATE) return null;
  return CHARACTER_SYNC_PROFILE_LABEL[raw] ?? CHARACTER_SYNC_PROFILE_UNKNOWN_LABEL;
}

/** 裝置條目上的一句補充（非 full-state 才有；`full-state`／沒有回報就是 `null`）。 */
export function characterSyncProfileNote(profile: unknown): string | null {
  const label = characterSyncProfileLabel(profile);
  return label === null ? null : `${label}：這台裝置收不到完整的角色狀態，不算已同步。`;
}

/**
 * 「裝置識別碼 → 同步模式」。
 *
 * 兩種來源同一個形狀概念，所以同一支函式吃得下（呼叫端手上有哪一份就給哪一份）：
 *   * `GET /v1/status` 的 `characterSessionSync[]`（`{deviceId, syncProfile, …}`；
 *     沒有裝置成員時後端不序列化這個鍵）；
 *   * `GET /v1/character-session/diagnostics` 的 `members[]`
 *     （`{party:{kind,id}, syncProfile?}`；只有裝置成員算）。
 *
 * 形狀不可信：缺欄位、型別不對、整包不是物件都要能吃，而且一律不會生出假的模式。
 */
export function characterSyncProfiles(source: unknown): Record<string, string> {
  const root = record(source);
  if (!root) return {};
  const out: Record<string, string> = {};
  const put = (id: unknown, profile: unknown) => {
    if (Object.keys(out).length >= MAX_SYNC_PROFILES) return;
    const key = typeof id === "string" ? id.trim() : "";
    const value = typeof profile === "string" ? profile.trim() : "";
    if (key.length === 0 || value.length === 0 || key in out) return;
    out[key] = value;
  };
  const status = root["characterSessionSync"];
  if (Array.isArray(status)) {
    for (const entry of status) {
      const item = record(entry);
      if (item) put(item["deviceId"], item["syncProfile"]);
    }
  }
  const members = root["members"];
  if (Array.isArray(members)) {
    for (const entry of members) {
      const item = record(entry);
      const party = record(item?.["party"]);
      if (!item || party?.["kind"] !== "device") continue;
      put(party["id"], item["syncProfile"]);
    }
  }
  return out;
}

/**
 * 保存層（persistent store）的訊號。
 *
 * 兩類要分開，否則會變成假警報（M3 §4.3b）：
 *   * **現在正在發生的問題**（`parked`，或連續寫入失敗而且真的有錯誤）——這一輪的
 *     同步紀錄留不住，是狀態（`store-issue`）；
 *   * **歷史通知**（`reset`＝ diagnostics 的 `storeNote` 不是 null）——紀錄曾經壞掉並
 *     重建過。它在同一次 daemon 執行期間**永遠不會清**，拿它當警告等於讓使用者從此
 *     再也看不到綠色。所以它只是一句 muted 的附註（`ProjectedCharacterSync.note`）。
 * `migratedFrom`（舊格式→現行格式）連附註都不是：一般模式不顯示，只進進階模式。
 */
export interface CharacterSyncStoreSignals {
  /** diagnostics 的 `storeNote` 不是 null：紀錄曾壞掉、session 被重建過（歷史事實）。 */
  reset: boolean;
  /** 這一輪什麼都不會存（讀不到／未來格式／備份失敗）。 */
  parked: boolean;
  /** 累計寫入失敗次數。 */
  persistFailures: number;
  /** 最近一次寫入錯誤（後端原文；**不得**進一般模式的畫面）。 */
  lastPersistError: string | null;
  /** 已經成功落地過的版本；null＝到目前為止一次都還沒存成功。 */
  lastPersistedRevision: number | null;
  /** 從舊格式遷移過來（只在進階模式顯示）。 */
  migratedFrom: number | null;
}

/** 投影需要、但不屬於權威狀態的訊號（全部由呼叫端從真實回應算出來）。 */
export interface CharacterSyncSignals {
  /** Runtime 有沒有啟用角色同步（關閉時要誠實說關閉，不說成沒有裝置）。 */
  enabled: boolean;
  /** 連續讀不到權威狀態的次數（達 3 次＝無法恢復，契約 §7.5）。 */
  failedReads: number;
  /**
   * 有裝置的授權被撤銷過，還沒重新確認。
   *
   * 這是**歷史事實**：Runtime 的 provider 列永遠留著 revoked 條目。零裝置時它代表
   * 「以前移除過手機」＝`local-only` 的終態，不是一個要人動手的警告。
   */
  revokedDevice: boolean;
  /** 有手機**現在**連著這台電腦，但還不是角色同步的成員（要重新確認才會同步）。 */
  connectedButNotSynced: boolean;
  /**
   * 那些「連著但還不是成員」的裝置在這台電腦上的**顯示名稱**（指出是哪一台）。
   * 一般模式只給名字：查不到名字的用中性稱呼，永遠不給裝置識別碼。
   */
  pendingDeviceNames?: readonly string[];
  /** 保存層訊號；`null`／未提供＝讀不到診斷（不猜好也不猜壞）。 */
  store?: CharacterSyncStoreSignals | null;
  /**
   * host 明說 `recovery`、把這台電腦的副本帶回較舊的權威狀態
   * （`docs/aip/character-session.md` §7.2 的決策表規則 6）。
   *
   * 這**不是**故障：host 從較舊的快照還原過，桌面照它說的重新對齊。但畫面上剛剛看得到的
   * 東西被換掉了，靜默處理就是讓使用者以為自己記錯——所以掛一句 muted 的附註。
   * 附註裡不得出現 revision／任何數字：那是進階模式「連接診斷」的事。
   */
  recovered?: boolean;
}

export interface ProjectedCharacterSync extends CharacterSyncProjection {
  state: CharacterSyncState;
  /** 介面認得目前每一台裝置回報的狀態嗎；false＝不猜，只說尚未完成。 */
  known: boolean;
  /**
   * 一句 muted 的附註（目前只有保存層的歷史通知會用）。它**不是**警告，
   * 也不改 tone——「曾經發生過」不該永遠壓著「現在好好的」。
   */
  note: string | null;
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/**
 * `GET /v1/character-session/diagnostics` → 保存層訊號（純資料轉換，不做判斷）。
 *
 * 讀不到診斷就是 `null`：不假設存得下來，也不假設壞了。舊 Runtime 沒有 `store`
 * 欄位時只認得 `storeNote`（歷史通知），其餘欄位一律用「不知道」的值，
 * 不會被誤判成 active issue。
 */
export function characterSyncStoreSignals(diagnostics: unknown): CharacterSyncStoreSignals | null {
  const value = record(diagnostics);
  if (!value) return null;
  const store = record(value["store"]);
  return {
    reset: value["storeNote"] != null,
    parked: store?.["parked"] === true,
    persistFailures: finiteNumber(store?.["persistFailures"]) ?? 0,
    lastPersistError:
      typeof store?.["lastPersistError"] === "string" ? String(store["lastPersistError"]) : null,
    lastPersistedRevision: finiteNumber(store?.["lastPersistedRevision"]),
    migratedFrom: finiteNumber(store?.["migratedFrom"]),
  };
}

/** 現在真的存不下來嗎（active issue，不是歷史）。 */
function storeIssue(store: CharacterSyncStoreSignals | null | undefined): boolean {
  if (!store) return false;
  // 只有計數、沒有錯誤原文＝證據不足，不製造警報（誠實：不確定就不宣稱）。
  return store.parked || (store.persistFailures > 0 && store.lastPersistError !== null);
}

/** 歷史通知的一句人話（沒有就是 null）。 */
function storeNotice(store: CharacterSyncStoreSignals | null | undefined): string | null {
  if (!store || !store.reset) return null;
  return store.lastPersistedRevision === null
    ? "先前的同步紀錄曾經重建過一次；新的紀錄還沒存下來過。"
    : "先前的同步紀錄曾經重建過一次；之後的紀錄都已經正常存下來。";
}

/**
 * host 說它從較舊的權威狀態還原了（決策表規則 6 的 `recover`）之後的那一句人話。
 *
 * 只講「發生了什麼」，不講 revision——一般模式一個數字都不給。
 */
export const CHARACTER_SYNC_RECOVERED_NOTE = "已依桌面的權威狀態重新對齊。";

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
  names: Record<string, string>,
  /** 「裝置識別碼 → 同步模式」（[`characterSyncProfiles`]）；查不到就是不知道。 */
  profiles: Record<string, string> = {}
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
      // 這台裝置那條線送得到多少狀態（Runtime 推導；查不到就是不知道，不猜）。
      syncProfile: remote ? (profiles[id] ?? null) : null,
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
 * 關閉 → 連續讀不到 → 讀不到這一次 → 認不得的回報 → 紀錄存不下來 → online →
 * reconnecting → offline → 需要重新確認 → 只在這台電腦 → 沒有裝置。
 *
 * 「紀錄存不下來」排在 online 之前：那一刻技術上也許真的同步著，但綠色徽章讀起來
 * 是「一切正常」，會把「這一輪的紀錄留不住」這件事蓋掉。它仍然排在「讀不到」
 * 之後——連現在的狀態都讀不到時，先講讀不到。**只有現在正在發生的問題**才排在這裡；
 * 「曾經重建過」是歷史，只掛成 `note`，不壓過任何狀態（M3 §4.3b）。
 * 緊急停止的固定安全句由呼叫端（可信 host 介面）另外顯示，永遠壓過這一句。
 */
export function projectCharacterSession(
  snapshot: unknown,
  members: readonly CharacterSyncMember[],
  signals: CharacterSyncSignals
): ProjectedCharacterSync {
  const active = storeIssue(signals.store);
  // 保存層的歷史通知只在「現在沒問題」時才掛：同一件事不講兩次。
  // 「host 從較舊的狀態還原過」是另一件事（講的是角色狀態，不是紀錄），兩句可以同時成立。
  const notes = [
    active ? null : storeNotice(signals.store),
    signals.recovered === true ? CHARACTER_SYNC_RECOVERED_NOTE : null,
  ].filter((line): line is string => line !== null);
  const note = notes.length > 0 ? notes.join(" ") : null;
  const project = (state: CharacterSyncState, known = true): ProjectedCharacterSync => ({
    state,
    known,
    note,
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
  if (active) {
    const parked = signals.store?.parked === true;
    return {
      ...project("store-issue"),
      // 兩種原因說法不同：parked 是「這一輪不存」，寫入失敗是「現在寫不進去」。
      // 兩句都不回顯後端錯誤原文。
      detail: parked
        ? CHARACTER_SYNC_PROJECTION["store-issue"].detail
        : "同步紀錄目前寫不進去，之前存下來的還在；角色和裝置的連線不受影響。",
    };
  }
  const online = remote.filter((m) => m.presence === "online");
  if (online.length > 0) {
    // 這條線送不到完整狀態（`intent-only`／`event-source`）→ 排在能力判定**之前**：
    // partial-capability 的文案說「狀態已經對齊了」，而這一態連狀態都沒有完整送到，
    // 講成「對齊了、只是演不出全部」就是把兩件事說反（device-profile §3.1）。
    if (online.some((m) => characterSyncProfileLabel(m.syncProfile) !== null)) {
      return project("partial-sync");
    }
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
    if (signals.connectedButNotSynced) return needsReconfirmation(project, signals);
    // 協商結果拿不到 → 不給綠色，也不誣賴它做不到。
    if (online.some((m) => m.degraded === null)) return project("capability-unknown");
    return project("synced");
  }
  if (remote.some((m) => m.presence === "reconnecting")) return project("reconnecting");
  if (remote.length > 0) return project("offline");
  // 零裝置。只有「裝置正嘗試回來」（現在連著、還不是成員）才要人動手；
  // 「以前移除過」是終態，不是待辦（M3 §4.3）。
  if (signals.connectedButNotSynced) return needsReconfirmation(project, signals);
  if (signals.revokedDevice) return project("local-only");
  return project("no-device");
}

/** 需要重新確認：把「是哪一台」講出來（只給名字，永遠不給裝置識別碼）。 */
function needsReconfirmation(
  project: (state: CharacterSyncState, known?: boolean) => ProjectedCharacterSync,
  signals: CharacterSyncSignals
): ProjectedCharacterSync {
  const base = project("needs-reconfirmation");
  const names = (signals.pendingDeviceNames ?? []).filter((name) => name.length > 0);
  if (names.length === 0) return base;
  return { ...base, detail: `${names.join("、")}：${base.detail}` };
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
export function characterSyncDeviceLine(
  snapshot: unknown,
  deviceId: string,
  /** 這台裝置的同步模式（[`characterSyncProfiles`]）；沒有就是沒有回報，不猜。 */
  syncProfile?: unknown
): string {
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
      // 互相矛盾的結論（對抗審查 general-mode-ux-025）。同步模式一樣排在最前面。
      const profile = characterSyncProfileLabel(syncProfile);
      if (profile !== null) return `角色同步：${profile}（不是完整同步）`;
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
