// 備份與還原（更多 → 備份與還原）：可讀的備份檔、逐筆重新驗證的還原。
// 這是匯出／還原唯一的家——「記憶與資料」只留一個指路按鈕，不放第二份控制項。
//
// 誠實原則：
// - 匯出結果一定呈現在畫面上（不是只寫 console），否則不得宣稱「已匯出」。
// - 匯出的**範圍**要說清楚：這裡只有記憶，不含知識、素材與角色互動記憶；
//   後端單次上限 1,000 條，達到上限時必須明說「較舊的沒有匯出」——
//   按鈕叫「匯出全部」＋只回「已匯出 N 條」，會讓使用者以為手上是完整備份。
// - 還原不信任備份檔裡的身分、時間與狀態：每一筆都以「目前的你明確匯入」重新經過
//   Runtime 驗證並取得新 ID；中途失敗會照實說已經寫進去幾筆，不假裝整批原子性。
// - 檔案大小與筆數有上限（5 MiB／1,000 筆），超過直接拒絕，不做無界迴圈。

import React from "react";
import { api } from "../api";
import { Section } from "../ui";

/** 備份檔上限：避免一次把整台機器的記憶塞進前端逐筆重放。 */
export const MAX_BACKUP_BYTES = 5 * 1024 * 1024;
export const MAX_BACKUP_ITEMS = 1000;

export function BackupSection({ onNavigate }: { onNavigate?: (tab: string) => void }) {
  const [notice, setNotice] = React.useState<string | null>(null);
  const [failed, setFailed] = React.useState(false);
  // 匯出結果必須真的呈現在畫面上（不是只丟 devtools console）才可宣稱「已在下方顯示」。
  const [exported, setExported] = React.useState<Record<string, unknown> | null>(null);

  const report = (message: string, ok: boolean) => {
    setNotice(message);
    setFailed(!ok);
  };

  const restoreBackup = async (file: File | undefined) => {
    if (!file) return;
    if (file.size > MAX_BACKUP_BYTES) {
      report("還原失敗：備份檔超過 5 MiB 安全上限。", false);
      return;
    }
    let restored = 0;
    try {
      const parsed = JSON.parse(await file.text()) as Record<string, unknown>;
      const items = Array.isArray(parsed.items) ? parsed.items : null;
      if (!items || items.length > MAX_BACKUP_ITEMS) {
        throw new Error("格式不符或超過 1,000 條上限");
      }
      for (const raw of items) {
        if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
          throw new Error(`第 ${restored + 1} 條不是物件`);
        }
        const source = raw as Record<string, unknown>;
        // 不信任備份中的身分、時間與狀態；每一筆都以目前人類明確匯入，
        // 重新經過 Runtime schema、Secret 與 retention 驗證，並取得新 ID。
        await api.memoryCreate({
          layer: source.layer,
          kind: source.kind,
          title: source.title,
          content: source.content,
          provenance: source.provenance,
          confidence: source.confidence,
          tags: source.tags,
          agentVisibility: source.agentVisibility,
          agentDenylist: source.agentDenylist,
          retention: source.retention,
        });
        restored += 1;
      }
      report(`已還原 ${restored} 條；每一條都重新通過驗證並取得新編號。`, true);
    } catch (e) {
      report(`還原失敗：${e}。已成功寫入的 ${restored} 條會保留，請依訊息檢查備份。`, false);
    }
  };

  return (
    <div>
      <Section title="備份與還原">
        <p className="muted small">
          備份是可讀的純文字檔，你可以自行保存或檢查內容。還原時每一條都會重新經過安全
          檢查並取得新編號——不會沿用備份檔裡的來源、時間或狀態。
        </p>
        <p className="muted small">
          這個備份只含記憶：知識、素材與衍生資料，以及角色跟你相處累積的互動記憶
          都不在裡面（互動記憶在角色頁可以單獨清除）。單次最多匯出最近更新的 1,000 條，
          達到上限時會在下面明說。
        </p>
        <div className="row wrap">
          <button
            onClick={async () => {
              try {
                const out = (await api.memoryExport()) as Record<string, unknown>;
                setExported(out);
                // 後端說達到上限就照實轉述：靜默截斷的備份比沒有備份更危險。
                const capped = out.limitReached === true;
                const limit = Number(out.limit ?? 0) || 0;
                report(
                  capped
                    ? `已匯出 ${String(out.count)} 條記憶（內容已在下方顯示）。已達單次上限${
                        limit > 0 ? ` ${limit} 條` : ""
                      }：更舊的記憶沒有匯出，這不是完整備份。`
                    : `已匯出 ${String(out.count)} 條記憶（內容已在下方顯示，可自行複製保存）。不含知識、素材與角色互動記憶。`,
                  !capped
                );
              } catch (e) {
                setExported(null);
                report(`匯出失敗：${e}`, false);
              }
            }}
          >
            匯出記憶
          </button>
          <label className="button-like">
            還原備份
            <input
              className="visually-hidden"
              type="file"
              accept="application/json,.json"
              aria-label="選擇記憶備份檔"
              onChange={(event) => void restoreBackup(event.target.files?.[0])}
            />
          </label>
        </div>
        {notice && (
          <p className={failed ? "cap-card-error" : "muted small"} role={failed ? "alert" : "status"}>
            {notice}
          </p>
        )}
        {exported && (
          <div className="state-box">
            <div className="row space-between">
              <strong>匯出結果</strong>
              <button onClick={() => setExported(null)}>關閉</button>
            </div>
            <pre className="json-view small">{JSON.stringify(exported, null, 2)}</pre>
          </div>
        )}
      </Section>
      <Section title="刪除與保存期限">
        <p className="muted small">
          單筆刪除、保存期限修改與「清除短期記憶」都在「記憶與資料」；素材及其衍生物會在
          刪除前顯示影響預覽。重新執行首次設定不會清除既有資料。
        </p>
        {onNavigate && <button onClick={() => onNavigate("memory")}>前往記憶與資料</button>}
      </Section>
    </div>
  );
}
