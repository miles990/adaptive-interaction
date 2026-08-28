// 打包 rig 預覽入口 → 無頭 Chromium 截圖 → 供人工目視驗收。
// 用法：node scripts/shu/preview-rig.mjs [outPng]

import { createRequire } from "node:module";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(here, "..", "..");
const require_ = createRequire(path.join(appRoot, "package.json"));

// esbuild 是 vite 的傳遞依賴；由 pnpm store 解析。
const esbuildPkg = path.join(
  appRoot,
  "node_modules/.pnpm/esbuild@0.21.5/node_modules/esbuild"
);
const esbuild = require_(esbuildPkg);
const { chromium } = require_("@playwright/test");

const out = process.argv[2] ?? path.join(appRoot, "rig-preview.png");
const work = mkdtempSync(path.join(tmpdir(), "rig-preview-"));

const bundle = await esbuild.build({
  entryPoints: [path.join(appRoot, "src/companion/rig/preview-entry.ts")],
  bundle: true,
  format: "iife",
  write: false,
  target: "es2020",
});
const js = bundle.outputFiles[0].text;
const html = `<!doctype html><meta charset="utf-8"><body style="margin:0"><div id="root"></div><script>${js}</script></body>`;
const htmlPath = path.join(work, "preview.html");
writeFileSync(htmlPath, html);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1680, height: 1400 }, deviceScaleFactor: 2 });
await page.goto(`file://${htmlPath}`);
await page.waitForFunction(() => document.title === "rig-preview-ready");
await page.screenshot({ path: out, fullPage: true });
await browser.close();
console.log(`rig preview written to ${out}`);
