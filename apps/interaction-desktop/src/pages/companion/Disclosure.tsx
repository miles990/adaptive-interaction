// 按需展開的區塊（M3 §4.1）：原生 <details>／<summary>——鍵盤可達、有可及名稱、
// 沒有任何 transition／animation（Reduced Motion 下也不會動）。
//
// `summary` 是「收起來也看得到」的那一行：收合摘要必須帶著**有效值**
//（例如費用／次數上限、目前的安靜狀態），收起數字調校不等於藏起使用成本。

import React from "react";

export function Disclosure({
  id,
  title,
  summary,
  children,
}: {
  /** 穩定的區塊代號（測試與樣式用；不是使用者看得到的文字）。 */
  id: string;
  title: string;
  /** 收合時仍看得到的一行狀態；沒有就不畫。 */
  summary?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <details className="character-disclosure" data-disclosure={id}>
      <summary className="character-disclosure-summary">
        <span className="character-disclosure-title">{title}</span>
        {summary !== undefined && summary !== null && (
          <span className="muted small character-disclosure-line">{summary}</span>
        )}
      </summary>
      <div className="character-disclosure-body">{children}</div>
    </details>
  );
}
