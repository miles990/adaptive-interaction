// 同意與安全：能力選擇器（不需輸入技術 scope）、表單化安全規則、
// 緊急停止的狀態與安全解除流程（與觸發按鈕分離、二次確認、不自動恢復）。

import React from "react";
import { api, HumanCard, Session } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Icon } from "../icons";
import { riskTierOfCard } from "../riskTier";
import { Badge, Section, useAsync } from "../ui";
import { ConfirmButton, Dialog } from "../components/Dialog";

export function SafetyPage({
  refreshKey,
  advanced = false,
  onNavigate,
}: {
  refreshKey: number;
  /** 進階模式才顯示原始技術識別（能力 id 等）；一般模式只說人話。 */
  advanced?: boolean;
  onNavigate?: (tab: string) => void;
}) {
  // 角色名稱一律走共用 hook（更換角色後跟著變；載入失敗顯示「角色」），不寫死。
  const { name } = useCharacterName();
  return (
    <div>
      <EmergencySection refreshKey={refreshKey} />
      <ConsentSection refreshKey={refreshKey} advanced={advanced} />
      <Section title="主動程度與安靜時段">
        <p className="muted small">
          AI 主動程度與安靜時段屬於{name}的表現設定，由「{name}」頁統一管理（單一主人，不放第二份開關）。
        </p>
        {onNavigate && <button onClick={() => onNavigate("companion")}>前往{name}</button>}
      </Section>
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
  // 「前往解除」把人導到這裡：解除入口要自己捲進視野並取得焦點，
  // 不能只是換頁後讓使用者自己找（每次掛載只做一次，不搶正在操作中的焦點）。
  const recoverRef = React.useRef<HTMLButtonElement | null>(null);
  const focused = React.useRef(false);
  React.useEffect(() => {
    if (!estop || focused.current) return;
    const el = recoverRef.current;
    if (!el) return;
    focused.current = true;
    el.scrollIntoView?.({ block: "center", behavior: "smooth" });
    el.focus?.();
  }, [estop]);

  const lastStop = (audit.data ?? []).find(
    (a) => (a as Record<string, unknown>)["kind"] === "emergency.stop"
  ) as Record<string, unknown> | undefined;

  return (
    <Section title="緊急停止">
      {estop ? (
        <div className="estop-panel" id="emergency-recovery">
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
          <button className="estop-recover" ref={recoverRef} onClick={() => setRecovery(true)}>
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
  // 對齊後端事實（runtime `clear_emergency_stop`）：解除只解開緊急停止的閂鎖，
  // 不會啟用任何東西、也不會重新授權。因此
  //   * 你自己停用的能力 → 解除後仍是停用（不得說成「會恢復可用」）；
  //   * 需同意的能力 → 解除後仍不可用，直到重新同意；
  //   * 其餘目前可用的能力 → 恢復「可用」，但仍受安全規則限制。
  const actuators = human?.actuators ?? [];
  const needsConsent = (a: HumanCard) => a.consent.required === true || a.requiresConsent === true;
  const isDisabled = (a: HumanCard) => a.availability === "disabled";
  const willResume = actuators.filter((a) => !needsConsent(a) && !isDisabled(a));
  const willNotResume = actuators.filter((a) => needsConsent(a) && !isDisabled(a));
  const stayDisabled = actuators.filter(isDisabled);
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
      {stayDisabled.length > 0 && (
        <>
          <p>以下能力你先前已停用，解除後仍為停用，要用得先到「回應方式」重新啟用：</p>
          <ul className="plain-list">
            {stayDisabled.map((a) => (
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

function ConsentSection({
  refreshKey,
  advanced = false,
}: {
  refreshKey: number;
  advanced?: boolean;
}) {
  const { human, findCard } = useAppState();
  const [session, reload] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [granting, setGranting] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);

  const s = session.data as Session | null | undefined;
  const active = (s?.consents ?? []).filter((c) => !c.revokedAt);

  return (
    <Section
      title="使用授權"
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
            const kind =
              c.scope.kind === "channel"
                ? "整個通道"
                : c.scope.kind === "toolOperation"
                  ? "工具操作"
                  : c.scope.kind === "receptor"
                    ? "感知來源"
                    : c.scope.kind === "actuator"
                      ? "回應方式"
                      : "授權項目";
            const card =
              c.scope.kind === "actuator"
                ? findCard("actuator", c.scope.id)
                : c.scope.kind === "receptor"
                  ? findCard("receptor", c.scope.id)
                  : c.scope.kind === "toolOperation"
                    ? findCard("tool", c.scope.id)
                    : null;
            const source =
              c.scope.kind === "actuator"
                ? human?.actuators.find((a) => a.id === c.scope.id)
                : c.scope.kind === "receptor"
                  ? human?.receptors.find((r) => r.id === c.scope.id)
                  : c.scope.kind === "toolOperation"
                    ? human?.toolOperations.find((t) => t.id === c.scope.id)
                    : undefined;
            const risk = source ? riskTierOfCard(source) : null;
            return (
              <li key={i} className="consent-item">
                <div>
                  <strong>
                    {source && card ? (
                      <>
                        <Icon name={card.icon} size={14} /> {card.name}
                      </>
                    ) : c.scope.kind === "channel" ? (
                      `整個通道${advanced ? `　${c.scope.id}` : ""}`
                    ) : !human ? (
                      `${kind}（名稱載入中）`
                    ) : (
                      // 能力清單裡查不到：不把原始 id 丟給一般模式的使用者（進階模式才附上）。
                      `${kind}（介面沒有這項能力的名稱）${advanced ? `　${c.scope.id}` : ""}`
                    )}
                  </strong>
                  <span className="muted small">
                    　{c.expiresAt
                      ? `有效至 ${new Date(c.expiresAt).toLocaleString()}`
                      : "整個工作階段有效"}
                  </span>
                  {risk && (
                    <div className="muted small">
                      <Badge
                        kind={
                          risk.tier >= 4
                            ? "bad"
                            : risk.tier === 3
                              ? "warn"
                              : risk.tier === 2
                                ? "info"
                                : "muted"
                        }
                      >
                        {risk.label}
                      </Badge>{" "}
                      {risk.policy}
                      {risk.hardLimits ? `　${risk.hardLimits}` : ""}
                    </div>
                  )}
                </div>
                <button
                  onClick={async () => {
                    try {
                      const scope = `${c.scope.kind === "toolOperation" ? "tool" : c.scope.kind}:${c.scope.id}`;
                      await api.consentRevoke(scope);
                      setMessage(
                        "已撤回。新的動作會立即被阻止；進行中的動作已要求取消（無法取消的會標示「結果未知」）。"
                      );
                    } catch (e) {
                      setMessage(`撤回失敗：${e}。授權狀態未變，請重試。`);
                    }
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

/** 「只這一次」在動器上現在是後端真的算得出來的單次授權：`maxUses: 1`，
 *  第一次成功派工就用掉（Rust Policy Governor 強制，不是畫面上的約定）。
 *  5 分鐘的有效期間留著當雙重保險——授權了卻一直沒用到也會自己失效。 */
export const SHORT_LIVED_CONSENT_MINUTES = 5;

/** 「只這一次」送給後端的次數上限。 */
export const ONE_SHOT_MAX_USES = 1;

/** 後端只在動器派工的路徑上真的把單次授權用掉（executor 的授權臨界區）。
 *  受器（麥克風／攝影機）與工具操作沒有等價的原子消耗點，那裡的「只這一次」
 *  仍然只是最短的有效期間——照實講，不假裝那是用完即失效。 */
export function oneShotIsEnforced(card: HumanCard | null): boolean {
  return card?.kind === "actuator";
}

/** 這張卡可以選的授權範圍。
 *  L4（攝影機、持續麥克風、定位、Agent 寫入檔案）的政策文字明寫「每次使用都要你
 *  同意（或只給短效授權）」——所以 L4 只提供短效選項，**不提供**整個工作階段，
 *  預設也一定是最短的那個；把 L4 預設成整個工作階段等於畫面說一套、做一套。 */
export function consentScopeOptions(card: HumanCard | null): { value: string; label: string }[] {
  const shortLived = {
    value: String(SHORT_LIVED_CONSENT_MINUTES),
    label: oneShotIsEnforced(card)
      ? `只這一次（用過一次即失效；${SHORT_LIVED_CONSENT_MINUTES} 分鐘內未使用也失效）`
      : `只這一次（${SHORT_LIVED_CONSENT_MINUTES} 分鐘內有效）`,
  };
  if (card && riskTierOfCard(card).tier >= 4) {
    return [shortLived, { value: "30", label: "30 分鐘" }];
  }
  return [
    shortLived,
    { value: "30", label: "30 分鐘" },
    { value: "120", label: "2 小時" },
    { value: "session", label: "整個工作階段" },
  ];
}

/** 預設選項：L4 一律是最短的短效授權；其餘維持原本的整個工作階段。 */
export function defaultConsentScope(card: HumanCard | null): string {
  return card && riskTierOfCard(card).tier >= 4
    ? String(SHORT_LIVED_CONSENT_MINUTES)
    : "session";
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
  const [expires, setExpires] = React.useState<string>(defaultConsentScope(null));
  const [error, setError] = React.useState<string | null>(null);
  return (
    <Dialog title="授予新權限" onClose={onClose}>
      {cards.length === 0 ? (
        <p className="muted">目前沒有需要同意的能力。</p>
      ) : !selected ? (
        <ul className="grant-list">
          {cards.map((c) => (
            <li key={`${c.kind}-${c.id}`}>
              <button
                className="grant-choice"
                onClick={() => {
                  setSelected(c);
                  // 預設範圍跟著這張卡的風險分級走（L4 一律短效）。
                  setExpires(defaultConsentScope(c));
                }}
              >
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
          {(() => {
            const risk = riskTierOfCard(selected);
            return (
              <p className="muted small">
                <Badge
                  kind={risk.tier >= 4 ? "bad" : risk.tier === 3 ? "warn" : risk.tier === 2 ? "info" : "muted"}
                >
                  {risk.label}
                </Badge>{" "}
                {risk.policy}
                {risk.hardLimits ? `　${risk.hardLimits}` : ""}
              </p>
            );
          })()}
          {selected.riskNote && (
            <p className="risk-note">
              <Icon name="triangle-alert" size={14} /> {selected.riskNote}
            </p>
          )}
          <label className="row">
            有效期間：
            <select value={expires} onChange={(e) => setExpires(e.target.value)}>
              {consentScopeOptions(selected).map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          {riskTierOfCard(selected).tier >= 4 && (
            <p className="muted small">
              這是高敏感能力：只能給短效授權，不提供「整個工作階段」。
              {oneShotIsEnforced(selected)
                ? "用過一次或時間到就自動失效，要再用會再問你一次。"
                : "時間到就自動失效，要再用會再問你一次。"}
            </p>
          )}
          {error && <p className="cap-card-error" role="alert">{error}</p>}
          <div className="row wrap" style={{ marginTop: 10 }}>
            <button
              onClick={async () => {
                try {
                  const scope = `${selected.kind === "tool-operation" ? "tool" : selected.kind}:${selected.id}`;
                  // 保險：L4 永遠不會送出「沒有到期時間」的同意，即使選單被繞過。
                  const allowed = consentScopeOptions(selected).some((o) => o.value === expires)
                    ? expires
                    : defaultConsentScope(selected);
                  // 「只這一次」＝ maxUses 1（後端用掉即失效）＋短效 TTL 雙重保險。
                  // 只在後端真的會消耗的範圍送次數，其餘維持純 TTL（別讓畫面
                  // 承諾後端沒有強制的事）。
                  const oneShot =
                    allowed === String(SHORT_LIVED_CONSENT_MINUTES) && oneShotIsEnforced(selected);
                  await api.consentGrant(
                    scope,
                    allowed === "session" ? undefined : Number(allowed),
                    oneShot ? ONE_SHOT_MAX_USES : undefined
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
