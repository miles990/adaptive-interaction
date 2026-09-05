// docs/aip/architecture-boundaries.md §4：host 不得再依 entrypoint 字串分岔。
//
// 加一個角色只能是「多一個 adapter 模組 ＋ 多一列 registry 註冊 ＋ 多一份 manifest」；
// 只要 CompanionApp／gatewayWiring／gateway／negotiate 裡還留著 `entrypoint === "shu-rig"`
// 這種字面比較，第二個角色就會需要改 host。這個測試直接讀原始碼把它釘死。
//
// 允許出現的地方：`character/adapters/**`（adapter 自己知道自己是誰）、
// `character/adapterRegistry.ts`（registry 宣告 id）。

import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const SRC = join(__dirname, "..");

/** 受檢檔案：host 端的接線層。 */
function guardedFiles(): string[] {
  const out: string[] = [];
  const companion = join(SRC, "companion");
  for (const name of readdirSync(companion)) {
    if (name.endsWith(".ts") || name.endsWith(".tsx")) out.push(join(companion, name));
  }
  out.push(join(SRC, "character", "gateway.ts"));
  out.push(join(SRC, "character", "negotiate.ts"));
  return out.sort();
}

/** host 不得直接寫死的 builtin adapter id。 */
const ADAPTER_IDS = ["shu-rig", "sprite", "text", "shape"];

/**
 * 字面分岔的偵測。
 *
 * 對抗審查 runtime-boundaries-067：舊版只認 `=== "id"`／`case "id":` 兩種寫法，
 * 於是 `entrypoint == "shu-rig"`（鬆散比較）、`entrypoint.startsWith("shu")`、
 * `entrypoint.includes("shu-rig")`、以及以 id 當 key 的物件查表全部逃得掉。
 * 現在四類都認：
 *   1. 比較（`==`／`===`／`!=`／`!==`，兩邊都算）；
 *   2. `case "id":`；
 *   3. 字串前綴／包含判斷（連 id 的片段也算，`startsWith("shu")` 逃不掉）；
 *   4. 以 id 當物件 key 或索引（`{"shu-rig": …}`／`table["sprite"]`）。
 */
