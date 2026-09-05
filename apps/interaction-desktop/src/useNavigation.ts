// 導覽狀態（目前路由＋內容區的掛載 key）。路由表與純函式在 `routing.ts`。

import React from "react";
import type { Tab } from "./routing";

/**
 * 導覽狀態：目前路由＋內容區的掛載 key。
 *
 * `setTab(目前的路由)` 是 React 的同值 bail-out（不重新渲染），而 hub 頁（連接與
 * 權限／工作／更多）的內部分頁只在 `initial` prop 的值改變時才同步。兩件事加起來，
 * 「導到已經在的路由」原本完全沒有作用——例如緊急停止中、人已在安全頁但把內部分頁
 * 切到「裝置與能力」，再按頂列（或 ⌘K）的「前往解除」就是死點擊，安全關鍵的解除
 * 流程到不了。`mountKey` 每次導覽都改變，所以目標頁一定重新掛載、內部分頁一定回到
 * route 指定的那一個。
 */
/**
 * 深連結的附帶參數（例如角色同步卡的「下一步」：`{ hub: "providers", deviceId }`）。
 * 只在**這一次**導覽有效：下一次 `goTo` 沒帶就清空，所以「導到已經在的那一頁」不會
 * 殘留上一次的 hub 分頁。route id 本身不變（`connect` 仍是 `connect`），深連結盤點與
 * 舊錨點不受影響。
 */
export type NavigateOptions = Record<string, unknown>;

export function useNavigation(initial: Tab): {
  tab: Tab;
  /** 內容區的 key：同一個路由被再次導覽也會變，強制重新掛載。 */
  mountKey: string;
  goTo: (next: Tab, opts?: NavigateOptions) => void;
  /** 這一次導覽附帶的參數（沒有就是 undefined）。 */
  options: NavigateOptions | undefined;
} {
  const [tab, setTab] = React.useState<Tab>(initial);
  const [nonce, setNonce] = React.useState(0);
  const [options, setOptions] = React.useState<NavigateOptions | undefined>(undefined);
  const goTo = React.useCallback((next: Tab, opts?: NavigateOptions) => {
    setNonce((n) => n + 1);
    setOptions(opts);
    setTab(next);
  }, []);
  return { tab, mountKey: `${tab}#${nonce}`, goTo, options };
}
