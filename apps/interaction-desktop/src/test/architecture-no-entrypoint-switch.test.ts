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

/** `x === "shu-rig"`／`=== 'sprite'`／`case "text":` 這類字面分岔。 */
function literalBranches(source: string): string[] {
  const hits: string[] = [];
  for (const id of ADAPTER_IDS) {
    const comparison = new RegExp(`[!=]==\\s*["'\`]${id}["'\`]|["'\`]${id}["'\`]\\s*[!=]==`, "g");
    const switchCase = new RegExp(`case\\s+["'\`]${id}["'\`]\\s*:`, "g");
    for (const re of [comparison, switchCase]) {
      const found = source.match(re);
      if (found) hits.push(...found);
    }
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
function stripSourceDiscriminants(source: string): string {
  return source.replace(/\w+\.kind\s*[!=]==\s*["'`][a-z-]+["'`]/g, "SOURCE_KIND_CHECK");
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

  it("CompanionApp 只透過 registry 建 adapter，不直接 new 任何角色類別", () => {
    const source = stripComments(readFileSync(join(SRC, "companion", "CompanionApp.tsx"), "utf8"));
    expect(source).toContain("createBuiltinAdapter");
    for (const cls of ["ShuCharacterAdapter", "SpriteCharacterAdapter", "TextCharacterAdapter", "ShapeCharacterAdapter"]) {
      expect(source, `${cls} 應由 registry 建`).not.toContain(`new ${cls}(`);
    }
  });
});
