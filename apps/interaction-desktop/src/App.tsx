import React from "react";
import { api, onRuntimeError, onRuntimeEvent, onRuntimeReady, RuntimeEvent } from "./api";
import {
  bootstrapSupervisor,
  desktop,
  onCloseRequested,
  onNavigate,
  onSupervisorState,
  onTrayActionError,
  SupervisorInfo,
} from "./desktop";
import { AppStateProvider, useAppState } from "./appstate";
import { Icon } from "./icons";
import { Badge } from "./ui";
import { ConfirmButton, Dialog } from "./components/Dialog";
import { HomePage } from "./pages/HomePage";
import { CapabilitiesPage } from "./pages/CapabilitiesPage";
import { AutomationsPage } from "./pages/AutomationsPage";
import { SafetyPage } from "./pages/SafetyPage";
import { ActivityPage } from "./pages/ActivityPage";
import { SettingsPage } from "./pages/SettingsPage";
import { Onboarding } from "./pages/Onboarding";
// 進階模式保留原有技術頁面（零能力退化）。
import { OverviewPage } from "./pages/Overview";
import { ReceptorsPage } from "./pages/Receptors";
import { ActuatorsPage } from "./pages/Actuators";
import { ToolsPage } from "./pages/Tools";
import { RecipesPage } from "./pages/Recipes";
import { PolicyPage } from "./pages/Policy";
import { TimelinePage } from "./pages/Timeline";
import { CompanionPage } from "./pages/CompanionPage";
import { AiPage } from "./pages/AiPage";
import { CapabilitiesHub } from "./pages/CapabilitiesHub";
import { MemoryKnowledgePage } from "./pages/MemoryKnowledgePage";
import { ProvidersAdvancedPage } from "./pages/ProvidersAdvanced";
import { KnowledgeAdvancedPage } from "./pages/KnowledgeAdvanced";
import { GlobalSearch } from "./components/GlobalSearch";

type Tab = string;

// v0.4 資訊架構（spec §16-1.A）：8 個一級頁＋自動互動（保留 v0.3 能力）。
const SIMPLE_NAV: { id: Tab; label: string; icon: string }[] = [
  { id: "home", label: "首頁", icon: "house" },
  { id: "companion", label: "小樞", icon: "cat" },
  { id: "ai", label: "AI 與工作階段", icon: "bot" },
  { id: "capabilities", label: "能力與裝置", icon: "plug" },
  { id: "memory", label: "記憶與知識", icon: "book-open" },
  { id: "automations", label: "自動互動", icon: "workflow" },
  { id: "activity", label: "活動與確認", icon: "history" },
  { id: "safety", label: "隱私與安全", icon: "shield-check" },
  { id: "settings", label: "設定", icon: "settings" },
];

const ADVANCED_NAV: { id: Tab; label: string }[] = [
  { id: "adv-overview", label: "總覽（原始）" },
  { id: "adv-receptors", label: "受器" },
  { id: "adv-actuators", label: "動器" },
  { id: "adv-tools", label: "工具" },
  { id: "adv-recipes", label: "配方 YAML" },
  { id: "adv-policy", label: "政策／同意" },
  { id: "adv-timeline", label: "時間軸" },
  { id: "adv-providers", label: "Provider Registry" },
  { id: "adv-knowledge", label: "Knowledge Graph" },
];

type RuntimeState = "connecting" | "ready" | "offline";

