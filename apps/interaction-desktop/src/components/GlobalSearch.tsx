// Global Search＋Command Palette（spec §16-1.J）：⌘K／Ctrl+K 開啟。
// 搜尋：設定、能力、裝置、AI 工作階段、記憶、知識、互動結果與知識更新紀錄。
// 指令：只列出目前可執行且符合權限的操作（緊急停止永遠在）。
// 安全設計：緊急停止沿用二段確認（不可單鍵誤觸）；IME 組字的 Enter
// 不執行任何項目；指令失敗一律浮出到 Shell 警示列，不得靜默。

import React from "react";
import { api } from "../api";
import { actionStatusLabel, useAppState } from "../appstate";
import { Icon } from "../icons";
import { useFocusTrap } from "./Dialog";
import { K_STATUS_LABEL, memoryLayerLabel } from "../pages/MemoryKnowledgePage";
import {
  capabilityKindLabel,
  knowledgeTriggerLabel,
  projectProviderState,
  projectSensorStop,
  projectWorkState,
  receiptIntentLabel,
} from "../statusProjection";
import { useCharacterName } from "../characterName";

/** 一般模式看得懂的 id：只留尾 6 碼。進階模式才給完整 UUID。
 *  （搜尋比對用的是 label＋detail，所以縮短後仍搜得到後綴。） */
export function shortId(id: string, advanced: boolean): string {
  const trimmed = id.trim();
  if (advanced || trimmed.length <= 8) return trimmed;
  return `…${trimmed.slice(-6)}`;
}

interface SearchItem {
  kind:
    | "page"
    | "command"
    | "capability"
    | "provider"
    | "session"
    | "memory"
    | "knowledge"
    | "receipt";
  label: string;
  detail?: string;
  /** 回傳 CommandOutcome 的指令自己決定回報什麼、算不算成功（例如停止感測要看
   *  重讀到的真實狀態）；回傳 void 的指令沿用 doneMessage＋成功。 */
  action: () => void | Promise<void | CommandOutcome>;
  /** 只進入下一步（例如緊急停止的確認態），面板保持開啟。 */
  keepOpen?: boolean;
  /** 成功後回報給 Shell 的訊息；不設定則靜默成功（純導頁類）。 */
  doneMessage?: string;
}

/** 指令自己回報的結果。`ok=false` 會走 Shell 的警示列——
 *  「已送出」不等於「已完成」，不確定一律不算成功。 */
export interface CommandOutcome {
  ok: boolean;
  message: string;
}

// v0.5 IA：5 個一級入口＋常用細項（細項導到對應 hub 分頁的相容 id）。
// 角色頁的 label 由 useCharacterName 在執行期代入（pagesFor）。
const PAGES: { id: string; label: string }[] = [
  { id: "home", label: "現在" },
  { id: "companion", label: "角色" },
  { id: "work", label: "工作" },
  { id: "connect", label: "連接與權限" },
  { id: "more", label: "更多" },
  { id: "ai", label: "AI 工作階段" },
  { id: "automations", label: "自動互動" },
  { id: "capabilities", label: "裝置與能力" },
  { id: "safety", label: "同意與安全" },
  { id: "memory", label: "記憶與資料" },
  { id: "activity", label: "活動紀錄" },
  { id: "settings", label: "外觀與語言" },
  { id: "backup", label: "備份與還原" },
  // 隱藏的相容路由：「更多」已沒有這個分頁按鈕，但舊書籤與搜尋仍到得了。
  { id: "manage", label: "角色與整合管理" },
  { id: "advanced-features", label: "進階模式" },
];

