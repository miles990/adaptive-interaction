// 首次設定精靈（v0.5）：3 步（認識小樞／AI 幫手／安全預設）、draft/commit。
// 所有選擇最後一次性套用到後端（enable/disable、policy patch、starter recipes、
// 偏好），中途關閉只留草稿，不會留下半套用狀態。
// 保守原則不變：只有「確定安全」的本機低風險能力會自動啟用；攝影機、麥克風、
// 位置、對外寫入與實體效果一律預設關閉，之後首次需要時逐項詢問。
// 硬體掃描、iPhone 配對等移出精靈——第一次真正需要時再問。

import React from "react";
import { api, HumanCard, OnboardingState } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";
import { desktop, isTauri } from "../desktop";
import { PackManifest, validateManifest } from "../companion/renderer";

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
  companionVisible: boolean;
  expressiveness: string;
  dialogueMode: string;
  agentChoice: string;
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
  companionVisible: true,
  expressiveness: "natural",
  dialogueMode: "necessary",
  agentChoice: "later",
};

const STEPS = ["認識小樞", "AI 幫手", "安全預設"];

export function Onboarding({ onDone, onSkip }: { onDone: () => void; onSkip: () => void }) {
  const { human, findCard } = useAppState();
  const [state, setState] = React.useState<OnboardingState | null>(null);
  const [draft, setDraft] = React.useState<Draft>(EMPTY_DRAFT);
  const [committing, setCommitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const [loadError, setLoadError] = React.useState<string | null>(null);
  React.useEffect(() => {
    api.onboardingGet().catch((e) => {
      setLoadError(String(e));
      throw e;
    }).then((s) => {
      setState(s);
      const d = s.draft as Partial<Draft> | null | undefined;
      if (d && typeof d.step === "number") {
        setDraft({ ...EMPTY_DRAFT, ...d, step: Math.min(d.step, STEPS.length - 1) });
      } else if (human) {
        // 預選：只挑低風險、本機、推薦新手的能力（保守原則，未知不預選）。
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
    } catch (e) {
      setError(String(e));
      setCommitting(false);
      return;
    }
    // 主動對話模式（預設「必要」）：基本設定已套用；此步失敗要誠實回報，
    // 不宣稱全部完成。
    try {
      await api.proactiveDialoguePatch({ mode: draft.dialogueMode });
    } catch (e) {
      setError(`基本設定已套用，但主動對話模式設定失敗：${String(e)}。可稍後在「小樞」頁調整。`);
      setCommitting(false);
      return;
    }
    // 桌面角色顯示與表現（只在桌面版存在；瀏覽器檢視誠實跳過）。
    if (isTauri) {
      try {
        await desktop.prefsPatch({
          companionVisible: draft.companionVisible,
          companionExpressiveness: draft.expressiveness,
        });
        await desktop.companionApplyPrefs();
      } catch (e) {
        setError(`基本設定已套用，但桌面角色設定失敗：${String(e)}。可稍後在「小樞」頁調整。`);
        setCommitting(false);
        return;
      }
    }
    setCommitting(false);
    onDone();
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
            <h1>認識小樞</h1>
            <p className="muted">
              小樞是住在你桌面上的貓系數位精靈。她會眨眼、伸展、打瞌睡——這些本機微動作
              即時發生、不呼叫 AI；連接 AI 或裝置之後，她會誠實呈現真實狀態。
            </p>
            <PackPeek />
            <label className="radio-row">
              <input
                type="checkbox"
                checked={draft.companionVisible}
                onChange={(e) => update({ companionVisible: e.target.checked })}
              />
              在桌面上顯示小樞（可隨時隱藏）
            </label>
            <fieldset>
              <legend>表現程度</legend>
              {[
                ["quiet", "安靜——只顯示安全訊息"],
                ["natural", "自然（建議）"],
                ["lively", "活潑"],
              ].map(([v, label]) => (
                <label key={v} className="radio-row">
                  <input
                    type="radio"
                    name="ob-expressiveness"
                    checked={draft.expressiveness === v}
                    onChange={() => update({ expressiveness: v })}
                  />
                  {label}
                </label>
              ))}
            </fieldset>
            <p className="muted small">
              音效預設關閉；外觀、大小、透明度與更多表現設定都在「小樞」頁。
            </p>
          </section>
        )}

        {step === 1 && (
          <AgentStep choice={draft.agentChoice} onChoice={(agentChoice) => update({ agentChoice })} />
        )}

        {step === 2 && (
          <section>
            <h1>安全預設</h1>
            <div className="notice-box">
              <p>這些保證不需要你做任何事，預設就成立：</p>
              <ul>
                <li>麥克風、攝影機、定位<strong>預設關閉</strong>；使用中一定有持續可見的指示與立即停止。</li>
                <li>外部裝置與實體動作（燈光、震動）第一次使用時會先詢問。</li>
                <li>AI Agent 要寫入哪個資料夾，每個都需要你明確確認。</li>
                <li>右上角的<strong>緊急停止</strong>隨時可以立即停止一切——不經過佇列、不依賴 AI。</li>
                <li>能力存在不等於 AI 自動獲得權限——每一項都由你決定，隨時可撤回。</li>
              </ul>
            </div>
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
            <fieldset>
              <legend>小樞主動說話</legend>
              {[
                ["necessary", "必要時（建議）——只有等待確認、失敗、未知與感測提示"],
                ["natural", "自然——加上任務進度與低頻建議"],
              ].map(([v, label]) => (
                <label key={v} className="radio-row">
                  <input
                    type="radio"
                    name="ob-dialogue"
                    checked={draft.dialogueMode === v}
                    onChange={() => update({ dialogueMode: v })}
                  />
                  {label}
                </label>
              ))}
            </fieldset>
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
                <dt>資料離開本機</dt>
                <dd>{dataFlowSummary(human, draft)}</dd>
              </div>
            </dl>
            <p className="muted small">
              以上是自動挑選的低風險本機能力。想逐項自訂？完成後到「連接與權限」
              隨時調整；安靜時段、硬體掃描與 iPhone 配對會在第一次需要時再問你。
            </p>
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

/** AI 幫手步驟：只做 Discovery／登入狀態檢查，不授權任何工作區寫入。 */
function AgentStep({
  choice,
  onChoice,
}: {
  choice: string;
  onChoice: (choice: string) => void;
}) {
  const [agents, setAgents] = React.useState<Record<string, unknown>[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    let alive = true;
    api
      .agentsDiscoveries()
      .then((result) => {
        if (alive) setAgents((result.agents as Record<string, unknown>[] | undefined) ?? []);
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  return (
    <section>
      <h1>要讓小樞幫忙工作嗎？</h1>
      <p className="muted">
        小樞可以把任務交給本機的 AI Agent（Codex／Claude Code）。這一步只檢查
        安裝與登入狀態，<strong>不會</strong>授權讀寫任何資料夾——實際建立工作
        階段時才逐項授權，且隨時可取消。
      </p>
      {error ? (
        <div className="state-box state-error">無法檢查 Agent 狀態：{error}</div>
      ) : agents === null ? (
        <div className="state-box">正在檢查本機 AI Agent…</div>
      ) : (
        <ul className="plain-list">
          {agents.length === 0 && <li className="muted">目前沒有偵測到本機 AI Agent。</li>}
          {agents.map((agent) => (
            <li key={String(agent.kind)}>
              <Icon name="bot" size={14} /> {String(agent.kind)}：
              {agent.found === true && agent.loggedIn === true
                ? "已安裝、已登入"
                : agent.found === true
                  ? "已安裝，尚未登入"
                  : String(agent.detail ?? "未偵測到")}
            </li>
          ))}
        </ul>
      )}
      <fieldset>
        <legend>你的選擇（之後隨時可改）</legend>
        {[
          ["codex", "用 Codex 幫忙"],
          ["claude", "用 Claude Code 幫忙"],
          ["both", "兩者都用（依任務挑選）"],
          ["later", "稍後再說"],
        ].map(([v, label]) => (
          <label key={v} className="radio-row">
            <input
              type="radio"
              name="ob-agent"
              checked={choice === v}
              onChange={() => onChoice(v)}
            />
            {label}
          </label>
        ))}
      </fieldset>
    </section>
  );
}

/** 從真實角色包 sheet 取 idle 幀當預覽 — 不畫設計稿、不用假圖。 */
function PackPeek() {
  const ref = React.useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = React.useState<string | null>(null);
  React.useEffect(() => {
    let disposed = false;
    (async () => {
      try {
        const res = await fetch("/packs/shu-agile/manifest.json");
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const manifest = (await res.json()) as PackManifest;
        const issues = validateManifest(manifest);
        if (issues.length > 0) throw new Error(issues.join("; "));
        const img = new Image();
        img.src = `/packs/shu-agile/${manifest.sheet}`;
        await img.decode();
        if (disposed) return;
        const canvas = ref.current;
        const ctx = canvas?.getContext("2d");
        const anim = manifest.animations["idle"];
        if (!canvas || !ctx || !anim) throw new Error("idle animation missing");
        const frameIdx = anim.frames[0];
        const [fw, fh] = manifest.frameSize;
        ctx.imageSmoothingEnabled = false;
        ctx.drawImage(
          img,
          (frameIdx % manifest.columns) * fw,
          Math.floor(frameIdx / manifest.columns) * fh,
          fw,
          fh,
          0,
          0,
          canvas.width,
          canvas.height
        );
      } catch (e) {
        if (!disposed) setFailed(String(e));
      }
    })();
    return () => {
      disposed = true;
    };
  }, []);
  if (failed)
    return <p className="muted small">（角色預覽載入失敗：{failed}）</p>;
  return (
    <div className="row" style={{ justifyContent: "center" }}>
      <canvas ref={ref} width={128} height={128} aria-label="小樞預覽（取自實際角色素材）" />
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
