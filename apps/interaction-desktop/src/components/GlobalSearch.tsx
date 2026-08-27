// Global Search＋Command Palette（spec §16-1.J）：⌘K／Ctrl+K 開啟。
// 搜尋：設定、能力、裝置、Agent 工作階段、記憶、知識、收據。
// 指令：只列出目前可執行且符合權限的操作（緊急停止永遠在）。
// 安全設計：緊急停止沿用二段確認（不可單鍵誤觸）；IME 組字的 Enter
// 不執行任何項目；指令失敗一律浮出到 Shell 警示列，不得靜默。

import React from "react";
import { api } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";

interface SearchItem {
  kind: "page" | "command" | "capability" | "provider" | "session" | "memory" | "knowledge";
  label: string;
  detail?: string;
  action: () => void | Promise<void>;
  /** 只進入下一步（例如緊急停止的確認態），面板保持開啟。 */
  keepOpen?: boolean;
  /** 成功後回報給 Shell 的訊息；不設定則靜默成功（純導頁類）。 */
  doneMessage?: string;
}

const PAGES: { id: string; label: string }[] = [
  { id: "home", label: "首頁" },
  { id: "companion", label: "小樞" },
  { id: "ai", label: "AI 與工作階段" },
  { id: "capabilities", label: "能力與裝置" },
  { id: "memory", label: "記憶與知識" },
  { id: "automations", label: "自動互動" },
  { id: "activity", label: "活動與確認" },
  { id: "safety", label: "隱私與安全" },
  { id: "settings", label: "設定" },
];

