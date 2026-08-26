// Generates the Shū character packs: sprite sheet + manifest + preview per
// variant. Deterministic: same inputs → identical sheets (modulo PNG encoder).
//
//   node scripts/shu/generate.mjs
//
// Output: public/packs/shu-<variant>/{sheet.png, manifest.json, preview.png}

import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import { svgFrame } from "./design.mjs";
import { ANIMATIONS } from "./animations.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const outRoot = join(here, "../../public/packs");

const FRAME = 128;
const COLUMNS = 8;

const VARIANT_META = {
  standard: { zh: "小樞・標準型", en: "Shū · Standard" },
  lively: { zh: "小樞・活潑型", en: "Shū · Lively" },
  minimal: { zh: "小樞・極簡型", en: "Shū · Minimal" },
};

async function generateVariant(variant) {
  const names = Object.keys(ANIMATIONS);
  const allFrames = [];
  const animIndex = {};
  for (const name of names) {
    const anim = ANIMATIONS[name];
    const start = allFrames.length;
    for (const params of anim.frames) {
      allFrames.push(params);
    }
    animIndex[name] = {
      frames: anim.frames.map((_, i) => start + i),
      fps: anim.fps,
      loop: anim.loop,
    };
  }

  const rows = Math.ceil(allFrames.length / COLUMNS);
  const buffers = await Promise.all(
    allFrames.map((p) => sharp(Buffer.from(svgFrame(p, variant))).png().toBuffer())
  );
  const sheet = await sharp({
    create: {
      width: COLUMNS * FRAME,
      height: rows * FRAME,
      channels: 4,
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    },
  })
    .composite(
      buffers.map((input, i) => ({
        input,
        left: (i % COLUMNS) * FRAME,
        top: Math.floor(i / COLUMNS) * FRAME,
      }))
    )
    .png()
    .toBuffer();

  const dir = join(outRoot, `shu-${variant}`);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "sheet.png"), sheet);

  // Preview: idle first frame at 256px on transparent.
  const preview = await sharp(Buffer.from(svgFrame(ANIMATIONS.idle.frames[0], variant)))
    .resize(256, 256, { kernel: "nearest" })
    .png()
    .toBuffer();
  writeFileSync(join(dir, "preview.png"), preview);

  const meta = VARIANT_META[variant];
  const manifest = {
    schemaVersion: "1.0",
    kind: "character-pack",
    id: `shu-${variant}`,
    name: { "zh-TW": meta.zh, en: meta.en },
    description: {
      "zh-TW": "原創桌面互動角色小樞：介於小型機器生命與貓科夥伴之間的互動中樞。",
      en: "Shū, an original desktop companion between a small machine lifeform and a feline partner.",
    },
    author: "Adaptive Interaction Project",
    version: "0.3.0",
    license: "MIT",
    generator: "scripts/shu/generate.mjs (parametric SVG rig; no AI raster generation)",
    frameSize: [FRAME, FRAME],
    anchor: [64, 120],
    sheet: "sheet.png",
    columns: COLUMNS,
    animations: animIndex,
    preview: "preview.png",
  };
  writeFileSync(join(dir, "manifest.json"), JSON.stringify(manifest, null, 2));
  return { variant, frames: allFrames.length, rows };
}

const results = [];
for (const variant of Object.keys(VARIANT_META)) {
  results.push(await generateVariant(variant));
}
console.log(results.map((r) => `${r.variant}: ${r.frames} frames, ${r.rows} rows`).join("\n"));