export default function App() {
  const [runtimeState, setRuntimeState] = React.useState<RuntimeState>("connecting");
  const [offlineReason, setOfflineReason] = React.useState<string>("");
  const [events, setEvents] = React.useState<RuntimeEvent[]>([]);
  const [refreshKey, setRefreshKey] = React.useState(0);
  const [supervisor, setSupervisor] = React.useState<SupervisorInfo | null>(null);
  const [disconnected, setDisconnected] = React.useState(false);

  React.useEffect(() => {
    const unlistens: Promise<() => void>[] = [];
    let probe: ReturnType<typeof setInterval> | undefined;
    let cancelled = false;

    // Supervisor decides transport (embedded IPC vs external-daemon HTTP)
    // BEFORE we start probing the runtime.
    void bootstrapSupervisor().then((info) => {
      if (cancelled) return;
      setSupervisor(info);
      unlistens.push(onRuntimeReady(() => setRuntimeState("ready")));
      unlistens.push(
        onRuntimeError((message) => {
          setRuntimeState("offline");
          setOfflineReason(message);
        })
      );
      unlistens.push(
        onRuntimeEvent((event) => {
          setEvents((prev) => [...prev.slice(-299), event]);
          setRefreshKey((k) => k + 1);
        })
      );
      unlistens.push(
        onSupervisorState((s) => {
          setDisconnected(s === "disconnected");
          if (s === "connected-to-external") setRefreshKey((k) => k + 1);
        })
      );
      probe = setInterval(async () => {
        try {
          await api.status();
          setRuntimeState("ready");
          if (probe) clearInterval(probe);
          const recent = await api.eventsRecent(200);
          setEvents(recent);
        } catch {
          /* keep connecting */
        }
      }, 500);
    });

    return () => {
      cancelled = true;
      if (probe) clearInterval(probe);
      unlistens.forEach((u) => u.then((f) => f()).catch(() => {}));
    };
  }, []);

  if (runtimeState === "offline") {
    return (
      <div className="app offline-screen">
        <h1>系統無法啟動</h1>
        <p className="state-box state-error">{offlineReason}</p>
        <p>
          {supervisor?.mode === "external"
            ? "偵測到外部 interact-ai daemon，但無法建立授權連線。請檢查該 daemon 的狀態與 token 檔案。"
            : "Runtime 無法啟動。若剛剛才關閉另一個實例，請稍候幾秒再重新開啟；也可以直接使用 CLI／HTTP 管理既有實例。"}
        </p>
      </div>
    );
  }

  return (
    <AppStateProvider ready={runtimeState === "ready"} refreshKey={refreshKey}>
      <Shell
        connecting={runtimeState !== "ready"}
        events={events}
        refreshKey={refreshKey}
        bumpRefresh={() => setRefreshKey((k) => k + 1)}
        supervisor={supervisor}
        disconnected={disconnected}
      />
    </AppStateProvider>
  );
}

