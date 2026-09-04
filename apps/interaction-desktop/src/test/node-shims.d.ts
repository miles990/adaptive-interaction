// vitest 在 Node 上執行，但這個前端專案刻意不安裝 @types/node
// （瀏覽器程式碼不該用到 Node API）。靜態守門測試需要讀 styles.css 原文，
// 而 Vite 在 vitest 下會把 CSS 的 ?raw 匯入清成空字串，因此只在測試層
// 宣告最小可用的 node 型別。正式程式碼不得使用這些模組。
declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf8"): string;
  export function readdirSync(path: string): string[];
}

declare module "node:path" {
  export function resolve(...parts: string[]): string;
  export function join(...parts: string[]): string;
}

declare const __dirname: string;
