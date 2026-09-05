// AIP §6：canonical JSON ＋ state hash（TypeScript 端）。
//
// 契約：`docs/aip/README.md` §6。權威實作是 Rust 的 `interaction_aip::canonical_json`／
// `canonical_hash`；這一份必須對同一份 state 產生**逐位元組相同**的文字，否則桌面端
// 根本沒有能力核對 host 送來的 `hash`，AIP §6 的那條防線在這一端等於不存在。
//
// 為什麼不是 `JSON.stringify`：
//
//   * 鍵順序：Rust 用 `String` 的 Ord（UTF-8 位元組序）＝ Unicode code point 序；
//     JS 的 `<` 是 UTF-16 code unit 序，遇到 U+E000..U+FFFF 與非 BMP 字元混排時會不同。
//   * 數字字面：`serde_json` 的 f64 是最短 round-trip 十進位，指數在 `[-5, 16)` 內帶
//     小數點（`0.0`、`1.0`、`-0.0`、`0.00001`），之外是 `1e+16`／`1e-6` 這種指數形
//     （JS 的 `String()` 分界是 `(-7, 21)`，兩者不同）；整數型別則是純數字。
//     JS 的 `number` **留不住**整數／f64 這個區別
//     ——Rust 送出的 `0.0` 經過 `JSON.parse` 之後就只是 `0`。所以哪些路徑是 f64
//     必須另外給：`doublePaths`（由 `pnpm aip:codegen` 從
//     `crates/interaction-aip/tests/fixtures/manifest.json` 的 `stateHashDoublePaths`
//     產成 `./generated.ts` 的 `SEMANTIC_STATE_DOUBLE_PATHS`，手改會被 `pnpm aip:check` 擋下）。
//   * 字串跳脫：`serde_json` 只跳脫 `"`、`\` 與 < U+0020 的控制字元（`\b \t \n \f \r`
//     有短寫，其餘是小寫 `\u00xx`）。非 ASCII 原樣、`/` 不跳脫、U+007F 不跳脫。
//
// 為什麼自己寫 SHA-256 而不用 Web Crypto：`crypto.subtle.digest` 是非同步的，
// 而收到 state 時的決策必須是同步純函式（reducer 不能 await，否則就會出現
// 「一半套用了、一半還沒」的中間狀態）。實作在檔案下半部，附已知向量測試。

import { SEMANTIC_STATE_DOUBLE_PATHS } from "./generated";

/** RFC 6901 pointer 的段落還原（`~1` → `/`、`~0` → `~`；順序不可反）。 */
function unescapePointerSegment(segment: string): string {
  return segment.replace(/~1/g, "/").replace(/~0/g, "~");
}

/** double 路徑的 trie 節點。`*` 是萬用段（可用於陣列索引或任意鍵）。 */
interface PathNode {
  children: Map<string, PathNode>;
  wildcard: PathNode | null;
  terminal: boolean;
}

function newNode(): PathNode {
  return { children: new Map(), wildcard: null, terminal: false };
}

/**
 * 把 RFC 6901 pointer 清單編成 trie。空清單回 `null`（代表「沒有 double 欄位」，
 * 走最省的路徑，不做任何比對）。
 */
function buildPathTrie(paths: Iterable<string> | undefined): PathNode | null {
  if (!paths) return null;
  const root = newNode();
  let count = 0;
  for (const path of paths) {
    if (typeof path !== "string" || path.length === 0) continue;
    // "" 代表根；本函式只描述「某個數字欄位」，根一定不是數字，忽略。
    const segments = path.split("/").slice(1).map(unescapePointerSegment);
    if (segments.length === 0) continue;
    let node = root;
    for (const segment of segments) {
      if (segment === "*") {
        node.wildcard ??= newNode();
        node = node.wildcard;
      } else {
        let child = node.children.get(segment);
        if (!child) {
          child = newNode();
          node.children.set(segment, child);
        }
        node = child;
      }
    }
    node.terminal = true;
    count += 1;
  }
  return count > 0 ? root : null;
}

