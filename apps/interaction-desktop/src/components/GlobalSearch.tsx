// Global Search＋Command Palette（spec §16-1.J）：⌘K／Ctrl+K 開啟。
// 搜尋：設定、能力、裝置、Agent 工作階段、記憶、知識、收據。
// 指令：只列出目前可執行且符合權限的操作（緊急停止永遠在）。

import React from "react";
import { api } from "../api";
import { useAppState } from "../appstate";
import { Icon } from "../icons";

interface SearchItem {
  kind: "page" | "command" | "capability" | "provider" | "session" | "memory" | "knowledge";
  label: string;
  detail?: string;
  action: () => void | Promise<void>;
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
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: (tab: string) => void;
  estopped: boolean;
}) {
  const { human } = useAppState();
  const [query, setQuery] = React.useState("");
  const [dynamic, setDynamic] = React.useState<SearchItem[]>([]);
  const [active, setActive] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [open]);

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
    const commands: SearchItem[] = [
      {
        kind: "command",
        label: estopped ? "前往解除緊急停止" : "緊急停止",
        detail: estopped ? "已在緊急停止狀態" : "立即停止一切",
        action: async () => {
          if (!estopped) await api.emergencyStop("global search");
          onNavigate("safety");
        },
      },
      {
        kind: "command",
        label: "暫停主動互動一小時",
        action: () => api.pauseSet(60, "global search").then(() => {}),
      },
      {
        kind: "command",
        label: "一小時內不要主動說話",
        action: () => api.proactiveDialogueQuiet(60).then(() => {}),
      },
      {
        kind: "command",
        label: "停止所有感測",
        action: () => api.sensorsStop().then(() => {}),
      },
      {
        kind: "command",
        label: "重新偵測裝置與 AI agent",
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
  }, [human, dynamic, estopped, onNavigate]);

  const q = query.trim().toLowerCase();
  const filtered = q
    ? items.filter(
        (i) =>
          i.label.toLowerCase().includes(q) || (i.detail ?? "").toLowerCase().includes(q)
      )
    : items.slice(0, 12);

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
            }}
            onKeyDown={(e) => {
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
                void filtered[active].action();
                onClose();
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
                onClick={() => {
                  void item.action();
                  onClose();
                }}
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
