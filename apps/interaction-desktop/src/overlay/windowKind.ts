// 一個前端 bundle、三個視窗：控制中心（main）、桌面角色（companion）、
// 可信安全 overlay（overlay）。Tauri 內以視窗 label 決定（query string 不會
// 跟著 WebviewUrl::App 路徑走）；`?window=` 只給 dev／E2E 用。

export type WindowKind = "main" | "companion" | "overlay";

export function resolveWindowKind(label: string | undefined, search: string): WindowKind {
  if (label === "overlay") return "overlay";
  if (label === "companion") return "companion";
  const query = new URLSearchParams(search).get("window");
  if (query === "overlay") return "overlay";
  if (query === "companion") return "companion";
  return "main";
}
