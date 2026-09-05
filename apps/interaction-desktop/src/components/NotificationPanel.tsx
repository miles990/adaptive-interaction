// 右上角通知中心（只列待決定）與它的徽章文字。
//
// 誠實：徽章的數字來自截斷前的全量 `pendingCount`；後端說 `pendingCountExact:
// false` 時它只是下限，一律說「至少 N」，這一頁裝不下的也照實說「還有 N 項」。

import { Badge } from "../ui";
import { useFocusTrap } from "./Dialog";
import { decisionPage } from "../pages/ConnectPage";
import {
  inboxItemTitle,
  isPendingCountExact,
  PENDING_INCOMPLETE_NOTE,
  pendingCountLabel,
  projectInboxStatus,
} from "../statusProjection";

/** 收件匣狀態的人話：走共用的狀態投影（statusProjection.ts），與 AiPage／
 *  HomePage／收件匣／全域搜尋同一份文案。未知狀態不回原始字串，
 *  投影成「結果不確定」——不假裝看得懂，也不把 enum 外洩到一般模式。 */
export function inboxStatusLabel(status: string): string {
  return projectInboxStatus(status).label;
}

/** 右上角徽章的數字。後端說 `pendingCountExact: false` 時 pendingCount 只是
 *  下限，徽章要說「至少 N」——不得讓使用者以為那就是全部。 */
export function inboxBadgeText(inbox: Record<string, unknown> | null): string {
  const raw = inbox?.pendingCount;
  const count = typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : 0;
  return isPendingCountExact(inbox) ? String(count) : `至少 ${count}`;
}

/** 徽章的螢幕閱讀器說明（同一份真相，含「至少」）。 */
export function inboxBadgeLabel(inbox: Record<string, unknown> | null): string {
  if (!inbox) return "未知 項";
  const raw = inbox.pendingCount;
  const count = typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : 0;
  return pendingCountLabel(count, isPendingCountExact(inbox));
}

/** 右上角通知中心：與 Dialog 共用同一個焦點陷阱（Escape 關閉並還原焦點、
 *  Tab 在面板內循環），不是只能用滑鼠點的浮層。 */
export function NotificationPanel({
  inbox,
  onClose,
  onNavigate,
}: {
  inbox: Record<string, unknown> | null;
  onClose: () => void;
  onNavigate: (tab: string) => void;
}) {
  const { ref, onKeyDown } = useFocusTrap(onClose);
  // 徽章用的是截斷前的全量 pendingCount；本頁（最多 10 筆）裝不下的要照實說「還有 N 項」。
  const decisions = decisionPage(inbox, 10);
  return (
    // aria-modal="true" 現在是誠實的：套用跟 components/Dialog.tsx 同一套真 modal
    // 行為——共用的 .dialog-backdrop（點外面關閉）＋焦點陷阱＋Escape 關閉。修復前
    // 這裡沒有 backdrop，頂列的緊急停止按鈕在面板開著時仍可被滑鼠點到，但宣稱
    // aria-modal 會讓螢幕閱讀器使用者以為面板外的內容不存在，兩者行為不一致。
    // 现在跟 App 裡其餘的 Dialog（RecoveryDialog／CloseDialog／GlobalSearch）
    // 一樣：面板開著時 Escape／「關閉」隨時能立刻收起，不影響「隨時能停」。
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="notification-panel"
        role="dialog"
        aria-modal="true"
        aria-label="通知中心"
        tabIndex={-1}
        ref={ref}
        onKeyDown={onKeyDown}
      >
        <div className="row space-between">
          <strong>待你決定</strong>
          <button onClick={onClose}>關閉</button>
        </div>
        {!inbox ? (
          <div className="state-box state-error">目前無法確認通知狀態。</div>
        ) : decisions.shown.length === 0 && decisions.notShown === 0 && !decisions.exact ? (
          // 後端說 pendingCount 只是下限：這一頁空的不代表沒有待決定。
          <div className="state-box" role="status">
            {PENDING_INCOMPLETE_NOTE}。
          </div>
        ) : decisions.shown.length === 0 && decisions.notShown === 0 ? (
          <div className="state-box">目前沒有待決定事項。</div>
        ) : (
          <>
            {decisions.shown.length > 0 && (
              <ul className="plain-list">
                {decisions.shown.map((item) => (
                  <li
                    key={`${String(item.kind)}-${String(item.itemId)}`}
                    className="row space-between"
                  >
                    <span>
                      <Badge kind="warn">{inboxStatusLabel(String(item.status))}</Badge>{" "}
                      {inboxItemTitle(item)}
                    </span>
                    <button onClick={() => onNavigate(String(item.route))}>前往</button>
                  </li>
                ))}
              </ul>
            )}
            {decisions.notShown > 0 && (
              // 誠實：徽章數來自全量，這一頁裝不下（或舊 daemon 只給最近 20 筆）——
              // 不得宣稱「沒有待決定事項」。
              <div className="state-box" role="status">
                {decisions.exact ? "還有" : "至少還有"} {decisions.notShown}{" "}
                項待決定不在這一頁，前往活動歷史。
              </div>
            )}
            {decisions.notShown === 0 && !decisions.exact && (
              <div className="state-box" role="status">
                {PENDING_INCOMPLETE_NOTE}。
              </div>
            )}
          </>
        )}
        <button onClick={() => onNavigate("activity")}>查看完整活動歷史</button>
      </div>
    </div>
  );
}
