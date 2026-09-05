// CPP §12 `shu-rig` 的遊玩場設定 UI（玩耍開關、場景、使魔、互動記憶、roll call）。
//
// M2 §3.4：這一整塊以前住在角色頁（`pages/CompanionPage.tsx`），頁面得先用
// `entrypoint === "shu-rig"` 判斷「現在是不是小樞」才決定要不要畫，還直接 import 這個
// rig 的配色表——等於呈現層知道某個特定角色的部位與配色名（CLAUDE.md 不變量）。
// 現在它是 adapter 自己的東西：由 `SHU_META.playfieldControls` 宣告，host 只看
// `builtinAdapterMeta(entrypoint)?.hasPlayfield` 決定掛不掛，不認得裡面有什麼。
//
// 這裡只讀寫桌面偏好（host 提供的 `patch`，回傳 `true` 才代表 host 真的接受寫入），
// 不碰任何權限、不直接控制裝置；roll call 只轉述角色視窗回報的內容，沒有回報就誠實說沒有。

import { emptyMemory, memorySummary, noteReactionDisabled, sanitizeMemory } from "../../companion/interactionMemory";
import { rollCallKey } from "../../companion/playfield";
import { Toggle } from "../../ui";
import type { PlayfieldControlsProps } from "../adapterRegistry";
import { PALETTES } from "./shu";

/** 這個 rig 的遊玩場場景（透明桌面模式下只加一點小道具）。 */
const SCENES: { id: string; label: string }[] = [
  { id: "none", label: "透明桌面（預設）" },
  { id: "nest", label: "桌面巢穴" },
  { id: "desk", label: "工作桌" },
  { id: "sill", label: "窗台" },
  { id: "night", label: "夜間" },
];

/**
 * 這個 rig 的場景 id（`SHU_META.scenes` 用它宣告）。清單只有一份：設定匯入驗證與這裡的
 * 選單讀的是同一份，host 端不再自帶一份「五個場景」的全域白名單。
 */
export const SHU_SCENE_IDS: readonly string[] = SCENES.map((s) => s.id);

/** 使魔上限（純陪伴、沒有任何權限）。 */
const MAX_FAMILIARS = 3;

export function ShuPlayControls({ prefs, patch, name, pronoun, presence }: PlayfieldControlsProps) {
  const familiars = prefs.companionFamiliars ?? [];
  const memory = sanitizeMemory(prefs.companionInteractionMemory);
  const remembers = memorySummary(memory);

  /** 關掉某個反應時，順便記進角色互動記憶（純呈現，不推論人格）。 */
  const patchReaction = async (reaction: string, p: Partial<typeof prefs>, enabled: boolean) => {
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
            {SCENES.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
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
      <h4>使魔（最多 {MAX_FAMILIARS} 隻，純陪伴、沒有任何權限）</h4>
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
            {PALETTES.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
          <button onClick={() => void patch({ companionFamiliars: familiars.filter((_, j) => j !== i) })}>移除</button>
        </div>
      ))}
      {familiars.length < MAX_FAMILIARS && (
        <button
          onClick={() =>
            void patch({
              companionFamiliars: [
                ...familiars,
                { id: `fam-${Date.now() % 100000}`, name: `使魔${familiars.length + 1}`, palette: PALETTES[0].id },
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
