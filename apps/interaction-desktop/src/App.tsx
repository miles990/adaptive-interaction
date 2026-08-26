import React from "react";
import { api, onRuntimeError, onRuntimeEvent, onRuntimeReady, RuntimeEvent } from "./api";
import { OverviewPage } from "./pages/Overview";
import { ReceptorsPage } from "./pages/Receptors";
import { ActuatorsPage } from "./pages/Actuators";
import { ToolsPage } from "./pages/Tools";
import { RecipesPage } from "./pages/Recipes";
import { PolicyPage } from "./pages/Policy";
import { TimelinePage } from "./pages/Timeline";
import { Badge } from "./ui";

type Tab = "overview" | "receptors" | "actuators" | "tools" | "recipes" | "policy" | "timeline";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "總覽" },
  { id: "receptors", label: "受器" },
  { id: "actuators", label: "動器" },
  { id: "tools", label: "工具" },
  { id: "recipes", label: "配方" },
  { id: "policy", label: "政策／同意" },
  { id: "timeline", label: "時間軸" },
];

type RuntimeState = "connecting" | "ready" | "offline";

export default function App() {
  const [tab, setTab] = React.useState<Tab>("overview");
  const [runtimeState, setRuntimeState] = React.useState<RuntimeState>("connecting");
  const [offlineReason, setOfflineReason] = React.useState<string>("");
  const [estop, setEstop] = React.useState(false);
  const [events, setEvents] = React.useState<RuntimeEvent[]>([]);
  const [refreshKey, setRefreshKey] = React.useState(0);

  React.useEffect(() => {
    const unlistens: Promise<() => void>[] = [];
    unlistens.push(
      onRuntimeReady(() => {
        setRuntimeState("ready");
        refreshEstop();
      })
    );
    unlistens.push(
      onRuntimeError((message) => {
        setRuntimeState("offline");
        setOfflineReason(message);
      })
    );
    unlistens.push(
      onRuntimeEvent((event) => {
        setEvents((prev) => [...prev.slice(-299), event]);
        if (event.eventType === "emergency.stop") {
          refreshEstop();
        }
        setRefreshKey((k) => k + 1);
      })
    );
    // Also poll once in case runtime-ready fired before we listened.
    const probe = setInterval(async () => {
      try {
        const status = await api.status();
        setRuntimeState("ready");
        setEstop(Boolean(status["emergencyStop"]));
        clearInterval(probe);
        const recent = await api.eventsRecent(200);
        setEvents(recent);
      } catch (e) {
        // keep connecting; startup error listener handles hard failures
      }
    }, 500);
    return () => {
      clearInterval(probe);
      unlistens.forEach((u) => u.then((f) => f()).catch(() => {}));
    };
  }, []);

  async function refreshEstop() {
    try {
      const status = await api.status();
      setEstop(Boolean(status["emergencyStop"]));
    } catch {
      /* offline */
    }
  }

  async function triggerEstop() {
    try {
      if (estop) {
        await api.emergencyStopClear();
      } else {
        await api.emergencyStop("desktop button");
      }
      await refreshEstop();
      setRefreshKey((k) => k + 1);
    } catch (e) {
      alertToConsole(e);
    }
  }

  if (runtimeState === "offline") {
    return (
      <div className="app offline-screen">
        <h1>Runtime 無法啟動</h1>
        <p className="state-box state-error">{offlineReason}</p>
        <p>
          可能已有 <code>interact-ai serve</code> daemon 佔用了 instance lock。
          請先停止它，或直接使用 CLI／HTTP 管理該實例。
        </p>
      </div>
    );
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-title">Interaction</div>
          <div className="brand-sub">Control Center</div>
        </div>
        <nav>
          {TABS.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? "nav-item active" : "nav-item"}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          {runtimeState === "connecting" ? (
            <Badge kind="pending">連線中…</Badge>
          ) : (
            <Badge kind={estop ? "bad" : "ok"}>{estop ? "緊急停止中" : "運作中"}</Badge>
          )}
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <div className="topbar-title">{TABS.find((t) => t.id === tab)?.label}</div>
          <button
            className={estop ? "estop estop-clear" : "estop"}
            onClick={triggerEstop}
            title="不經過任何佇列，立即停止所有輸出"
          >
            {estop ? "解除緊急停止" : "緊急停止"}
          </button>
        </header>
        {estop && (
          <div className="estop-banner">
            緊急停止已啟動：所有動器已停止、未完成動作已中止、同意已撤回。解除需要明確操作，不會自動恢復。
          </div>
        )}
        <div className="content" key={`${tab}`}>
          {runtimeState === "connecting" ? (
            <div className="state-box">正在啟動 Runtime…</div>
          ) : (
            <PageBody tab={tab} refreshKey={refreshKey} events={events} />
          )}
        </div>
      </main>
    </div>
  );
}

function PageBody({
  tab,
  refreshKey,
  events,
}: {
  tab: Tab;
  refreshKey: number;
  events: RuntimeEvent[];
}) {
  switch (tab) {
    case "overview":
      return <OverviewPage refreshKey={refreshKey} />;
    case "receptors":
      return <ReceptorsPage refreshKey={refreshKey} />;
    case "actuators":
      return <ActuatorsPage refreshKey={refreshKey} />;
    case "tools":
      return <ToolsPage />;
    case "recipes":
      return <RecipesPage refreshKey={refreshKey} />;
    case "policy":
      return <PolicyPage refreshKey={refreshKey} />;
    case "timeline":
      return <TimelinePage events={events} />;
  }
}

function alertToConsole(e: unknown) {
  // Browser modal dialogs are avoided by design; errors land in the console
  // and the UI state boxes.
  console.error("[control-center]", e);
}
