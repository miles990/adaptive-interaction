// 「未解決停止」的人話投影（Runtime `sensor_source.rs` 的 `UnresolvedStop`）。
//
// 這張表回答的**不是**「現在有什麼在感測」（那是 `activeSensors`），也不是歷史
// （歷史在稽核裡），而是「有哪些擷取，我們不知道它停了沒有」：來源被移除時還在
// 擷取，之後沒有任何人／任何裝置確認過它停下來。
//
// 誠實階梯：
// - 這些紀錄既不是「還在感測」，也不是「已經停下來」的保證——不知道就說不知道；
// - 人為解除只是「人類看過了」，不是裝置回報，文案一定要說清楚；
// - `sourceId`／`generation` 是內部識別，只用來呼叫 API，**絕不進畫面文字**
//   （X5：一般模式不外洩技術詞與識別碼）。
//
// 對外一律經由 `../statusProjection.ts` 這個匯總檔。

import { sensorKindLabel } from "./provider";

/** 逐筆最多顯示幾列（有界：後端的表本身有上限，畫面再收一次）。 */
export const MAX_UNRESOLVED_LINES = 20;

/** 名字查不到時的中性稱呼（不退回 `sourceId`）。 */
const UNKNOWN_SOURCE = "某個裝置";

/** 固定的誠實說明：這一區在說什麼、不在說什麼。 */
export const UNRESOLVED_STOPS_NOTE =
  "這些是「不知道有沒有停下來」的紀錄：來源離開使用中清單時還在擷取，之後沒有人確認過。" +
  "它既不代表現在還在感測，也不代表它停了。";

/** 人為解除按鈕的固定文案（二段確認的第二段一定要說清楚這是誰的確認）。 */
export const UNRESOLVED_DISMISS_LABEL = "我確認它已經停了";
export const UNRESOLVED_DISMISS_CONFIRM =
  "確定：這是你的確認，系統沒有收到裝置的回覆";
export const UNRESOLVED_DISMISSED_MESSAGE =
  "已記下你的確認（這是你的確認，系統沒有收到裝置的回覆）。";

/** 一筆未解決停止投影後的樣子。`sourceId`／`generation` 只給 API 用，不上畫面。 */
export interface UnresolvedStopLine {
  /** 內部識別：呼叫解除 API 用。**不得**渲染。 */
  sourceId: string;
  /** 哪一次登記：解除一定要指名世代，才不會誤清掉新的一筆。**不得**渲染。 */
  generation: number;
  /** 人話名稱（`sourceLabel` 有就用，沒有就是中性稱呼）。 */
  label: string;
  /** 涵蓋哪些感測（人話種類，去重）。 */
  sensorsText: string;
  /** 這一筆變成「未解決」多久了。 */
  sinceText: string;
  /** 完整的一句話。 */
  line: string;
}

export interface UnresolvedStopsProjection {
  /** 後端回報的總筆數（不是這一頁列出的數量）。 */
  count: number;
  /** 狀態列／摘要那一行；沒有未解決停止時是 `null`。 */
  summary: string | null;
  /** 逐筆（最多 `MAX_UNRESOLVED_LINES` 筆）。 */
  items: UnresolvedStopLine[];
  /** 沒有列出來的筆數（`count - items.length`）。 */
  notShown: number;
  note: string;
}

/** `status.unresolvedStops` 空／缺席時的固定結果（沒有未解決的事）。 */
const EMPTY: UnresolvedStopsProjection = {
  count: 0,
  summary: null,
  items: [],
  notShown: 0,
  note: UNRESOLVED_STOPS_NOTE,
};

function textOf(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/**
 * 相對時間的人話。時間讀不出來就說「時間不明」——不猜、也不印原始字串。
 * 未來時間（時鐘偏差）一律當「剛剛」，不會出現「-3 分鐘前」。
 */
export function relativeSince(value: unknown, now: number = Date.now()): string {
  const at = Date.parse(textOf(value));
  if (!Number.isFinite(at)) return "時間不明";
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 60) return "剛剛";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} 分鐘前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} 小時前`;
  return `${Math.floor(hours / 24)} 天前`;
}

/** 一筆紀錄涵蓋的感測種類（人話、去重）。認不得的種類走共用投影，不外洩原始 id。 */
function sensorsTextOf(record: Record<string, unknown>): string {
  const raw = Array.isArray(record.sensors) ? record.sensors : [];
  const labels: string[] = [];
  for (const kind of raw) {
    const label = sensorKindLabel(typeof kind === "string" ? kind : "");
    if (!labels.includes(label)) labels.push(label);
  }
  return labels.length > 0 ? labels.join("、") : "感測";
}

/**
 * 把 `status`（或 `GET /v1/sensors/unresolved` 的回應）投影成人話。
 *
 * 兩者用同一個鍵 `unresolvedStops`，所以同一支函式吃得下；欄位缺席、型別不對、
 * 整包不是物件都要能吃（形狀不可信）。
 *
 * @param status `/v1/status` 或 `/v1/sensors/unresolved` 的回應。
 * @param now    現在時間（測試可注入）。
 */
export function projectUnresolvedStops(
  status: unknown,
  now: number = Date.now()
): UnresolvedStopsProjection {
  const root = status && typeof status === "object" ? (status as Record<string, unknown>) : null;
  const list = root && Array.isArray(root.unresolvedStops) ? root.unresolvedStops : [];
  const records = list.filter(
    (entry): entry is Record<string, unknown> => !!entry && typeof entry === "object"
  );
  if (records.length === 0) return EMPTY;
  const items: UnresolvedStopLine[] = records.slice(0, MAX_UNRESOLVED_LINES).map((record) => {
    const label = textOf(record.sourceLabel) || UNKNOWN_SOURCE;
    const sensorsText = sensorsTextOf(record);
    const sinceText = relativeSince(record.since, now);
    return {
      sourceId: textOf(record.sourceId),
      generation: Number.isFinite(Number(record.generation)) ? Number(record.generation) : 0,
      label,
      sensorsText,
      sinceText,
      line: `${label}的${sensorsText}：${sinceText}離開使用中清單，沒有人確認過它。`,
    };
  });
  return {
    count: records.length,
    summary: `有 ${records.length} 筆感測停止沒有人確認`,
    items,
    notShown: records.length - items.length,
    note: UNRESOLVED_STOPS_NOTE,
  };
}
