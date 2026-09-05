// 角色頁（M3 §4.1 首屏收斂）：
//
// 首屏只回答三件事——
//   1. 目前角色：名字、現在怎麼樣、預覽，以及顯示／暫停這個主要動作；
//   2. 陪伴方式：一句話的預設摘要（安靜／自然／活潑／自訂）＋「調整」展開；
//   3. 手機連接／同步：CharacterSyncCard（`onNavigate` 由這一頁往下傳）。
// 其餘（外觀與名字、細部行為、安靜與勿擾、完整頻率與 AI 生成設定、角色庫）改為
// 按需展開的 <details>：鍵盤可達、有可及名稱、沒有動畫。
//
// 收合 ≠ 隱藏事實（誠實階梯）：
//   - 主動式對話收起來時，摘要行仍然帶著**有效值**——每小時／最短間隔／每日次數／
//     費用上限／指定的 AI 幫手。收起數字調校不等於藏起使用成本或重新授權。
//   - 六組「安靜」語意各自獨立，攤在「安靜與勿擾」逐項列出（項目／由哪個設定控制／
//     現在的有效狀態），**不**合併成一個布林；安全提示與感測提示永遠不受安靜設定影響。
//   - 套用陪伴預設只寫既有的三個欄位（見 `src/companion/presets.ts`）：不覆蓋其它自訂值、
//     不改費用上限、不啟用任何權限、不更換指定的 AI 幫手。
//
// 技術資料（安全宣告、manifest 原文、schema 版本、引擎、adapter、通道、Behavior State、
// 執行位置／可執行程式／需要網路旗標、貼上角色描述檔原文、事件合併窗）只在進階模式。
//
// 角色名稱一律 useCharacterName()；預覽依 manifest.entrypoint 分流（不用 pack id 字串）。
// 安全語句固定不可覆寫；成功綠勾只在 verified。主動對話／主動程度／安靜時段的編輯器
// 只住在這一頁（單一主人；見 regressions-v05 守門測試）。

import React from "react";
import { api, type CharacterInstanceView, type RuntimeEvent } from "../api";
import { useAppState } from "../appstate";
import { refreshCharacterName, useCharacterName } from "../characterName";
import { desktop, DesktopPrefs, isTauri } from "../desktop";
import { projectCharacterLifecycle } from "../statusProjection";
import { Section, Toggle, useAsync } from "../ui";
import { CharacterSyncCard } from "../components/CharacterSyncCard";
import { PRIMARY_INSTANCE_ID } from "../companion/gatewayWiring";
import { noteReactionDisabled, sanitizeMemory } from "../companion/interactionMemory";
import {
  describeCompanionState,
  expressivenessLabel,
  presetFor,
  proactiveModeLabel,
  type CompanionPresetId,
} from "../companion/presets";
import {
  beginPresetOp,
  markerOf,
  projectPresetStatus,
  readPendingPresetOp,
  shouldResumePendingOp,
  type PresetOpMarker,
} from "../companion/applyPresetPlan";
// 註冊 builtin adapter 工廠與 meta（副作用）：這一頁的角色專屬區塊全部靠 meta 決定。
import "../character/adapters";
import { builtinAdapterMeta } from "../character/adapterRegistry";
import { CharacterLibrarySection } from "./character/CharacterLibrary";
import { CharacterPreview } from "./character/CharacterPreview";
import { PreferencesForm } from "./character/PreferencesForm";
import { TechnicalDetails } from "./character/TechnicalDetails";
import { CompanionPresetRow } from "./companion/CompanionPresets";
import { CurrentCharacterCard } from "./companion/FirstScreen";
import { Disclosure } from "./companion/Disclosure";
import { libraryDigest } from "./companion/libraryDigest";
import { ProactiveSettings, type ProactiveConfig } from "./companion/ProactiveSettings";
import { QuietImpactList, type QuietImpactItem } from "./companion/QuietControls";
import {
  extraPermissionLine,
  sanitizeErrorText,
  siblingForVariant,
  TEXT_FALLBACK_CHARACTER_ID,
  useCharacterCatalog,
  variantName,
  type CharacterCard,
} from "./character/catalog";
import {
  effectivePreferences,
  persistCharacterPreferences,
  VARIANT_PREFERENCE_KEY,
  type PreferenceSource,
  type PreferenceValue,
} from "./character/preferences";
import { buildQuietHoursPatch, QUIET_SILENCED_CHANNELS as CANONICAL_QUIET_SILENCED_CHANNELS } from "../quietHours";

/**
 * 目前角色 id：使用者選的 → 角色視窗回報的 → 索引宣告的預設 → 純文字角色。
 * 最後一段刻意是永遠可用的純文字角色，頁面不引用任何特定角色的 id。
 */
export function resolveActiveCharacterId(
  companionPack: string | null | undefined,
  presencePackId: unknown,
  defaultId: string | null | undefined
): string {
  if (typeof companionPack === "string" && companionPack.length > 0) return companionPack;
  if (typeof presencePackId === "string" && presencePackId.length > 0) return presencePackId;
  if (typeof defaultId === "string" && defaultId.length > 0) return defaultId;
  return TEXT_FALLBACK_CHARACTER_ID;
}

// ---------------------------------------------------------------------------
// 主動式對話：狀態與寫入（v0.5 起唯一主人＝角色頁；模式與頻率由 Rust 確定性強制）。
// 表單本身在 `./companion/ProactiveSettings`，它不碰 api——設定只有一個主人。
// ---------------------------------------------------------------------------

const AGENT_LABELS: Record<string, string> = { codex: "Codex", "claude-code": "Claude Code" };

function agentLabel(kind: string): string {
  return AGENT_LABELS[kind] ?? kind;
}