export function literalBranches(source: string): string[] {
  const hits: string[] = [];
  for (const id of ADAPTER_IDS) {
    const q = `["'\`]`;
    const comparison = new RegExp(`[!=]=+\\s*${q}${id}${q}|${q}${id}${q}\\s*[!=]=+`, "g");
    const switchCase = new RegExp(`case\\s+${q}${id}${q}\\s*:`, "g");
    const objectKey = new RegExp(`${q}${id}${q}\\s*:|\\[\\s*${q}${id}${q}\\s*\\]`, "g");
    for (const re of [comparison, switchCase, objectKey]) {
      const found = source.match(re);
      if (found) hits.push(...found);
    }
  }
  // 3：`.startsWith("shu")` 這種以 id **片段**做的判斷。片段至少 3 個字元才算
  //（避免把無關的短字串誤判），且必須真的是某個 id 的子字串。
  const fragment = /\.(startsWith|endsWith|includes|indexOf|search)\(\s*["'`]([^"'`]{3,})["'`]/g;
  for (const m of source.matchAll(fragment)) {
    const needle = m[2] ?? "";
    if (ADAPTER_IDS.some((id) => id.includes(needle))) hits.push(m[0]);
  }
  return hits;
}

/** 去掉註解：文件裡提到 "shu-rig" 是說明，不是分岔。 */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/**
 * `CharacterSource.kind`（index／legacy-pack／imported／text 退路）是**來源**的判別標籤，
 * 不是 entrypoint id；它和有哪些 adapter 無關，加角色也不會多一個 source kind。
 * 這種 `<物件>.kind === "…"` 的比較不算 entrypoint 分岔。
 */
export function stripSourceDiscriminants(source: string): string {
  // 只允許**真的** source kind 值。舊版用 `[a-z-]+` 把任何 `<x>.kind === "<小寫-減號>"`
  // 都當成來源判別，連 `entrypoint.kind === "shu-rig"` 這種真正的 entrypoint 分岔
  // 也一起放行（對抗審查 runtime-boundaries-067）。
  return source.replace(/\w+\.kind\s*[!=]==\s*["'`](?:index|legacy-pack|imported|text)["'`]/g, "SOURCE_KIND_CHECK");
}

describe("架構：host 不依 entrypoint 字串分岔", () => {
  it("companion/*.ts(x)、character/gateway.ts、character/negotiate.ts 裡沒有 builtin id 的字面分岔", () => {
    const offenders: string[] = [];
    for (const file of guardedFiles()) {
      const hits = literalBranches(stripSourceDiscriminants(stripComments(readFileSync(file, "utf8"))));
      if (hits.length > 0) offenders.push(`${file.slice(SRC.length + 1)}: ${hits.join(", ")}`);
    }
    expect(offenders, "改用 adapterRegistry（createBuiltinAdapter／builtinAdapterMeta）").toEqual([]);
  });

  it("adapter 模組與 registry 才是允許認得 id 的地方", () => {
    const registry = readFileSync(join(SRC, "character", "adapterRegistry.ts"), "utf8");
    for (const id of ADAPTER_IDS) expect(registry).toContain(`"${id}"`);
    const index = readFileSync(join(SRC, "character", "adapters", "index.ts"), "utf8");
    for (const id of ADAPTER_IDS) expect(index).toContain(`registerBuiltinAdapter(\n  "${id}"`);
  });

  it("協定核心 character/*.ts（adapters/ 以外）不含任何角色專屬字串", () => {
    const dir = join(SRC, "character");
    // 唯一例外：adapterRegistry.ts 宣告 host 白名單（docs/aip/reference-character.md §3.1），
    // 那一行的 "shu-rig" 是 id 字面，不是角色知識；其餘角色字串一律不得出現。
    const offenders: string[] = [];
    for (const name of readdirSync(dir)) {
      if (!name.endsWith(".ts")) continue;
      let source = stripComments(readFileSync(join(dir, name), "utf8"));
      if (name === "adapterRegistry.ts") source = source.replace(/"shu-rig"/g, "DECLARED_BUILTIN_ID");
      const hits = source.match(/shu|maid|character-rig/gi);
      if (hits) offenders.push(`${name}: ${Array.from(new Set(hits)).join(", ")}`);
    }
    expect(offenders, "小樞專屬的能力集／配色／rig 遷移屬於 character/adapters/shu.ts").toEqual([]);
  });

  it("CompanionApp 只透過 registry 建 adapter，不直接 new 任何角色類別", () => {
    const source = stripComments(readFileSync(join(SRC, "companion", "CompanionApp.tsx"), "utf8"));
    expect(source).toContain("createBuiltinAdapter");
    for (const cls of ["ShuCharacterAdapter", "SpriteCharacterAdapter", "TextCharacterAdapter", "ShapeCharacterAdapter"]) {
      expect(source, `${cls} 應由 registry 建`).not.toContain(`new ${cls}(`);
    }
  });
});

describe("架構守門本身：偵測器抓得到繞道寫法（對抗審查 runtime-boundaries-067）", () => {
  // 舊版偵測器對這四種寫法全部回 0 hit：綠燈不代表 host 真的沒有角色分岔。
  const evasions = [
    'if (entrypoint.kind === "shu-rig") return legacyShuPath();',
    'if (entrypoint == "shu-rig") return legacyShuPath();',
    'if (entrypoint.startsWith("shu")) return legacyShuPath();',
    'if (entrypoint.includes("shu-rig")) return legacyShuPath();',
    'const table = { "shu-rig": rigPath, sprite: spritePath };',
    'const build = TABLE["sprite"];',
  ];
  for (const snippet of evasions) {
    it(`抓得到：${snippet}`, () => {
      const hits = literalBranches(stripSourceDiscriminants(stripComments(snippet)));
      expect(hits.length, snippet).toBeGreaterThan(0);
    });
  }

  it("真正的 CharacterSource 判別標籤仍然放行（不是 entrypoint 分岔）", () => {
    for (const snippet of [
      'if (source.kind === "imported") return planImported(source);',
      'if (source.kind === "legacy-pack") return planLegacy(source);',
      'if (source.kind === "index") return planIndex(source);',
    ]) {
      expect(literalBranches(stripSourceDiscriminants(stripComments(snippet))), snippet).toEqual([]);
    }
  });

  it("註解裡提到 id 不算分岔", () => {
    const snippet = '// entrypoint === "shu-rig" 是說明，不是分岔\nconst x = 1;';
    expect(literalBranches(stripSourceDiscriminants(stripComments(snippet)))).toEqual([]);
  });
});

describe("架構：adapter 規劃的接線層不得 import 角色專屬模組（對抗審查 character-package-018）", () => {
  // 「初始 variant 怎麼算」「沒有 manifest 時要不要造舊 pack」以前寫在 CompanionApp，
  // 靠的是某個 rig 專屬的 helper；任何**別的** adapter 只要宣告 variants 就會拿到那個
  // rig 的預設配色。現在由 adapter meta 的 hook 提供，接線層不必也不得認得它們。
  //
  // v0.6.x M2 §3.4 起 `companion/settingsTransfer.ts` 也收斂了：匯入的「使魔配色」與
  // 「說話風格」改用**目標 characterId 的** adapter meta 驗證，不再 import 任何 rig 的表
  //（同一節的「設定匯入」測試釘住行為，下面那條測試釘住它不再 import）。
  const WIRING = ["CompanionApp.tsx", "gatewayWiring.ts"];

  it("CompanionApp／gatewayWiring 只有副作用註冊與型別 import 碰得到 character/adapters", () => {
    const offenders: string[] = [];
    for (const name of WIRING) {
      const source = readFileSync(join(SRC, "companion", name), "utf8");
      for (const line of source.split("\n")) {
        if (!/character\/adapters/.test(line)) continue;
        const trimmed = line.trim();
        if (!/^(import|export)\b/.test(trimmed)) continue;
        // 允許：`import "../character/adapters";`（註冊工廠）與 `import type …`。
        if (/^import\s+["'][^"']*character\/adapters["'];?$/.test(trimmed)) continue;
        if (/^import\s+type\b/.test(trimmed)) continue;
        offenders.push(`${name}: ${trimmed}`);
      }
    }
    expect(offenders, "角色專屬知識請放進 adapter meta（defaultVariant／legacyPackForEntry）").toEqual([]);
  });
});

describe("架構：角色設定頁不得寫死角色專屬字面（M2 §3.4）", () => {
  /** 受檢的頁面層檔案（角色設定介面）。 */
  function pageFiles(): string[] {
    const out = [join(SRC, "pages", "CompanionPage.tsx")];
    const dir = join(SRC, "pages", "character");
    for (const name of readdirSync(dir)) {
      if (name.endsWith(".ts") || name.endsWith(".tsx")) out.push(join(dir, name));
    }
    return out.sort();
  }

  /**
   * 尚未收斂的頁面（M3 待辦，只准縮短不准變長）：
   *   - CharacterPreview.tsx 仍以 `switch (card.entrypoint)` 分流，且直接 import rig 的表；
   *   - CharacterLibrary.tsx 的「停用」鈕仍比對純文字角色的 entrypoint id。
   */
  const PENDING = new Set(["pages/character/CharacterPreview.tsx", "pages/character/CharacterLibrary.tsx"]);

  const rel = (file: string) => file.slice(SRC.length + 1);

  it("CompanionPage 與 catalog 沒有 builtin id 的字面分岔", () => {
    const offenders: string[] = [];
    for (const file of pageFiles()) {
      if (PENDING.has(rel(file))) continue;
      const hits = literalBranches(stripSourceDiscriminants(stripComments(readFileSync(file, "utf8"))));
      if (hits.length > 0) offenders.push(`${rel(file)}: ${hits.join(", ")}`);
    }
    expect(offenders, "改用 adapterRegistry（builtinAdapterMeta／isBuiltinEntrypointId）").toEqual([]);
  });

  // 棘輪：待收斂清單既不得成長，也不得留著已經修好的檔案（修好就要把它從名單刪掉，
  // 否則這份清單會慢慢變成一張沒有人看的謊）。
  it("待收斂清單剛好等於還留著 entrypoint 字面的頁面", () => {
    const offenders = pageFiles()
      .filter((file) => literalBranches(stripSourceDiscriminants(stripComments(readFileSync(file, "utf8")))).length > 0)
      .map(rel)
      .sort();
    expect(offenders, "修好的頁面請從 PENDING 移除；新出現的請改用 adapterRegistry").toEqual(
      [...PENDING].sort()
    );
  });

  it("CompanionPage 與 catalog 不含任何角色專屬詞彙（角色名／配色／rig）", () => {
    const offenders: string[] = [];
    for (const file of [join(SRC, "pages", "CompanionPage.tsx"), join(SRC, "pages", "character", "catalog.ts")]) {
      const hits = stripComments(readFileSync(file, "utf8")).match(/shu|maid|persona-|character-rig/gi);
      if (hits) offenders.push(`${rel(file)}: ${Array.from(new Set(hits)).join(", ")}`);
    }
    expect(offenders, "角色專屬的配色／說話風格／遊玩場 UI 屬於 character/adapters/**").toEqual([]);
  });

  it("settingsTransfer 不再自帶任何角色的說話風格或配色清單", () => {
    const source = stripComments(readFileSync(join(SRC, "companion", "settingsTransfer.ts"), "utf8"));
    expect(source, "說話風格由 adapter meta 宣告").not.toMatch(/persona-/);
    expect(source, "配色清單由 adapter meta 宣告").not.toMatch(/character\/adapters\/shu/);
  });
});
