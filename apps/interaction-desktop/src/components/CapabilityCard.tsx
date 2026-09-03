// One capability, human-first. Facts (data flow / impact) come from the
// backend resolver verbatim; this component never invents or upgrades them.

import React from "react";
import { api, HumanCard, Receipt } from "../api";
import { availabilityLabel, confirmationLabel, triLabel, useAppState } from "../appstate";
import { Icon } from "../icons";
import { Badge, JsonView } from "../ui";
import { riskTierOfCard } from "../riskTier";
import { projectInboxStatus } from "../statusProjection";
import { Dialog } from "./Dialog";

const MAX_CARD_BADGES = 4;

export function CapabilityCard({
  card,
  advanced,
  onChanged,
}: {
  card: HumanCard;
  advanced: boolean;
  onChanged: () => void;
}) {
  const [detail, setDetail] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [testResult, setTestResult] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const enabled = card.availability !== "disabled";
  const risk = riskTierOfCard(card);
  const kindLabel =
    card.kind === "receptor" ? "感知來源" : card.kind === "actuator" ? "回應方式" : "工具操作";

  async function setEnabled(next: boolean) {
    setBusy(true);
    setError(null);
    try {
      if (card.kind === "receptor") await api.setReceptorEnabled(card.id, next);
      else if (card.kind === "actuator") await api.setActuatorEnabled(card.id, next);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    setBusy(true);
    setError(null);
    setTestResult(null);
    try {
      if (card.kind === "receptor") {
        const obs = await api.testReceptor(card.id);
        const facts = JSON.stringify(obs["facts"] ?? {});
        setTestResult(`讀取成功：${facts.length > 120 ? facts.slice(0, 120) + "…" : facts}`);
      } else if (card.kind === "actuator") {
        const receipts = (await api.testActuator(card.id)) as Receipt[];
        const r = receipts[0];
        if (!r) {
          setTestResult("測試沒有產生任何動作。");
        } else {
          setTestResult(`測試結果：${honestStatus(r.currentStatus)}`);
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <article className={`cap-card${enabled ? "" : " cap-card-disabled"}`} aria-label={card.displayName}>
      <header className="cap-card-head">
        <Icon name={card.icon} size={22} className={`icon-${card.colorRole}`} />
        <div className="cap-card-title">
          <h3>{card.displayName}</h3>
          <span className="muted small">{kindLabel}</span>
        </div>
        <Badge kind={availabilityBadge(card.availability)}>{availabilityLabel(card.availability)}</Badge>
      </header>

      <p className="cap-card-desc">
        {card.shortDescription ?? card.conservativeNotice ?? "（沒有說明）"}
      </p>
      {card.undescribed && card.shortDescription && (
        <p className="cap-card-notice">{card.conservativeNotice}</p>
      )}

      <div className="cap-card-badges">
        <Badge kind={riskBadgeKind(risk.tier)}>{risk.label}</Badge>
        {card.badges.slice(0, MAX_CARD_BADGES).map((b) => (
          <Badge key={b.key} kind={toneToBadge(b.tone)}>
            {b.label}
          </Badge>
        ))}
      </div>
      <p className="muted small cap-card-risk">{risk.policy}</p>
      {risk.hardLimits && <p className="muted small cap-card-risk">{risk.hardLimits}</p>}

      {card.kind === "receptor" && card.data && (
        <dl className="cap-facts">
          <div>
            <dt>資料來源</dt>
            <dd>{sourceLabel(card.data.source)}</dd>
          </div>
          <div>
            <dt>個人資料</dt>
            <dd>{triLabel(card.data.personalData, "包含", "不包含", "未知")}</dd>
          </div>
          {(card.data.inferenceFields?.length ?? 0) > 0 && (
            <div>
              <dt>系統推測</dt>
              <dd>包含（會標示信心程度，不會當成事實）</dd>
            </div>
          )}
        </dl>
      )}

      {card.kind !== "receptor" && card.effect && (
        <dl className="cap-facts">
          <div>
            <dt>可以確認</dt>
            <dd>{confirmationLabel(card.effect.confirmationLevel).can || "未知"}</dd>
          </div>
          {confirmationLabel(card.effect.confirmationLevel).cannot && (
            <div>
              <dt>無法確認</dt>
              <dd>{confirmationLabel(card.effect.confirmationLevel).cannot}</dd>
            </div>
          )}
        </dl>
      )}

      {testResult && <p className="cap-card-result">{testResult}</p>}
      {error && <p className="cap-card-error" role="alert">操作失敗：{error}</p>}

      <footer className="cap-card-actions">
        {card.kind !== "tool-operation" && (
          <>
            <button onClick={runTest} disabled={busy || !enabled}>
              測試
            </button>
            <button onClick={() => setEnabled(!enabled)} disabled={busy}>
              {enabled ? "停用" : "啟用"}
            </button>
          </>
        )}
        <button onClick={() => setDetail(true)}>詳情</button>
      </footer>

      {detail && (
        <CapabilityDetail card={card} advanced={advanced} onClose={() => setDetail(false)} />
      )}
    </article>
  );
}

function CapabilityDetail({
  card,
  advanced,
  onClose,
}: {
  card: HumanCard;
  advanced: boolean;
  onClose: () => void;
}) {
  const { setCustomName } = useAppState();
  const [rename, setRename] = React.useState("");
  const prefKey = `${card.kind === "tool-operation" ? "tool" : card.kind}:${card.id}`;
  return (
    <Dialog title={card.displayName} onClose={onClose}>
      <p>{card.longDescription ?? card.shortDescription ?? card.conservativeNotice}</p>
      {card.aiDescription && (
        <p className="ai-assisted">
          <Badge kind="info">AI 補充說明</Badge> {card.aiDescription}
          <span className="muted small">（僅是說明潤飾，不改變上方的能力事實）</span>
        </p>
      )}
      {card.riskNote && (
        <p className="risk-note">
          <Icon name="triangle-alert" size={16} /> {card.riskNote}
        </p>
      )}
      <div className="cap-card-badges">
        <Badge kind={riskBadgeKind(riskTierOfCard(card).tier)}>{riskTierOfCard(card).label}</Badge>
        {card.badges.map((b) => (
          <Badge key={b.key} kind={toneToBadge(b.tone)}>
            {b.label}
          </Badge>
        ))}
      </div>
      <p className="muted small">{riskTierOfCard(card).policy}</p>
      {riskTierOfCard(card).hardLimits && (
        <p className="risk-note">
          <Icon name="triangle-alert" size={14} /> {riskTierOfCard(card).hardLimits}
        </p>
      )}

      {card.data && (
        <dl className="cap-facts">
          <div>
            <dt>資料流向</dt>
            <dd>{triLabel(card.data.leavesDevice, "會離開這台電腦", "僅限本機", "未知")}</dd>
          </div>
          <div>
            <dt>保存方式</dt>
            <dd>{retentionLabel(card.data.retention)}</dd>
          </div>
          {(card.data.factFields?.length ?? 0) > 0 && (
            <div>
              <dt>直接觀察</dt>
              <dd>{card.data.factFields!.join("、")}</dd>
            </div>
          )}
          {(card.data.inferenceFields?.length ?? 0) > 0 && (
            <div>
              <dt>系統推測</dt>
              <dd>{card.data.inferenceFields!.join("、")}</dd>
            </div>
          )}
        </dl>
      )}
      {card.effect && (
        <dl className="cap-facts">
          <div>
            <dt>影響外部服務</dt>
            <dd>{triLabel(card.effect.externalSideEffect, "會", "不會", "未知")}</dd>
          </div>
          <div>
            <dt>實體效果</dt>
            <dd>{triLabel(card.effect.physicalEffect, "有", "沒有", "未知")}</dd>
          </div>
          <div>
            <dt>可以撤銷</dt>
            <dd>{triLabel(card.effect.reversible, "可以", "無法復原", "未知")}</dd>
          </div>
        </dl>
      )}
      {card.consent.required === true && (
        <p className="muted">
          <Icon name="hand" size={16} /> 使用前需要你的同意
          {card.consent.reason ? `：${card.consent.reason}` : "。"}
        </p>
      )}

      <div className="row wrap" style={{ marginTop: 12 }}>
        <input
          placeholder="自訂顯示名稱（只改名稱，不改行為）"
          value={rename}
          onChange={(e) => setRename(e.target.value)}
          aria-label="自訂顯示名稱"
        />
        <button
          disabled={!rename.trim()}
          onClick={async () => {
            await setCustomName(prefKey, rename.trim());
            setRename("");
          }}
        >
          改名
        </button>
        {card.nameSource === "user" && (
          <button onClick={() => setCustomName(prefKey, null)}>還原預設名稱</button>
        )}
      </div>

      {advanced && (
        <details className="tech-details">
          <summary>技術詳細資料</summary>
          <table className="kv">
            <tbody>
              <tr>
                <td>技術 ID</td>
                <td>
                  <code>{card.id}</code>
                </td>
              </tr>
              {card.driver && (
                <tr>
                  <td>Driver</td>
                  <td>
                    <code>{card.driver}</code>
                  </td>
                </tr>
              )}
              {card.canonicalId && (
                <tr>
                  <td>目錄 ID</td>
                  <td>
                    <code>{card.canonicalId}</code>
                  </td>
                </tr>
              )}
              <tr>
                <td>Manifest hash</td>
                <td>
                  <code>{card.manifestHash}</code>
                </td>
              </tr>
            </tbody>
          </table>
          <JsonView value={card} />
        </details>
      )}
    </Dialog>
  );
}

function honestStatus(status: string): string {
  switch (status) {
    case "completed":
      return "已完成";
    case "accepted":
      return "已排入（尚未確認執行）";
    case "acknowledged":
      return "已收到（效果未確認）";
    case "blocked":
      return "被安全規則阻止";
    default:
      // uncertain 與介面不認得的狀態都走共用投影（→「結果不確定」，固定安全文字），
      // 不把原始字串當標籤。
      return projectInboxStatus(status).label;
  }
}

function availabilityBadge(a: string): string {
  switch (a) {
    case "available":
      return "ok";
    case "disabled":
      return "muted";
    case "offline":
      return "muted";
    case "degraded":
      return "warn";
    default:
      return "muted";
  }
}

/** 分級徽章色：L0/L1 中性、L2 資訊、L3 警示、L4 危險。 */
function riskBadgeKind(tier: number): string {
  if (tier >= 4) return "bad";
  if (tier === 3) return "warn";
  if (tier === 2) return "info";
  return "muted";
}

function toneToBadge(tone: string): string {
  switch (tone) {
    case "ok":
      return "ok";
    case "warn":
      return "warn";
    case "danger":
      return "bad";
    default:
      return "info";
  }
}

function sourceLabel(s: string): string {
  switch (s) {
    case "local":
      return "本機";
    case "device":
      return "外接裝置";
    case "external-service":
      return "外部服務";
    default:
      return "未知";
  }
}

function retentionLabel(r: string): string {
  switch (r) {
    case "none":
      return "不保存";
    case "session":
      return "只保存到工作階段結束";
    case "persistent":
      return "長期保存（可刪除）";
    default:
      return "未知";
  }
}