/** 後端回報的設定 → 有效值（缺值時退回後端同一組預設；不猜、不放寬）。 */
function proactiveConfigOf(status: Record<string, unknown> | null): ProactiveConfig {
  const c = (status?.config as Record<string, unknown> | undefined) ?? {};
  const agent = c.generativeAgent;
  return {
    mode: String(c.mode ?? "natural"),
    custom: (c.custom as Record<string, unknown> | undefined) ?? {},
    maxPerHour: Number(c.maxPerHour ?? 3),
    minIntervalMinutes: Number(c.minIntervalMinutes ?? 12),
    mergeWindowSeconds: Number(c.mergeWindowSeconds ?? 30),
    noFollowUp: c.noFollowUp !== false,
    dndDefer: c.dndDefer !== false,
    generativeAgent: typeof agent === "string" && agent.length > 0 ? agent : null,
    dailyGenerativeSessions: Number(c.dailyGenerativeSessions ?? 8),
    dailyGenerativeCostUsd: Number(c.dailyGenerativeCostUsd ?? 1),
  };
}

function useProactiveDialogue() {
  const [status, setStatus] = React.useState<Record<string, unknown> | null>(null);
  const [agents, setAgents] = React.useState<Record<string, unknown>[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  /**
   * 有效值讀不回來（GET 失敗）。這與「寫入失敗」不同：寫入失敗至少還知道現在生效的是什麼，
   * 讀不回來就是**不知道**——畫面不得顯示任何檔位結論（`unverified`）。
   */
  const [readbackFailed, setReadbackFailed] = React.useState(false);

  /**
   * 寫入的世代計數器（對抗審查 general-mode-ux-013）。
   *
   * `status` 這份共享狀態只有一個 owner，但同一時間可能有好幾個請求在飛：進頁面時的 GET、
   * 使用者按下的檔位、進階區的逐項修改。回應**不保證**照送出順序回來，先前的寫法是誰後
   * 回來誰說了算——舊回應蓋掉新回應之後，畫面顯示的模式與後端真正生效的就不一致了。
   * 每一次寫入開一個新世代；比目前世代舊的回應（含 mount 時那次 GET）一律不寫 status。
   */
  const generation = React.useRef(0);

  React.useEffect(() => {
    let alive = true;
    const issued = generation.current;
    /** 這次讀取還算不算數：中途有人寫入過就不算（不用舊讀數覆蓋新寫入）。 */
    const current = () => alive && generation.current === issued;
    void api
      .proactiveDialogueGet()
      .then((r) => {
        if (!current()) return;
        setStatus(r);
        setError(null);
        setReadbackFailed(false);
      })
      .catch((e) => {
        if (!current()) return;
        setError(sanitizeErrorText(e));
        setReadbackFailed(true);
      });
    void api
      .agentsDiscoveries()
      .then((result) => alive && setAgents((result.agents as Record<string, unknown>[] | undefined) ?? []))
      .catch(() => alive && setAgents([]));
    return () => {
      alive = false;
    };
  }, []);

  /**
   * 一次寫入：開新世代 → 送出 → **只有**這次仍是最新的世代時才寫回 status／error。
   * 被後來的請求取代掉的舊回應（成功或失敗）都不寫：那一則講的是已經被覆蓋的世代，
   * 而使用者看到的必須是最後一次請求的結果（那一則自己的成敗照樣會顯示）。
   */
  const write = React.useCallback(
    async (send: () => Promise<Record<string, unknown>>): Promise<boolean> => {
      generation.current += 1;
      const issued = generation.current;
      try {
        const next = await send();
        if (generation.current !== issued) return false;
        setStatus(next);
        setError(null);
        // 寫入成功回來的就是權威狀態：先前讀不回來的疑慮到此解除。
        setReadbackFailed(false);
        return true;
      } catch (e) {
        if (generation.current !== issued) return false;
        setError(sanitizeErrorText(e));
        return false;
      }
    },
    []
  );

  const patch = React.useCallback(
    async (value: Record<string, unknown>) => await write(() => api.proactiveDialoguePatch(value)),
    [write]
  );

  const quiet = React.useCallback(
    async (minutes: number) => await write(() => api.proactiveDialogueQuiet(minutes)),
    [write]
  );

  /**
   * 讀回目前的有效設定。**不**開新世代（這是讀，不是寫），但一樣受世代保護：
   * 中途有人寫入過就不用這份讀數覆蓋。
   *
   * 也**不**清除 `error`：那一則講的是剛剛那次寫入失敗，讀回不替它背書——
   * 「後端其實收到了」與「這次寫入回報失敗」兩件事可以同時為真，兩件都要說。
   */
  const readback = React.useCallback(async (): Promise<Record<string, unknown> | null> => {
    const issued = generation.current;
    try {
      const next = await api.proactiveDialogueGet();
      if (generation.current !== issued) return null;
      setStatus(next);
      setReadbackFailed(false);
      return next;
    } catch {
      if (generation.current !== issued) return null;
      setReadbackFailed(true);
      return null;
    }
  }, []);

  return { status, agents, error, readbackFailed, patch, quiet, readback };
}

// ---------------------------------------------------------------------------
// 頁面
// ---------------------------------------------------------------------------

export function CompanionPage({
  refreshKey,
  advanced: advancedProp,
  events,
  connectionKey = 0,
  onNavigate,
}: {
  refreshKey: number;
  /** 未提供時讀 AppState（prefs.mode）。 */
  advanced?: boolean;
  /** Runtime SSE 事件；同步卡用其中的 `character.session.state` 對齊本地副本。 */
  events?: RuntimeEvent[];
  /**
   * 「這條連線換了一條」的訊號（supervisor 連線狀態變化／SSE 重連時 +1）。
   * 同步卡收到就重新對齊一次；它**不**隨每則事件變動。
   */
  connectionKey?: number;
  /** 同步卡的「下一步」要一鍵到得了連接與權限頁（M3 §4.2）。 */
  onNavigate?: (tab: string, opts?: Record<string, unknown>) => void;
}) {
  const { prefs: uiPrefs } = useAppState();
  const advanced = advancedProp ?? uiPrefs.mode === "advanced";
  const { name, pronoun } = useCharacterName({ refreshKey });
  const [prefs, setPrefs] = React.useState<DesktopPrefs | null>(null);
  const [presence, setPresence] = React.useState<Record<string, unknown> | null>(null);
  const [instance, setInstance] = React.useState<CharacterInstanceView | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  /** 桌面偏好**讀取**失敗（Tauri host 在，但讀不到）。與「這是瀏覽器檢視」是兩回事。 */
  const [prefsError, setPrefsError] = React.useState<string | null>(null);
  const [prefSource, setPrefSource] = React.useState<PreferenceSource | null>(null);
  const [showAllCharacters, setShowAllCharacters] = React.useState(false);
  const catalog = useCharacterCatalog(refreshKey);
  const proactive = useProactiveDialogue();
  /** 桌面偏好的寫入世代（見 `patch()`／`load()`）。 */
  const prefsGeneration = React.useRef(0);

  const load = React.useCallback(async () => {
    try {
      setPresence(await api.presentationStatus());
    } catch (e) {
      setError(sanitizeErrorText(e));
    }
    try {
      const r = await api.characterInstances();
      const list = Array.isArray(r?.instances) ? r.instances : [];
      setInstance(list.find((i) => i.instanceId === PRIMARY_INSTANCE_ID) ?? null);
    } catch {
      setInstance(null);
    }
    if (isTauri) {
      // 讀取也受寫入世代保護：五秒一次的輪詢不得用一份出發得比較早的讀數
      // 覆蓋掉剛剛寫進去的偏好（與 `patch()` 對稱）。
      const issued = prefsGeneration.current;
      try {
        const next = await desktop.prefsGet();
        if (prefsGeneration.current !== issued) return;
        setPrefs(next);
        setPrefsError(null);
      } catch (e) {
        // 桌面版讀不到偏好是**失敗**，不是「這是瀏覽器檢視」：說出原因，
        // 並讓相關區塊顯示「無法確認生效值」而不是假裝在瀏覽器裡。
        if (prefsGeneration.current !== issued) return;
        setPrefsError(sanitizeErrorText(e));
      }
    }
  }, []);
  React.useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => window.clearInterval(timer);
  }, [load, refreshKey]);

  /**
   * 寫入桌面角色偏好。回傳 `true` **只**代表 host 真的接受了這次寫入。
   *
   * 誠實階梯（送出 ≠ 完成）：這裡曾經把失敗吞成 error 狀態之後照樣 resolve，
   * 於是呼叫端（`selectCharacter`／`disableCharacter`）在錯誤訊息旁邊又貼上
   * 「已改用…」「已停用目前角色…」的成功文案——瀏覽器檢視沒有 Tauri host，
   * 每一次都同時顯示兩種互相矛盾的說法。失敗就只留錯誤，成功才有成功文案。
   */
  const patch = React.useCallback(async (p: Partial<DesktopPrefs>): Promise<boolean> => {
    // 寫入世代（與 `useProactiveDialogue.write()` 對稱）：偏好也是一份共享狀態，
    // 回應不保證照送出順序回來。舊回應寫回 state 就會把使用者剛選的值蓋掉，
    // 畫面顯示的與 host 真正保存的從此不一致。比目前世代舊的回應一律不寫。
    prefsGeneration.current += 1;
    const issued = prefsGeneration.current;
    const current = () => prefsGeneration.current === issued;
    setBusy(true);
    try {
      const next = await desktop.prefsPatch(p);
      await desktop.companionApplyPrefs();
      if (!current()) return false;
      setPrefs(next);
      setError(null);
      if (p.companionPack !== undefined || p.companionName !== undefined) {
        void refreshCharacterName({ force: true });
      }
      return true;
    } catch (e) {
      if (!current()) return false;
      setError(sanitizeErrorText(e));
      // 上一次成功留下的提示不得替這一次失敗背書。
      setNotice(null);
      return false;
    } finally {
      // 還有比較新的請求在飛時不解鎖：忙碌要涵蓋到最後一次請求結束。
      if (current()) setBusy(false);
    }
  }, []);

  const activeId = resolveActiveCharacterId(prefs?.companionPack, presence?.packId, catalog.defaultId);
  const active = catalog.cards.find((c) => c.characterId === activeId) ?? null;
  // 角色專屬區塊（說話風格、遊玩場設定）全部由該角色的 adapter meta 宣告；
  // 這一頁不認得任何角色 id，也不 import 任何角色的配色／說話風格表（M2 §3.4）。
  const adapterMeta = builtinAdapterMeta(active?.entrypoint);
  const personas = adapterMeta?.personas ?? [];
  const PlayfieldControls = adapterMeta?.hasPlayfield ? (adapterMeta.playfieldControls ?? null) : null;
  const schema = active?.manifest?.preferencesSchema;
  const variants = active?.manifest?.variants ?? [];
  const variantIds = React.useMemo(() => variants.map((v) => v.id), [variants]);
  const effective = React.useMemo(
    () => effectivePreferences(schema, activeId, prefs, { variantIds }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [schema, activeId, prefs, variantIds, prefSource]
  );

  const selectCharacter = React.useCallback(
    async (characterId: string) => {
      const target = catalog.cards.find((c) => c.characterId === characterId);
      if (!(await patch({ companionPack: characterId }))) return;
      setNotice(
        target
          ? `已改用「${target.name}」。桌面角色視窗會重新載入；無法顯示時會改用文字。`
          : `已改用「${characterId}」。`
      );
    },
    [catalog.cards, patch]
  );

  const disableCharacter = React.useCallback(async () => {
    if (!(await patch({ companionPack: TEXT_FALLBACK_CHARACTER_ID }))) return;
    setNotice("已停用目前角色，改用純文字角色；安全訊息照常以固定文字顯示。");
  }, [patch]);

  const removeCharacter = React.useCallback(
    async (characterId: string) => {
      setBusy(true);
      try {
        await desktop.characterRemove(characterId);
        setError(null);
        setNotice("已移除角色。");
      } catch (e) {
        setError(`移除失敗：${sanitizeErrorText(e)}`);
        setBusy(false);
        return;
      }
      setBusy(false);
      if (characterId === activeId) {
        await selectCharacter(catalog.defaultId ?? TEXT_FALLBACK_CHARACTER_ID);
      }
      catalog.reload();
    },
    [activeId, catalog, selectCharacter]
  );

  const changePreference = React.useCallback(
    async (key: string, value: PreferenceValue) => {
      if (!prefs) {
        setError("角色偏好需要桌面版控制中心（此為瀏覽器檢視）。");
        return;
      }
      const values = { ...effective.values, [key]: value };
      try {
        const r = await persistCharacterPreferences(activeId, values, prefs);
        setPrefs(r.prefs);
        setPrefSource(r.persisted);
        setError(null);
      } catch (e) {
        setError(sanitizeErrorText(e));
      }
    },
    [activeId, effective.values, prefs]
  );

  const chooseVariant = React.useCallback(
    async (variantId: string) => {
      if (!active) return;
      const sibling = siblingForVariant(catalog.cards, active, variantId);
      if (sibling) {
        if (sibling !== active.characterId) await selectCharacter(sibling);
        return;
      }
      await changePreference(VARIANT_PREFERENCE_KEY, variantId);
    },
    [active, catalog.cards, changePreference, selectCharacter]
  );

  const live = projectCharacterLifecycle(instance, presence);
  const explanation = String(
    presence?.behaviorExplanation ??
      (presence?.behaviorState as Record<string, unknown> | null | undefined)?.explanation ??
      ""
  );
  // Runtime 回報「已測試」只在實例就是這個角色時才採信（視窗可能還跑著上一個角色）。
  const summaryLines = React.useMemo(() => {
    if (!active) return [];
    if (instance?.tested === true && instance.characterId === active.characterId && !active.tested) {
      return active.summary.map((line) =>
        line.startsWith("已測試：") ? "已測試：是（角色視窗完成過一次完整演出並回報）" : line
      );
    }
    return active.summary;
  }, [active, instance?.tested, instance?.characterId]);
  const extraPermission = active ? extraPermissionLine(active) : null;

  const currentVariant =
    typeof effective.values[VARIANT_PREFERENCE_KEY] === "string"
      ? String(effective.values[VARIANT_PREFERENCE_KEY])
      : (variants[0]?.id ?? "");

  // ---- 陪伴預設（既有欄位的組合；套用只寫那三個欄位） ----
  const config = proactiveConfigOf(proactive.status);
  const presetInputs = {
    expressiveness: prefs?.companionExpressiveness ?? null,
    doNotDisturb: prefs ? prefs.companionDoNotDisturb === true : null,
    proactiveMode: proactive.status ? config.mode : null,
  };
  const presetChoice = presetFor(presetInputs);
  /**
   * 套用檔位是**兩段寫入**（桌面偏好 → 後端主動對話模式）。`patch()` 自己的 busy 只涵蓋
   * 第一段，第二段還在飛的時候按鈕就解鎖了，快速切換檔位會讓兩次套用交錯
   *（對抗審查 general-mode-ux-013）。這個旗標涵蓋整個 applyPreset。
   */
  const [presetBusy, setPresetBusy] = React.useState(false);
  /** 正在補送上一次沒完成的第二段（mount 恢復）。 */
  const [recovering, setRecovering] = React.useState(false);
  /** 交易期間：檔位按鈕與同一組欄位的個別控制項都不得同時被改。 */
  const presetTransaction = presetBusy || recovering;
  /** 桌面偏好裡那筆「還有一段沒確認完成」的標記（驗過才用）。 */
  const pendingOp = React.useMemo(
    () => readPendingPresetOp(prefs?.companionPendingPresetOp),
    [prefs?.companionPendingPresetOp]
  );
  const presetStatus = projectPresetStatus({
    presetChoice,
    pendingOp,
    busy: presetBusy,
    recovering,
    readbackFailed: proactive.readbackFailed,
  });

  /**
   * 第二段：把 marker 記下來的 mode 送出去，然後**確認**。
   *
   * 送出失敗不等於沒送到（回應可能只是遺失），所以失敗後先讀回：讀回等於目標就是
   * 完成，清掉 marker；讀不回、或讀回還不是目標，就把 marker 留著——畫面會說
   * 「半套用」並給補送。清 marker 是**第三次**寫入，它自己失敗只會讓狀態多留一輪
   *（下次補送是冪等的），不會讓使用者以為套用成功。
   */
  const runSecondStage = React.useCallback(
    async (marker: PresetOpMarker): Promise<boolean> => {
      if (!(await proactive.patch(marker.proactivePatch))) {
        // 讀回**明說**的模式才算數：回應裡沒有 mode 就是不知道（不用預設值頂替，
        // 那會讓「後端沒說」被當成「已經是自然」而誤判成完成）。
        const readback = await proactive.readback();
        const config = (readback?.config as Record<string, unknown> | undefined) ?? {};
        const landed = typeof config.mode === "string" && config.mode === marker.proactivePatch.mode;
        if (!landed) return false;
      }
      await patch({ companionPendingPresetOp: null });
      return true;
    },
    [patch, proactive.patch, proactive.readback]
  );

  const applyPreset = React.useCallback(
    async (id: CompanionPresetId) => {
      const plan = beginPresetOp(id, Date.now());
      if (!plan) return;
      const marker = markerOf(plan);
      setPresetBusy(true);
      try {
        // 第一段與 marker 是**同一次**寫入：偏好寫進去了，marker 就一定也在
        //（不可能出現「偏好改了但沒人記得第二段還沒送」的空窗）。
        // 送出 ≠ 完成：第一段沒寫成功就不要再去動後端的主動對話模式。
        if (!(await patch({ ...plan.prefs, companionPendingPresetOp: marker }))) return;
        await runSecondStage(marker);
      } finally {
        setPresetBusy(false);
      }
    },
    [patch, runSecondStage]
  );

  /** 補送：重送同一段（只有 mode，冪等）。 */
  const retryPendingPreset = React.useCallback(async () => {
    if (!pendingOp) return;
    setPresetBusy(true);
    try {
      await runSecondStage(pendingOp);
    } finally {
      setPresetBusy(false);
    }
  }, [pendingOp, runSecondStage]);

  /**
   * 重開之後的恢復（每次 mount 最多一次，有界）：第一次讀到桌面偏好時就決定。
   *
   * 只有 marker 鎖定的偏好欄位**仍等於**目前值才補送——使用者事後改過就只把 marker
   * 清掉，不用一份過時的意圖覆蓋他剛選的設定。marker 壞掉（被手改／舊版本）同樣清掉。
   * 兩個視窗同時開已由 single-instance 擋住，所以這裡不必再處理跨視窗的競爭。
   */
  const resumeChecked = React.useRef(false);
  React.useEffect(() => {
    if (!prefs || resumeChecked.current) return;
    resumeChecked.current = true;
    const marker = readPendingPresetOp(prefs.companionPendingPresetOp);
    if (!marker) {
      if (prefs.companionPendingPresetOp) void patch({ companionPendingPresetOp: null });
      return;
    }
    const resumable = shouldResumePendingOp(marker, {
      expressiveness: prefs.companionExpressiveness,
      doNotDisturb: prefs.companionDoNotDisturb === true,
    });
    if (!resumable) {
      void patch({ companionPendingPresetOp: null });
      return;
    }
    setRecovering(true);
    void (async () => {
      try {
        await runSecondStage(marker);
      } finally {
        setRecovering(false);
      }
    })();
  }, [prefs, patch, runSecondStage]);

  // ---- 安靜與勿擾：六組語意各自的底層設定與有效狀態 ----
  const [policy, reloadPolicy] = useAsync(() => api.policyGet(), [refreshKey]);
  const [policySaving, setPolicySaving] = React.useState(false);
  const [policySaved, setPolicySaved] = React.useState(false);
  const [policyError, setPolicyError] = React.useState<string | null>(null);
  async function patchPolicy(p: Record<string, unknown>) {
    setPolicySaving(true);
    setPolicyError(null);
    setPolicySaved(false);
    try {
      await api.policyPatch(p);
      setPolicySaved(true);
      reloadPolicy();
    } catch (e) {
      setPolicyError(sanitizeErrorText(e));
    } finally {
      setPolicySaving(false);
    }
  }
  const quietHours = (policy.data?.["quietHours"] as { start: string; end: string }[] | undefined)?.[0];
  const localQuietUntil = Number(prefs?.companionProactiveQuietUntil ?? 0);
  const localQuietActive = localQuietUntil > Date.now();
  const proactiveQuietUntil = proactive.status?.quietUntil ? new Date(String(proactive.status.quietUntil)) : null;
  const proactiveQuietActive = proactiveQuietUntil !== null && proactiveQuietUntil.getTime() > Date.now();
  const dndLabel = prefs
    ? prefs.companionDoNotDisturb === true
      ? "開啟"
      : "關閉"
    : isTauri
      ? "不明（讀不到桌面角色設定）"
      : "不明（需要桌面版控制中心）";
  const quietHoursLabel = policy.loading
    ? "讀取中…"
    : quietHours
      ? `${quietHours.start}–${quietHours.end}`
      : "未啟用";
  const quietImpact: QuietImpactItem[] = [
    {
      id: "safety",
      label: "安全提示（緊急停止中、被阻擋、結果不確定、失敗）",
      source: "固定安全文字",
      state: "永遠顯示，不受任何安靜設定影響",
    },
    {
      id: "sensing",
      label: "感測提示（麥克風、攝影機）",
      source: "感測器本身的開關",
      state: "只要感測使用中就一定顯示；安靜設定不會讓它安靜下來",
    },
    {
      id: "companion",
      label: `視覺陪伴（${pronoun}主動靠近、隨口說話）`,
      source: "勿擾",
      state: prefs
        ? prefs.companionDoNotDisturb === true
          ? "勿擾中：不主動靠近、不主動說話"
          : "照常陪伴"
        : isTauri
          ? "不明（讀不到桌面角色設定）"
          : "不明（桌面角色設定需要桌面版控制中心）",
      notes: localQuietActive
        ? [
            `另外還有一段本機安靜期，至 ${formatClock(localQuietUntil)}（由桌面角色自己的選單設定，這一頁只顯示）。`,
          ]
        : [],
    },
    {
      id: "proactive",
      label: `主動說話（${name}主動開口）`,
      source: "主動式對話",
      state: proactive.status ? proactiveModeLabel(config.mode) : "讀取中…",
      notes: [
        proactiveQuietActive && proactiveQuietUntil
          ? `目前安靜中，至 ${proactiveQuietUntil.toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" })}。`
          : "目前沒有設定安靜期。",
        config.dndDefer ? "勿擾時段會延後非必要訊息（必要訊息照送）。" : "勿擾時段不延後非必要訊息。",
      ],
    },
    {
      id: "notifications",
      label: "工作通知（聲音、震動、通知、燈光）",
      source: "安靜時段",
      state: quietHoursLabel,
    },
  ];

  // ---- 角色庫：預設只列使用中＋最近／常用；其餘收在「顯示全部角色」後面 ----
  const usedIds = React.useMemo(
    () => Object.keys(prefs?.companionPreferences ?? {}),
    [prefs?.companionPreferences]
  );
  const digest = React.useMemo(
    () => libraryDigest(catalog.cards, activeId, { usedIds }),
    [catalog.cards, activeId, usedIds]
  );
  const libraryCatalog = React.useMemo(
    () => (showAllCharacters ? catalog : { ...catalog, cards: digest.shown }),
    [showAllCharacters, catalog, digest.shown]
  );

  /**
   * M3a 會在 CharacterSyncCard 補上 `onNavigate?: (tab, opts?) => void`
   *（同步卡的「下一步」要一鍵到得了連接與權限頁）。在它補上之前用 spread 傳遞——
   * 補上之後這裡不必再改，沒有這個 prop 的版本會忽略它。
   */
  const syncNavProps: { onNavigate?: (tab: string, opts?: Record<string, unknown>) => void } = { onNavigate };

  return (
    <div className="character-page">
      <div className="character-first-screen">
        {/* 1. 目前角色 */}
        <Section title="目前角色">
          <CurrentCharacterCard
            name={name}
            active={active}
            advanced={advanced}
            live={live}
            explanation={explanation}
            summaryLines={summaryLines}
            extraPermission={extraPermission}
            catalogLoaded={catalog.loaded}
            visible={prefs ? prefs.companionVisible : null}
            onVisibleChange={(on) => void patch({ companionVisible: on })}
            preview={<CharacterPreview card={active} name={name} />}
            error={error}
            notice={notice}
          />
          {/* 角色資料載入失敗是首屏的事：收在「更換或加入角色」裡會讓「找不到角色資料」
              看起來沒有原因（收合區塊不得把失敗藏起來）。 */}
          {catalog.errors.map((e) => (
            <p key={e} className="cap-card-error" role="alert">
              {e}
            </p>
          ))}
        </Section>

        {/* 2. 陪伴方式：一句話摘要＋三個檔位；細部行為收在「調整」。 */}
        <Section title="陪伴方式">
          {!prefs ? (
            <PrefsUnavailable error={prefsError} primary />
          ) : (
            <CompanionPresetRow
              choice={presetChoice}
              effectiveLines={describeCompanionState(presetInputs)}
              // 兩段寫入都算忙碌：後端那一段還在飛（或正在補送）時不得再按下一個檔位。
              busy={busy || presetTransaction}
              status={presetStatus}
              pendingPresetId={pendingOp?.presetId ?? null}
              onApply={(id) => void applyPreset(id)}
              onRetry={() => void retryPendingPreset()}
            />
          )}
          {/* 主動說話的設定失敗只有這一個家，而且在首屏：從收合區塊裡按下去失敗時
              不會連錯誤都跟著被收起來（送出 ≠ 完成）。 */}
          {proactive.error && (
            <p className="cap-card-error" role="alert">
              主動說話的設定沒有寫入成功：{proactive.error}
            </p>
          )}
          <Disclosure
            id="behavior"
            title="調整陪伴方式"
            summary={
              prefs
                ? `表現程度：${expressivenessLabel(prefs.companionExpressiveness)}・說話風格與細部行為`
                : isTauri
                  ? "讀不到桌面角色設定"
                  : "需要桌面版控制中心"
            }
          >
            {!prefs ? (
              <PrefsUnavailable error={prefsError} />
            ) : (
              <>
                <div className="settings-grid">
                  <label className="field-label">
                    表現程度（只影響表演與說話頻率，不影響任何權限）
                    <select
                      value={prefs.companionExpressiveness}
                      // 檔位交易正在寫同一個欄位：中途改成別的值會讓補送的判斷失真
                      //（`shouldResumePendingOp` 靠「使用者沒改過」）。
                      disabled={presetTransaction}
                      onChange={(e) => void patch({ companionExpressiveness: e.target.value })}
                    >
                      <option value="quiet">安靜</option>
                      <option value="natural">自然</option>
                      <option value="lively">活潑</option>
                    </select>
                  </label>
                  {personas.length > 0 && (
                    <label className="field-label">
                      說話風格
                      <select
                        value={prefs.companionPersona}
                        onChange={(e) => void patch({ companionPersona: e.target.value })}
                      >
                        {personas.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.followsName ? `${name}・${p.label}` : p.label}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                </div>
                {schema && (
                  <>
                    <h3>{name}的偏好</h3>
                    <PreferencesForm
                      schema={schema}
                      values={effective.values}
                      disabled={busy}
                      onChange={(k, v) => void changePreference(k, v)}
                    />
                    {(prefSource === "local" || effective.source === "local") && (
                      <p className="muted small" role="status">
                        這個版本的桌面程式尚未保存角色偏好；目前只在這個視窗記住，重新啟動後會回到預設。
                      </p>
                    )}
                  </>
                )}
                {PlayfieldControls ? (
                  <PlayfieldControls prefs={prefs} patch={patch} name={name} pronoun={pronoun} presence={presence} />
                ) : (
                  <BasicCompanionToggles prefs={prefs} patch={patch} pronoun={pronoun} />
                )}
                <p className="muted small">
                  說話風格與表現程度只改變表達方式；緊急停止、被阻擋、結果不確定等安全訊息永遠使用固定文字，
                  任何角色都無法覆寫或隱藏。
                </p>
              </>
            )}
          </Disclosure>
        </Section>

        {/* 3. 同步（AIP Character Session；`docs/aip/character-session.md` §11）：
            手機上的角色跟這台電腦是不是同一個狀態。一般模式只有人話，
            revision／sequence／計數留在進階模式的「連接診斷」。 */}
        <Section title="同步">
          <CharacterSyncCard
            refreshKey={refreshKey}
            advanced={advanced}
            sessionEvents={events}
            connectionKey={connectionKey}
            {...syncNavProps}
          />
        </Section>
      </div>

      {/* 以下按需展開。收合摘要一律帶著有效值，收起來不等於看不到。 */}
      <Disclosure id="appearance" title="外觀與名字" summary="名字、外觀、大小、透明度、位置">
        {!prefs ? (
          <PrefsUnavailable error={prefsError} />
        ) : (
          <AppearanceControls
            prefs={prefs}
            active={active}
            busy={busy}
            patch={patch}
            setError={setError}
            variants={variants}
            currentVariant={currentVariant}
            onVariant={chooseVariant}
          />
        )}
      </Disclosure>

      <Disclosure
        id="quiet"
        title="安靜與勿擾"
        summary={`勿擾：${dndLabel}・主動說話：${proactive.status ? proactiveModeLabel(config.mode) : "讀取中…"}・安靜時段：${quietHoursLabel}`}
      >
        <p className="muted small">
          這一區決定{name}什麼時候保持安靜。「安靜」有好幾種不同的意思，下面逐項列出各自由哪個設定控制、
          現在的有效狀態是什麼——它們**不是**同一個開關。安全訊息與感測提示不受任何安靜設定影響。
        </p>
        <QuietImpactList items={quietImpact} />
        {prefs ? (
          <>
            <Toggle
              checked={prefs.companionDoNotDisturb === true}
              onChange={(on) => void patch({ companionDoNotDisturb: on })}
              // 勿擾也是檔位交易寫的欄位之一：交易期間不得同時被改。
              disabled={presetTransaction}
              label="勿擾（安靜陪伴：不主動靠近、不主動說話）"
            />
            {localQuietActive && (
              <div className="row wrap">
                <span className="muted small" role="status">
                  本機安靜期至 {formatClock(localQuietUntil)}。
                </span>
                <button onClick={() => void patch({ companionProactiveQuietUntil: 0 })} disabled={busy}>
                  取消本機安靜期
                </button>
              </div>
            )}
          </>
        ) : (
          <PrefsUnavailable
            error={prefsError}
            browserText="勿擾開關需要桌面版控制中心；下方的主動程度與安靜時段在瀏覽器也能設定。"
          />
        )}
        <div className="row-gap">
          <button onClick={() => void proactive.quiet(60)}>一小時內不要主動說話</button>
          <button onClick={() => void proactive.quiet(12 * 60)}>今天安靜一點</button>
        </div>
        <hr />
        <PolicySettings
          loading={policy.loading}
          loadError={policy.error}
          initiative={String(policy.data?.["initiative"] ?? "suggest")}
          quiet={quietHours}
          patch={(p) => void patchPolicy(p)}
          saving={policySaving}
          saved={policySaved}
          error={policyError}
        />
      </Disclosure>

      <Disclosure
        id="proactive"
        title="主動式對話"
        summary={`每小時最多 ${config.maxPerHour} 則・最短間隔 ${config.minIntervalMinutes} 分鐘・每日最多 ${config.dailyGenerativeSessions} 則・費用上限 USD ${config.dailyGenerativeCostUsd}・AI 幫手：${config.generativeAgent ? agentLabel(config.generativeAgent) : "不使用"}`}
      >
        <ProactiveSettings
          name={name}
          advanced={advanced}
          config={config}
          agents={proactive.agents.map((agent) => ({
            kind: String(agent.kind),
            label: agentLabel(String(agent.kind)),
            detail: agent.found === true && agent.loggedIn === true ? "可用" : String(agent.detail ?? "不可用"),
          }))}
          sentThisHour={Number(proactive.status?.sentThisHour ?? 0)}
          generativeToday={{
            sessions: Number((proactive.status?.generativeToday as Record<string, unknown> | undefined)?.sessions ?? 0),
            costUsd: Number((proactive.status?.generativeToday as Record<string, unknown> | undefined)?.costUsd ?? 0),
          }}
          onPatch={(value) => void proactive.patch(value)}
        />
      </Disclosure>

      <Disclosure
        id="library"
        title="更換或加入角色"
        summary={`目前使用「${name}」・共 ${catalog.cards.length} 個角色`}
      >
        <CharacterLibrarySection
          catalog={libraryCatalog}
          activeId={activeId}
          prefs={prefs}
          busy={busy}
          advanced={advanced}
          onSelect={selectCharacter}
          onDisable={disableCharacter}
          onRemove={removeCharacter}
          onPatch={patch}
          setError={setError}
        />
        {digest.hidden > 0 && (
          <div className="row wrap character-library-more">
            <button onClick={() => setShowAllCharacters((v) => !v)}>
              {showAllCharacters ? "只顯示使用中與常用角色" : `顯示全部角色（另外 ${digest.hidden} 個）`}
            </button>
          </div>
        )}
      </Disclosure>

      {advanced && <TechnicalDetails card={active} instance={instance} presence={presence} />}
    </div>
  );
}

/**
 * 讀不到桌面偏好時的說明。
 *
 * 兩件完全不同的事以前共用同一句話：**瀏覽器檢視**（本來就沒有 Tauri host，正常）與
 * **桌面版讀取失敗**（host 在，但 `desktop_prefs_get` 失敗）。後者被寫成前者是誤報：
 * 使用者在桌面程式裡看到「此為瀏覽器檢視」，會以為設定沒問題、只是這個視窗不管事，
 * 而實際上是讀不到設定、畫面上的任何值都不能當成生效值（誠實階梯：不知道就說不知道）。
 */
function PrefsUnavailable({
  error,
  primary,
  browserText = "桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。",
}: {
  error: string | null;
  /** 首屏那一份：真的失敗時要讓螢幕閱讀器聽得到（其餘幾份不重複播報）。 */
  primary?: boolean;
  browserText?: string;
}) {
  if (!isTauri) return <div className="state-box">{browserText}</div>;
  return (
    <div
      className="state-box"
      data-testid={primary ? "companion-prefs-unavailable" : undefined}
      role={primary ? "alert" : undefined}
    >
      讀不到桌面角色設定{error ? `：${error}` : "（原因不明）"}
      。目前無法確認這些設定的生效值。
    </div>
  );
}

/** epoch ms → 時：分（本地時區）。 */
function formatClock(at: number): string {
  return new Date(at).toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" });
}

// ---------------------------------------------------------------------------
// 外觀與名字
// ---------------------------------------------------------------------------

function AppearanceControls({
  prefs,
  active,
  busy,
  patch,
  setError,
  variants,
  currentVariant,
  onVariant,
}: {
  prefs: DesktopPrefs;
  active: CharacterCard | null;
  busy: boolean;
  /** 回傳 `true` 只在 host 真的接受寫入時；失敗時呼叫端不得顯示成功文案。 */
  patch: (p: Partial<DesktopPrefs>) => Promise<boolean>;
  setError: (e: string | null) => void;
  variants: { id: string; displayName?: Record<string, string> }[];
  currentVariant: string;
  onVariant: (variantId: string) => Promise<void>;
}) {
  const [nameDraft, setNameDraft] = React.useState(prefs.companionName ?? "");
  React.useEffect(() => setNameDraft(prefs.companionName ?? ""), [prefs.companionName]);
  const placeholder = active?.name ?? "角色";
  return (
    <>
      <div className="settings-grid">
        <label className="field-label">
          名字（只影響顯示與稱呼；留空就用角色原名）
          <input
            value={nameDraft}
            maxLength={24}
            placeholder={placeholder}
            onChange={(e) => setNameDraft(e.target.value)}
            onBlur={() => {
              const next = nameDraft.trim().slice(0, 24);
              if (next !== (prefs.companionName ?? "")) void patch({ companionName: next });
            }}
            aria-label="角色名字"
          />
        </label>
        {variants.length > 0 && (
          <label className="field-label">
            外觀
            <select
              value={currentVariant}
              aria-label="外觀"
              disabled={busy}
              onChange={(e) => void onVariant(e.target.value)}
            >
              {variants.map((v) => (
                <option key={v.id} value={v.id}>
                  {variantName(v)}
                </option>
              ))}
            </select>
          </label>
        )}
        <label className="field-label">
          大小
          <select
            value={String(prefs.companionSize?.[0] ?? 200)}
            onChange={(event) => {
              const width = Number(event.target.value);
              void patch({ companionSize: [width, Math.round(width * 1.05)] });
            }}
          >
            <option value="160">小</option>
            <option value="200">標準</option>
            <option value="260">大</option>
            <option value="320">特大</option>
          </select>
        </label>
        <label className="field-label">
          透明度 {Math.round((prefs.companionOpacity ?? 1) * 100)}%
          <input
            type="range"
            min={20}
            max={100}
            step={5}
            value={Math.round((prefs.companionOpacity ?? 1) * 100)}
            onChange={(event) => void patch({ companionOpacity: Number(event.target.value) / 100 })}
          />
        </label>
      </div>
      <Toggle
        checked={prefs.companionAlwaysOnTop}
        onChange={(on) => void patch({ companionAlwaysOnTop: on })}
        label="保持在其他視窗上方"
      />
      <div className="row wrap">
        <button
          onClick={async () => {
            try {
              await desktop.companionResetPosition();
              await patch({});
            } catch (reason) {
              setError(sanitizeErrorText(reason));
            }
          }}
        >
          重設角色位置
        </button>
        <button onClick={() => void patch({ storyProgress: {} })} title="重看初次見面等劇情段落">
          重看角色劇情
        </button>
      </div>
      <p className="muted small">
        外觀只改變表現方式，不改變任何權限；安全訊息永遠是固定文字。
      </p>
    </>
  );
}

// ---------------------------------------------------------------------------
// 平常如何陪伴：宣告了遊玩場的角色由它自己的 adapter 提供設定 UI（meta.playfieldControls）；
// 其他角色只有 host 層的基本開關。這一頁不認得任何角色的玩具、配色或部位名。
// ---------------------------------------------------------------------------

function BasicCompanionToggles({
  prefs,
  patch,
  pronoun,
}: {
  prefs: DesktopPrefs;
  /** 回傳 `true` 只在 host 真的接受寫入時；失敗時呼叫端不得顯示成功文案。 */
  patch: (p: Partial<DesktopPrefs>) => Promise<boolean>;
  pronoun: string;
}) {
  const memory = sanitizeMemory(prefs.companionInteractionMemory);
  const patchReaction = async (reaction: string, p: Partial<DesktopPrefs>, enabled: boolean) => {
    if (enabled) {
      await patch(p);
      return;
    }
    await patch({ ...p, companionInteractionMemory: noteReactionDisabled(memory, reaction, Date.now()) });
  };
  return (
    <div className="character-basic-toggles">
      <Toggle
        checked={prefs.companionBubbles !== false}
        onChange={(on) => void patchReaction("bubbles", { companionBubbles: on }, on)}
        label="說話氣泡（關掉後只剩固定的安全訊息）"
      />
      <Toggle
        checked={prefs.companionSound === true}
        onChange={(on) => void patchReaction("sound", { companionSound: on }, on)}
        label="角色音效（預設關閉）"
      />
      <Toggle
        checked={prefs.companionDragEnabled !== false}
        onChange={(on) => void patchReaction("drag", { companionDragEnabled: on }, on)}
        label={`可以用滑鼠把${pronoun}拖到別的位置`}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// 主動程度與安靜時段（v0.5 起唯一主人＝角色頁；由本機安全層強制執行）。
// ---------------------------------------------------------------------------

function PolicySettings({
  loading,
  loadError,
  initiative,
  quiet,
  patch,
  saving,
  saved,
  error,
}: {
  loading: boolean;
  loadError?: string;
  initiative: string;
  quiet?: { start: string; end: string };
  patch: (p: Record<string, unknown>) => void;
  saving: boolean;
  saved: boolean;
  error: string | null;
}) {
  if (loading) return <div className="state-box">載入中…</div>;
  if (loadError) return <div className="state-box state-error">{loadError}</div>;
  return (
    <>
      <h3>主動程度與安靜時段</h3>
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
              <input type="radio" name="initiative" checked={initiative === v} onChange={() => patch({ initiative: v })} />
              {label}
            </label>
          ))}
        </fieldset>

        <fieldset>
          <legend>安靜時段</legend>
          <QuietHoursEditor value={quiet} onChange={(q) => patch({ quietHours: q ? [q] : [] })} />
        </fieldset>
      </div>
      {saving && <p className="muted small">儲存中…</p>}
      {saved && !saving && (
        <p className="muted small" role="status">
          已儲存，立即生效。
        </p>
      )}
      {error && (
        <p className="cap-card-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

/**
 * 安靜時段要消音的干擾通道。刻意不含桌面角色（L0 純呈現）：角色只是安靜地待在桌面上，
 * 不會發出聲音或通知；把它一起消音只會產生使用者無事可決的「被阻止」項目。
 * 空陣列會讓後端套用內建預設清單（含桌面角色），所以這裡一定送出明確清單。
 *
 * Re-export 自 ../quietHours（ia-settings-012 的 canonical builder）——
 * 角色頁與首次設定精靈共用同一份清單，避免兩邊字面量各自維護而漂移。
 * 保留這個名字是為了不破壞既有 import（測試與其他模組）。
 */
export const QUIET_SILENCED_CHANNELS = CANONICAL_QUIET_SILENCED_CHANNELS;

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
          onChange(on ? buildQuietHoursPatch(start, end) : null);
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
              onBlur={() => onChange(buildQuietHoursPatch(start, end))}
            />
          </label>
          <label>
            到
            <input
              type="time"
              value={end}
              onChange={(e) => setEnd(e.target.value)}
              onBlur={() => onChange(buildQuietHoursPatch(start, end))}
            />
          </label>
          <span className="muted small">
            期間聲音、震動、通知、燈光會被消音；桌面角色仍會安靜待著（不出聲、不通知）
          </span>
        </>
      )}
    </div>
  );
}