export function GlobalSearch({
  open,
  onClose,
  onNavigate,
  estopped,
  onEstop,
  onCommandFeedback,
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: (tab: string) => void;
  estopped: boolean;
  /** Shell 的緊急停止流程：失敗會顯示重試警示列，絕不無聲。 */
  onEstop: () => Promise<void>;
  /** 指令結果回報（ok=false 時 Shell 以警示列顯示）。 */
  onCommandFeedback: (message: string, ok: boolean) => void;
}) {
  const { human } = useAppState();
  const [query, setQuery] = React.useState("");
  const [dynamic, setDynamic] = React.useState<SearchItem[]>([]);
  const [active, setActive] = React.useState(0);
  const [estopArmed, setEstopArmed] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setEstopArmed(false);
      setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [open]);

  // 確認態 5 秒未確認自動解除（與 ConfirmButton 相同節奏）。
  React.useEffect(() => {
    if (!estopArmed) return;
    const t = setTimeout(() => setEstopArmed(false), 5000);
    return () => clearTimeout(t);
  }, [estopArmed]);

  // 動態資料（session／記憶／知識）：開啟時載一次。
  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    void (async () => {
      const items: SearchItem[] = [];
      try {
        const sessions = await api.agentSessionsList();
        for (const s of sessions.slice(0, 30)) {
          items.push({
            kind: "session",
            label: `工作階段：${s.label ?? s.agentId}`,
            detail: s.state,
            action: () => onNavigate("ai"),
          });
        }
      } catch { /* offline */ }
      try {
        const mem = (await api.memoryList(undefined, 100)) as Record<string, unknown>;
        for (const m of ((mem.items as Record<string, unknown>[]) ?? []).slice(0, 50)) {
          items.push({
            kind: "memory",
            label: `記憶：${String(m.title)}`,
            detail: String(m.layer),
            action: () => onNavigate("memory"),
          });
        }
      } catch { /* offline */ }
      try {
        const kn = (await api.knowledgeList(undefined, 100)) as Record<string, unknown>;
        for (const n of ((kn.nodes as Record<string, unknown>[]) ?? []).slice(0, 50)) {
          items.push({
            kind: "knowledge",
            label: `知識：${String(n.title)}`,
            detail: String(n.status),
            action: () => onNavigate("memory"),
          });
        }
      } catch { /* offline */ }
      if (alive) setDynamic(items);
    })();
    return () => {
      alive = false;
    };
  }, [open, onNavigate]);

  const items = React.useMemo<SearchItem[]>(() => {
    // 緊急停止：第一下只進入確認態（面板保持開啟），第二下才執行。
    // 失敗回饋走 Shell 的 estop 重試警示列（onEstop 內建），不在此重複。
    const estopItem: SearchItem = estopped
      ? {
          kind: "command",
          label: "前往解除緊急停止",
          detail: "已在緊急停止狀態",
          action: () => onNavigate("safety"),
        }
      : estopArmed
        ? {
            kind: "command",
            label: "立即停止一切？",
            detail: "再按一次確認緊急停止",
            action: async () => {
              await onEstop();
              onNavigate("safety");
            },
          }
        : {
            kind: "command",
            label: "緊急停止",
            detail: "立即停止一切（需再確認一次）",
            keepOpen: true,
            action: () => setEstopArmed(true),
          };
    const commands: SearchItem[] = [
      estopItem,
      {
        kind: "command",
        label: "暫停主動互動一小時",
        doneMessage: "已暫停主動互動一小時。",
        action: () => api.pauseSet(60, "global search").then(() => {}),
      },
      {
        kind: "command",
        label: "一小時內不要主動說話",
        doneMessage: "已設定：一小時內不主動說話。",
        action: () => api.proactiveDialogueQuiet(60).then(() => {}),
      },
      {
        kind: "command",
        label: "停止所有感測",
        doneMessage: "已停止所有感測。",
        action: () => api.sensorsStop().then(() => {}),
      },
      {
        kind: "command",
        label: "重新偵測裝置與 AI agent",
        doneMessage: "已完成重新偵測。",
        action: () => api.agentsRefresh().then(() => {}),
      },
    ];
    const pages: SearchItem[] = PAGES.map((p) => ({
      kind: "page",
      label: p.label,
      detail: "頁面",
      action: () => onNavigate(p.id),
    }));
    const caps: SearchItem[] = [
      ...(human?.receptors ?? []),
      ...(human?.actuators ?? []),
      ...(human?.toolOperations ?? []),
    ]
      .slice(0, 200)
      .map((c) => ({
        kind: "capability" as const,
        label: `能力：${c.displayName}`,
        detail: c.kind,
        action: () => onNavigate("capabilities"),
      }));
    return [...commands, ...pages, ...caps, ...dynamic];
  }, [human, dynamic, estopped, estopArmed, onNavigate, onEstop]);

  const q = query.trim().toLowerCase();
  const filtered = q
    ? items.filter(
        (i) =>
          i.label.toLowerCase().includes(q) || (i.detail ?? "").toLowerCase().includes(q)
      )
    : items.slice(0, 12);

  // 執行項目：等待結果並回報（queued≠completed——只有 resolve 才回報成功；
  // reject 一律浮出，尤其是停止感測／緊急停止這類安全指令）。
  const runItem = (item: SearchItem) => {
    if (item.keepOpen) {
      void item.action();
      return;
    }
    Promise.resolve()
      .then(() => item.action())
      .then(() => {
        if (item.doneMessage) onCommandFeedback(item.doneMessage, true);
      })
      .catch((e) => onCommandFeedback(`「${item.label}」失敗：${String(e)}`, false));
    onClose();
  };

  if (!open) return null;
  return (
    <div className="search-overlay" role="dialog" aria-label="全域搜尋" onClick={onClose}>
      <div className="search-panel" onClick={(e) => e.stopPropagation()}>
        <div className="search-input-row">
          <Icon name="search" size={16} />
          <input
            ref={inputRef}
            value={query}
            placeholder="搜尋設定、能力、記憶、知識…或輸入指令"
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
              // 改變搜尋內容即解除確認態，避免誤觸與過期的確認。
              setEstopArmed(false);
            }}
            onKeyDown={(e) => {
              // IME 組字（選字）的 Enter 不是執行指令；keyCode 229 涵蓋
              // WebKit 在 compositionend 後才送出的 commit-Enter。
              if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((a) => Math.min(a + 1, filtered.length - 1));
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => Math.max(a - 1, 0));
              }
              if (e.key === "Enter" && filtered[active]) {
                runItem(filtered[active]);
              }
            }}
          />
        </div>
        <ul className="search-results" role="listbox">
          {filtered.length === 0 && <li className="muted small">沒有符合的結果。</li>}
          {filtered.map((item, i) => (
            <li key={`${item.kind}-${item.label}`}>
              <button
                role="option"
                aria-selected={i === active}
                className={i === active ? "search-item active" : "search-item"}
                onMouseEnter={() => setActive(i)}
                onClick={() => runItem(item)}
              >
                <span className="search-kind">{kindLabel(item.kind)}</span>
                <span>{item.label}</span>
                {item.detail && <span className="muted small">{item.detail}</span>}
              </button>
            </li>
          ))}
        </ul>
        <div className="muted small search-hint">↑↓ 選擇・Enter 執行・Esc 關閉</div>
      </div>
    </div>
  );
}

function kindLabel(kind: SearchItem["kind"]): string {
  switch (kind) {
    case "page":
      return "頁面";
    case "command":
      return "指令";
    case "capability":
      return "能力";
    case "provider":
      return "裝置";
    case "session":
      return "工作階段";
    case "memory":
      return "記憶";
    case "knowledge":
      return "知識";
  }
}