/** 沿著 trie 走一段；沒有任何候選就回 `null`（之後整棵子樹都不必再比對）。 */
function descend(nodes: readonly PathNode[] | null, segment: string): readonly PathNode[] | null {
  if (!nodes) return null;
  const next: PathNode[] = [];
  for (const node of nodes) {
    const child = node.children.get(segment);
    if (child) next.push(child);
    if (node.wildcard) next.push(node.wildcard);
  }
  return next.length > 0 ? next : null;
}

function isDouble(nodes: readonly PathNode[] | null): boolean {
  return nodes !== null && nodes.some((node) => node.terminal);
}

/**
 * Unicode code point 序（＝ UTF-8 位元組序，Rust `String` 的 Ord）。
 *
 * JS 內建的字串比較是 UTF-16 code unit 序：`"\u{10000}" < "�"` 在 code unit
 * 序是 true（代理對開頭是 0xD800），在 code point 序是 false。鍵順序一旦不同，
 * canonical 文字就不同，hash 就永遠對不上。
 *
 * 所以這裡**逐 code point** 比（`Array.from` 會把代理對併成一個字元）：曾經用過的
 * 「把代理對開頭 +0x2000」只做了半個轉換（0xD800..0xDBFF → 0xF800..0xFBFF），補充
 * 平面的鍵會排在 U+F801..U+FFFF 的 BMP 鍵**之前**，與 Rust（`keys.sort()`）／Swift
 * （`Array(key.utf8).lexicographicallyPrecedes`）相反——同一份 state 兩個 hash
 * （對抗審查 hash-numeric-contract-017）。
 */
export function compareCodePoints(a: string, b: string): number {
  if (a === b) return 0;
  const left = Array.from(a);
  const right = Array.from(b);
  const length = Math.min(left.length, right.length);
  for (let i = 0; i < length; i += 1) {
    // `Array.from` 的每一項都至少一個 code unit，`?? 0` 只是不用非空斷言的寫法。
    const ka = left[i]?.codePointAt(0) ?? 0;
    const kb = right[i]?.codePointAt(0) ?? 0;
    if (ka === kb) continue;
    return ka < kb ? -1 : 1;
  }
  return left.length < right.length ? -1 : 1;
}

const SHORT_ESCAPES: Record<number, string> = {
  0x08: "\\b",
  0x09: "\\t",
  0x0a: "\\n",
  0x0c: "\\f",
  0x0d: "\\r",
  0x22: '\\"',
  0x5c: "\\\\",
};

/** `serde_json` 的字串規則：只跳脫 `"`、`\` 與控制字元；非 ASCII 與 `/` 原樣。 */
export function canonicalString(value: string): string {
  let out = '"';
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i);
    const short = SHORT_ESCAPES[code];
    if (short !== undefined) {
      out += short;
    } else if (code < 0x20) {
      out += `\\u${code.toString(16).padStart(4, "0")}`;
    } else {
      out += value[i];
    }
  }
  return `${out}"`;
}

/**
 * `serde_json` 的數字字面。
 *
 * 兩條路：`asDouble` 為假時那個欄位在 host 是整數型別（u64／i64／…），serde_json 寫純數字；
 * 為真時是 f64，走 [`formatDouble`]。非有限值在 JSON 裡不存在，`serde_json` 寫成 `null`，
 * 這裡照做（不猜）。
 *
 * `-0`：host 的整數欄位收到 `-0` 這個字面時，serde_json 讀成整數 0、寫回去是 `0`；JS 的
 * `JSON.parse("-0")` 是 `-0`，`Object.is` 分得出來，所以這裡得主動抹平。f64 欄位則相反，
 * `-0.0` 是要保留的字面（見 `state-hash-intensity-negative-zero` fixture）。
 *
 * 已知邊界：|value| > 2^53 的整數在 `JSON.parse` 那一步就已經失真，這裡救不回來
 * （`crates/interaction-aip/tests/canonical_vectors.rs` 的向量刻意停在 ±2^53）。
 */
export function canonicalNumber(value: number, asDouble: boolean): string {
  if (!Number.isFinite(value)) return "null";
  if (!asDouble && Number.isInteger(value)) {
    // Object.is 才分得出 -0；`-0 === 0` 是 true。
    return Object.is(value, -0) ? "0" : String(value);
  }
  return formatDouble(value);
}

