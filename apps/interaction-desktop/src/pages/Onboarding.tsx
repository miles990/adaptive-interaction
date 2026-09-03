// 首次設定精靈（v0.5）：3 步（選擇角色與陪伴方式／選擇 AI 工作方式／確認安全與權限預設）、
// draft → 套用前確認 → commit。所有選擇最後一次性套用到後端（enable/disable、policy patch、
// 起步範本、偏好），中途關閉只留草稿，不會留下半套用狀態。
// 保守原則不變：只有「確定安全」的本機低風險能力會自動啟用；攝影機、麥克風、
// 位置、對外寫入與實體效果一律預設關閉，之後首次需要時逐項詢問。
// 硬體掃描、iPhone 配對等移出精靈——第一次真正需要時再問。
//
// 重新執行（rerun）的三條規則：
//  1. 預選＝目前真的開著的能力，不是重新推薦，所以不會靜默關掉任何已啟用能力。
//  2. 套用前一定先跑後端試算（無副作用）並列出每一項變更，使用者按「套用」才動手；
//     試算拿不到就退回本機快照估算，而且畫面必須標示那是估算。
//  3. 沒被使用者在精靈裡真的改過的設定（主動說話、桌面角色偏好、顯示語言、
//     起步範本內容）一律不送，後端也會跳過狀態相同的能力。
//
// 角色名稱與代詞一律來自 useCharacterName()（小樞的 manifest 宣告「她」；其他角色中立）；
// 貓系／女僕等物種與服裝文案只給小樞家族。commit 之後接「首次成功體驗」（可略過，不是第四步）。

import React from "react";
import { api, HumanCard, OnboardingPreview, OnboardingState } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";
import { desktop, isTauri } from "../desktop";
import { drawExpressionPreview } from "../companion/rig/renderer";
import { useCharacterName } from "../characterName";
import { LEGACY_CHARACTER_IDS } from "../companion/settingsTransfer";
import { Dialog } from "../components/Dialog";
import { FirstSuccess, isFirstSuccessSeen } from "./FirstSuccess";
import { buildQuietHoursPatch } from "../quietHours";

/** 小樞家族（8 個內建 shu-* 角色）：只有這些角色用貓系數位精靈的物種文案。 */
export function isShuFamily(characterId: string | null | undefined): boolean {
  return typeof characterId === "string" && LEGACY_CHARACTER_IDS.includes(characterId);
}

/** 步驟一的介紹句：小樞家族保留原文案；其他角色只講事實（本機微動作、誠實狀態），不講物種或服裝。 */
export function introCopy(name: string, pronoun: string, shu: boolean): string {
  if (shu) {
    return `${name}是住在你桌面上的貓系數位精靈。${pronoun}會眨眼、伸展、打瞌睡——這些本機微動作即時發生、不呼叫 AI；連接 AI 或裝置之後，${pronoun}會誠實呈現真實狀態。`;
  }
  return `${name}是住在你桌面上的角色。${pronoun}的本機微動作即時發生、不呼叫 AI；連接 AI 或裝置之後，${pronoun}會誠實呈現真實狀態，不會假裝完成。`;
}

interface Draft {
  step: number;
  senses: string[];
  responses: string[];
  initiative: string;
  quietStart: string;
  quietEnd: string;
  quietEnabled: boolean;
  starters: string[];
  /** 使用者是否打開「進一步自訂」。沒打開就不寫安靜時段。 */
  customize: boolean;
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
  // 精靈不再靜默設定安靜時段：文案說「第一次需要時再問」，行為就必須一致。
  quietEnabled: false,
  starters: ["starter-task-complete"],
  customize: false,
  companionVisible: true,
  expressiveness: "natural",
  dialogueMode: "necessary",
  agentChoice: "later",
};

/** 三個步驟的名稱固定；重跑與首次執行看到的是同一組。 */
const STEPS = ["選擇角色與陪伴方式", "選擇 AI 工作方式", "確認安全與權限預設"];
const STEP_COUNT = STEPS.length;

const DIALOGUE_LABELS: Record<string, string> = { necessary: "必要時", natural: "自然" };
const EXPRESSIVENESS_LABELS: Record<string, string> = {
  quiet: "安靜",
  natural: "自然",
  lively: "活潑",
};
const AGENT_CHOICE_LABELS: Record<string, string> = {
  codex: "用 Codex 幫忙",
  claude: "用 Claude Code 幫忙",
  both: "兩者都用（依任務挑選）",
  later: "稍後再說",
};