/** 頁面清單的執行期版本：角色頁用目前角色的名字。 */
export function pagesFor(characterName: string): { id: string; label: string }[] {
  return PAGES.map((p) => (p.id === "companion" ? { ...p, label: characterName } : p));
}

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
  const { human, prefs } = useAppState();
  const advanced = prefs.mode === "advanced";
  const character = useCharacterName({ locale: prefs.locale });
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
      const [sessionsResult, providersResult, memoryResult, knowledgeResult, packsResult, actionsResult, receiptsResult] =
        await Promise.allSettled([
          api.agentSessionsList(),
          api.providersList(),
          api.memoryList(undefined, 100),
          api.knowledgeList(undefined, 100),
          api.domainPacks(),
          api.actionsList(100),
          api.knowledgeReceipts(),
        ]);
      if (sessionsResult.status === "fulfilled") {
        const sessions = sessionsResult.value;
        for (const s of sessions.slice(0, 30)) {
          // 狀態走共用投影：未知狀態是「結果不確定」，原始碼只在進階模式附帶。
          const status = projectWorkState(s.state);
          items.push({
            kind: "session",
            label: `工作階段：${s.label ?? s.agentId}`,
            detail: advanced ? `${status.label}・${s.state}` : status.label,
            action: () => onNavigate("ai"),
          });
        }
      }
      if (providersResult.status === "fulfilled") {
        for (const provider of providersResult.value.slice(0, 50)) {
          const identity = (provider.identity as Record<string, unknown> | undefined) ?? {};
          const rawState = String(provider.state ?? "");
          const providerState = projectProviderState(rawState);
          items.push({
            kind: "provider",
            label: `裝置：${String(identity.displayName ?? identity.id)}`,
            detail: advanced
              ? `${providerState.label}・${String(provider.state ?? identity.kind ?? "")}`
              : providerState.label,
            action: () => onNavigate("capabilities"),
          });
        }
      }
      if (memoryResult.status === "fulfilled") {
        const mem = memoryResult.value as Record<string, unknown>;
        for (const m of ((mem.items as Record<string, unknown>[]) ?? []).slice(0, 50)) {
          items.push({
            kind: "memory",
            label: `記憶：${String(m.title)}`,
            detail: memoryLayerLabel(String(m.layer), advanced, character.name),
            action: () => onNavigate("memory"),
          });
        }
      }
      if (knowledgeResult.status === "fulfilled") {
        const kn = knowledgeResult.value as Record<string, unknown>;
        for (const n of ((kn.nodes as Record<string, unknown>[]) ?? []).slice(0, 50)) {
          items.push({
            kind: "knowledge",
            label: `知識：${String(n.title)}`,
            detail: K_STATUS_LABEL[String(n.status)]?.text ?? String(n.status),
            action: () => onNavigate("memory"),
          });
        }
      }
      if (packsResult.status === "fulfilled") {
        for (const entry of ((packsResult.value.packs as Record<string, unknown>[]) ?? []).slice(0, 20)) {
          const pack = (entry.pack as Record<string, unknown> | undefined) ?? {};
          items.push({
            kind: "knowledge",
            label: `${advanced ? "Domain Pack" : "知識包"}：${String(pack.displayName)}`,
            // 原始包 id 只在進階模式顯示；一般模式只說安裝狀態。
            detail: advanced
              ? `${String(pack.id)}・${entry.installed === true ? "已安裝" : "未安裝"}`
              : entry.installed === true
                ? "已安裝"
                : "未安裝",
            action: () => onNavigate("memory"),
          });
        }
      }
      if (actionsResult.status === "fulfilled") {
        for (const receipt of actionsResult.value.slice(0, 50)) {
          // 意圖走共用投影：`emergency-stop` 這種 Runtime 原始 id 不進一般模式的
          // 標籤（進階模式才在 detail 附上原值，搜尋仍比對得到）。
          items.push({
            kind: "receipt",
            label: `${advanced ? "結果收據" : "互動結果"}：${receiptIntentLabel(receipt.intent)}`,
            detail: advanced
              ? `${actionStatusLabel(receipt.currentStatus)}・${receipt.intent}`
              : actionStatusLabel(receipt.currentStatus),
            action: () => onNavigate("activity"),
          });
        }
      }
      if (receiptsResult.status === "fulfilled") {
        const receipts = receiptsResult.value;
        for (const receipt of ((receipts.receipts as Record<string, unknown>[]) ?? []).slice(0, 50)) {
          // 來由同樣走共用投影：`user-correction` 這種原始值不上一般模式的標籤。
          items.push({
            kind: "receipt",
            label: `${advanced ? "知識收據" : "知識更新"}：${knowledgeTriggerLabel(
              String(receipt.triggeredBy)
            )}`,
            detail: advanced
              ? `${shortId(String(receipt.updateId), advanced)}・${String(receipt.triggeredBy)}`
              : shortId(String(receipt.updateId), advanced),
            action: () => onNavigate("memory"),
          });
        }
      }
      if (alive) setDynamic(items);
    })();
    return () => {
      alive = false;
    };
  }, [open, onNavigate, advanced, character.name]);

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
        // 誠實階梯：送出 ≠ 已停止。送出後重讀狀態，只有真的沒有感測在用才算成功；
        // 有裝置沒回覆是「結果不確定」（ok=false），不得謊稱「已停止所有感測」。
        action: async (): Promise<CommandOutcome> => {
          const report = await api.sensorsStop();
          let remaining: import("../api").SensorUse[] | null = null;
          try {
            const s = await api.status();
            remaining = (s["activeSensors"] as import("../api").SensorUse[] | undefined) ?? [];
          } catch {
            remaining = null;
          }
          return projectSensorStop(report, remaining);
        },
      },
      {
        kind: "command",
        label: "重新偵測裝置與 AI 幫手",
        doneMessage: "已完成重新偵測。",
        action: () => Promise.all([api.agentsRefresh(), api.hardwareScan()]).then(() => {}),
      },
    ];
    const pages: SearchItem[] = pagesFor(character.name).map((p) => ({
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
        detail: advanced ? `${capabilityKindLabel(c.kind)}・${c.kind}` : capabilityKindLabel(c.kind),
        action: () => onNavigate("capabilities"),
      }));
    return [...commands, ...pages, ...caps, ...dynamic];
  }, [human, dynamic, estopped, estopArmed, onNavigate, onEstop, advanced, character.name]);

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
      .then((outcome) => {
        // 指令自己回報的結果優先（可能是「結果不確定」＝ ok:false）。
        if (outcome && typeof outcome === "object" && typeof outcome.message === "string") {
          onCommandFeedback(outcome.message, outcome.ok === true);
        } else if (item.doneMessage) {
          onCommandFeedback(item.doneMessage, true);
        }
      })
      .catch((e) => onCommandFeedback(`「${item.label}」失敗：${String(e)}`, false));
    onClose();
  };

  if (!open) return null;
  return (
    <SearchOverlay onClose={onClose}>
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
              // Escape 由對話框容器統一處理（焦點在選項上也收得掉）。
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
                <span className="search-kind">{kindLabel(item.kind, advanced)}</span>
                <span>{item.label}</span>
                {item.detail && <span className="muted small">{item.detail}</span>}
              </button>
            </li>
          ))}
        </ul>
        <div className="muted small search-hint">↑↓ 選擇・Enter 執行・Esc 關閉</div>
      </div>
    </SearchOverlay>
  );
}