function Shell({
  connecting,
  events,
  refreshKey,
  bumpRefresh,
  supervisor,
  disconnected,
}: {
  connecting: boolean;
  events: RuntimeEvent[];
  refreshKey: number;
  bumpRefresh: () => void;
  supervisor: SupervisorInfo | null;
  disconnected: boolean;
}) {
  const { prefs, pause } = useAppState();
  const [tab, setTab] = React.useState<Tab>("home");
  const [estop, setEstop] = React.useState(false);
  const [estopError, setEstopError] = React.useState<string | null>(null);
  const [onboarding, setOnboarding] = React.useState<"unknown" | "open" | "closed">("unknown");
  const [closeDialog, setCloseDialog] = React.useState(false);
  const [trayError, setTrayError] = React.useState<string | null>(null);
  const [sensors, setSensors] = React.useState<import("./api").SensorUse[]>([]);
  const [searchOpen, setSearchOpen] = React.useState(false);
  const advanced = prefs.mode === "advanced";

  // 全域搜尋／指令面板：⌘K（macOS）／Ctrl+K。
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearchOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  React.useEffect(() => {
    const unlistens = [
      onCloseRequested(() => setCloseDialog(true)),
      onNavigate((t) => setTab(t)),
      onTrayActionError((m) => setTrayError(m)),
    ];
    return () => unlistens.forEach((u) => u.then((f) => f()).catch(() => {}));
  }, []);

  React.useEffect(() => {
    if (connecting) return;
    api
      .status()
      .then((s) => {
        setEstop(Boolean(s["emergencyStop"]));
        setSensors((s["activeSensors"] as import("./api").SensorUse[] | undefined) ?? []);
        if (onboarding === "unknown") {
          setOnboarding(s["onboardingCompleted"] === true ? "closed" : "open");
        }
      })
      .catch(() => {
        /* transient backend hiccup: keep last known state; next event retries */
      });
  }, [connecting, refreshKey]);

  async function triggerEstop() {
    try {
      await api.emergencyStop("desktop button");
      const s = await api.status();
      setEstop(Boolean(s["emergencyStop"]));
      setEstopError(null);
      bumpRefresh();
    } catch (e) {
      // 緊急停止失敗絕不能無聲：顯著顯示並保留重試按鈕。
      setEstopError(String(e));
    }
  }

  if (!connecting && onboarding === "open") {
    return (
      <Onboarding
        onDone={() => {
          setOnboarding("closed");
          bumpRefresh();
        }}
        onSkip={() => setOnboarding("closed")}
      />
    );
  }

  const title =
    SIMPLE_NAV.find((t) => t.id === tab)?.label ??
    ADVANCED_NAV.find((t) => t.id === tab)?.label ??
    "";

  return (
    <div className="app">
      <a className="skip-link" href="#main-content">
        跳到主要內容
      </a>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-title">Interaction</div>
          <div className="brand-sub">Control Center</div>
        </div>
        <nav aria-label="主要導覽">
          {SIMPLE_NAV.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? "nav-item active" : "nav-item"}
              onClick={() => setTab(t.id)}
              aria-current={tab === t.id ? "page" : undefined}
            >
              <Icon name={t.icon} size={16} /> <span>{t.label}</span>
            </button>
          ))}
          {advanced && (
            <>
              <div className="nav-group-label">
                <Icon name="code2" size={13} /> 進階
              </div>
              {ADVANCED_NAV.map((t) => (
                <button
                  key={t.id}
                  className={tab === t.id ? "nav-item active" : "nav-item"}
                  onClick={() => setTab(t.id)}
                  aria-current={tab === t.id ? "page" : undefined}
                >
                  <span>{t.label}</span>
                </button>
              ))}
            </>
          )}
        </nav>
        <div className="sidebar-footer">
          {connecting ? (
            <Badge kind="pending">連線中…</Badge>
          ) : disconnected ? (
            <Badge kind="bad">Runtime 連線中斷</Badge>
          ) : estop ? (
            <Badge kind="bad">緊急停止中</Badge>
          ) : pause.paused ? (
            <Badge kind="warn">主動互動已暫停</Badge>
          ) : (
            <Badge kind="ok">運作中</Badge>
          )}
          {supervisor?.mode === "external" && (
            <div className="muted small" title={supervisor.apiBase}>
              外部 Runtime
            </div>
          )}
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <div className="topbar-title">{title}</div>
          <button
            className="search-trigger"
            onClick={() => setSearchOpen(true)}
            aria-label="全域搜尋與指令（Cmd+K）"
            title="搜尋與指令（⌘K）"
          >
            <Icon name="search" size={15} /> 搜尋
          </button>
          {/* 觸發是一鍵；「解除」刻意不在這裡 — 要走安全頁的恢復流程。 */}
          {estop ? (
            <button className="estop-indicator" onClick={() => setTab("safety")}>
              <Icon name="octagon-x" size={16} /> 緊急停止中 — 前往解除
            </button>
          ) : (
            <ConfirmButton
              className="estop"
              label="緊急停止"
              confirmLabel="立即停止一切？"
              onConfirm={triggerEstop}
            />
          )}
        </header>
        {estopError && (
          <div className="estop-banner" role="alert">
            ⚠️ 緊急停止指令失敗：{estopError} — 系統可能仍在運作，請立即重試，或直接關閉應用程式
            （關閉視窗會安全停止整個 Runtime）。
            <button className="danger" style={{ marginLeft: 8 }} onClick={triggerEstop}>
              重試緊急停止
            </button>
          </div>
        )}
        {estop && (
          <div className="estop-banner" role="alert">
            緊急停止已啟動：所有回應已停止、未完成動作已中止。解除需到「同意與安全」頁走安全流程，不會自動恢復。
          </div>
        )}
        {sensors.length > 0 && (
          <div className="sensor-banner" role="status">
            {sensors.map((s) => (
              <span key={s.kind}>
                {s.kind === "microphone" ? "🎙 正在使用麥克風" : `使用中：${s.kind}`}
                （由 {s.startedBy === "desktop" ? "你" : s.startedBy} 啟動・{s.purpose}
                {s.autoStopAt
                  ? `・${Math.max(0, Math.round((new Date(s.autoStopAt).getTime() - Date.now()) / 1000))} 秒後自動停止`
                  : ""}
                ）
              </span>
            ))}
            <button
              style={{ marginLeft: 8 }}
              onClick={() => api.sensorsStop().then(bumpRefresh).catch(() => {})}
            >
              立即停止
            </button>
          </div>
        )}
        {disconnected && (
          <div className="estop-banner" role="alert">
            與外部 Runtime 的連線中斷 — 顯示的資料可能已過期，指令暫時無法送達。系統會自動重新連線。
          </div>
        )}
        {trayError && (
          <div className="estop-banner" role="alert">
            狀態列指令失敗：{trayError}
            <button style={{ marginLeft: 8 }} onClick={() => setTrayError(null)}>
              知道了
            </button>
          </div>
        )}
        <div className="content" id="main-content" key={tab}>
          {connecting ? (
            <div className="state-box">正在啟動系統…</div>
          ) : (
            <PageBody
              tab={tab}
              refreshKey={refreshKey}
              events={events}
              advanced={advanced}
              onNavigate={setTab}
              onRerunOnboarding={() => setOnboarding("open")}
            />
          )}
        </div>
      </main>
      <GlobalSearch
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onNavigate={(t) => {
          setTab(t);
          setSearchOpen(false);
        }}
        estopped={estop}
      />
      {closeDialog && (
        <CloseDialog external={supervisor?.mode === "external"} onClose={() => setCloseDialog(false)} />
      )}
      <NarrowNav
        tab={tab}
        onNavigate={setTab}
        advanced={advanced}
        statusBadge={
          connecting ? (
            <Badge kind="pending">連線中…</Badge>
          ) : estop ? (
            <Badge kind="bad">緊急停止中</Badge>
          ) : pause.paused ? (
            <Badge kind="warn">主動互動已暫停</Badge>
          ) : (
            <Badge kind="ok">運作中</Badge>
          )
        }
      />
    </div>
  );
}