/** 套用前確認畫面的一列。`from` 為 undefined 表示目前值未知，就不假裝知道。 */
export interface ChangeRow {
  key: string;
  label: string;
  from?: string;
  to: string;
}

/** 精靈載入時讀到的「目前實際設定」；null＝讀不到，不當成任何值。 */
interface CurrentSettings {
  dialogueMode: string | null;
  companionVisible: boolean | null;
  expressiveness: string | null;
}

interface PendingApply {
  commit: Record<string, unknown>;
  rows: ChangeRow[];
  /** 後端試算失敗時的原因；有值就代表畫面上的差異是本機估算。 */
  estimatedBecause: string | null;
}

/** 一項能力現在是否開著：只有「已停用」才算關閉（離線仍是開著、只是不健康）。 */
function capabilityOn(card: HumanCard): boolean {
  return card.availability !== "disabled";
}

/** 重跑時可由精靈開關的能力：需要同意的一律不碰（要走逐項同意流程）。 */
function wizardControllable(card: HumanCard): boolean {
  return !card.requiresConsent;
}

/**
 * 後端試算拿不到時的本機估算：用畫面上的能力快照算差異。
 * 只列真的會變的項目；同意閘門的能力永遠不列。
 */
export function localCapabilityRows(
  human: { receptors: HumanCard[]; actuators: HumanCard[] },
  senses: string[],
  responses: string[]
): ChangeRow[] {
  const rows: ChangeRow[] = [];
  const scan = (cards: HumanCard[], selected: string[], kind: string) => {
    for (const card of cards) {
      if (!wizardControllable(card)) continue;
      const from = capabilityOn(card);
      const to = selected.includes(card.id);
      if (from === to) continue;
      rows.push({
        key: `${kind}:${card.id}`,
        label: card.displayName,
        from: from ? "開啟" : "關閉",
        to: to ? "開啟" : "關閉",
      });
    }
  };
  scan(human.receptors, senses, "receptor");
  scan(human.actuators, responses, "actuator");
  return rows;
}

/** 後端試算 → 確認畫面的列；只留真的會變的項目。 */
export function previewRows(
  preview: OnboardingPreview,
  findCard: (kind: "receptor" | "actuator" | "tool", id: string) => { name: string }
): ChangeRow[] {
  const rows: ChangeRow[] = [];
  const scan = (list: OnboardingPreview["receptors"], kind: "receptor" | "actuator") => {
    for (const change of list ?? []) {
      if (!change.changed) continue;
      rows.push({
        key: `${kind}:${change.id}`,
        label: findCard(kind, change.id).name,
        from: change.from === "on" ? "開啟" : "關閉",
        to: change.to === "on" ? "開啟" : "關閉",
      });
    }
  };
  scan(preview.receptors, "receptor");
  scan(preview.actuators, "actuator");
  return rows;
}

/** 兩份 AI 路由偏好是否完全一樣（一樣就不送，避免覆蓋使用者後來的調整）。 */
function sameRoutes(
  next: Record<string, string>,
  currentRoutes: Record<string, string> | undefined
): boolean {
  if (!currentRoutes) return false;
  const keys = new Set([...Object.keys(next), ...Object.keys(currentRoutes)]);
  for (const key of keys) {
    if (next[key] !== currentRoutes[key]) return false;
  }
  return true;
}

