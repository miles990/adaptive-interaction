// 「沒有人確認的感測停止」（Runtime `sensor_source.rs` 的未解決停止）。
//
// 感測不靜默的另一半：來源被移除時還在擷取、之後沒有任何確認的那些擷取，不能
// 因為離開了即時清單就從畫面上消失。這一區只回答一件事——「有哪些擷取，我們
// 不知道它停了沒有」。
//
// 誠實：
// - 不說「已經停了」，也不說「還在感測」——不知道就說不知道；
// - 「我確認它已經停了」是**人類的**確認，二段確認的第二段一定要說出後端沒有
//   收到裝置的回覆；解除之後的回報也照樣說一次；
// - `sourceId`／`generation` 只拿去呼叫 API，不進畫面文字（X5）。

import React from "react";
import { api } from "../../api";
import { ConfirmButton } from "../../components/Dialog";
import { Icon } from "../../icons";
import {
  projectUnresolvedStops,
  UNRESOLVED_DISMISS_CONFIRM,
  UNRESOLVED_DISMISS_LABEL,
  UNRESOLVED_DISMISSED_MESSAGE,
} from "../../statusProjection";
import { useAsync } from "../../ui";

export function UnresolvedStopsSection({ refreshKey }: { refreshKey: number }) {
  const [unresolved, reload] = useAsync(() => api.sensorsUnresolved(), [refreshKey]);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const view = projectUnresolvedStops(unresolved.data ?? null);

  async function dismiss(sourceId: string, generation: number) {
    setBusy(true);
    setNotice(null);
    try {
      await api.sensorsDismissUnresolved(sourceId, generation);
      setNotice(UNRESOLVED_DISMISSED_MESSAGE);
    } catch (e) {
      // 解除失敗＝紀錄還在；不得靜默，也不得說成已經處理掉。
      setNotice(`沒有記下你的確認（${String(e)}）：這一筆還在，請再試一次。`);
    }
    setBusy(false);
    reload();
  }

  return (
    <div data-testid="unresolved-stops">
      <h3 className="connect-area-subhead">沒有人確認的感測停止</h3>
      {unresolved.loading && !unresolved.data ? (
        <div className="state-box">載入中…</div>
      ) : unresolved.error && !unresolved.data ? (
        <p className="muted small">讀不到「沒有人確認的感測停止」（稍後再試）。</p>
      ) : view.count === 0 ? (
        <p className="muted small">目前沒有這一類紀錄。</p>
      ) : (
        <>
          <p className="small" role="status">
            {view.summary}。
          </p>
          <p className="muted small">{view.note}</p>
          <ul className="connect-area-list">
            {view.items.map((item, index) => (
              <li key={`unresolved-${index}`} data-testid={`unresolved-stop-${index}`}>
                <Icon name="circle-help" size={14} />
                <span>{item.line}</span>
                <ConfirmButton
                  label={UNRESOLVED_DISMISS_LABEL}
                  confirmLabel={UNRESOLVED_DISMISS_CONFIRM}
                  disabled={busy}
                  onConfirm={() => {
                    void dismiss(item.sourceId, item.generation);
                  }}
                />
              </li>
            ))}
            {view.notShown > 0 && (
              <li className="muted small">…還有 {view.notShown} 筆沒有列出來。</li>
            )}
          </ul>
        </>
      )}
      {notice && (
        <p className="notice-box small" role="status">
          {notice}
        </p>
      )}
    </div>
  );
}
