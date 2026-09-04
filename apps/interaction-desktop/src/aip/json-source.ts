// JSON 原文裡的數字字面值：JSON Pointer → 原文（含在文字裡的位移）。
//
// 為什麼需要它：JavaScript 的 number 只有 double。`1`、`1.0`、`1e30`、`9007199254740993`
// 走完 `JSON.parse` 之後全都只剩一個 double，於是有兩件事在桌面端變成看不見的：
//
//   * 權威 host（Rust `interaction-aip`）把 `"sequence": 1.0`、`"revision": 1e30`、
//     `"revision": 18446744073709551616` 判成 `schema-invalid`（u64 欄位不收浮點字面值、
//     不收超出範圍的值），桌面端只看 `Number.isInteger()` 的話三者全部放行——同一則訊息
//     兩個實作結論不同，正是 docs/aip/conformance.md §1 要擋的漏洞。
//   * AIP §1 要求未知的頂層選填欄位「round-trip 不遺失」，但 9007199254740993 重新序列化
//     會變成 9007199254740992，1000000000000000001 會變成 1e+18。
//
// 兩件事都只能看**原文**才判得出來，所以這個模組在 `JSON.parse` 之外再掃一次文字。
//
// 純函式、單次掃描、用顯式堆疊而**不**遞迴：訊息上限 64 KiB，一則 `[[[[…` 就有上萬層巢狀，
// 遞迴解析器會直接爆掉呼叫堆疊（未知輸入不得把驗證器打掛）。

/** 一個數字字面值在原文裡的位置與寫法。 */
export interface NumberLiteral {
  /** RFC 6901 JSON Pointer；頂層純量是空字串。 */
  readonly pointer: string;
  /** 原文（例如 `1.0`、`1e30`、`9007199254740993`）。 */
  readonly raw: string;
  readonly start: number;
  readonly end: number;
}

/** u64 上限（AIP §6 的 `sequence`／`baseRevision`／`revision` 都是 u64）。 */
const MAX_UINT64 = 18446744073709551615n;

/** JSON 的整數語法（不允許前導零、`+`、小數點與指數），且落在 u64 範圍內。 */
export function isUint64Literal(raw: string): boolean {
  if (!/^(?:0|[1-9][0-9]*)$/.test(raw)) return false;
  try {
    return BigInt(raw) <= MAX_UINT64;
  } catch {
    return false;
  }
}

function isJsonWhitespace(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\n" || ch === "\r";
}

/** RFC 6901：`~` → `~0`、`/` → `~1`。 */
function escapePointerSegment(segment: string): string {
  return segment.replace(/~/g, "~0").replace(/\//g, "~1");
}

/** 把帶引號的 JSON 字串還原成鍵名；文字已經被 `JSON.parse` 驗過，這裡只做轉義還原。 */
function unquote(quoted: string): string {
  try {
    const value: unknown = JSON.parse(quoted);
    return typeof value === "string" ? value : "";
  } catch {
    return "";
  }
}

/** 從 `from` 讀一個 JSON number 字面值；不是數字就回 `from`。 */
function readNumber(text: string, from: number): number {
  let i = from;
  if (text[i] === "-") i += 1;
  const digits = i;
  while (i < text.length && text[i] >= "0" && text[i] <= "9") i += 1;
  if (i === digits) return from;
  if (text[i] === ".") {
    i += 1;
    while (i < text.length && text[i] >= "0" && text[i] <= "9") i += 1;
  }
  if (text[i] === "e" || text[i] === "E") {
    i += 1;
    if (text[i] === "+" || text[i] === "-") i += 1;
    while (i < text.length && text[i] >= "0" && text[i] <= "9") i += 1;
  }
  return i;
}

/** 跳過一個 JSON 字串（含轉義），回傳結尾引號之後的位移。 */
function skipString(text: string, from: number): number {
  let i = from + 1;
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\\") {
      i += 2;
      continue;
    }
    if (ch === '"') return i + 1;
    i += 1;
  }
  return text.length;
}

/**
 * 掃出 `text` 裡每個數字字面值，以 JSON Pointer 為鍵。
 *
 * 只在**已經被 `JSON.parse` 接受**的文字上使用：這個掃描器不做語法驗證，
 * 它的工作是把 parse 過程丟掉的資訊（原本怎麼寫的）補回來。
 */
export function scanNumberLiterals(text: string): Map<string, NumberLiteral> {
  const literals = new Map<string, NumberLiteral>();
  // 每一層容器記住「現在這個值」的路徑片段：物件是最後讀到的鍵，陣列是索引。
  const stack: Array<{ array: boolean; segment: string }> = [];
  const pointer = (): string => {
    let path = "";
    for (const frame of stack) path += `/${escapePointerSegment(frame.segment)}`;
    return path;
  };

  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (isJsonWhitespace(ch) || ch === ":") {
      i += 1;
      continue;
    }
    if (ch === "{") {
      stack.push({ array: false, segment: "" });
      i += 1;
      continue;
    }
    if (ch === "[") {
      stack.push({ array: true, segment: "0" });
      i += 1;
      continue;
    }
    if (ch === "}" || ch === "]") {
      stack.pop();
      i += 1;
      continue;
    }
    if (ch === ",") {
      const top = stack[stack.length - 1];
      if (top?.array) top.segment = String(Number(top.segment) + 1);
      i += 1;
      continue;
    }
    if (ch === '"') {
      const end = skipString(text, i);
      const top = stack[stack.length - 1];
      if (top && !top.array) {
        // 物件裡的字串：後面接 `:` 的是鍵，否則是值。
        let after = end;
        while (after < text.length && isJsonWhitespace(text[after])) after += 1;
        if (text[after] === ":") top.segment = unquote(text.slice(i, end));
      }
      i = end;
      continue;
    }
    const end = readNumber(text, i);
    if (end > i) {
      const path = pointer();
      literals.set(path, { pointer: path, raw: text.slice(i, end), start: i, end });
      i = end;
      continue;
    }
    // `true`／`false`／`null`：逐字跳過就好，它們不帶要保留的資訊。
    i += 1;
  }
  return literals;
}
