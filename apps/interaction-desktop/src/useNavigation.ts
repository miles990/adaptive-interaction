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
export function useNavigation(initial: Tab): {
  tab: Tab;
  /** 內容區的 key：同一個路由被再次導覽也會變，強制重新掛載。 */
  mountKey: string;
  goTo: (next: Tab) => void;
} {
  const [tab, setTab] = React.useState<Tab>(initial);
  const [nonce, setNonce] = React.useState(0);
  const goTo = React.useCallback((next: Tab) => {
    setNonce((n) => n + 1);
    setTab(next);
  }, []);
  return { tab, mountKey: `${tab}#${nonce}`, goTo };
}
