// 首次設定精靈：7 步、draft/commit。所有選擇最後一次性套用到後端
// （enable/disable、policy patch、starter recipes、偏好），中途關閉只留草稿，
// 不會留下半套用狀態。

import React from "react";
import { api, HardwareScanReport, HumanCard, OnboardingState } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";
import { Badge } from "../ui";

interface Draft {
  step: number;
  senses: string[];
  responses: string[];
  initiative: string;
  quietStart: string;
  quietEnd: string;
  quietEnabled: boolean;
  maxPerHour: number;
  starters: string[];
  askHighRisk: boolean;
}

const EMPTY_DRAFT: Draft = {
  step: 0,
  senses: [],
  responses: [],
  initiative: "suggest",
  quietStart: "22:00",
  quietEnd: "08:00",
  quietEnabled: true,
  maxPerHour: 6,
  starters: ["starter-task-complete"],
  askHighRisk: true,
};

const STEPS = ["歡迎", "感知來源", "回應方式", "工具操作", "互動偏好", "起始情境", "確認"];

export function Onboarding({ onDone, onSkip }: { onDone: () => void; onSkip: () => void }) {
  const { human, findCard } = useAppState();
  const [state, setState] = React.useState<OnboardingState | null>(null);
  const [draft, setDraft] = React.useState<Draft>(EMPTY_DRAFT);
  const [committing, setCommitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [hardwareScan, setHardwareScan] = React.useState<HardwareScanReport | null>(null);
  const [hardwareScanning, setHardwareScanning] = React.useState(false);
  const [hardwareScanError, setHardwareScanError] = React.useState<string | null>(null);

  const [loadError, setLoadError] = React.useState<string | null>(null);
  React.useEffect(() => {
    api.onboardingGet().catch((e) => {
      setLoadError(String(e));
      throw e;
    }).then((s) => {
      setState(s);
      const d = s.draft as Partial<Draft> | null | undefined;
      if (d && typeof d.step === "number") {
        setDraft({ ...EMPTY_DRAFT, ...d });
      } else if (human) {
        // 預選：只挑低風險、本機、推薦新手的能力。
        setDraft((prev) => ({
          ...prev,
          senses: human.receptors
            .filter((r) => beginnerSafe(r) && r.availability === "available")
            .map((r) => r.id),
          responses: human.actuators
            .filter((a) => beginnerSafe(a) && a.availability === "available")
            .map((a) => a.id),
        }));
      }
    });
  }, [human !== undefined]);

  function update(patch: Partial<Draft>) {
    setDraft((prev) => {
      const next = { ...prev, ...patch };
      api.onboardingDraft(next as unknown as Record<string, unknown>).catch(() => {});
      return next;
    });
  }

  async function commit() {
    if (!human) return;
    setCommitting(true);
    setError(null);
    try {
      const enableR = draft.senses;
      const disableR = human.receptors
        .filter((r) => !draft.senses.includes(r.id) && r.availability !== "disabled" && !r.requiresConsent)
        .map((r) => r.id);
      const enableA = draft.responses;
      const disableA = human.actuators
        .filter((a) => !draft.responses.includes(a.id) && a.availability !== "disabled" && !a.requiresConsent)
        .map((a) => a.id);
      await api.onboardingCommit({
        enableReceptors: enableR,
        disableReceptors: disableR,
        enableActuators: enableA,
        disableActuators: disableA,
        starterRecipes: draft.starters,
        policyPatch: {
          initiative: draft.initiative,
          quietHours: draft.quietEnabled
            ? [{ start: draft.quietStart, end: draft.quietEnd, silencedChannels: [] }]
            : [],
          channelLimits: { "*": { maxPerHour: draft.maxPerHour } },
          requireApprovalAt: draft.askHighRisk ? "high" : "critical",
        },
        preferences: { locale: "zh-TW" },
      });
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setCommitting(false);
    }
  }

  if (loadError)
    return (
      <div className="onboarding">
        <div className="onboarding-panel">
          <div className="state-box state-error">無法載入設定精靈：{loadError}</div>
          <div className="row" style={{ marginTop: 12 }}>
            <button onClick={() => window.location.reload()}>重試</button>
            <button onClick={onSkip}>略過，直接進入主畫面</button>
          </div>
        </div>
      </div>
    );
  if (!human || !state) return <div className="onboarding"><div className="state-box">載入中…</div></div>;

  const step = draft.step;

  return (
    <div className="onboarding" role="dialog" aria-label="首次設定">
      <div className="onboarding-panel">
        <nav className="onboarding-steps" aria-label="設定進度">
          {STEPS.map((s, i) => (
            <span key={s} className={i === step ? "step active" : i < step ? "step done" : "step"}>
              {s}
            </span>
          ))}
        </nav>

        {step === 0 && (
          <section>
            <h1>歡迎使用自適應互動</h1>
            <p>這個系統讓 AI 能在安全範圍內感知狀態並主動回應。四個核心概念：</p>
            <ul className="concept-list">
              <li>
                <Icon name="scan-eye" size={18} /> <strong>感知來源</strong> — AI 可以知道什麼
              </li>
              <li>
                <Icon name="send" size={18} /> <strong>回應方式</strong> — AI 可以怎麼回應
              </li>
              <li>
                <Icon name="wrench" size={18} /> <strong>工具操作</strong> — AI 可以讀取或改變什麼
              </li>
              <li>
                <Icon name="shield-check" size={18} /> <strong>安全規則</strong> — 什麼時候要停下或先問
              </li>
            </ul>
            <div className="notice-box">
              <p>幾個重要的保證：</p>
              <ul>
                <li>能力存在不等於 AI 自動獲得權限 — 每一項都由你決定。</li>
                <li>所有選擇都可以隨時停用或撤回。</li>
                <li>AI 不會因為安裝了程式就持續監聽你的資料。</li>
                <li>右上角的緊急停止隨時可以立即阻止所有互動。</li>
              </ul>
            </div>
          </section>
        )}

        {step === 1 && (
          <section>
            <h1>AI 可以知道什麼？</h1>
            <p className="muted">
              勾選你願意讓系統感知的資訊。高敏感來源（攝影機、麥克風、位置）預設不啟用，之後需要時再個別同意。
            </p>
            <div className="notice-box">
              <strong>掃描目前可用的互動能力</strong>
              <p className="muted small">
                掃描只讀取名稱、類型、穩定識別、權限需求與可用狀態，不會開啟攝影機、麥克風或開始原始感測；結果不代表找到所有硬體。
              </p>
              <button
                disabled={hardwareScanning}
                onClick={async () => {
                  setHardwareScanning(true);
                  setHardwareScanError(null);
                  try {
                    setHardwareScan(await api.hardwareScan());
                  } catch (e) {
                    setHardwareScanError(`掃描失敗：${String(e)}`);
                  } finally {
                    setHardwareScanning(false);
                  }
                }}
              >
                {hardwareScanning ? "掃描中…" : "掃描目前可用裝置"}
              </button>
              {hardwareScan && (
                <p className="muted small" role="status">
                  已偵測到目前可用裝置與能力，共 {hardwareScan.devices.length} 筆；
                  感測器啟動：{hardwareScan.sensorActivationAttempted ? "曾嘗試（異常）" : "否"}。
                  可在完成設定後到「能力與裝置」查看逐項結果、配對與授權狀態。
                </p>
              )}
              {hardwareScanError && <p className="cap-card-error" role="alert">{hardwareScanError}</p>}
            </div>
            <PickCards
              cards={human.receptors.filter(
                (r) => !r.requiresConsent && r.consent.required !== true
              )}
              selected={draft.senses}
              onChange={(senses) => update({ senses })}
            />
          </section>
        )}

        {step === 2 && (
          <PickStep
            title="AI 可以怎麼回應？"
            intro="勾選允許的回應方式。每張卡片標示干擾程度與影響範圍；實體與對外能力預設關閉。"
            cards={human.actuators.filter((a) => !a.requiresConsent && a.consent.required !== true)}
            selected={draft.responses}
            onChange={(responses) => update({ responses })}
          />
        )}

        {step === 3 && (
          <section>
            <h1>工具操作的界線</h1>
            <p className="muted">
              工具操作是 AI 可以讀取、建立或修改的軟體能力。讀取類操作風險低；
              對外寫入、刪除、金錢或訊息傳送屬於高風險。
            </p>
            <label className="radio-row">
              <input
                type="checkbox"
                checked={draft.askHighRisk}
                onChange={(e) => update({ askHighRisk: e.target.checked })}
              />
              高風險操作每次都先詢問我（建議保持開啟）
            </label>
            <p className="muted small">
              無論如何，「危險／不可回復」級的操作永遠需要明確確認 — 這條底線無法被關閉。
            </p>
          </section>
        )}

        {step === 4 && (
          <section>
            <h1>互動偏好</h1>
            <fieldset>
              <legend>AI 主動程度</legend>
              {[
                ["passive", "只在我要求時"],
                ["suggest", "重要時提醒（建議）"],
                ["active", "可以主動協助"],
              ].map(([v, label]) => (
                <label key={v} className="radio-row">
                  <input
                    type="radio"
                    name="ob-initiative"
                    checked={draft.initiative === v}
                    onChange={() => update({ initiative: v })}
                  />
                  {label}
                </label>
              ))}
            </fieldset>
            <fieldset>
              <legend>安靜時段</legend>
              <label className="radio-row">
                <input
                  type="checkbox"
                  checked={draft.quietEnabled}
                  onChange={(e) => update({ quietEnabled: e.target.checked })}
                />
                啟用安靜時段
              </label>
              {draft.quietEnabled && (
                <div className="row">
                  <input
                    type="time"
                    value={draft.quietStart}
                    aria-label="安靜開始"
                    onChange={(e) => update({ quietStart: e.target.value })}
                  />
                  <span>到</span>
                  <input
                    type="time"
                    value={draft.quietEnd}
                    aria-label="安靜結束"
                    onChange={(e) => update({ quietEnd: e.target.value })}
                  />
                </div>
              )}
            </fieldset>
            <fieldset>
              <legend>頻率上限</legend>
              <label className="row">
                每小時最多
                <input
                  type="number"
                  min={1}
                  max={60}
                  value={draft.maxPerHour}
                  aria-label="每小時最大互動次數"
                  onChange={(e) => update({ maxPerHour: Number(e.target.value) || 6 })}
                />
                次主動互動
              </label>
            </fieldset>
          </section>
        )}

        {step === 5 && (
          <section>
            <h1>起始情境</h1>
            <p className="muted">挑選要安裝的自動互動範本（之後都能修改或刪除）：</p>
            {state.starterRecipes.map((s) => (
              <label key={s.id} className="starter-row">
                <input
                  type="checkbox"
                  checked={draft.starters.includes(s.id)}
                  onChange={(e) =>
                    update({
                      starters: e.target.checked
                        ? [...draft.starters, s.id]
                        : draft.starters.filter((x) => x !== s.id),
                    })
                  }
                />
                {s.title}
              </label>
            ))}
          </section>
        )}

        {step === 6 && (
          <section>
            <h1>確認</h1>
            <dl className="confirm-summary">
              <div>
                <dt>AI 可以知道</dt>
                <dd>
                  {draft.senses.length === 0
                    ? "（什麼都不知道）"
                    : draft.senses.map((id) => findCard("receptor", id).name).join("、")}
                </dd>
              </div>
              <div>
                <dt>AI 可以做</dt>
                <dd>
                  {draft.responses.length === 0
                    ? "（不能做任何事）"
                    : draft.responses.map((id) => findCard("actuator", id).name).join("、")}
                </dd>
              </div>
              <div>
                <dt>必須先問</dt>
                <dd>
                  高敏感感知（攝影機、麥克風）、實體控制、對外寫入
                  {draft.askHighRisk ? "，以及所有高風險操作" : ""}
                </dd>
              </div>
              <div>
                <dt>資料離開本機</dt>
                <dd>{dataFlowSummary(human, draft)}</dd>
              </div>
              <div>
                <dt>主動程度</dt>
                <dd>
                  {draft.initiative === "passive"
                    ? "只在要求時"
                    : draft.initiative === "active"
                      ? "可以主動協助"
                      : "重要時提醒"}
                  {draft.quietEnabled ? `；${draft.quietStart}–${draft.quietEnd} 保持安靜` : ""}
                  ；每小時最多 {draft.maxPerHour} 次
                </dd>
              </div>
              <div>
                <dt>自動互動</dt>
                <dd>
                  {draft.starters.length === 0
                    ? "（不安裝範本）"
                    : state.starterRecipes
                        .filter((s) => draft.starters.includes(s.id))
                        .map((s) => s.title)
                        .join("、")}
                </dd>
              </div>
              <div>
                <dt>什麼時候呼叫 AI</dt>
                <dd>預設範本完全不用 AI；只有你在配方裡明確開啟時才會請 AI 協助</dd>
              </div>
            </dl>
            {error && <p className="cap-card-error" role="alert">套用失敗：{error}</p>}
          </section>
        )}

        <footer className="onboarding-footer">
          <button onClick={onSkip} className="ghost">
            略過（稍後可從設定重來）
          </button>
          <div className="row">
            {step > 0 && <button onClick={() => update({ step: step - 1 })}>上一步</button>}
            {step < STEPS.length - 1 ? (
              <button className="primary" onClick={() => update({ step: step + 1 })}>
                下一步
              </button>
            ) : (
              <button className="primary" onClick={commit} disabled={committing}>
                {committing ? "套用中…" : "完成設定"}
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}

/** 由實際選擇計算資料流摘要 — 絕不硬編「都在本機」。 */
function dataFlowSummary(
  human: { receptors: HumanCard[]; actuators: HumanCard[] },
  draft: Draft
): string {
  const selected = [
    ...human.receptors.filter((r) => draft.senses.includes(r.id)),
    ...human.actuators.filter((a) => draft.responses.includes(a.id)),
  ];
  const leaves = selected.filter(
    (c) => c.data?.leavesDevice === true || c.effect?.externalSideEffect === true
  );
  const unknown = selected.filter(
    (c) =>
      (c.data && c.data.leavesDevice === "unknown") ||
      (c.effect && c.effect.externalSideEffect === "unknown") ||
      (!c.data && !c.effect)
  );
  if (leaves.length > 0)
    return `注意：「${leaves.map((c) => c.displayName).join("、")}」會將資料傳到外部`;
  if (unknown.length > 0)
    return `大多為本機能力；「${unknown.map((c) => c.displayName).join("、")}」的資料流向未知，使用前請確認`;
  return "目前選擇的能力都確定只在本機運作";
}

function beginnerSafe(card: HumanCard): boolean {
  // 保守原則：只有「確定安全」才能預選；未知一律不預選。
  if (card.requiresConsent || card.consent.required === true) return false;
  if (card.data) {
    if (card.data.sensitivity === "high" || card.data.sensitivity === "unknown") return false;
    if (card.data.leavesDevice !== false) return false; // true 或 unknown 都不預選
  }
  if (card.effect) {
    if (card.effect.physicalEffect !== false) return false;
    if (card.effect.externalSideEffect !== false) return false;
  }
  if (!card.data && !card.effect) return false; // 完全沒有語意宣告 → 不預選
  return true;
}

function PickStep({
  title,
  intro,
  cards,
  selected,
  onChange,
}: {
  title: string;
  intro: string;
  cards: HumanCard[];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  return (
    <section>
      <h1>{title}</h1>
      <p className="muted">{intro}</p>
      <PickCards cards={cards} selected={selected} onChange={onChange} />
    </section>
  );
}

function PickCards({
  cards,
  selected,
  onChange,
}: {
  cards: HumanCard[];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  return (
    <div className="pick-grid">
      {cards.map((c) => {
        const on = selected.includes(c.id);
        return (
          <label key={c.id} className={on ? "pick-card on" : "pick-card"}>
            <input
              type="checkbox"
              checked={on}
              onChange={(e) =>
                onChange(e.target.checked ? [...selected, c.id] : selected.filter((x) => x !== c.id))
              }
            />
            <div className="pick-card-body">
              <div className="row">
                <Icon name={c.icon} size={18} />
                <strong>{c.displayName}</strong>
                {c.beginnerRecommended && <Badge kind="info">推薦</Badge>}
              </div>
              <p className="muted small">{c.shortDescription ?? c.conservativeNotice}</p>
              <div>
                {c.badges.slice(0, 3).map((b) => (
                  <Badge key={b.key} kind={b.tone === "danger" ? "bad" : b.tone === "warn" ? "warn" : b.tone === "ok" ? "ok" : "info"}>
                    {b.label}
                  </Badge>
                ))}
              </div>
            </div>
          </label>
        );
      })}
    </div>
  );
}
