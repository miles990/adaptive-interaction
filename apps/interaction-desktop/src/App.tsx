import React from "react";
import { api, onRuntimeError, onRuntimeEvent, onRuntimeReady, RuntimeEvent } from "./api";
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

type Tab = string;

const SIMPLE_NAV: { id: Tab; label: string; icon: string }[] = [
  { id: "home", label: "首頁", icon: "house" },
  { id: "senses", label: "感知來源", icon: "scan-eye" },
  { id: "responses", label: "回應方式", icon: "send" },
  { id: "toolops", label: "工具操作", icon: "wrench" },
  { id: "automations", label: "自動互動", icon: "workflow" },
  { id: "safety", label: "同意與安全", icon: "shield-check" },
  { id: "activity", label: "活動紀錄", icon: "history" },
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
];

type RuntimeState = "connecting" | "ready" | "offline";

export default function App() {
  const [runtimeState, setRuntimeState] = React.useState<RuntimeState>("connecting");
  const [offlineReason, setOfflineReason] = React.useState<string>("");
  const [events, setEvents] = React.useState<RuntimeEvent[]>([]);
  const [refreshKey, setRefreshKey] = React.useState(0);

  React.useEffect(() => {
    const unlistens: Promise<() => void>[] = [];
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
    const probe = setInterval(async () => {
      try {
        await api.status();
        setRuntimeState("ready");
        clearInterval(probe);
        const recent = await api.eventsRecent(200);
        setEvents(recent);
      } catch {
        /* keep connecting */
      }
    }, 500);
    return () => {
      clearInterval(probe);
      unlistens.forEach((u) => u.then((f) => f()).catch(() => {}));
    };
  }, []);

  if (runtimeState === "offline") {
    return (
      <div className="app offline-screen">
        <h1>系統無法啟動</h1>
        <p className="state-box state-error">{offlineReason}</p>
        <p>
          可能已有另一個 <code>interact-ai serve</code> 正在執行。請先停止它，
          或直接使用 CLI／HTTP 管理該實例。
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
      />
    </AppStateProvider>
  );
}

function Shell({
  connecting,
  events,
  refreshKey,
  bumpRefresh,
}: {
  connecting: boolean;
  events: RuntimeEvent[];
  refreshKey: number;
  bumpRefresh: () => void;
}) {
  const { prefs, pause } = useAppState();
  const [tab, setTab] = React.useState<Tab>("home");
  const [estop, setEstop] = React.useState(false);
  const [estopError, setEstopError] = React.useState<string | null>(null);
  const [onboarding, setOnboarding] = React.useState<"unknown" | "open" | "closed">("unknown");
  const advanced = prefs.mode === "advanced";

  React.useEffect(() => {
    if (connecting) return;
    api
      .status()
      .then((s) => {
        setEstop(Boolean(s["emergencyStop"]));
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
          ) : estop ? (
            <Badge kind="bad">緊急停止中</Badge>
          ) : pause.paused ? (
            <Badge kind="warn">主動互動已暫停</Badge>
          ) : (
            <Badge kind="ok">運作中</Badge>
          )}
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <div className="topbar-title">{title}</div>
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

/** 窄視窗（<700px）底部導覽：4 個主要功能＋「更多」選單。
 *  所有頁面都可抵達、鍵盤可操作、永遠有文字標籤（不只靠 Icon）。 */
const NARROW_PRIMARY: string[] = ["home", "automations", "safety", "activity"];

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
      return <ActivityPage refreshKey={refreshKey} events={events} advanced={advanced} />;
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
    default:
      return null;
  }
}
