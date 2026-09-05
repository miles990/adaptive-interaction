// 安靜與勿擾的實際影響（M3 §4.1）：把六組彼此**不同**的「安靜」語意攤開列出，
// 每一項標示它由哪個底層設定控制、現在的有效狀態是什麼。
//
// 刻意**不**合併成一個布林：
//   - 安全提示（緊急停止中、被阻擋、結果不確定）：固定文字，沒有任何設定可以關掉。
//   - 感測提示：只要感測使用中就一定顯示（感測不靜默）。
//   - 視覺陪伴：勿擾（桌面偏好）＋桌寵右鍵設的本機安靜期（唯讀來源，可在這裡清掉）。
//   - 主動說話：主動式對話的模式（後端強制）＋它自己的安靜期＋「勿擾時段延後非必要訊息」。
//   - 工作通知：安靜時段（本機安全層真的擋下動器）。
// 這是純呈現元件：所有狀態文字由呼叫端算好傳進來，這裡不碰任何設定。


export interface QuietImpactItem {
  /** 穩定代號：safety／sensing／companion／proactive／notifications。 */
  id: string;
  label: string;
  /** 由哪個設定控制（人話，對得上畫面上的控制項名稱）。 */
  source: string;
  /** 現在的有效狀態。 */
  state: string;
  /** 同一項底下的其它獨立事實（例如本機安靜期、延後非必要訊息）——不併進 state。 */
  notes?: string[];
}

export function QuietImpactList({ items }: { items: QuietImpactItem[] }) {
  return (
    <ul className="plain-list character-quiet-list" aria-label="安靜與勿擾的實際影響">
      {items.map((item) => (
        <li key={item.id} data-quiet-item={item.id}>
          <strong className="character-quiet-label">{item.label}</strong>
          <span className="muted small" data-quiet-source="">
            由「{item.source}」控制
          </span>
          <span className="small" data-quiet-state="">
            現在：{item.state}
          </span>
          {(item.notes ?? []).map((note) => (
            <span className="muted small character-quiet-note" key={note}>
              {note}
            </span>
          ))}
        </li>
      ))}
    </ul>
  );
}
