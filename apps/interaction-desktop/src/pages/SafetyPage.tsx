// 同意與安全：能力選擇器（不需輸入技術 scope）、表單化安全規則、
// 緊急停止的狀態與安全解除流程（與觸發按鈕分離、二次確認、不自動恢復）。

import React from "react";
import { api, HumanCard, Session } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";
import { Section, Toggle, useAsync } from "../ui";
import { ConfirmButton, Dialog } from "../components/Dialog";

export function SafetyPage({ refreshKey }: { refreshKey: number }) {
  return (
    <div>
      <EmergencySection refreshKey={refreshKey} />
      <ConsentSection refreshKey={refreshKey} />
      <PolicySection refreshKey={refreshKey} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// 緊急停止與安全恢復
// ---------------------------------------------------------------------------

function EmergencySection({ refreshKey }: { refreshKey: number }) {
  const [status, reloadStatus] = useAsync(() => api.status(), [refreshKey]);
  const [audit] = useAsync(() => api.auditTail(50), [refreshKey]);
  const [recovery, setRecovery] = React.useState(false);
  const estop = Boolean(status.data?.["emergencyStop"]);

  const lastStop = (audit.data ?? []).find(
    (a) => (a as Record<string, unknown>)["kind"] === "emergency.stop"
  ) as Record<string, unknown> | undefined;

  return (
    <Section title="緊急停止">
      {estop ? (
        <div className="estop-panel">
          <p className="home-status-line bad">
            <Icon name="octagon-x" size={18} /> 緊急停止已啟動。所有回應方式已停止，未完成的動作已中止。
          </p>
          {lastStop && (
            <p className="muted small">
              啟動時間：{String(lastStop["at"] ?? "")}
              {(() => {
                const detail = lastStop["detail"] as Record<string, unknown> | undefined;
                const reason = detail?.["reason"];
                return reason ? `　原因：${String(reason)}` : "";
              })()}
            </p>
          )}
          <p className="muted small">
            解除後：一般的回應方式（對話、紀錄）會恢復可用；
            高風險與實體能力<strong>不會自動恢復</strong>，需要重新啟用並重新取得同意。
          </p>
          <button className="estop-recover" onClick={() => setRecovery(true)}>
            開始安全解除流程…
          </button>
        </div>
      ) : (
        <p className="muted">
          <Icon name="shield-check" size={16} /> 緊急停止未啟動。右上角的紅色按鈕隨時可以立即停止一切
          —— 它不經過任何佇列，也不依賴 AI。
        </p>
      )}
      {recovery && (
        <RecoveryDialog
          onClose={() => setRecovery(false)}
          onCleared={() => {
            setRecovery(false);
            reloadStatus();
          }}
        />
      )}
    </Section>
  );
}

function RecoveryDialog({ onClose, onCleared }: { onClose: () => void; onCleared: () => void }) {
  const { human } = useAppState();
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const willResume = (human?.actuators ?? []).filter(
    (a) => a.consent.required !== true && a.effect?.physicalEffect !== true
  );
  const willNotResume = (human?.actuators ?? []).filter(
    (a) => a.consent.required === true || a.effect?.physicalEffect === true
  );
  return (
    <Dialog title="解除緊急停止" onClose={onClose} danger>
      <p>解除後，以下能力會恢復「可用」（仍受安全規則限制）：</p>
      <ul className="plain-list">
        {willResume.map((a) => (
          <li key={a.id}>
            <Icon name={a.icon} size={14} /> {a.displayName}
          </li>
        ))}
      </ul>
      {willNotResume.length > 0 && (
        <>
          <p>以下能力「不會」自動恢復，需要你重新啟用／重新同意：</p>
          <ul className="plain-list">
            {willNotResume.map((a) => (
              <li key={a.id}>
                <Icon name={a.icon} size={14} /> {a.displayName}
              </li>
            ))}
          </ul>
        </>
      )}
      {error && <p className="cap-card-error" role="alert">{error}</p>}
      <div className="row wrap" style={{ marginTop: 12 }}>
        <ConfirmButton
          label="我了解，解除緊急停止"
          confirmLabel="確定解除？"
          disabled={working}
          onConfirm={async () => {
            setWorking(true);
            setError(null);
            try {
              await api.emergencyStopClear();
              onCleared(); // 後端確認後才收起
            } catch (e) {
              setError(String(e));
            } finally {
              setWorking(false);
            }
          }}
        />
        <button onClick={onClose}>取消</button>
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// 使用授權（Consent）：能力選擇器
// ---------------------------------------------------------------------------

function ConsentSection({ refreshKey }: { refreshKey: number }) {
  const { human, findCard } = useAppState();
  const [session, reload] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [granting, setGranting] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);

  const s = session.data as Session | null | undefined;
  const active = (s?.consents ?? []).filter((c) => !c.revokedAt);

  return (
    <Section
      title="使用授權（Consent）"
      actions={
        s ? <button onClick={() => setGranting(true)}>授予新權限…</button> : undefined
      }
    >
      {!s ? (
        <p className="muted">
          還沒有工作階段。授權屬於工作階段：先在首頁開始一個，結束時授權自動失效。
        </p>
      ) : active.length === 0 ? (
        <p className="muted">目前沒有額外授權。需要同意的能力在使用前會先徵求你的同意。</p>
      ) : (
        <ul className="consent-list">
          {active.map((c, i) => {
            const kind = c.scope.kind === "channel" ? "整個通道" : "";
            const card =
              c.scope.kind === "actuator"
                ? findCard("actuator", c.scope.id)
                : c.scope.kind === "receptor"
                  ? findCard("receptor", c.scope.id)
                  : null;
            return (
              <li key={i} className="consent-item">
                <div>
                  <strong>
                    {card ? (
                      <>
                        <Icon name={card.icon} size={14} /> {card.name}
                      </>
                    ) : (
                      `${kind} ${c.scope.id}`
                    )}
                  </strong>
                  <span className="muted small">
                    　{c.expiresAt
                      ? `有效至 ${new Date(c.expiresAt).toLocaleString()}`
                      : "整個工作階段有效"}
                  </span>
                </div>
                <button
                  onClick={async () => {
                    const scope = `${c.scope.kind === "toolOperation" ? "tool" : c.scope.kind}:${c.scope.id}`;
                    await api.consentRevoke(scope);
                    setMessage(
                      "已撤回。新的動作會立即被阻止；進行中的動作已要求取消（無法取消的會標示「結果未知」）。"
                    );
                    reload();
                  }}
                >
                  撤回
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {message && <p className="notice-box" role="status">{message}</p>}
      {granting && human && (
        <GrantDialog
          cards={[...human.actuators, ...human.receptors].filter((c) => c.consent.required === true)}
          onClose={() => setGranting(false)}
          onGranted={() => {
            setGranting(false);
            reload();
          }}
        />
      )}
    </Section>
  );
}

function GrantDialog({
  cards,
  onClose,
  onGranted,
}: {
  cards: HumanCard[];
  onClose: () => void;
  onGranted: () => void;
}) {
  const [selected, setSelected] = React.useState<HumanCard | null>(null);
  const [expires, setExpires] = React.useState<string>("session");
  const [error, setError] = React.useState<string | null>(null);
  return (
    <Dialog title="授予新權限" onClose={onClose}>
      {cards.length === 0 ? (
        <p className="muted">目前沒有需要同意的能力。</p>
      ) : !selected ? (
        <ul className="grant-list">
          {cards.map((c) => (
            <li key={`${c.kind}-${c.id}`}>
              <button className="grant-choice" onClick={() => setSelected(c)}>
                <Icon name={c.icon} size={18} />
                <span>
                  <strong>{c.displayName}</strong>
                  <span className="muted small">
                    {c.shortDescription ?? c.conservativeNotice}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <div>
          <p>
            <Icon name={selected.icon} size={18} /> <strong>{selected.displayName}</strong>
          </p>
          <p className="muted small">{selected.consent.reason ?? selected.shortDescription}</p>
          {selected.riskNote && (
            <p className="risk-note">
              <Icon name="triangle-alert" size={14} /> {selected.riskNote}
            </p>
          )}
          <label className="row">
            有效期間：
            <select value={expires} onChange={(e) => setExpires(e.target.value)}>
              <option value="30">30 分鐘</option>
              <option value="120">2 小時</option>
              <option value="session">整個工作階段</option>
            </select>
          </label>
          {error && <p className="cap-card-error" role="alert">{error}</p>}
          <div className="row wrap" style={{ marginTop: 10 }}>
            <button
              onClick={async () => {
                try {
                  const scope = `${selected.kind === "tool-operation" ? "tool" : selected.kind}:${selected.id}`;
                  await api.consentGrant(
                    scope,
                    expires === "session" ? undefined : Number(expires)
                  );
                  onGranted();
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              同意
            </button>
            <button onClick={() => setSelected(null)}>返回</button>
          </div>
        </div>
      )}
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// 安全規則（Policy）— 表單版
// ---------------------------------------------------------------------------

function PolicySection({ refreshKey }: { refreshKey: number }) {
  const [policy, reload] = useAsync(() => api.policyGet(), [refreshKey]);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);

  async function patch(p: Record<string, unknown>) {
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      await api.policyPatch(p);
      setSaved(true);
      reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  if (policy.loading) return <Section title="安全規則"><div className="state-box">載入中…</div></Section>;
  if (policy.error)
    return (
      <Section title="安全規則">
        <div className="state-box state-error">{policy.error}</div>
      </Section>
    );
  const p = policy.data!;
  const quiet = (p["quietHours"] as { start: string; end: string }[] | undefined)?.[0];
  const initiative = String(p["initiative"] ?? "suggest");

  return (
    <Section title="安全規則">
      <p className="muted small">
        這些規則由本機的安全層強制執行；AI 的任何建議都只能在這個範圍內生效，你的設定只會收緊、不會放寬硬性上限。
      </p>
      <div className="policy-form">
        <fieldset>
          <legend>AI 主動程度</legend>
          {[
            ["passive", "只在我要求時"],
            ["suggest", "重要時提醒（預設）"],
            ["active", "可以主動協助"],
          ].map(([v, label]) => (
            <label key={v} className="radio-row">
              <input
                type="radio"
                name="initiative"
                checked={initiative === v}
                onChange={() => patch({ initiative: v })}
              />
              {label}
            </label>
          ))}
        </fieldset>

        <fieldset>
          <legend>安靜時段</legend>
          <QuietHoursEditor
            value={quiet}
            onChange={(q) => patch({ quietHours: q ? [q] : [] })}
          />
        </fieldset>
      </div>
      {saving && <p className="muted small">儲存中…</p>}
      {saved && !saving && <p className="muted small" role="status">已儲存，立即生效。</p>}
      {error && <p className="cap-card-error" role="alert">{error}</p>}
    </Section>
  );
}

function QuietHoursEditor({
  value,
  onChange,
}: {
  value?: { start: string; end: string };
  onChange: (q: { start: string; end: string; silencedChannels: string[] } | null) => void;
}) {
  const [enabled, setEnabled] = React.useState(Boolean(value));
  const [start, setStart] = React.useState(value?.start ?? "22:00");
  const [end, setEnd] = React.useState(value?.end ?? "08:00");
  React.useEffect(() => {
    setEnabled(Boolean(value));
    if (value) {
      setStart(value.start);
      setEnd(value.end);
    }
  }, [value?.start, value?.end]);
  return (
    <div className="row wrap">
      <Toggle
        checked={enabled}
        onChange={(on) => {
          setEnabled(on);
          onChange(on ? { start, end, silencedChannels: [] } : null);
        }}
        label={enabled ? "已啟用" : "未啟用"}
      />
      {enabled && (
        <>
          <label>
            從
            <input
              type="time"
              value={start}
              onChange={(e) => setStart(e.target.value)}
              onBlur={() => onChange({ start, end, silencedChannels: [] })}
            />
          </label>
          <label>
            到
            <input
              type="time"
              value={end}
              onChange={(e) => setEnd(e.target.value)}
              onBlur={() => onChange({ start, end, silencedChannels: [] })}
            />
          </label>
          <span className="muted small">期間聲音、震動、通知等干擾通道會被消音</span>
        </>
      )}
    </div>
  );
}
