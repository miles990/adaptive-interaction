// Desktop-app-only bridge (supervisor, lifecycle, tray) — always Tauri IPC,
// even when the runtime traffic itself flows over HTTP to an external daemon.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { configureHttp, isTauri } from "./transport";
import type { InteractionMemory } from "./companion/interactionMemory";
import type { LocalizedText } from "./character/protocol";
import type { ManifestReport } from "./character/manifest";

export interface SupervisorInfo {
  mode: "embedded" | "external" | "undecided";
  state:
    | "starting"
    | "embedded-owned"
    | "connected-to-external"
    | "ready"
    | "degraded"
    | "disconnected"
    | "stopping"
    | "stopped";
  apiBase: string;
  token?: string;
  detail?: string;
}

export interface DesktopPrefs {
  closeBehavior: "keep-running" | "hide-companion" | "quit" | null;
  askOnClose: boolean;
  launchAtLogin: boolean;
  showCompanionOnStart: boolean;
  openControlCenterOnStart: boolean;
  companionVisible: boolean;
  companionPosition?: [number, number] | null;
  companionSize: [number, number];
  companionOpacity: number;
  companionPack: string;
  companionPersona: string;
  companionExpressiveness: "quiet" | "natural" | "lively" | string;
  companionAlwaysOnTop: boolean;
  storyProgress: Record<string, boolean>;
  /** v0.5 遊玩偏好（純呈現，無任何權限語意）。 */
  companionName: string;
  companionScene: "none" | "nest" | "desk" | "sill" | "night" | string;
  companionPlay: boolean;
  companionCursorPlay: boolean;
  companionApproach: boolean;
  companionDeskMove: boolean;
  companionFamiliars: { id: string; name: string; palette: string }[];
  /** 勿擾：安靜基態（不主動靠近、不主動說話）。預設 false。 */
  companionDoNotDisturb: boolean;
  /** 說話氣泡（關掉後只剩固定的安全文字）。預設 true。 */
  companionBubbles: boolean;
  /** 角色音效。**預設 false**（不主動出聲）。 */
  companionSound: boolean;
  /** 允許拖曳角色視窗。預設 true。 */
  companionDragEnabled: boolean;
  /** 使用者要求的本機安靜期到期時間（epoch ms；0＝沒有）。
   *  只擋主動行為（隨口氣泡、hover 短句、ambient 表演）——安全文字照常。 */
  companionProactiveQuietUntil: number;
  /** 角色互動記憶（有界；純呈現，不會升級成正式知識）。 */
  companionInteractionMemory?: InteractionMemory;
  /**
   * 陪伴預設「兩段寫入」的恢復標記（M4；`src/companion/applyPresetPlan.ts`）。
   * 與第一段（表現程度＋勿擾）在**同一次** patch 原子寫入；第二段（後端主動說話模式）
   * 確認送到之後才清成 `null`。它不是新的設定層：有效值仍然只看那三個既有欄位，
   * 這裡只記「還有一段沒確認完成」，好讓重開之後補得回來、也說得出半套用。
   */
  companionPendingPresetOp?: {
    opId: string;
    presetId: string;
    proactivePatch: { mode: string };
    issuedAtMs: number;
  } | null;
  /** 各角色由 manifest.preferencesSchema 宣告的偏好值（characterId → 值表；純呈現）。
   *  host 尚未保存這個欄位時 patch 會被丟棄——角色頁會偵測回傳值並誠實告知。 */
  companionPreferences?: Record<string, Record<string, boolean | number | string>>;
  schemaVersion: number;
}

// ---- 角色匯入（Tauri host 指令；docs/character-protocol/README.md §2／§9） ----

export interface CharacterImportInput {
  /** manifest 原文（≤ 256 KB；host 端再驗證一次）。 */
  manifestText: string;
  /** manifest.assets 宣告的每個資產（id ＋ base64 內容）。 */
  assets: { id: string; base64: string }[];
}

export interface CharacterImportResult {
  characterId: string;
  displayName: LocalizedText;
  report: ManifestReport;
  assets: string[];
}

