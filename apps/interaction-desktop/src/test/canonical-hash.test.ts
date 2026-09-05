// @vitest-environment node
//
// AIP §6 的 canonical JSON ＋ state hash（TypeScript 端）。
//
// 讀的是 Rust crate 底下**同一份** fixture（`crates/interaction-aip/tests/fixtures/`）：
// `manifest.json` 的 `stateHashes` 是索引，每一筆的 `state`／`canonical`／`hash` 由 Rust
// 端每次測試重新推導。三個實作對同一份 state 必須得到同一個位元組序列與同一個 hash——
// 對不上就是桌面端沒有能力核對 host 送來的 hash，那條防線等於不存在。
//
// 為什麼需要 `doublePaths`：JS 的 `number` 留不住 JSON 字面（Rust 的 `0.0` 經
// `JSON.parse` 之後就只是 `0`）。所以 double 欄位的路徑由 codegen 從同一份 manifest
// 的 `stateHashDoublePaths` 產進 `../aip/generated.ts`，重印時整數值要寫回 `0.0`。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { canonicalJson, compareCodePoints, sha256Hex, stateHash } from "../aip/canonical";
import { SEMANTIC_STATE_DOUBLE_PATHS } from "../aip/generated";

const FIXTURES = decodeURIComponent(
  new URL("../../../../crates/interaction-aip/tests/fixtures/", import.meta.url).pathname,
);
const read = (name: string) => readFileSync(`${FIXTURES}${name}`, "utf8");
const manifest = JSON.parse(read("manifest.json")) as {
  stateHashes: { id: string; file: string }[];
  stateHashDoublePaths: string[];
};

describe("canonical JSON / state hash：跨語言 fixture", () => {
  it("codegen 產出的 double 路徑就是 manifest 裡那一份（漂移 gate）", () => {
    expect([...SEMANTIC_STATE_DOUBLE_PATHS]).toEqual(manifest.stateHashDoublePaths);
  });

  it.each(manifest.stateHashes.map((entry) => [entry.id, entry.file] as const))(
    "fixture %s 的 canonical 與 hash 逐位元組相同",
    (id, file) => {
      const fixture = JSON.parse(read(file)) as {
        state: unknown;
        canonical: string;
        hash: string;
      };
      expect(canonicalJson(fixture.state, SEMANTIC_STATE_DOUBLE_PATHS), `${id} canonical`).toBe(
        fixture.canonical,
      );
      expect(stateHash(fixture.state), `${id} hash`).toBe(fixture.hash);
    },
  );

  it("鍵的順序／空白不影響結果（unsorted-input 與 fresh 同 hash）", () => {
    const fresh = JSON.parse(read("state-hash-fresh.json")) as { hash: string };
    const unsorted = JSON.parse(read("state-hash-unsorted-input.json")) as { state: unknown };
    expect(stateHash(unsorted.state)).toBe(fresh.hash);
  });
});

describe("canonical JSON：數字字面", () => {
  it("double 路徑上的整數值仍帶小數（0 → 0.0、1 → 1.0）", () => {
    expect(canonicalJson({ mood: { intensity: 1 } }, ["/mood/intensity"])).toBe(
      '{"mood":{"intensity":1.0}}',
    );
    expect(canonicalJson({ mood: { intensity: 0 } }, ["/mood/intensity"])).toBe(
      '{"mood":{"intensity":0.0}}',
    );
  });

  it("負零寫成 -0.0（host 永不產生它，但核對時必須看得懂）", () => {
    expect(canonicalJson({ mood: { intensity: -0 } }, ["/mood/intensity"])).toBe(
      '{"mood":{"intensity":-0.0}}',
    );
  });

  it("不在 double 路徑上的整數維持整數字面", () => {
    expect(canonicalJson({ mood: { intensity: 1 }, revision: 1 }, ["/mood/intensity"])).toBe(
      '{"mood":{"intensity":1.0},"revision":1}',
    );
  });

  it("double 路徑上的小數用最短 round-trip 十進位，不加尾數", () => {
    expect(canonicalJson({ mood: { intensity: 0.123 } }, ["/mood/intensity"])).toBe(
      '{"mood":{"intensity":0.123}}',
    );
  });
});