/**
 * ryu（`serde_json` 的 f64 序列化器）的固定小數 ↔ 科學記號分界。
 *
 * 令 `k` 是最短 round-trip 十進位表示中**第一位數的十進位指數**（`value = d.ddd × 10^k`）。
 * ryu 在 `k ∈ [-5, 16)` 印固定小數，其餘印 `1e+16`／`1e-6` 這種帶正負號、不補零的指數形。
 * JS 的 `String()` 分界卻是 `(-7, 21)`——`1e-6` 它印 `0.000001`、`1e16` 它印
 * `10000000000000000`、`1e20` 它印 `100000000000000000000`。這一整段（`intensity`
 * 之類的 f64 欄位真的到得了 `0.000001`）兩端會算出不同的 hash，桌面端就會卡在
 * 「hash 不符 → 要 snapshot」的迴圈裡。
 */
const RYU_FIXED_MIN_EXPONENT = -5;
/** 上界不含：`k = 16` 起改用科學記號。 */
const RYU_FIXED_MAX_EXPONENT = 16;

function formatDouble(value: number): string {
  // toExponential() 不帶參數＝「唯一決定這個 double 的最少位數」（ECMA-262），
  // 與 ryu 的最短 round-trip 是同一組數字，所以只剩「怎麼排版」要對齊。
  if (value === 0) return Object.is(value, -0) ? "-0.0" : "0.0";
  const sign = value < 0 ? "-" : "";
  const scientific = Math.abs(value).toExponential();
  const marker = scientific.indexOf("e");
  const digits = scientific.slice(0, marker).replace(".", "");
  const exponent = Number(scientific.slice(marker + 1));

  if (exponent >= RYU_FIXED_MIN_EXPONENT && exponent < RYU_FIXED_MAX_EXPONENT) {
    if (exponent < 0) return `${sign}0.${"0".repeat(-exponent - 1)}${digits}`;
    const split = exponent + 1;
    // 位數不足就補零到小數點；serde_json 的 f64 永遠帶小數點，所以沒有小數位時補 `.0`。
    const whole = digits.length > split ? digits.slice(0, split) : digits.padEnd(split, "0");
    const fraction = digits.length > split ? digits.slice(split) : "0";
    return `${sign}${whole}.${fraction}`;
  }

  const head = digits.slice(0, 1);
  const rest = digits.slice(1);
  const mantissa = rest.length > 0 ? `${head}.${rest}` : head;
  return `${sign}${mantissa}e${exponent < 0 ? "-" : "+"}${Math.abs(exponent)}`;
}

/**
 * Canonical JSON 文字：鍵以 code point 序排序、無空白、字串／數字依 `serde_json` 規則。
 *
 * `doublePaths` 是 RFC 6901 pointer 清單（支援 `*` 萬用段），指出哪些路徑上的
 * `number` 在 host 端是 f64。沒給就當成「全部都是整數型別」——那對
 * `SemanticState` 是錯的，所以 `stateHash()` 一律帶上產生出來的常數。
 */
export function canonicalJson(value: unknown, doublePaths?: Iterable<string>): string {
  const root = buildPathTrie(doublePaths);
  const out: string[] = [];
  write(value, root ? [root] : null, out);
  return out.join("");
}

function write(value: unknown, nodes: readonly PathNode[] | null, out: string[]): void {
  if (value === null || value === undefined) {
    // `undefined` 不是 JSON 值；出現在這裡代表上游餵了非 JSON 物件，寫成 null 而不是消失。
    out.push("null");
    return;
  }
  switch (typeof value) {
    case "boolean":
      out.push(value ? "true" : "false");
      return;
    case "number":
      out.push(canonicalNumber(value, isDouble(nodes)));
      return;
    case "string":
      out.push(canonicalString(value));
      return;
    default:
      break;
  }
  if (Array.isArray(value)) {
    out.push("[");
    for (let i = 0; i < value.length; i += 1) {
      if (i > 0) out.push(",");
      write(value[i], descend(nodes, String(i)), out);
    }
    out.push("]");
    return;
  }
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record).sort(compareCodePoints);
    out.push("{");
    for (let i = 0; i < keys.length; i += 1) {
      const key = keys[i] as string;
      if (i > 0) out.push(",");
      out.push(canonicalString(key));
      out.push(":");
      write(record[key], descend(nodes, key), out);
    }
    out.push("}");
    return;
  }
  // function／symbol／bigint 都不是 JSON 值：不猜一個看似成功的字面。
  out.push("null");
}

