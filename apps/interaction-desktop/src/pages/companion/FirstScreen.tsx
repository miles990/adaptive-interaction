// 首屏第一件事（M3 §4.1）：目前角色——名字、現在怎麼樣、預覽，以及顯示／暫停這個主要動作。
//
// 誠實：需要額外授權的角色在一般模式也要給一句人話；隱藏角色只是停掉角色視窗內的
// 感知與呈現，**不等於**緊急停止（這句固定文字不得移除）。
// 這是純呈現元件：狀態與寫入都由角色頁提供。

import React from "react";
import { Badge } from "../../ui";
import { Toggle } from "../../ui";
import { originLabel, type CharacterCard } from "../character/catalog";

export interface CharacterLiveView {
  kind: string;
  label: string;
  detail: string;
}

export function CurrentCharacterCard({
  name,
  active,
  advanced,
  live,
  explanation,
  summaryLines,
  extraPermission,
  catalogLoaded,
  visible,
  onVisibleChange,
  preview,
  error,
  notice,
}: {
  name: string;
  active: CharacterCard | null;
  advanced: boolean;
  live: CharacterLiveView;
  explanation: string;
  summaryLines: string[];
  /** 一般模式的一句人話（需要額外授權時）；不需要時傳 null。 */
  extraPermission: string | null;
  catalogLoaded: boolean;
  /** 桌面偏好尚未載入（瀏覽器檢視）時傳 null——不畫開關，也不假裝有狀態。 */
  visible: boolean | null;
  onVisibleChange: (on: boolean) => void;
  preview: React.ReactNode;
  error: string | null;
  notice: string | null;
}) {
  return (
    <div className="character-current">
      <div className="character-current-head">
        <h3 className="character-current-name">{name}</h3>
        {active && <Badge kind={active.origin === "builtin" ? "info" : "warn"}>{originLabel(active.origin)}</Badge>}
        {advanced && active?.flags.external && <Badge kind="warn">外部</Badge>}
        {advanced && active?.flags.executable && <Badge kind="bad">有可執行程式</Badge>}
        {advanced && active?.flags.network && <Badge kind="warn">需要網路</Badge>}
        <Badge kind={live.kind}>{live.label}</Badge>
      </div>
      <p className="muted small">{live.detail}</p>
      {/* 需要額外授權的事實在一般模式也不能藏，只是改成一句人話。 */}
      {!advanced && active && extraPermission && <p className="muted small">{extraPermission}</p>}
      {explanation.length > 0 && (
        <p className="small" role="status">
          現在：{explanation}
        </p>
      )}
      {active ? (
        <ul className="plain-list small character-summary" aria-label="角色能力摘要">
          {summaryLines.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      ) : catalogLoaded ? (
        <div className="state-box">找不到目前設定的角色資料；桌面角色視窗會改用文字顯示。</div>
      ) : (
        <div className="state-box">正在讀取角色資料…</div>
      )}
      {visible !== null && <Toggle checked={visible} onChange={onVisibleChange} label="顯示桌面角色" />}
      <p className="muted small">
        隱藏角色只會停止角色視窗內的感知與呈現；系統、狀態列與進行中的工作都會繼續。隱藏不等於緊急停止。
      </p>
      {preview}
      {error && (
        <p className="cap-card-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
    </div>
  );
}
