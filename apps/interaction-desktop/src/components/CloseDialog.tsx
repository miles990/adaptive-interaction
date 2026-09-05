// 關閉控制中心的說明對話框（v0.2 → v0.3 行為改變的明確告知）。

import React from "react";
import { desktop } from "../desktop";
import { Dialog } from "./Dialog";

/** 第一次關閉控制中心的說明對話框（也是 v0.2 → v0.3 行為改變的明確告知）。 */
export function CloseDialog({ external, onClose }: { external: boolean; onClose: () => void }) {
  const [remember, setRemember] = React.useState(false);
  return (
    <Dialog title="關閉控制中心？" onClose={onClose}>
      <p>
        Adaptive Interaction 會繼續在<strong>狀態列</strong>運作。
        桌面角色與你允許的自動互動仍會保持啟用。
      </p>
      <p className="muted small">
        你可以從狀態列重新開啟控制中心，或選擇「完全結束」停止所有功能。
        {external && "（目前連線到外部系統：完全結束只會關閉這個視窗，不會停止那個系統。）"}
      </p>
      <p className="muted small">
        提醒：舊版（v0.2）關閉視窗會直接停止系統；新版預設改為保持在背景運作。
      </p>
      <label className="toggle">
        <input
          type="checkbox"
          checked={remember}
          onChange={(e) => setRemember(e.target.checked)}
        />
        <span>下次不再顯示</span>
      </label>
      <div className="row wrap" style={{ marginTop: 12 }}>
        <button
          className="primary"
          onClick={async () => {
            await desktop.closeDecision("keep-running", remember).catch(() => {});
            onClose();
          }}
        >
          保持運作
        </button>
        <button
          onClick={async () => {
            await desktop.closeDecision("quit", remember).catch(() => {});
            onClose();
          }}
        >
          完全結束
        </button>
      </div>
    </Dialog>
  );
}
