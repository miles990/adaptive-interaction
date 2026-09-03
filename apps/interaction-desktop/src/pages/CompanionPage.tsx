// 角色頁（v0.5 一般模式五分區，依序）：目前角色／外觀與名字／平常如何陪伴／安靜與勿擾／
// 更換或加入角色。技術資料（manifest 原文、schema 版本、引擎、adapter、通道、Behavior State
// 數值）只在進階模式的收合區塊。
//
// 角色名稱一律 useCharacterName()；預覽依 manifest.entrypoint 分流（不用 pack id 字串）。
// 安全語句固定不可覆寫；成功綠勾只在 verified。主動對話／主動程度／安靜時段的編輯器
// 只住在這一頁（單一主人；見 regressions-v05 守門測試）。

import React from "react";
import { api, type CharacterInstanceView } from "../api";
import { useAppState } from "../appstate";
import { refreshCharacterName, useCharacterName } from "../characterName";
import { desktop, DesktopPrefs, isTauri } from "../desktop";
import { Badge, Section, Toggle, useAsync } from "../ui";
import { PRIMARY_INSTANCE_ID } from "../companion/gatewayWiring";
import { emptyMemory, memorySummary, noteReactionDisabled, sanitizeMemory } from "../companion/interactionMemory";
import { rollCallKey } from "../companion/playfield";
import { CharacterLibrarySection } from "./character/CharacterLibrary";
import { CharacterPreview } from "./character/CharacterPreview";
import { PreferencesForm } from "./character/PreferencesForm";
import { TechnicalDetails } from "./character/TechnicalDetails";
import {
  isShuRig,
  originLabel,
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

// ---------------------------------------------------------------------------
// 角色即時狀態（可信 host 文案；崩潰／失聯一律「改用文字」）
// ---------------------------------------------------------------------------

export const CHARACTER_UNAVAILABLE_TEXT = "角色目前無法顯示，改用文字";

export interface CharacterLiveState {
  label: string;
  kind: "ok" | "warn" | "bad" | "muted" | "pending";
  detail: string;
}

const CRASHED_LIFECYCLES = new Set(["crashed", "reconnecting", "disposed"]);
const HIDDEN_LIFECYCLES = new Set(["hidden", "suspended"]);
const READY_LIFECYCLES = new Set(["ready", "shown", "resumed", "reconfiguring"]);

/** Runtime 實例（優先）＋ presence 推導；沒有任何回報就誠實說未連線。 */
export function characterLiveState(
  instance: Pick<CharacterInstanceView, "lifecycle" | "connected"> | null,
  presence: Record<string, unknown> | null
): CharacterLiveState {
  if (instance) {
    if (!instance.connected || CRASHED_LIFECYCLES.has(instance.lifecycle)) {
      return {
        label: CHARACTER_UNAVAILABLE_TEXT,
        kind: "warn",
        detail: "角色的呈現程式已停止或失去連線；安全訊息會改以固定文字顯示，系統與進行中的工作不受影響。",
      };
    }
    if (HIDDEN_LIFECYCLES.has(instance.lifecycle) || presence?.visible === false) {
      return { label: "已隱藏", kind: "muted", detail: "角色視窗已連線但目前隱藏；打開「顯示桌面角色」就會出現。" };
    }
    if (READY_LIFECYCLES.has(instance.lifecycle)) {
      return { label: "角色視窗運作中", kind: "ok", detail: "角色視窗已連線並正在呈現。" };
    }
    return { label: "準備中", kind: "pending", detail: "角色視窗正在載入或協商中。" };
  }
  if (presence?.connected === true) {
    return presence.visible === true
      ? { label: "角色視窗運作中", kind: "ok", detail: "角色視窗已連線並正在呈現。" }
      : { label: "已隱藏", kind: "muted", detail: "角色視窗已連線但目前隱藏；打開「顯示桌面角色」就會出現。" };
  }
  return {
    label: "角色視窗未連線",
    kind: "bad",
    detail: "桌面角色視窗沒有連上（瀏覽器檢視沒有角色視窗）。安全訊息仍會以固定文字顯示在控制中心。",
  };
}

// ---------------------------------------------------------------------------
// 頁面
// ---------------------------------------------------------------------------

export function CompanionPage({
  refreshKey,
  advanced: advancedProp,
}: {
  refreshKey: number;
  /** 未提供時讀 AppState（prefs.mode）。 */
  advanced?: boolean;
  onNavigate?: (tab: string) => void;
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
  const [prefSource, setPrefSource] = React.useState<PreferenceSource | null>(null);
  const catalog = useCharacterCatalog(refreshKey);

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
      try {
        setPrefs(await desktop.prefsGet());
      } catch {
        /* 桌面 prefs 只在 Tauri 存在 */
      }
    }
  }, []);
  React.useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => window.clearInterval(timer);
  }, [load, refreshKey]);

  const patch = React.useCallback(async (p: Partial<DesktopPrefs>) => {
    setBusy(true);
    try {
      setPrefs(await desktop.prefsPatch(p));
      await desktop.companionApplyPrefs();
      setError(null);
      if (p.companionPack !== undefined || p.companionName !== undefined) {
        void refreshCharacterName({ force: true });
      }
    } catch (e) {
      setError(sanitizeErrorText(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const activeId =
    prefs?.companionPack ??
    (typeof presence?.packId === "string" && presence.packId.length > 0 ? presence.packId : null) ??
    catalog.defaultId ??
    "shu-maid";
  const active = catalog.cards.find((c) => c.characterId === activeId) ?? null;
  const shu = isShuRig(active);
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
      await patch({ companionPack: characterId });
      setNotice(
        target
          ? `已改用「${target.name}」。桌面角色視窗會重新載入；無法顯示時會改用文字。`
          : `已改用「${characterId}」。`
      );
    },
    [catalog.cards, patch]
  );

  const disableCharacter = React.useCallback(async () => {
    await patch({ companionPack: TEXT_FALLBACK_CHARACTER_ID });
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

  const live = characterLiveState(instance, presence);
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

  const currentVariant =
    typeof effective.values[VARIANT_PREFERENCE_KEY] === "string"
      ? String(effective.values[VARIANT_PREFERENCE_KEY])
      : (variants[0]?.id ?? "");

  return (
    <div className="character-page">
      {/* 1. 目前角色 */}
      <Section title="目前角色">
        <div className="character-current">
          <div className="character-current-head">
            <h3 className="character-current-name">{name}</h3>
            {active && <Badge kind={active.origin === "builtin" ? "info" : "warn"}>{originLabel(active.origin)}</Badge>}
            {active?.flags.external && <Badge kind="warn">外部</Badge>}
            {active?.flags.executable && <Badge kind="bad">有可執行程式</Badge>}
            {active?.flags.network && <Badge kind="warn">需要網路</Badge>}
            <Badge kind={live.kind}>{live.label}</Badge>
          </div>
          <p className="muted small">{live.detail}</p>
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
          ) : catalog.loaded ? (
            <div className="state-box">找不到目前設定的角色資料；桌面角色視窗會改用文字顯示。</div>
          ) : (
            <div className="state-box">正在讀取角色資料…</div>
          )}
          {prefs && (
            <Toggle
              checked={prefs.companionVisible}
              onChange={(on) => void patch({ companionVisible: on })}
              label="顯示桌面角色"
            />
          )}
          <p className="muted small">
            隱藏角色只會停止角色視窗內的感知與呈現；系統、狀態列與進行中的工作都會繼續。隱藏不等於緊急停止。
          </p>
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
      </Section>

      {/* 2. 外觀與名字 */}
      <Section title="外觀與名字">
        {!prefs ? (
          <div className="state-box">桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。</div>
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
        <CharacterPreview card={active} />
      </Section>

      {/* 3. 平常如何陪伴 */}
      <Section title="平常如何陪伴">
        {!prefs ? (
          <div className="state-box">桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。</div>
        ) : (
          <>
            <div className="settings-grid">
              <label className="field-label">
                表現程度（只影響表演與說話頻率，不影響任何權限）
                <select
                  value={prefs.companionExpressiveness}
                  onChange={(e) => void patch({ companionExpressiveness: e.target.value })}
                >
                  <option value="quiet">安靜</option>
                  <option value="natural">自然</option>
                  <option value="lively">活潑</option>
                </select>
              </label>
              {shu && (
                <label className="field-label">
                  說話風格
                  <select
                    value={prefs.companionPersona}
                    onChange={(e) => void patch({ companionPersona: e.target.value })}
                  >
                    <option value="persona-shu">{name}・預設</option>
                    <option value="persona-navigator">導航員（世界觀範例）</option>
                  </select>
                </label>
              )}
            </div>
            {schema && (
              <>
                <h3>{name}的偏好</h3>
                <PreferencesForm schema={schema} values={effective.values} disabled={busy} onChange={(k, v) => void changePreference(k, v)} />
                {(prefSource === "local" || effective.source === "local") && (
                  <p className="muted small" role="status">
                    這個版本的桌面程式尚未保存角色偏好；目前只在這個視窗記住，重新啟動後會回到預設。
                  </p>
                )}
              </>
            )}
            {shu ? (
              <ShuPlayControls prefs={prefs} patch={patch} name={name} pronoun={pronoun} presence={presence} />
            ) : (
              <BasicCompanionToggles prefs={prefs} patch={patch} pronoun={pronoun} />
            )}
            <p className="muted small">
              說話風格與表現程度只改變表達方式；緊急停止、被阻擋、結果不確定等安全訊息永遠使用固定文字，
              任何角色都無法覆寫或隱藏。
            </p>
          </>
        )}
      </Section>

      {/* 4. 安靜與勿擾 */}
      <Section title="安靜與勿擾">
        <p className="muted small">
          這一區決定{name}什麼時候保持安靜。安全訊息（緊急停止中、被阻擋、結果不確定、感測使用中）
          不受任何安靜設定影響，一定會顯示。
        </p>
        {prefs ? (
          <>
            <Toggle
              checked={prefs.companionDoNotDisturb === true}
              onChange={(on) => void patch({ companionDoNotDisturb: on })}
              label="勿擾（安靜陪伴：不主動靠近、不主動說話）"
            />
            {Number(prefs.companionProactiveQuietUntil ?? 0) > Date.now() && (
              <p className="muted small" role="status">
                本機安靜期至{" "}
                {new Date(Number(prefs.companionProactiveQuietUntil)).toLocaleTimeString("zh-TW", {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
                。
              </p>
            )}
          </>
        ) : (
          <div className="state-box">勿擾開關需要桌面版控制中心；下方的主動對話與安靜時段在瀏覽器也能設定。</div>
        )}
      </Section>
      <ProactiveDialogueSection name={name} />
      <InitiativeQuietSection refreshKey={refreshKey} />

      {/* 5. 更換或加入角色 */}
      <CharacterLibrarySection
        catalog={catalog}
        activeId={activeId}
        prefs={prefs}
        busy={busy}
        onSelect={selectCharacter}
        onDisable={disableCharacter}
        onRemove={removeCharacter}
        onPatch={patch}
        setError={setError}
      />

      {advanced && <TechnicalDetails card={active} instance={instance} presence={presence} />}
    </div>
  );
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
  patch: (p: Partial<DesktopPrefs>) => Promise<void>;
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
// 平常如何陪伴：小樞（builtin shu-rig）的玩耍設定；其他角色只有 host 層的基本開關
// ---------------------------------------------------------------------------

function BasicCompanionToggles({
  prefs,
  patch,
  pronoun,
}: {
  prefs: DesktopPrefs;
  patch: (p: Partial<DesktopPrefs>) => Promise<void>;
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

function ShuPlayControls({
  prefs,
  patch,
  name,
  pronoun,
  presence,
}: {
  prefs: DesktopPrefs;
  patch: (p: Partial<DesktopPrefs>) => Promise<void>;
  name: string;
  pronoun: string;
  presence: Record<string, unknown> | null;
}) {
  const familiars = prefs.companionFamiliars ?? [];
  const memory = sanitizeMemory(prefs.companionInteractionMemory);
  const remembers = memorySummary(memory);

  /** 關掉某個反應時，順便記進角色互動記憶（純呈現，不推論人格）。 */
  const patchReaction = async (reaction: string, p: Partial<DesktopPrefs>, enabled: boolean) => {
    if (enabled) {
      await patch(p);
      return;
    }
    await patch({ ...p, companionInteractionMemory: noteReactionDisabled(memory, reaction, Date.now()) });
  };

  return (
    <div className="character-play">
      <div className="settings-grid">
        <label className="field-label">
          場景（透明桌面模式下只加一點小道具）
          <select value={String(prefs.companionScene ?? "none")} onChange={(e) => void patch({ companionScene: e.target.value })}>
            <option value="none">透明桌面（預設）</option>
            <option value="nest">桌面巢穴</option>
            <option value="desk">工作桌</option>
            <option value="sill">窗台</option>
            <option value="night">夜間</option>
          </select>
        </label>
      </div>
      <Toggle checked={prefs.companionPlay !== false} onChange={(on) => void patch({ companionPlay: on })} label="玩耍（玩具、追逐、撲抓）" />
      <Toggle
        checked={prefs.companionCursorPlay !== false}
        onChange={(on) => void patch({ companionCursorPlay: on })}
        label="游標互動（光點、逗貓棒跟著游標）"
      />
      <Toggle checked={prefs.companionApproach !== false} onChange={(on) => void patch({ companionApproach: on })} label="游標靠近時看過來" />
      <Toggle checked={prefs.companionDeskMove !== false} onChange={(on) => void patch({ companionDeskMove: on })} label="在遊玩場內自主散步" />
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
      <p className="muted small">
        玩具與游標互動只發生在角色的透明小視窗內；游標座標不會送到系統或 AI、也不會被保存。
        減少動態效果開啟時自動停止玩耍與移動。
      </p>
      <h4>使魔（最多 3 隻，純陪伴、沒有任何權限）</h4>
      {familiars.length === 0 && <p className="muted small">還沒有使魔。</p>}
      {familiars.map((f, i) => (
        <div className="row wrap" key={f.id} style={{ marginBottom: 4 }}>
          <input
            value={f.name}
            maxLength={24}
            aria-label={`使魔 ${i + 1} 名字`}
            onChange={(e) => {
              const next = familiars.map((x, j) => (j === i ? { ...x, name: e.target.value } : x));
              void patch({ companionFamiliars: next });
            }}
          />
          <select
            value={f.palette}
            aria-label={`使魔 ${i + 1} 配色`}
            onChange={(e) => {
              const next = familiars.map((x, j) => (j === i ? { ...x, palette: e.target.value } : x));
              void patch({ companionFamiliars: next });
            }}
          >
            <option value="maid-classic">經典</option>
            <option value="maid-dusk">暮色</option>
            <option value="maid-sakura">櫻花</option>
          </select>
          <button onClick={() => void patch({ companionFamiliars: familiars.filter((_, j) => j !== i) })}>移除</button>
        </div>
      ))}
      {familiars.length < 3 && (
        <button
          onClick={() =>
            void patch({
              companionFamiliars: [
                ...familiars,
                { id: `fam-${Date.now() % 100000}`, name: `使魔${familiars.length + 1}`, palette: "maid-classic" },
              ],
            })
          }
        >
          新增使魔
        </button>
      )}
      <h4>{name}記得</h4>
      {remembers.length === 0 ? (
        <p className="muted small">
          還沒有互動記憶。（只會記玩過的玩具、你常關掉的反應與相處天數；不會變成正式知識，也不會離開本機。）
        </p>
      ) : (
        <>
          <ul className="plain-list muted small">
            {remembers.map((linetext, i) => (
              <li key={`${i}-${linetext}`}>
                {name}記得：{linetext}
              </li>
            ))}
          </ul>
          {/* 沒有你不能刪除的記憶：互動記憶也一樣，一鍵清空並寫回偏好（不是只清畫面）。 */}
          <p className="muted small">
            <button onClick={() => void patch({ companionInteractionMemory: emptyMemory() })}>
              忘記這些
            </button>{" "}
            會清掉玩過的玩具、常關掉的反應與相處天數；{name}會從頭開始記。
          </p>
        </>
      )}
      <RollCall presence={presence} />
    </div>
  );
}

/** Roll Call：現在大家在做什麼（來自角色視窗的真實回報；離線就誠實說）。 */
function RollCall({ presence }: { presence: Record<string, unknown> | null }) {
  const state = (presence?.behaviorState as Record<string, unknown> | null | undefined) ?? null;
  const roll = (state?.rollCall as { name: string; activity: string }[] | undefined) ?? null;
  return (
    <>
      <h4>現在大家在做什麼</h4>
      {!roll ? (
        <div className="state-box">尚未收到角色視窗的回報（角色隱藏、離線或剛啟動時不會用預設值冒充）。</div>
      ) : (
        <ul className="plain-list">
          {roll.map((r, i) => (
            <li key={rollCallKey(i, r.name)}>
              <strong>{r.name}</strong>：{r.activity}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// 主動式對話（v0.5 起唯一主人＝角色頁；模式與頻率由 Rust 確定性強制）。
// ---------------------------------------------------------------------------

function ProactiveDialogueSection({ name }: { name: string }) {
  const [status, setStatus] = React.useState<Record<string, unknown> | null>(null);
  const [agents, setAgents] = React.useState<Record<string, unknown>[]>([]);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      setStatus(await api.proactiveDialogueGet());
      setError(null);
    } catch (e) {
      setError(sanitizeErrorText(e));
    }
  }, []);
  React.useEffect(() => {
    void load();
    void api
      .agentsDiscoveries()
      .then((result) => setAgents((result.agents as Record<string, unknown>[] | undefined) ?? []))
      .catch(() => setAgents([]));
  }, [load]);

  const config = (status?.config as Record<string, unknown> | undefined) ?? {};
  const mode = String(config.mode ?? "natural");
  const custom = (config.custom as Record<string, unknown> | undefined) ?? {};
  const quietUntil = status?.quietUntil ? new Date(String(status.quietUntil)) : null;
  const quietActive = quietUntil !== null && quietUntil.getTime() > Date.now();

  const setMode = async (m: string) => {
    try {
      setStatus(await api.proactiveDialoguePatch({ mode: m }));
      setError(null);
    } catch (e) {
      setError(sanitizeErrorText(e));
    }
  };
  const patch = async (value: Record<string, unknown>) => {
    try {
      setStatus(await api.proactiveDialoguePatch(value));
      setError(null);
    } catch (e) {
      setError(sanitizeErrorText(e));
    }
  };
  const agentLabel = (kind: string) => (kind === "codex" ? "Codex" : kind === "claude-code" ? "Claude Code" : kind);
  const generativeToday = (status?.generativeToday as Record<string, unknown> | undefined) ?? {};

  return (
    <Section title="主動式對話">
      <p className="muted small">
        {name}什麼情況下可以主動說話。頻率限制（每小時最多 {String(config.maxPerHour ?? 3)} 則、 最短間隔{" "}
        {String(config.minIntervalMinutes ?? 12)} 分鐘、沒有回覆不追問）由系統強制執行；
        安全與權限提示不受模式影響，一定會顯示。主動說話不代表可以主動做事——任何行動仍需授權。
      </p>
      <label className="field-label">
        模式
        <select value={mode} onChange={(e) => void setMode(e.target.value)}>
          <option value="off">關閉——不主動說話</option>
          <option value="necessary">必要——只有等待確認、失敗、結果不確定與感測提示</option>
          <option value="natural">自然（建議）——加上任務進度與低頻建議</option>
          <option value="lively">活潑——再加問候與輕量陪伴</option>
          <option value="custom">自訂——個別選擇訊息類型</option>
        </select>
      </label>
      {mode === "custom" && (
        <fieldset>
          <legend>自訂觸發類型</legend>
          {(
            [
              ["taskProgress", "任務進度"],
              ["completion", "任務完成"],
              ["suggestion", "情境建議"],
              ["greeting", "問候"],
              ["companionship", "輕量陪伴"],
              ["worldEvent", "世界觀小事件"],
            ] as const
          ).map(([key, label]) => (
            <label className="row" key={key}>
              <input
                type="checkbox"
                checked={custom[key] === true}
                onChange={(event) => void patch({ custom: { ...custom, [key]: event.target.checked } })}
              />
              {label}
            </label>
          ))}
        </fieldset>
      )}
      <div className="settings-grid">
        <label className="field-label">
          每小時最多則數
          <input
            type="number"
            min={1}
            max={12}
            value={Number(config.maxPerHour ?? 3)}
            onChange={(event) => void patch({ maxPerHour: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          最短間隔（分鐘）
          <input
            type="number"
            min={1}
            max={60}
            value={Number(config.minIntervalMinutes ?? 12)}
            onChange={(event) => void patch({ minIntervalMinutes: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          事件合併窗（秒）
          <input
            type="number"
            min={5}
            max={300}
            value={Number(config.mergeWindowSeconds ?? 30)}
            onChange={(event) => void patch({ mergeWindowSeconds: Number(event.target.value) })}
          />
        </label>
      </div>
      <label className="row">
        <input
          type="checkbox"
          checked={config.noFollowUp !== false}
          onChange={(event) => void patch({ noFollowUp: event.target.checked })}
        />
        沒有回覆時不追問
      </label>
      <label className="row">
        <input
          type="checkbox"
          checked={config.dndDefer !== false}
          onChange={(event) => void patch({ dndDefer: event.target.checked })}
        />
        勿擾時段延後非必要訊息
      </label>
      <hr />
      <h4>由本機 AI 幫手產生的主動訊息</h4>
      <p className="muted small">
        沒有選擇 AI 幫手時只保留本機微反應與固定安全提示。選擇不會授予讀檔、工具、網路或行動權；
        每一則都是獨立、唯讀的一次性工作，不會留下長期工作。
      </p>
      <label className="field-label">
        指定 AI 幫手（不可用時不會自動改送另一家）
        <select
          value={String(config.generativeAgent ?? "")}
          onChange={(event) => void patch({ generativeAgent: event.target.value || null })}
        >
          <option value="">不使用 AI 幫手產生主動訊息</option>
          <option value="codex">Codex（寫程式與整理資料的本機 AI 幫手）</option>
          <option value="claude-code">Claude Code（對話、知識與審閱的本機 AI 幫手）</option>
        </select>
      </label>
      <div className="muted small">
        {agents.map((agent) => (
          <span key={String(agent.kind)} style={{ marginRight: 12 }}>
            {agentLabel(String(agent.kind))}：
            {agent.found === true && agent.loggedIn === true ? "可用" : String(agent.detail ?? "不可用")}
          </span>
        ))}
      </div>
      <div className="settings-grid">
        <label className="field-label">
          每日產生次數上限
          <input
            type="number"
            min={0}
            max={50}
            value={Number(config.dailyGenerativeSessions ?? 8)}
            onChange={(event) => void patch({ dailyGenerativeSessions: Number(event.target.value) })}
          />
        </label>
        <label className="field-label">
          每日費用上限（USD）
          <input
            type="number"
            min={0}
            max={100}
            step="0.1"
            value={Number(config.dailyGenerativeCostUsd ?? 1)}
            onChange={(event) => void patch({ dailyGenerativeCostUsd: Number(event.target.value) })}
          />
        </label>
      </div>
      <p className="muted small">
        今天已由 AI 幫手產生 {String(generativeToday.sessions ?? 0)} 則，費用回報 USD {String(generativeToday.costUsd ?? 0)}。
      </p>
      <p className="muted small">
        本小時已發送 {String(status?.sentThisHour ?? 0)} 則。
        {quietActive && quietUntil
          ? ` 安靜中，至 ${quietUntil.toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" })}。`
          : ""}
      </p>
      <div className="row-gap">
        <button
          onClick={async () => {
            try {
              setStatus(await api.proactiveDialogueQuiet(60));
            } catch (e) {
              setError(sanitizeErrorText(e));
            }
          }}
        >
          一小時內不要主動說話
        </button>
        <button
          onClick={async () => {
            try {
              setStatus(await api.proactiveDialogueQuiet(12 * 60));
            } catch (e) {
              setError(sanitizeErrorText(e));
            }
          }}
        >
          今天安靜一點
        </button>
      </div>
      {error && (
        <p className="cap-card-error" role="alert">
          {error}
        </p>
      )}
    </Section>
  );
}

// ---------------------------------------------------------------------------
// 主動程度與安靜時段（v0.5 起唯一主人＝角色頁；由本機安全層強制執行）。
// ---------------------------------------------------------------------------

function InitiativeQuietSection({ refreshKey }: { refreshKey: number }) {
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
      setError(sanitizeErrorText(e));
    } finally {
      setSaving(false);
    }
  }

  if (policy.loading)
    return (
      <Section title="主動程度與安靜時段">
        <div className="state-box">載入中…</div>
      </Section>
    );
  if (policy.error)
    return (
      <Section title="主動程度與安靜時段">
        <div className="state-box state-error">{policy.error}</div>
      </Section>
    );
  const p = policy.data!;
  const quiet = (p["quietHours"] as { start: string; end: string }[] | undefined)?.[0];
  const initiative = String(p["initiative"] ?? "suggest");

  return (
    <Section title="主動程度與安靜時段">
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