/** 第一次關閉控制中心的說明對話框（也是 v0.2 → v0.3 行為改變的明確告知）。 */
function CloseDialog({ external, onClose }: { external: boolean; onClose: () => void }) {
  const [remember, setRemember] = React.useState(false);
  return (
    <Dialog title="關閉控制中心？" onClose={onClose}>
      <p>
        Adaptive Interaction 會繼續在<strong>狀態列</strong>運作。
        桌面角色與你允許的自動互動仍會保持啟用。
      </p>
      <p className="muted small">
        你可以從狀態列重新開啟控制中心，或選擇「完全結束」停止所有功能。
        {external && "（目前連線到外部 Runtime：完全結束只會關閉這個視窗，不會停止外部 Runtime。）"}
      </p>
      <p className="muted small">
        提醒：舊版（v0.2）關閉視窗會直接停止系統；新版預設改為保持在背景運作。
      </p>
      <label className="toggle">
        <input
          type="checkbox"
          checked={remember}
          onChange={(e) => setRemember(e.target.checked)}
        />
        <span>下次不再顯示</span>
      </label>
      <div className="row wrap" style={{ marginTop: 12 }}>
        <button
          className="primary"
          onClick={async () => {
            await desktop.closeDecision("keep-running", remember).catch(() => {});
            onClose();
          }}
        >
          保持運作
        </button>
        <button
          onClick={async () => {
            await desktop.closeDecision("quit", remember).catch(() => {});
            onClose();
          }}
        >
          完全結束
        </button>
      </div>
    </Dialog>
  );
}

/** 窄視窗（<700px）底部導覽：4 個主要功能＋「更多」選單。
 *  所有頁面都可抵達、鍵盤可操作、永遠有文字標籤（不只靠 Icon）。 */
const NARROW_PRIMARY: string[] = ["home", "ai", "activity", "safety"];

