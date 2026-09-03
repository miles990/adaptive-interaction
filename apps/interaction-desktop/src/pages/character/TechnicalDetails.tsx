// 技術資料（只在進階模式渲染；預設收合）：安全宣告（執行方式、可執行程式、需要網路、
// 檔案存取、簽章）、manifest 原文、schema 版本、引擎／entrypoint、adapter 種類、通道、
// 狀態、能力、資源上限、Runtime 實例與 Behavior State 數值。
// 一般模式永遠不 import／不渲染這個元件。

import type { CharacterInstanceView } from "../../api";
import { JsonView } from "../../ui";
import type { CharacterCard } from "./catalog";

function entrypointText(card: CharacterCard): string {
  const ep = card.manifest?.entrypoint;
  if (!ep) return card.entrypoint;
  switch (ep.kind) {
    case "builtin":
      return `builtin:${ep.id}`;
    case "module":
      return `module:${ep.path}`;
    case "process":
      return `process:${ep.command.join(" ")}`;
    default:
      return `url:${ep.url}`;
  }
}

export function TechnicalDetails({
  card,
  instance,
  presence,
}: {
  card: CharacterCard | null;
  instance: CharacterInstanceView | null;
  presence: Record<string, unknown> | null;
}) {
  const m = card?.manifest ?? null;
  const state = (presence?.behaviorState as Record<string, unknown> | null | undefined) ?? null;
  const percent = (key: string) => Math.round(Math.max(0, Math.min(1, Number(state?.[key] ?? 0))) * 100);
  return (
    <details className="tech-details character-tech">
      <summary>技術資料</summary>
      {!card ? (
        <p className="muted small">角色資料尚未載入。</p>
      ) : (
        <>
          <h4>安全宣告</h4>
          {card.technical.length > 0 ? (
            <ul className="plain-list small" aria-label="角色安全宣告">
              {card.technical.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          ) : (
            <p className="muted small">這個角色沒有可轉述的宣告（角色資料尚未載入）。</p>
          )}
          <dl className="definition-list small">
          <dt>characterId</dt>
          <dd>{card.characterId}</dd>
          <dt>schemaVersion／版本</dt>
          <dd>
            {m?.schemaVersion ?? "—"}／{card.version ?? "—"}
          </dd>
          <dt>adapterKind／entrypoint</dt>
          <dd>
            {m?.adapterKind ?? (card.flags.external ? "external" : "in-process")}／{entrypointText(card)}
          </dd>
          <dt>channels</dt>
          <dd>{m ? m.channels.join(", ") || "—" : "—"}</dd>
          <dt>states／intents</dt>
          <dd>
            {m ? `${m.states.length} 個狀態／${m.intents.length} 個 intent` : "—"}
          </dd>
          <dt>capabilities</dt>
          <dd>{m ? Object.keys(m.capabilities).join(", ") || "—" : "—"}</dd>
          <dt>inputCapabilities</dt>
          <dd>{m ? Object.keys(m.inputCapabilities).join(", ") || "—" : "—"}</dd>
          <dt>securityRequirements</dt>
          <dd>{m ? JSON.stringify(m.securityRequirements) : "—"}</dd>
          <dt>resourceLimits</dt>
          <dd>{m ? JSON.stringify(m.resourceLimits) : "—"}</dd>
          <dt>report</dt>
          <dd>
            {card.report
              ? `flags ${JSON.stringify(card.report.flags)}; warnings ${card.report.warnings.length}; newerMinor ${String(card.report.newerMinor)}`
              : "—"}
          </dd>
          <dt>index hints</dt>
          <dd>
            assetBase {card.assetBase ?? "—"}；persona {card.persona ?? "—"}；story {card.story ?? "—"}
          </dd>
          <dt>Runtime instance</dt>
          <dd>
            {instance
              ? `${instance.instanceId} generation ${instance.generation} lifecycle ${instance.lifecycle} connected ${String(instance.connected)} negotiated ${String(instance.negotiated)} pending ${instance.pending} origin ${instance.origin}`
              : "尚未 hello"}
          </dd>
          <dt>presentation</dt>
          <dd>
            connected {String(presence?.connected === true)}；visible {String(presence?.visible === true)}；pendingCommands{" "}
            {String(presence?.pendingCommands ?? 0)}
          </dd>
          </dl>
        </>
      )}
      <h4>Behavior State</h4>
      {!state ? (
        <p className="muted small">尚未收到角色視窗的即時狀態（角色隱藏、離線或剛啟動時不會用預設值冒充）。</p>
      ) : (
        <div className="settings-grid" aria-label="角色行為狀態數值">
          {(
            [
              ["activation", "activation"],
              ["attention", "attention"],
              ["taskLoad", "taskLoad"],
              ["interactionReadiness", "interactionReadiness"],
              ["familiarity", "familiarity"],
            ] as const
          ).map(([key, label]) => (
            <div className="field-label" key={key}>
              <span>
                {label}：{percent(key)}%
              </span>
              <progress value={percent(key)} max={100} aria-label={label} />
            </div>
          ))}
          <p className="muted small">
            base {String(state.base)}；transient {String(state.transient ?? "—")}；focus {String(state.currentFocus ?? "—")}；
            recentInterruptions {Number(state.recentInterruptions ?? 0).toFixed(1)}
          </p>
        </div>
      )}
      {m && (
        <>
          <h4>manifest JSON</h4>
          <JsonView value={m} />
        </>
      )}
    </details>
  );
}
