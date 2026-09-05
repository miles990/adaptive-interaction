// @vitest-environment node
//
// AIP §6 的 **canonical 向量**（TypeScript 端）。
//
// `canonical-hash.test.ts` 跑的是真的 `SemanticState`——鍵全是 ASCII 欄位名。這一份跑的是
// `crates/interaction-aip/tests/fixtures/manifest.json` 的 `canonicalVectors` 段：非 ASCII 鍵、
// 補充平面鍵、需要跳脫的鍵與值、以及數字字面的邊界。權威值由 Rust 的
// `crates/interaction-aip/tests/canonical_vectors.rs` 產生（`AIP_UPDATE_FIXTURES=1` 重生）。
//
// 為什麼這一份非有不可：對抗審查 `hash-numeric-contract-017` 指出這裡的鍵序曾經是
// UTF-16 code unit 序（補充平面鍵排到 U+F801..U+FFFF 之前）。修掉了，但當時沒有任何一筆
// fixture 抓得住它——ASCII 欄位名在兩種排序底下長得一模一樣。
//
// 向量不遷就實作：對不上就是這一端的 canonical 實作錯了。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { canonicalJson, compareCodePoints, sha256Hex } from "../aip/canonical";

const MANIFEST = decodeURIComponent(
  new URL(
    "../../../../crates/interaction-aip/tests/fixtures/manifest.json",
    import.meta.url,
  ).pathname,
);

interface CanonicalVector {
  id: string;
  note: string;
  /** 這筆向量裡所有 f64 值的 RFC 6901 pointer（JS 的 number 分不出 `1` 與 `1.0`）。 */
  doublePaths: string[];
  input: unknown;
  canonical: string;
  sha256: string;
}

const manifest = JSON.parse(readFileSync(MANIFEST, "utf8")) as {
  canonicalVectors: CanonicalVector[];
};
const vectors = manifest.canonicalVectors;

describe("canonical JSON 向量：鍵序／跳脫／數字字面", () => {
  it("manifest 帶著這一段（至少 8 筆）", () => {
    expect(Array.isArray(vectors)).toBe(true);
    expect(vectors.length).toBeGreaterThanOrEqual(8);
  });

  it.each(vectors.map((vector) => [vector.id, vector] as const))(
    "%s 的 canonical 文字逐位元組相同",
    (id, vector) => {
      expect(canonicalJson(vector.input, vector.doublePaths), `${id} canonical`).toBe(
        vector.canonical,
      );
    },
  );

  it.each(vectors.map((vector) => [vector.id, vector] as const))(
    "%s 的 SHA-256 與 Rust 相同",
    (id, vector) => {
      expect(sha256Hex(canonicalJson(vector.input, vector.doublePaths)), `${id} sha256`).toBe(
        vector.sha256,
      );
    },
  );

  it("每一筆向量的 hash 都不同（沒有兩筆在測同一件事）", () => {
    expect(new Set(vectors.map((vector) => vector.sha256)).size).toBe(vectors.length);
  });

  // 這一條是 hash-numeric-contract-017 的回歸：JS 內建的 `<` 是 UTF-16 code unit 序，
  // 代理對開頭 0xD800 會讓補充平面鍵排到 U+F801..U+FFFF 的 BMP 鍵之前。
  it("鍵序是 code point 序，不是 UTF-16 code unit 序", () => {
    const vector = vectors.find((entry) => entry.id === "code-point-order-not-utf16");
    expect(vector, "manifest 缺 code-point-order-not-utf16 向量").toBeDefined();
    const keys = Object.keys(vector?.input as Record<string, unknown>);

    const byCodePoint = [...keys].sort(compareCodePoints);
    const byUtf16 = [...keys].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    expect(byCodePoint, "這筆向量沒有分開兩種排序，證明不了任何事").not.toEqual(byUtf16);

    // 向量的值就是 code point 序的位置。
    const input = vector?.input as Record<string, number>;
    expect(byCodePoint.map((key) => input[key])).toEqual(byCodePoint.map((_, i) => i));
  });

  it("U+2028／U+2029／U+007F／`/` 原樣輸出，不跳脫", () => {
    const vector = vectors.find((entry) => entry.id === "unescaped-passthrough");
    expect(vector, "manifest 缺 unescaped-passthrough 向量").toBeDefined();
    const text = canonicalJson(vector?.input, vector?.doublePaths);
    for (const ch of ["\u2028", "\u2029", "\u007f", "\u00a0", "/"]) {
      expect(text.includes(ch), `${JSON.stringify(ch)} 必須原樣出現`).toBe(true);
    }
    expect(text.includes("\\u2028")).toBe(false);
    expect(text.includes("\\/")).toBe(false);
  });

  // JS 的 `String()` 在 |x| ≥ 1e21 才切成指數形，serde_json（ryu）在十進位指數
  // k ∉ [-5, 16) 就切——`1e-6` 與 `1e+16`..`1e+20` 這一整段兩邊印得完全不一樣。
  it("f64 的固定小數 ↔ 科學記號分界與 serde_json 相同", () => {
    const vector = vectors.find((entry) => entry.id === "numbers-exponent-forms");
    expect(vector, "manifest 缺 numbers-exponent-forms 向量").toBeDefined();
    const text = canonicalJson(vector?.input, vector?.doublePaths);
    expect(text).toContain("0.00001"); // k = -5：固定小數
    expect(text).toContain("1e-6"); // k = -6：科學記號
    expect(text).toContain("1000000000000000.0"); // k = 15：固定小數
    expect(text).toContain("1e+16"); // k = 16：科學記號
    expect(text).toContain("1e+20"); // JS 的 String() 在這裡印 100000000000000000000
    expect(text).toBe(vector?.canonical);
  });
});