/** `character_list_imported` 的每一列（壞掉的資料夾也列出，valid:false，讓 UI 提供移除）。 */
export interface ImportedCharacterEntry {
  characterId: string;
  valid: boolean;
  error?: string;
  displayName: LocalizedText;
  adapterKind?: "in-process" | string;
  entrypoint?: "shu-rig" | "sprite" | "text" | string;
  version?: string;
  executable: boolean;
  network: boolean;
  external: boolean;
  report?: ManifestReport;
  assets: string[];
  origin: "imported";
}

const CHARACTER_IMPORT_NEEDS_DESKTOP =
  "角色匯入與管理需要桌面版控制中心（此為瀏覽器檢視，沒有本機角色資料夾）";

export const desktop = {
  /** 匯入第三方角色（只寫入本機角色資料夾；不執行任何 entrypoint）。瀏覽器模式下明確拒絕。 */
  characterImport: (input: CharacterImportInput) =>
    isTauri
      ? invoke<CharacterImportResult>("character_import", {
          manifestText: input.manifestText,
          assets: input.assets,
        })
      : Promise.reject(new Error(CHARACTER_IMPORT_NEEDS_DESKTOP)),
  /** 已匯入角色清單；瀏覽器模式下是空清單（不是錯誤）。 */
  characterListImported: () =>
    isTauri
      ? invoke<ImportedCharacterEntry[]>("character_list_imported")
      : Promise.resolve([] as ImportedCharacterEntry[]),
  /** 已匯入角色的資產（data URL，≤ 8 MB；host 會重新核對 magic bytes）。 */
  characterAsset: (characterId: string, assetId: string) =>
    isTauri
      ? invoke<string>("character_asset", { characterId, assetId })
      : Promise.reject(new Error(CHARACTER_IMPORT_NEEDS_DESKTOP)),
  /** 移除已匯入角色（內建角色不可移除）。 */
  characterRemove: (characterId: string) =>
    isTauri
      ? invoke<{ removed: string }>("character_remove", { characterId })
      : Promise.reject(new Error(CHARACTER_IMPORT_NEEDS_DESKTOP)),
  supervisorInfo: () => invoke<SupervisorInfo>("supervisor_info"),
  prefsGet: () => invoke<DesktopPrefs>("desktop_prefs_get"),
  prefsPatch: (patch: Partial<DesktopPrefs>) =>
    invoke<DesktopPrefs>("desktop_prefs_patch", { patch }),
  closeDecision: (behavior: string, remember: boolean) =>
    invoke("close_decision", { behavior, remember }),
  fullQuit: () => invoke("full_quit"),
  companionApplyPrefs: () => invoke("companion_apply_prefs"),
  /**
   * 顯示／隱藏桌面角色視窗（host 真的 show／hide 並告訴 Runtime 表面在不在）。
   * `companion.presence.set` 只能走這條：只寫偏好不套用視窗卻 ack completed 是謊報
   * （對抗審查 director-pipeline-021）。拒絕＝沒發生，呼叫端必須回 failed。
   */
  companionSetVisible: (visible: boolean) =>
    invoke<Record<string, unknown> | void>("companion_set_visible", { visible }),
  companionWindowAdjust: (actionId: string) =>
    invoke("companion_window_adjust", { actionId }),
  companionResetPosition: () => invoke("companion_reset_position"),
};

/** Poll the supervisor until it decides embedded vs external; switch the
 *  runtime transport to HTTP when an external daemon owns the runtime.
 *  Resolves with the final info (or null outside Tauri). */
export async function bootstrapSupervisor(): Promise<SupervisorInfo | null> {
  if (!isTauri) return null; // browser mode: transport already HTTP
  for (let i = 0; i < 240; i++) {
    try {
      const info = await desktop.supervisorInfo();
      if (info.mode === "external" && info.token) {
        configureHttp(info.apiBase, info.token);
        return info;
      }
      if (info.mode === "embedded" || info.state === "disconnected") {
        return info;
      }
    } catch {
      /* command not ready yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  return null;
}

export function onCloseRequested(handler: () => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen("close-requested", () => handler());
}

export function onNavigate(handler: (tab: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("navigate", (e) => handler(e.payload));
}

export function onSupervisorState(handler: (state: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("supervisor-state", (e) => handler(e.payload));
}

export function onTrayActionError(handler: (message: string) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<string>("tray-action-error", (e) => handler(e.payload));
}

export { isTauri };