export function Onboarding({
  onDone,
  onSkip,
  onNavigate,
}: {
  onDone: () => void;
  onSkip: () => void;
  /** 首次成功體驗的「交代一件小工作」／「更換角色」用；沒提供就只關閉精靈。 */
  onNavigate?: (tab: string) => void;
}) {
  const { human, findCard, prefs } = useAppState();
  const character = useCharacterName();
  const name = character.name;
  const pronoun = character.pronoun;
  const shu = isShuFamily(character.characterId);
  const [state, setState] = React.useState<OnboardingState | null>(null);
  const [draft, setDraft] = React.useState<Draft>(EMPTY_DRAFT);
  const [committing, setCommitting] = React.useState(false);
  const [preparing, setPreparing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  /** 目前實際設定；套用前確認要顯示「現在 → 之後」。 */
  const [current, setCurrent] = React.useState<CurrentSettings>({
    dialogueMode: null,
    companionVisible: null,
    expressiveness: null,
  });
  /** 使用者在精靈裡真的動過的欄位；目前值讀不到時用它判斷「有沒有改」。 */
  const touched = React.useRef<Set<string>>(new Set());
  /** 已算好、等使用者按「套用」的變更；null＝還沒到確認階段。 */
  const [pending, setPending] = React.useState<PendingApply | null>(null);
  /** commit 成功後的可略過畫面；不是精靈的第四步。 */
  const [phase, setPhase] = React.useState<"wizard" | "first-success">("wizard");
  /** 重新執行（已完成過一次）：預選＝目前開著的能力，絕不靜默關掉任何一項。 */
  const rerun = state?.completed === true;

  const [loadError, setLoadError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (human === undefined) return;
    let alive = true;
    void (async () => {
      // 1. 先讀目前的實際設定（讀不到就維持「未知」，不用預設值冒充）。
      const cur: CurrentSettings = {
        dialogueMode: null,
        companionVisible: null,
        expressiveness: null,
      };
      try {
        const mode = (await api.proactiveDialogueGet())["mode"];
        if (typeof mode === "string") cur.dialogueMode = mode;
      } catch {
        cur.dialogueMode = null;
      }
      if (isTauri) {
        try {
          const p = await desktop.prefsGet();
          cur.companionVisible = p.companionVisible;
          cur.expressiveness = p.companionExpressiveness;
        } catch {
          cur.companionVisible = null;
          cur.expressiveness = null;
        }
      }
      if (!alive) return;
      setCurrent(cur);
      // 2. 精靈狀態與草稿。
      let s: OnboardingState;
      try {
        s = await api.onboardingGet();
      } catch (e) {
        if (alive) setLoadError(String(e));
        return;
      }
      if (!alive) return;
      setState(s);
      const isRerun = s.completed === true;
      const seeded: Draft = {
        ...EMPTY_DRAFT,
        // 重跑：畫面上的選項顯示目前真值，使用者不動就等於沒改。
        // 第一次：維持精靈本來的保守建議值（必要時說話），差異照樣會列在確認畫面上。
        ...(isRerun && cur.dialogueMode !== null ? { dialogueMode: cur.dialogueMode } : {}),
        ...(isRerun && cur.companionVisible !== null
          ? { companionVisible: cur.companionVisible }
          : {}),
        ...(isRerun && cur.expressiveness !== null ? { expressiveness: cur.expressiveness } : {}),
        // 重跑不重裝起步範本：那會覆寫使用者已經改過的自動互動內容。
        ...(isRerun ? { starters: [] } : {}),
      };
      // 第一次：只挑低風險、本機、推薦新手的能力（保守原則，未知不預選）。
      // 重跑：挑目前真的開著的，讓「不動就不會變」成立。
      const liveSenses = human.receptors
        .filter((r) =>
          isRerun
            ? wizardControllable(r) && capabilityOn(r)
            : beginnerSafe(r) && r.availability === "available"
        )
        .map((r) => r.id);
      const liveResponses = human.actuators
        .filter((a) =>
          isRerun
            ? wizardControllable(a) && capabilityOn(a)
            : beginnerSafe(a) && a.availability === "available"
        )
        .map((a) => a.id);
      const fresh: Draft = { ...seeded, senses: liveSenses, responses: liveResponses };
      const d = s.draft as Partial<Draft> | null | undefined;
      if (d && typeof d.step === "number") {
        // 重跑時的草稿可能是上一次沒送出的舊值，而使用者之後已經在別處改過設定。
        // 凡是「鏡射目前真實狀態」的欄位一律以真值為準，草稿不得蓋回舊值——
        // 否則按下「套用」就會靜默還原使用者後來的修改（起步範本同理，重跑不重裝）。
        const liveWins: Partial<Draft> = isRerun
          ? {
              ...(cur.dialogueMode !== null ? { dialogueMode: cur.dialogueMode } : {}),
              ...(cur.companionVisible !== null ? { companionVisible: cur.companionVisible } : {}),
              ...(cur.expressiveness !== null ? { expressiveness: cur.expressiveness } : {}),
              starters: [],
              senses: liveSenses,
              responses: liveResponses,
            }
          : {};
        setDraft({ ...fresh, ...d, ...liveWins, step: Math.min(d.step, STEP_COUNT - 1) });
        return;
      }
      setDraft(fresh);
    })();
    return () => {
      alive = false;
    };
  }, [human !== undefined]);

  function update(patch: Partial<Draft>) {
    for (const key of Object.keys(patch)) touched.current.add(key);
    setDraft((prev) => {
      const next = { ...prev, ...patch };
      api.onboardingDraft(next as unknown as Record<string, unknown>).catch(() => {});
      return next;
    });
  }

  /** 使用者在精靈裡改過這個設定嗎？目前值未知時，只認他真的動過的欄位。 */
  function settingChanged(key: "dialogueMode" | "companionVisible" | "expressiveness"): boolean {
    const now = current[key];
    if (now === null) return touched.current.has(key);
    return draft[key] !== now;
  }

  /** 這次要送出的 commit 內容；純計算，沒有任何副作用。 */
  function buildCommit(cards: { receptors: HumanCard[]; actuators: HumanCard[] }) {
    const enableR = draft.senses;
    const disableR = cards.receptors
      .filter((r) => !draft.senses.includes(r.id) && capabilityOn(r) && wizardControllable(r))
      .map((r) => r.id);
    const enableA = draft.responses;
    const disableA = cards.actuators
      .filter((a) => !draft.responses.includes(a.id) && capabilityOn(a) && wizardControllable(a))
      .map((a) => a.id);
    // 只寫使用者真的做過的決定。沒打開「進一步自訂」＝不碰主動程度與安靜時段
    // （後端既有預設維持不變）。
    // initiative 尤其重要：精靈根本沒有調整它的欄位，無條件送出草稿預設值
    // 等於用一個使用者沒看過的值覆蓋既有設定。
    // 每小時上限與高風險核准門檻不在這裡：一般模式沒有它們的主人，
    // 精靈就不該是唯一寫得進、卻改不回來的地方。
    const policyPatch: Record<string, unknown> = {};
    if (draft.customize) {
      if (draft.initiative !== EMPTY_DRAFT.initiative) {
        policyPatch.initiative = draft.initiative;
      }
      if (draft.quietEnabled) {
        // 空陣列會被後端解讀成含 desktop-pet 的內建預設（ia-settings-012），
        // 用與角色頁相同的 canonical builder 送出明確清單。
        policyPatch.quietHours = [buildQuietHoursPatch(draft.quietStart, draft.quietEnd)];
      }
    }
    // 步驟二的選擇寫進既有的 AI 路由偏好（只是路由建議：不授權
    // 任何工作目錄、不建立工作階段、不會自動改送另一家）。
    // 與目前偏好相同就不送，重跑才不會覆蓋使用者後來的調整。
    const routes = agentRoutesFor(draft.agentChoice);
    const routesChanged = routes !== null && !sameRoutes(routes, prefs.agentRoutes);
    const preferences: Record<string, unknown> = {
      // 顯示語言只在第一次設定時寫入；重跑不動使用者後來選的語言。
      ...(rerun ? {} : { locale: "zh-TW" }),
      ...(routesChanged ? { agentRoutes: routes } : {}),
    };
    return {
      enableReceptors: enableR,
      disableReceptors: disableR,
      enableActuators: enableA,
      disableActuators: disableA,
      starterRecipes: draft.starters,
      policyPatch,
      ...(Object.keys(preferences).length > 0 ? { preferences } : {}),
    } as Record<string, unknown>;
  }

  /** 精靈自己負責的設定（不在 commit 裡）會怎麼變。 */
  function settingRows(): ChangeRow[] {
    const rows: ChangeRow[] = [];
    if (settingChanged("dialogueMode")) {
      rows.push({
        key: "dialogueMode",
        label: `${name}主動說話`,
        from: current.dialogueMode
          ? (DIALOGUE_LABELS[current.dialogueMode] ?? current.dialogueMode)
          : undefined,
        to: DIALOGUE_LABELS[draft.dialogueMode] ?? draft.dialogueMode,
      });
    }
    if (isTauri && settingChanged("companionVisible")) {
      rows.push({
        key: "companionVisible",
        label: `在桌面上顯示${name}`,
        from: current.companionVisible === null ? undefined : current.companionVisible ? "開啟" : "關閉",
        to: draft.companionVisible ? "開啟" : "關閉",
      });
    }
    if (isTauri && settingChanged("expressiveness")) {
      rows.push({
        key: "expressiveness",
        label: "表現程度",
        from: current.expressiveness
          ? (EXPRESSIVENESS_LABELS[current.expressiveness] ?? current.expressiveness)
          : undefined,
        to: EXPRESSIVENESS_LABELS[draft.expressiveness] ?? draft.expressiveness,
      });
    }
    return rows;
  }

  /** 起步範本：已存在就是覆寫，講清楚再讓使用者決定。 */
  function starterRows(installed: { id: string; exists: boolean }[]): ChangeRow[] {
    return installed.map((item) => {
      const title = state?.starterRecipes.find((r) => r.id === item.id)?.title ?? item.id;
      return {
        key: `starter:${item.id}`,
        label: `自動互動範本「${title}」`,
        from: item.exists ? "已存在（內容會被覆寫）" : "沒有",
        to: "安裝",
      };
    });
  }

  function policyRows(patch: Record<string, unknown>): ChangeRow[] {
    const rows: ChangeRow[] = [];
    if (patch.quietHours) {
      rows.push({
        key: "quietHours",
        label: "安靜時段",
        to: `${draft.quietStart}–${draft.quietEnd}`,
      });
    }
    if (typeof patch.initiative === "string") {
      rows.push({ key: "initiative", label: "主動程度", to: patch.initiative });
    }
    if ((patch as { agentRoutes?: unknown }).agentRoutes) {
      rows.push({
        key: "agentRoutes",
        label: "AI 工作方式",
        to: AGENT_CHOICE_LABELS[draft.agentChoice] ?? draft.agentChoice,
      });
    }
    return rows;
  }

  /** 按「完成設定」：先跑後端試算，把每一項變更攤開來給使用者確認。 */
  async function prepare() {
    if (!human) return;
    setPreparing(true);
    setError(null);
    const payload = buildCommit(human);
    const prefsPatch = (payload.preferences ?? {}) as Record<string, unknown>;
    let rows: ChangeRow[] = [];
    let estimatedBecause: string | null = null;
    try {
      const preview = await api.onboardingPreview(payload);
      rows = previewRows(preview, findCard);
      rows = rows.concat(starterRows(preview.starterRecipes ?? []));
    } catch (e) {
      // 試算拿不到就退回本機快照，並且畫面上標示這是估算（不假裝是真值）。
      estimatedBecause = String(e);
      rows = localCapabilityRows(human, draft.senses, draft.responses);
      rows = rows.concat(
        starterRows(draft.starters.map((id) => ({ id, exists: false })))
      );
    }
    rows = rows.concat(
      policyRows({ ...(payload.policyPatch as Record<string, unknown>), ...prefsPatch })
    );
    rows = rows.concat(settingRows());
    setPreparing(false);
    setPending({ commit: payload, rows, estimatedBecause });
  }

  /** 使用者按下「套用」之後才真的動手。 */
  async function commit() {
    const approved = pending;
    if (!approved) return;
    setPending(null);
    setCommitting(true);
    setError(null);
    try {
      await api.onboardingCommit(approved.commit);
    } catch (e) {
      setError(String(e));
      setCommitting(false);
      return;
    }
    // 主動對話模式：只有使用者在精靈裡真的改過才送，否則不覆蓋他後來的調整。
    // 此步失敗要誠實回報，不宣稱全部完成。
    if (settingChanged("dialogueMode")) {
      try {
        await api.proactiveDialoguePatch({ mode: draft.dialogueMode });
      } catch (e) {
        setError(`基本設定已套用，但主動對話模式設定失敗：${String(e)}。可稍後在「${name}」頁調整。`);
        setCommitting(false);
        return;
      }
    }
    // 桌面角色顯示與表現（只在桌面版存在；瀏覽器檢視誠實跳過）。
    if (isTauri && (settingChanged("companionVisible") || settingChanged("expressiveness"))) {
      try {
        await desktop.prefsPatch({
          ...(settingChanged("companionVisible")
            ? { companionVisible: draft.companionVisible }
            : {}),
          ...(settingChanged("expressiveness")
            ? { companionExpressiveness: draft.expressiveness }
            : {}),
        });
        await desktop.companionApplyPrefs();
      } catch (e) {
        setError(`基本設定已套用，但桌面角色設定失敗：${String(e)}。可稍後在「${name}」頁調整。`);
        setCommitting(false);
        return;
      }
    }
    setCommitting(false);
    // 首次成功體驗：看過就不再打擾，直接完成。
    let seen = false;
    try {
      seen = await isFirstSuccessSeen();
    } catch {
      seen = false;
    }
    if (seen) {
      onDone();
      return;
    }
    setPhase("first-success");
  }

  if (phase === "first-success") {
    return <FirstSuccess onDone={onDone} onNavigate={onNavigate} />;
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
            <h1>{STEPS[0]}</h1>
            <p className="muted">{introCopy(name, pronoun, shu)}</p>
            {shu ? <PackPeek name={name} /> : <CharacterPeekText name={name} />}
            <label className="radio-row">
              <input
                type="checkbox"
                checked={draft.companionVisible}
                onChange={(e) => update({ companionVisible: e.target.checked })}
              />
              在桌面上顯示{name}（可隨時隱藏）
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
            <ul className="plain-list muted small">
              {shu && <li>玩耍與游標互動：預設開啟（本機即時反應，不呼叫 AI）。</li>}
              <li>音效預設關閉，之後可在「{name}」頁開啟。</li>
              <li>外觀、大小、透明度與更多陪伴設定也都在「{name}」頁。</li>
            </ul>
          </section>
        )}

        {step === 1 && (
          <AgentStep name={name} choice={draft.agentChoice} onChoice={(agentChoice) => update({ agentChoice })} />
        )}

        {step === 2 && (
          <section>
            <h1>{STEPS[2]}</h1>
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
            <p className="muted small">
              高風險操作預設每次都先詢問你；「危險／不可回復」級的操作永遠需要明確確認 —
              這條底線無法被關閉。
            </p>
            <fieldset>
              <legend>{name}主動說話</legend>
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
            <details className="tech-details">
              <summary>進一步自訂（選填）</summary>
              <p className="muted small">
                不打開這一段，精靈就<strong>不會</strong>動安靜時段。
                高風險核准門檻與各項頻率上限不在精靈裡，精靈也不會改它們。
              </p>
              <label className="radio-row">
                <input
                  type="checkbox"
                  checked={draft.customize}
                  onChange={(e) => update({ customize: e.target.checked })}
                />
                我要在這裡直接設定（否則第一次需要時再問我）
              </label>
              {draft.customize && (
                <>
                  <label className="radio-row">
                    <input
                      type="checkbox"
                      checked={draft.quietEnabled}
                      onChange={(e) => update({ quietEnabled: e.target.checked })}
                    />
                    設定安靜時段
                  </label>
                  {draft.quietEnabled && (
                    <div className="row wrap">
                      <label className="field-label">
                        從
                        <input
                          type="time"
                          value={draft.quietStart}
                          onChange={(e) => update({ quietStart: e.target.value })}
                        />
                      </label>
                      <label className="field-label">
                        到
                        <input
                          type="time"
                          value={draft.quietEnd}
                          onChange={(e) => update({ quietEnd: e.target.value })}
                        />
                      </label>
                    </div>
                  )}
                </>
              )}
            </details>
            <p className="muted small">
              {rerun
                ? "以上是你目前已經開啟的能力：重新執行不會自動關掉任何一項。個別能力請到「連接與權限」調整；按「完成設定」會先列出所有變更讓你確認，你按下「套用」才會生效。"
                : "以上是自動挑選的低風險本機能力。想逐項自訂？完成後到「連接與權限」隨時調整；安靜時段、硬體掃描與 iPhone 配對會在第一次需要時再問你。按「完成設定」會先列出所有變更讓你確認。"}
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
              <button className="primary" onClick={prepare} disabled={committing || preparing}>
                {committing ? "套用中…" : preparing ? "檢查變更中…" : "完成設定"}
              </button>
            )}
          </div>
        </footer>
      </div>
      {pending && (
        <ConfirmApply
          rows={pending.rows}
          estimatedBecause={pending.estimatedBecause}
          onCancel={() => setPending(null)}
          onApply={commit}
        />
      )}
    </div>
  );
}