/**
 * 面板的對話框容器：真正的 modal（焦點陷阱＋Escape 掛在容器上）。
 * 只在 open 時掛載，所以 useFocusTrap 的「掛載時聚焦容器、卸載時把焦點還回去」
 * 正好對應開／關；Escape 在面板內任何地方（選項、搜尋框）都收得掉，
 * Tab 只在面板內循環——以前 Escape 只掛在搜尋框上，焦點一離開就關不掉，
 * 面板底下卻寫著「Esc 關閉」（M3c 任務驗收發現）。
 */
function SearchOverlay({ onClose, children }: { onClose: () => void; children: React.ReactNode }) {
  const { ref, onKeyDown } = useFocusTrap(onClose);
  return (
    <div
      className="search-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="全域搜尋"
      tabIndex={-1}
      ref={ref}
      onClick={onClose}
      onKeyDown={(e) => {
        // IME 組字中的 Escape 是取消選字，不是關面板（與搜尋框的 Enter 守則一致）。
        if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
        onKeyDown(e);
      }}
    >
      {children}
    </div>
  );
}

/** 種類標籤：一般模式用人話（「收據」只在進階模式）。 */
export function kindLabel(kind: SearchItem["kind"], advanced = false): string {
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
    case "receipt":
      return advanced ? "收據" : "紀錄";
  }
}