/**
 * `SemanticState` 的 canonical state hash（AIP §6）：小寫 SHA-256 hex。
 *
 * double 路徑用的是 codegen 從跨語言 fixture manifest 產出的常數，所以這一端不會
 * 自己記一份「哪些欄位是小數」——`SemanticState` 新增一個 f64 欄位而沒重跑 codegen 時，
 * `pnpm aip:check` 會先擋下來（Rust 端每次測試重新推導那份清單）。
 */
export function stateHash(state: unknown): string {
  return sha256Hex(canonicalJson(state, SEMANTIC_STATE_DOUBLE_PATHS));
}

// ------------------------------------------------------------------- SHA-256

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const rotr = (x: number, n: number): number => (x >>> n) | (x << (32 - n));

/**
 * 同步 SHA-256（FIPS 180-4）→ 小寫 hex。
 *
 * 之所以自己寫：`crypto.subtle.digest` 是 Promise，state 決策必須是同步純函式。
 * 輸入是字串時以 UTF-8 位元組計算（與 Rust 的 `canonical.as_bytes()` 一致）。
 * 訊息長度以 64-bit 大端寫入；這裡只支援 < 2^32 位元組的輸入（實際上界是 AIP §11
 * 的訊息大小上限，遠小於它）。
 */
export function sha256Hex(input: string | Uint8Array): string {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
  const bitLength = bytes.length * 8;
  // 填充：0x80、若干 0x00，最後 8 個位元組是大端的位元長度。
  const withPadding = (((bytes.length + 8) >> 6) + 1) << 6;
  const block = new Uint8Array(withPadding);
  block.set(bytes);
  block[bytes.length] = 0x80;
  const view = new DataView(block.buffer);
  // 高 32 位：長度 < 2^32 位元組時最多 2^35 位元，仍寫得下（用除法避免 32-bit 位移溢位）。
  view.setUint32(withPadding - 8, Math.floor(bitLength / 0x1_0000_0000));
  view.setUint32(withPadding - 4, bitLength >>> 0);

  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);
  const w = new Uint32Array(64);
  for (let offset = 0; offset < withPadding; offset += 64) {
    for (let i = 0; i < 16; i += 1) w[i] = view.getUint32(offset + i * 4);
    for (let i = 16; i < 64; i += 1) {
      const a = w[i - 15] as number;
      const b = w[i - 2] as number;
      const s0 = rotr(a, 7) ^ rotr(a, 18) ^ (a >>> 3);
      const s1 = rotr(b, 17) ^ rotr(b, 19) ^ (b >>> 10);
      w[i] = ((w[i - 16] as number) + s0 + (w[i - 7] as number) + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, hh] = [
      h[0] as number,
      h[1] as number,
      h[2] as number,
      h[3] as number,
      h[4] as number,
      h[5] as number,
      h[6] as number,
      h[7] as number,
    ];
    for (let i = 0; i < 64; i += 1) {
      const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const t1 = (hh + s1 + ch + (K[i] as number) + (w[i] as number)) >>> 0;
      const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (s0 + maj) >>> 0;
      hh = g;
      g = f;
      f = e;
      e = (d + t1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) >>> 0;
    }
    h[0] = ((h[0] as number) + a) >>> 0;
    h[1] = ((h[1] as number) + b) >>> 0;
    h[2] = ((h[2] as number) + c) >>> 0;
    h[3] = ((h[3] as number) + d) >>> 0;
    h[4] = ((h[4] as number) + e) >>> 0;
    h[5] = ((h[5] as number) + f) >>> 0;
    h[6] = ((h[6] as number) + g) >>> 0;
    h[7] = ((h[7] as number) + hh) >>> 0;
  }
  let hex = "";
  for (let i = 0; i < 8; i += 1) hex += (h[i] as number).toString(16).padStart(8, "0");
  return hex;
}
