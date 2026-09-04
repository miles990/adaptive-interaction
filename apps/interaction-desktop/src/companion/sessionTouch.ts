// 桌面角色被摸了一下 → Character Session 的語意事件。
//
// 契約：AIP §1／§7（`docs/aip/README.md`）＋`docs/aip/transport-bindings.md` §2。
// 桌面視窗是**可信 host surface**，它能證明的身分只有 human token，所以 `source`
// 一定是 `{kind:"human-surface", id:"desktop"}`；宣稱其他身分只會被後端擋下來
// （identity-mismatch），這裡也不提供改寫的入口。
//
// 誠實邊界：
//   * 既有的 `companion.click` 觀察路徑**完全不變**——這只是在旁邊多送一則語意
//     事件，讓手機那頭也知道角色被摸了一下。
//   * 送出 ≠ 生效：後端回 `applied` 才代表權威狀態真的動了；`rejected`／`expired`
//     一律照實回報給呼叫端，絕不當成成功。
//   * 互動事件必帶 deadline（AIP §7，建議 5 秒）：重連之後不補播舊的觸摸。
//   * 信封先本地驗證再送；驗不過就不送（未知不執行），也不回顯輸入內容。

import { api } from "../api";
import { AIP_SPEC_VERSION, type Envelope } from "../aip/generated";
import { validateEnvelope } from "../aip/envelope";

/** 桌面 Runtime 的預設 session（`interaction_runtime::character_session::SESSION_ID`）。 */
export const CHARACTER_SESSION_ID = "session.home";
/** 可信 host surface 的身分（`DESKTOP_SURFACE_ID`）。 */
export const DESKTOP_SURFACE = { kind: "human-surface", id: "desktop" } as const;
/** 互動事件的 deadline（AIP `DEFAULT_INTERACTION_TTL_MS`）。 */
export const TOUCH_TTL_MS = 5000;

/** 1.0 的互動種類（`character-session.md` §4）。 */
export type TouchKind = "tap" | "longpress" | "pat" | "stroke";

let counter = 0;

/** 每個 source 內唯一的訊息識別碼（≤128 字；只在傳輸層用，不進畫面）。 */
export function nextTouchMessageId(nowMs: number): string {
  counter = (counter + 1) % 1_000_000;
  return `desktop-touch-${nowMs}-${counter}`;
}

/** 一則 `character.interaction.touch` 事件信封（純函式；時間由呼叫端注入）。 */
export function buildTouchEnvelope(
  nowMs: number,
  kind: TouchKind,
  messageId = nextTouchMessageId(nowMs)
): Envelope {
  return {
    specVersion: AIP_SPEC_VERSION,
    messageId,
    messageType: "event",
    name: "character.interaction.touch",
    source: { ...DESKTOP_SURFACE },
    sessionId: CHARACTER_SESSION_ID,
    occurredAt: new Date(nowMs).toISOString(),
    expiresAt: new Date(nowMs + TOUCH_TTL_MS).toISOString(),
    payload: { kind },
  };
}

/** 後端對這一則事件的處理結果（`applied` 之外都不是成功）。 */
export type TouchOutcome = "applied" | "accepted" | "rejected" | "expired" | "not-sent" | "unknown";

/** 同一時間只讓一則觸摸在路上：不排隊、不堆積（無界佇列是禁區）。 */
let inFlight = false;

/**
 * 送出一則觸摸事件。
 *
 * 回傳的是**後端說的話**：`applied` 才代表角色狀態真的改了；驗證沒過是 `not-sent`；
 * 送不出去或回應看不懂一律 `unknown`（不是失敗、也不是成功）。
 */
export async function sendCharacterTouch(
  kind: TouchKind = "tap",
  nowMs: number = Date.now()
): Promise<TouchOutcome> {
  if (inFlight) return "not-sent";
  const envelope = buildTouchEnvelope(nowMs, kind);
  const valid = validateEnvelope(envelope);
  if (!valid.ok) return "not-sent";
  inFlight = true;
  try {
    const result = await api.characterSessionEvent(envelope);
    const payload = result?.payload;
    const status =
      payload && typeof payload === "object" && !Array.isArray(payload)
        ? (payload as Record<string, unknown>)["status"]
        : undefined;
    if (status === "applied" || status === "accepted" || status === "rejected" || status === "expired") {
      return status;
    }
    return "unknown";
  } catch {
    // Runtime 離線／路由不存在：不知道有沒有生效就說不知道，不假裝送到了。
    return "unknown";
  } finally {
    inFlight = false;
  }
}
