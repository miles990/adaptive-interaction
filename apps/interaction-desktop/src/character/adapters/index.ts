// 桌面 host 的 builtin adapter 註冊表。
//
// 「怎麼把一份 manifest 變成一個活著的角色」是 host 的知識，全部集中在這裡：
// CompanionApp 只呼叫 createBuiltinAdapter(entrypoint, ctx)，不再認得任何角色名字。
// 加第二個 Reference Character（`ref-shape`）只需要在這裡多註冊一列，協定核心與
// CompanionApp 都不用動。
//
// import 這個模組即完成註冊（副作用），所以 host 入口要 `import "../character/adapters"`。

import { MixerRenderer } from "../../companion/mixerRenderer";
import { SpriteRenderer, validateManifest, type PackManifest } from "../../companion/renderer";
import {
  hostMigrationRegistry,
  registerBuiltinAdapter,
  registerHostMigrator,
  type BuiltinAdapterBuild,
  type BuiltinAdapterContext,
  type BuiltinAdapterMeta,
} from "../adapterRegistry";
import { setDefaultMigrationRegistry } from "../manifest";
import { ShapeCharacterAdapter } from "./shape";
import { rigPackMigrator, SHU_RIG_PALETTES, ShuCharacterAdapter } from "./shu";
import { SpriteCharacterAdapter } from "./sprite";
import { TextCharacterAdapter } from "./text";

const SHU_META: BuiltinAdapterMeta = {
  cssClass: "companion-stage",
  surface: "canvas",
  hasPlayfield: true,
  // rig 的配色偏好鍵：選定 variant 時同時以 `palette` 送給 adapter。
  variantAliasKeys: ["palette"],
  variants: SHU_RIG_PALETTES,
  legacyPackKinds: ["character-rig"],
};

const SPRITE_META: BuiltinAdapterMeta = {
  cssClass: "companion-canvas",
  surface: "canvas",
  hasPlayfield: false,
  // 舊 pack 相容層：沒有 `x-legacy` 版型就建不出來（host 依這個旗標判斷，不看 id）。
  requiresLegacyPackShape: true,
  legacyPackKinds: ["character-pack"],
};

const TEXT_META: BuiltinAdapterMeta = {
  cssClass: "companion-text",
  surface: "dom",
  hasPlayfield: false,
};

const SHAPE_META: BuiltinAdapterMeta = {
  cssClass: "companion-canvas",
  surface: "dom",
  hasPlayfield: false,
};

function requireCanvas(ctx: BuiltinAdapterContext): HTMLCanvasElement {
  if (!ctx.canvas) throw new Error("this character needs a canvas surface");
  return ctx.canvas;
}

registerBuiltinAdapter(
  "shu-rig",
  (ctx): BuiltinAdapterBuild => {
    const canvas = requireCanvas(ctx);
    const adapter = new ShuCharacterAdapter({
      ...(ctx.manifest ? { manifest: ctx.manifest } : { legacyRig: ctx.legacyPack }),
      ...(ctx.variant ? { palette: ctx.variant } : {}),
      canvas,
      scale: ctx.scale ?? 1,
      ...(ctx.mixer ? { mixer: ctx.mixer } : {}),
      ...(ctx.charName ? { charName: ctx.charName } : {}),
    });
    return { adapter, renderer: null, companion: adapter, meta: SHU_META };
  },
  SHU_META
);

registerBuiltinAdapter(
  "sprite",
  (ctx): BuiltinAdapterBuild => {
    const canvas = requireCanvas(ctx);
    const pack = ctx.legacyPack as PackManifest | undefined;
    if (!pack) throw new Error("sprite characters need their legacy pack shape");
    const issues = validateManifest(pack);
    if (issues.length > 0) throw new Error(`invalid character pack: ${issues.join("; ")}`);
    const assetBase = ctx.assetBase ?? "";
    const sheetUrl = ctx.sheetUrl ?? `${assetBase}/${pack.sheet}`;
    // 真正的 SpriteRenderer 由 host 擁有並由 syncPose 驅動；adapter 拿到的是 MixerRenderer
    // 門面，它的 setAnimation 進入同一台 machine（不互搶畫面）。
    const scale = ctx.scale ?? 1;
    const real = new SpriteRenderer(canvas, pack, sheetUrl, scale);
    const renderer = ctx.mixer ? new MixerRenderer(real, ctx.mixer) : real;
    const adapter = new SpriteCharacterAdapter({ pack, assetBase, renderer, scale });
    return { adapter, renderer: real, companion: null, meta: SPRITE_META };
  },
  SPRITE_META
);

registerBuiltinAdapter(
  "text",
  (ctx): BuiltinAdapterBuild => {
    const adapter = new TextCharacterAdapter({
      ...(ctx.textHost ? { container: ctx.textHost } : {}),
      ...(ctx.characterId ? { characterId: ctx.characterId } : {}),
      ...(ctx.manifest?.displayName ?? ctx.displayName
        ? { displayName: ctx.manifest?.displayName ?? ctx.displayName ?? {} }
        : {}),
      ...(ctx.manifest?.description ?? ctx.description
        ? { description: ctx.manifest?.description ?? ctx.description ?? {} }
        : {}),
    });
    return { adapter, renderer: null, companion: null, meta: TEXT_META };
  },
  TEXT_META
);

registerBuiltinAdapter(
  "shape",
  (ctx): BuiltinAdapterBuild => {
    const adapter = new ShapeCharacterAdapter({
      ...(ctx.textHost ? { container: ctx.textHost } : {}),
    });
    return { adapter, renderer: null, companion: null, meta: SHAPE_META };
  },
  SHAPE_META
);

// ---------------------------------------------------------------------------
// §2.2 舊 pack 遷移：核心只內建通用 sprite；具名角色的舊格式在這裡跟工廠一起註冊。
// ---------------------------------------------------------------------------

registerHostMigrator(rigPackMigrator);

// 拿得到 host registry 的呼叫端一律明確帶 `opts.registry`；這一行只是替拿不到的呼叫端
// （例如協定核心自己的 migratePackToManifest 預設值）補上同一份 registry。
setDefaultMigrationRegistry(hostMigrationRegistry());