describe("SHA-256：已知向量", () => {
  it("空字串", () => {
    expect(sha256Hex("")).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
  });

  it("abc", () => {
    expect(sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });

  it("448-bit 訊息（跨一個 block 的填充邊界）", () => {
    expect(sha256Hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")).toBe(
      "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
  });

  it("896-bit 訊息（跨兩個 block 的填充邊界）", () => {
    expect(
      sha256Hex(
        "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
      ),
    ).toBe("cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1");
  });

  it("百萬個 a（多 block）", () => {
    expect(sha256Hex("a".repeat(1_000_000))).toBe(
      "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
    );
  });

  it("非 ASCII 以 UTF-8 位元組計算", () => {
    // echo -n "角色" | shasum -a 256
    expect(sha256Hex("角色")).toBe(sha256Hex(new TextEncoder().encode("角色")));
  });
});

// ---------------------------------------------------------------------------
// 鍵序：code point 序（＝ UTF-8 位元組序），不是 UTF-16 code unit 序
// ---------------------------------------------------------------------------

/** 權威端的鍵序：Rust `keys.sort()`（`String` 的 Ord）＝ Swift `Array(key.utf8)` 的字典序。 */
function utf8Order(a: string, b: string): number {
  const left = new TextEncoder().encode(a);
  const right = new TextEncoder().encode(b);
  const length = Math.min(left.length, right.length);
  for (let i = 0; i < length; i += 1) {
    const x = left[i] ?? 0;
    const y = right[i] ?? 0;
    if (x !== y) return x < y ? -1 : 1;
  }
  if (left.length === right.length) return 0;
  return left.length < right.length ? -1 : 1;
}

describe("canonical JSON：鍵序與 UTF-8 位元組序一致（跨語言）", () => {
  // U+F801..U+FFFF 這一段是唯一會出錯的區間：代理對開頭（0xD800..0xDBFF）如果只被
  // 抬高 0x2000 就停在 0xFBFF，會排在這些 BMP 鍵**之前**，與 Rust／Swift 相反。
  const PAIRS: readonly (readonly [string, string])[] = [
    ["\u{10000}", "�"],
    ["\u{1F600}", "�"],
    ["\u{10FFFF}", "￿"],
    ["\u{10000}", ""],
    ["\u{10000}", "A"],
    ["", "퟿"],
    ["a", "ab"],
  ];

  it("compareCodePoints 對每一組鍵都與 UTF-8 位元組序同號", () => {
    for (const [a, b] of PAIRS) {
      expect(Math.sign(compareCodePoints(a, b)), `${JSON.stringify(a)} vs ${JSON.stringify(b)}`).toBe(
        Math.sign(utf8Order(a, b)),
      );
      expect(Math.sign(compareCodePoints(b, a)), `${JSON.stringify(b)} vs ${JSON.stringify(a)}`).toBe(
        Math.sign(utf8Order(b, a)),
      );
    }
  });

  it("補充平面的鍵排在所有 BMP 鍵之後（U+FFFD 之前是錯的）", () => {
    expect(canonicalJson({ "\u{10000}": 2, "�": 1 })).toBe('{"�":1,"\u{10000}":2}');
  });

  it("已知向量：補充平面鍵的 state hash（Rust／Swift 同一份文字的 SHA-256）", () => {
    // canonical 文字 `{"\u{FFFD}":1,"\u{10000}":2}` 的 UTF-8 位元組 SHA-256。
    expect(stateHash({ "\u{10000}": 2, "�": 1 })).toBe(
      "f0516938af0496fd78add6d50dedd04c35489a25bf4a03160eadd54051ed8b86",
    );
  });
});