function NarrowNav({
  tab,
  onNavigate,
  advanced,
  statusBadge,
}: {
  tab: Tab;
  onNavigate: (tab: Tab) => void;
  advanced: boolean;
  statusBadge: React.ReactNode;
}) {
  const [moreOpen, setMoreOpen] = React.useState(false);
  const primary = SIMPLE_NAV.filter((t) => NARROW_PRIMARY.includes(t.id));
  const secondary = SIMPLE_NAV.filter((t) => !NARROW_PRIMARY.includes(t.id));
  const moreActive = !NARROW_PRIMARY.includes(tab);
  return (
    <>
      <nav className="bottom-nav" aria-label="主要導覽（窄視窗）">
        {primary.map((t) => (
          <button
            key={t.id}
            className={tab === t.id ? "bottom-nav-item active" : "bottom-nav-item"}
            onClick={() => onNavigate(t.id)}
            aria-current={tab === t.id ? "page" : undefined}
          >
            <Icon name={t.icon} size={18} />
            <span>{t.label}</span>
          </button>
        ))}
        <button
          className={moreActive ? "bottom-nav-item active" : "bottom-nav-item"}
          onClick={() => setMoreOpen(true)}
          aria-haspopup="dialog"
          aria-expanded={moreOpen}
        >
          <Icon name="menu" size={18} />
          <span>更多</span>
        </button>
      </nav>
      {moreOpen && (
        <Dialog title="更多功能" onClose={() => setMoreOpen(false)}>
          <div className="more-sheet">
            <div className="more-status">{statusBadge}</div>
            {secondary.map((t) => (
              <button
                key={t.id}
                className={tab === t.id ? "more-item active" : "more-item"}
                onClick={() => {
                  onNavigate(t.id);
                  setMoreOpen(false);
                }}
              >
                <Icon name={t.icon} size={16} /> <span>{t.label}</span>
              </button>
            ))}
            {advanced && (
              <>
                <div className="nav-group-label">
                  <Icon name="code2" size={13} /> 進階
                </div>
                {ADVANCED_NAV.map((t) => (
                  <button
                    key={t.id}
                    className={tab === t.id ? "more-item active" : "more-item"}
                    onClick={() => {
                      onNavigate(t.id);
                      setMoreOpen(false);
                    }}
                  >
                    <span>{t.label}</span>
                  </button>
                ))}
              </>
            )}
          </div>
        </Dialog>
      )}
    </>
  );
}

function PageBody({
  tab,
  refreshKey,
  events,
  advanced,
  onNavigate,
  onRerunOnboarding,
}: {
  tab: Tab;
  refreshKey: number;
  events: RuntimeEvent[];
  advanced: boolean;
  onNavigate: (tab: Tab) => void;
  onRerunOnboarding: () => void;
}) {
  switch (tab) {
    case "home":
      return <HomePage refreshKey={refreshKey} events={events} onNavigate={onNavigate} />;
    case "companion":
      return <CompanionPage refreshKey={refreshKey} />;
    case "ai":
      return <AiPage refreshKey={refreshKey} onNavigate={onNavigate} />;
    case "capabilities":
      return <CapabilitiesHub refreshKey={refreshKey} advanced={advanced} />;
    case "memory":
      return <MemoryKnowledgePage refreshKey={refreshKey} />;
    // v0.3 相容路徑（tray 深連結／舊書籤）：導向新家。
    case "senses":
      return <CapabilitiesPage kind="receptor" advanced={advanced} />;
    case "responses":
      return <CapabilitiesPage kind="actuator" advanced={advanced} />;
    case "toolops":
      return <CapabilitiesPage kind="tool-operation" advanced={advanced} />;
    case "automations":
      return <AutomationsPage refreshKey={refreshKey} advanced={advanced} />;
    case "safety":
      return <SafetyPage refreshKey={refreshKey} />;
    case "activity":
      return <ActivityPage refreshKey={refreshKey} events={events} advanced={advanced} onNavigate={onNavigate} />;
    case "settings":
      return <SettingsPage onRerunOnboarding={onRerunOnboarding} />;
    case "adv-overview":
      return <OverviewPage refreshKey={refreshKey} />;
    case "adv-receptors":
      return <ReceptorsPage refreshKey={refreshKey} />;
    case "adv-actuators":
      return <ActuatorsPage refreshKey={refreshKey} />;
    case "adv-tools":
      return <ToolsPage />;
    case "adv-recipes":
      return <RecipesPage refreshKey={refreshKey} />;
    case "adv-policy":
      return <PolicyPage refreshKey={refreshKey} />;
    case "adv-timeline":
      return <TimelinePage events={events} />;
    case "adv-providers":
      return <ProvidersAdvancedPage refreshKey={refreshKey} />;
    case "adv-knowledge":
      return <KnowledgeAdvancedPage refreshKey={refreshKey} />;
    default:
      return null;
  }
}