/** 套用前確認：把每一項會變的東西攤開，使用者按「套用」才動手。 */
function ConfirmApply({
  rows,
  estimatedBecause,
  onCancel,
  onApply,
}: {
  rows: ChangeRow[];
  estimatedBecause: string | null;
  onCancel: () => void;
  onApply: () => void;
}) {
  return (
    <Dialog title="套用前確認" onClose={onCancel}>
      {estimatedBecause ? (
        <p className="state-box state-warn" role="note">
          以本機快照估算（無法取得系統試算：{estimatedBecause}）。實際結果以套用後的狀態為準。
        </p>
      ) : null}
      {rows.length === 0 ? (
        <p>沒有任何變更。按「套用」只會記錄你已完成設定。</p>
      ) : (
        <>
          <p className="muted small">按下「套用」之前，什麼都不會改變。以下是會變的項目：</p>
          <ul className="plain-list" aria-label="將要變更的項目">
            {rows.map((row) => (
              <li key={row.key}>
                {row.label}：{row.from ? `${row.from} → ${row.to}` : `設為 ${row.to}`}
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="row" style={{ marginTop: 12 }}>
        <button onClick={onCancel}>取消</button>
        <button className="primary" onClick={onApply}>
          套用
        </button>
      </div>
    </Dialog>
  );
}

/** AI 幫手步驟：只做 Discovery／登入狀態檢查，不授權任何工作區寫入。 */
function AgentStep({
  name,
  choice,
  onChoice,
}: {
  name: string;
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
      <h1>要讓{name}幫忙工作嗎？</h1>
      <p className="muted">
        {name}可以把任務交給本機的 AI 幫手（Codex 擅長寫程式與整理資料；Claude Code 擅長對話、
        知識與審閱）。這一步只檢查安裝與登入狀態，<strong>不會</strong>授權讀寫任何資料夾——實際建立工作
        時才逐項授權，且隨時可取消。
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
        <legend>你的選擇（之後可到「工作」頁調整）</legend>
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
      <p className="muted small">
        這個選擇只會寫進「工作」頁的 AI 路由偏好（哪類任務優先交給誰）。
        指定的 Agent 不可用時不會自動改送另一家。
      </p>
    </section>
  );
}

/** 步驟二選擇 → 既有 agent 路由偏好。「稍後再說」不動任何設定。 */
export function agentRoutesFor(choice: string): Record<string, string> | null {
  const all = (agent: string) => ({
    conversation: agent,
    programming: agent,
    knowledge: agent,
    review: agent,
  });
  switch (choice) {
    case "codex":
      return all("codex");
    case "claude":
      return all("claude-code");
    case "both":
      // 不限制到單一家：各用途沿用建議路由（程式 → Codex，其餘 → Claude Code）。
      return {
        conversation: "claude-code",
        programming: "codex",
        knowledge: "claude-code",
        review: "claude-code",
      };
    default:
      return null;
  }
}

/** 小樞家族的預覽：由桌面角色使用的同一套程式即時繪製，不是設計稿。 */
function PackPeek({ name }: { name: string }) {
  const ref = React.useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = React.useState<string | null>(null);
  React.useEffect(() => {
    try {
      const canvas = ref.current;
      const ctx = canvas?.getContext("2d");
      if (!canvas || !ctx) throw new Error("no canvas context");
      drawExpressionPreview(ctx, "idle", "maid-classic", 128);
    } catch (e) {
      setFailed(String(e));
    }
  }, []);
  if (failed)
    return <p className="muted small">（角色預覽載入失敗：{failed}）</p>;
  return (
    <div className="row" style={{ justifyContent: "center" }}>
      <canvas ref={ref} width={128} height={128} aria-label={`${name}預覽（與桌面角色同一套即時繪製）`} />
    </div>
  );
}

/** 非小樞角色：不畫 rig、不講物種；只用可信文字說明它會怎麼出現。 */
function CharacterPeekText({ name }: { name: string }) {
  return (
    <p className="muted small" role="note">
      {name}會依角色自己宣告的方式出現在桌面（圖像或文字）。緊急停止、被阻擋、結果不確定等安全訊息
      永遠是固定文字，角色無法改寫。
    </p>
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
