import React from "react";
import { api, onRuntimeError, onRuntimeEvent, onRuntimeReady, RuntimeEvent } from "./api";
import {
  bootstrapSupervisor,
  onCloseRequested,
  onNavigate,
  onSupervisorState,
  onTrayActionError,
  SupervisorInfo,
} from "./desktop";
import { AppStateProvider, useAppState } from "./appstate";
import { Icon } from "./icons";
import { Badge } from "./ui";
import { projectSensorStop, sensorKindLabel } from "./statusProjection";
import { ConfirmButton } from "./components/Dialog";
import { HomePage } from "./pages/HomePage";
import { CapabilitiesPage } from "./pages/CapabilitiesPage";
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
import { WorkPage } from "./pages/WorkPage";
import { ConnectPage, loadDecisionInbox } from "./pages/ConnectPage";
import { MorePage } from "./pages/MorePage";
import { ProvidersAdvancedPage } from "./pages/ProvidersAdvanced";
import { KnowledgeAdvancedPage } from "./pages/KnowledgeAdvanced";
import { GlobalSearch } from "./components/GlobalSearch";
import { refreshCharacterName, useCharacterName } from "./characterName";
import { ADVANCED_NAV, navAnchorFor, simpleNavFor, titleFor, type Tab } from "./routing";
import { useNavigation, type NavigateOptions } from "./useNavigation";
import { SensorBanner } from "./components/SensorBanner";
import { CloseDialog } from "./components/CloseDialog";
import { inboxBadgeLabel, inboxBadgeText, NotificationPanel } from "./components/NotificationPanel";
import { NarrowNav } from "./components/NarrowNav";

// 這個檔案只剩三件事：bootstrap（App）、外框與全域狀態（Shell）、頁面分派（PageBody）。
// 路由表與導覽的純函式在 `routing.ts`，導覽狀態在 `useNavigation.ts`，純 UI 元件在
// `components/`。以下的 re-export 是為了讓既有的 `from "./App"` 匯入路徑不變
// （零行為變更的搬家），新程式請直接從各自的模組匯入。
export type { NavEntry, Tab } from "./routing";
export {
  LEGACY_ANCHORS,
  moreSheetCurrent,
  NARROW_MORE_ITEMS,
  navAnchorFor,
  simpleNavFor,
  SIMPLE_NAV,
  titleFor,
} from "./routing";
export { useNavigation } from "./useNavigation";
export { SensorBanner, SensorCountdown } from "./components/SensorBanner";
export {
  inboxBadgeLabel,
  inboxBadgeText,
  inboxStatusLabel,
  NotificationPanel,
} from "./components/NotificationPanel";
export { NarrowNav } from "./components/NarrowNav";

/** 感測器種類的人話（橫幅用）。未知種類不猜、也不外洩原始 id：走共用投影
 *  （statusProjection.ts）說「其他感測器」，與「現在」頁、角色一句話同一份文案。 */
export { sensorKindLabel };

type RuntimeState = "connecting" | "ready" | "offline";


export default function App() {
  const [runtimeState, setRuntimeState] = React.useState<RuntimeState>("connecting");
  const [offlineReason, setOfflineReason] = React.useState<string>("");
  const [events, setEvents] = React.useState<RuntimeEvent[]>([]);
  const [refreshKey, setRefreshKey] = React.useState(0);
  /**
   * 「這條連線換了一條」的計數（supervisor 連線狀態變化／SSE 重新接上時 +1）。
   *
   * 與 `refreshKey` 分開：後者每一則 runtime 事件都 +1，拿它當對齊訊號會讓角色
   * 同步卡退回「每則事件三支 API」。斷線期間漏掉的狀態要靠一次 resume 補回來。
   */
  const [connectionKey, setConnectionKey] = React.useState(0);
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
          setConnectionKey((k) => k + 1);
        })
      );
      probe = setInterval(async () => {
        try {
          await api.status();
          setRuntimeState("ready");
          if (probe) clearInterval(probe);
          const recent = await api.eventsRecent(200);
          setEvents(recent);
          // 事件流是重新接上的：中間漏掉的狀態不會自己補回來，要重新對齊一次。
          setConnectionKey((k) => k + 1);
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
    // 這一頁在偏好載入前就會出現（不知道一般／進階），所以主文一律人話；
    // daemon／token／CLI 這類技術線索收進「技術細節」折疊區，不消失也不裸露。
    const external = supervisor?.mode === "external";
    return (
      <div className="app offline-screen">
        <h1>系統無法啟動</h1>
        <p className="state-box state-error">{offlineReason}</p>
        <p>
          {external
            ? "偵測到另一個已在執行的系統，但無法取得授權連線。請先確認它仍在運作，或關閉它之後再重新開啟這個應用程式。"
            : "系統沒有成功啟動。若剛剛才關閉另一個視窗，請稍候幾秒再重新開啟。"}
        </p>
        <details className="tech-details">
          <summary>技術細節</summary>
          <p className="muted small">
            {external
              ? "外部 interact-ai daemon 已在監聽，但無法讀取其授權 token 檔案（~/.adaptive-interaction/state/api-token）。"
              : "內嵌 Runtime 無法啟動；既有實例可用 interact-ai CLI 或 HTTP API 管理。"}
          </p>
        </details>
      </div>
    );
  }

  return (
    <AppStateProvider ready={runtimeState === "ready"} refreshKey={refreshKey}>
      <Shell
        connecting={runtimeState !== "ready"}
        events={events}
        refreshKey={refreshKey}
        connectionKey={connectionKey}
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
  connectionKey,
  bumpRefresh,
  supervisor,
  disconnected,
}: {
  connecting: boolean;
  events: RuntimeEvent[];
  refreshKey: number;
  /** 「這條連線換了一條」：只在 supervisor 連線狀態變化／SSE 重新接上時前進。 */
  connectionKey: number;
  bumpRefresh: () => void;
  supervisor: SupervisorInfo | null;
  disconnected: boolean;
}) {
  const { prefs, pause } = useAppState();
  const { tab, mountKey, goTo, options: navOptions } = useNavigation("home");
  // 目前角色（導覽第二項、標題、全域搜尋共用同一份）。
  const character = useCharacterName({ locale: prefs.locale });
  const [estop, setEstop] = React.useState(false);
  const [estopError, setEstopError] = React.useState<string | null>(null);
  const [onboarding, setOnboarding] = React.useState<"unknown" | "open" | "closed">("unknown");
  const [closeDialog, setCloseDialog] = React.useState(false);
  const [trayError, setTrayError] = React.useState<string | null>(null);
  const [sensors, setSensors] = React.useState<import("./api").SensorUse[]>([]);
  const [searchOpen, setSearchOpen] = React.useState(false);
  const [notificationOpen, setNotificationOpen] = React.useState(false);
  const [inbox, setInbox] = React.useState<Record<string, unknown> | null>(null);
  // 全域搜尋指令的結果回報：失敗以警示列顯示（不得靜默），成功短暫提示。
  const [commandNotice, setCommandNotice] = React.useState<{
    message: string;
    ok: boolean;
  } | null>(null);
  const advanced = prefs.mode === "advanced";

  React.useEffect(() => {
    if (prefs.appearance === "light" || prefs.appearance === "dark") {
      document.documentElement.dataset.theme = prefs.appearance;
    } else {
      delete document.documentElement.dataset.theme;
    }
    document.documentElement.classList.toggle("reduce-motion", prefs.reduceMotion === true);
    document.body.style.zoom = String((prefs.scalePercent ?? 100) / 100);
    return () => {
      document.body.style.zoom = "";
    };
  }, [prefs.appearance, prefs.reduceMotion, prefs.scalePercent]);

  React.useEffect(() => {
    if (!commandNotice?.ok) return;
    const t = setTimeout(() => setCommandNotice(null), 5000);
    return () => clearTimeout(t);
  }, [commandNotice]);

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
      onNavigate((t) => goTo(t)),
      onTrayActionError((m) => setTrayError(m)),
    ];
    return () => unlistens.forEach((u) => u.then((f) => f()).catch(() => {}));
  }, []);

  // 角色換了（hello／重協商／斷線）要立刻反映在導覽與標題；換頁時順便刷新
  // （受最短間隔保護），涵蓋在角色頁改名後回到其他頁的情況。
  const lastEvent = events.length > 0 ? events[events.length - 1] : null;
  React.useEffect(() => {
    if (!lastEvent) return;
    if (
      lastEvent.eventType === "character.instance" ||
      lastEvent.eventType.startsWith("presentation.")
    ) {
      void refreshCharacterName({ locale: prefs.locale, force: true });
    }
  }, [lastEvent, prefs.locale]);
  React.useEffect(() => {
    if (connecting) return;
    void refreshCharacterName({ locale: prefs.locale });
  }, [tab, connecting, prefs.locale]);

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
    // 通知中心只列待決定：優先用後端的 needsDecision 篩選，舊 daemon 退回最近 20 筆
    // （面板會用 pendingCount 對照，不把「這一頁沒有」說成「沒有待決定」）。
    loadDecisionInbox(20)
      .then(setInbox)
      .catch(() => setInbox(null));
  }, [connecting, refreshKey]);

  /** 「立即停止」：送出 ≠ 已停止。送出後重讀 status，把真實剩餘的感測誠實回報出來
   *  （成功走綠色狀態列、仍在使用／不確定／失敗走警示列）；不得靜默 catch。 */
  async function stopAllSensors() {
    let report: unknown;
    try {
      report = await api.sensorsStop();
    } catch (e) {
      setCommandNotice({ message: `停止所有感測失敗：${String(e)}`, ok: false });
      return;
    }
    let remaining: import("./api").SensorUse[] | null = null;
    try {
      const s = await api.status();
      remaining = (s["activeSensors"] as import("./api").SensorUse[] | undefined) ?? [];
      setSensors(remaining);
      setEstop(Boolean(s["emergencyStop"]));
    } catch {
      remaining = null;
    }
    setCommandNotice(projectSensorStop(report, remaining));
    bumpRefresh();
  }

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
        onNavigate={(t) => goTo(t)}
        onDone={() => {
          setOnboarding("closed");
          bumpRefresh();
        }}
        onSkip={() => setOnboarding("closed")}
      />
    );
  }

  const navTab = navAnchorFor(tab);
  const title = titleFor(tab, character.name);
  const nav = simpleNavFor(character);

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
          {nav.map((t) => (
            <button
              key={t.id}
              className={navTab === t.id ? "nav-item active" : "nav-item"}
              onClick={() => goTo(t.id)}
              aria-current={navTab === t.id ? "page" : undefined}
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
                  className={navTab === t.id ? "nav-item active" : "nav-item"}
                  onClick={() => goTo(t.id)}
                  aria-current={navTab === t.id ? "page" : undefined}
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
            <Badge kind="bad">系統連線中斷</Badge>
          ) : estop ? (
            <Badge kind="bad">緊急停止中</Badge>
          ) : pause.paused ? (
            <Badge kind="warn">主動互動已暫停</Badge>
          ) : (
            <Badge kind="ok">運作中</Badge>
          )}
          {supervisor?.mode === "external" && (
            <div className="muted small" title={advanced ? supervisor.apiBase : undefined}>
              {advanced ? "外部 Runtime" : "連線到外部系統"}
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
          <button
            className="notification-trigger"
            onClick={() => setNotificationOpen((open) => !open)}
            aria-label={`通知中心，${inboxBadgeLabel(inbox)}待決定`}
            aria-expanded={notificationOpen}
          >
            通知 {inbox ? inboxBadgeText(inbox) : "?"}
          </button>
          {/* 觸發是一鍵；「解除」刻意不在這裡 — 要走安全頁的恢復流程。 */}
          {estop ? (
            <button className="estop-indicator" onClick={() => goTo("safety")}>
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
        {notificationOpen && (
          <NotificationPanel
            inbox={inbox}
            onClose={() => setNotificationOpen(false)}
            onNavigate={(t) => {
              goTo(t);
              setNotificationOpen(false);
            }}
          />
        )}
        {estopError && (
          <div className="estop-banner" role="alert">
            ⚠️ 緊急停止指令失敗：{estopError} — 系統可能仍在運作，請立即重試，或直接關閉應用程式
            （關閉視窗會安全停止整個系統）。
            <button className="danger" style={{ marginLeft: 8 }} onClick={triggerEstop}>
              重試緊急停止
            </button>
          </div>
        )}
        {estop && (
          <div className="estop-banner" role="alert">
            緊急停止已啟動：所有回應已停止、未完成動作已中止。解除需到「連接與權限 → 同意與安全」走安全流程，不會自動恢復。
          </div>
        )}
        <SensorBanner sensors={sensors} advanced={advanced} onStopAll={() => void stopAllSensors()} />
        {disconnected && (
          <div className="estop-banner" role="alert">
            與外部系統的連線中斷 — 顯示的資料可能已過期，指令暫時無法送達。會自動重新連線。
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
        {commandNotice &&
          (commandNotice.ok ? (
            <div className="sensor-banner" role="status">
              {commandNotice.message}
            </div>
          ) : (
            <div className="estop-banner" role="alert">
              {commandNotice.message} — 系統狀態可能未改變，請重試或到對應頁面確認。
              <button style={{ marginLeft: 8 }} onClick={() => setCommandNotice(null)}>
                知道了
              </button>
            </div>
          ))}
        {/* key 含導覽序號：同一個路由被再次導覽（例如緊急停止中重複按「前往解除」）
            也會重新掛載，hub 頁的內部分頁因此一定回到 route 指定的那一個。 */}
        <div className="content" id="main-content" key={mountKey}>
          {connecting ? (
            <div className="state-box">正在啟動系統…</div>
          ) : (
            <PageBody
              tab={tab}
              refreshKey={refreshKey}
              connectionKey={connectionKey}
              events={events}
              advanced={advanced}
              onNavigate={goTo}
              navOptions={navOptions}
              onRerunOnboarding={() => setOnboarding("open")}
              estopped={estop}
              onEstop={triggerEstop}
            />
          )}
        </div>
      </main>
      <GlobalSearch
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onNavigate={(t) => {
          goTo(t);
          setSearchOpen(false);
        }}
        estopped={estop}
        onEstop={triggerEstop}
        onCommandFeedback={(message, ok) => setCommandNotice({ message, ok })}
      />
      {closeDialog && (
        <CloseDialog external={supervisor?.mode === "external"} onClose={() => setCloseDialog(false)} />
      )}
      <NarrowNav
        tab={tab}
        nav={nav}
        onNavigate={goTo}
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

export function PageBody({
  tab,
  refreshKey,
  connectionKey = 0,
  events,
  advanced,
  onNavigate,
  navOptions,
  onRerunOnboarding,
  estopped = false,
  onEstop,
}: {
  tab: Tab;
  refreshKey: number;
  /** 「這條連線換了一條」：角色同步卡收到就重新對齊一次（不隨每則事件變動）。 */
  connectionKey?: number;
  events: RuntimeEvent[];
  advanced: boolean;
  onNavigate: (tab: Tab, opts?: NavigateOptions) => void;
  /** 這一次導覽附帶的參數（例如同步卡要一鍵到配對區：`{ hub: "providers" }`）。 */
  navOptions?: NavigateOptions;
  onRerunOnboarding: () => void;
  /** Shell 已知的緊急停止狀態（供「現在」頁的快速操作顯示「前往解除」）。 */
  estopped?: boolean;
  /** Shell 的緊急停止流程；與頂部列、⌘K 是同一條路徑（失敗會有重試警示列）。 */
  onEstop?: () => Promise<void>;
}) {
  switch (tab) {
    case "home":
      return (
        <HomePage
          refreshKey={refreshKey}
          events={events}
          onNavigate={onNavigate}
          estopped={estopped}
          onEstop={onEstop}
        />
      );
    case "companion":
      // events：角色同步卡靠 SSE 的 `character.session.state` 對齊本地副本，
      // 不必每一則 runtime 事件都重問一次權威狀態（那會消耗 session sequence）。
      // onNavigate：角色同步卡的「下一步」要一鍵到得了連接與權限頁
      // （M3 §4.2；CompanionPage 再往下傳給 CharacterSyncCard）。
      return (
        <CompanionPage
          refreshKey={refreshKey}
          events={events}
          connectionKey={connectionKey}
          onNavigate={onNavigate}
        />
      );
    // 工作：AI 工作階段＋自動互動（舊 id 進到對應分頁）。
    case "work":
    case "ai":
      return (
        <WorkPage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
          initial="sessions"
        />
      );
    case "automations":
      return (
        <WorkPage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
          initial="automations"
        />
      );
    // 連接與權限：裝置與能力＋同意與安全（舊 id 進到對應分頁）。
    case "connect":
    case "capabilities":
      // 深連結帶 `hub: "providers"`（角色同步卡的「連接手機／重新確認」）就一步到配對區；
      // 其餘照舊落在「裝置與能力」第一層。route id 不變，只是落點更準。
      return (
        <ConnectPage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
          initial={navOptions?.["hub"] === "providers" ? "providers" : "devices"}
        />
      );
    case "safety":
      return (
        <ConnectPage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
          initial="safety"
        />
      );
    // v0.3 相容路徑（tray 深連結／舊書籤）：聚焦單類能力清單。
    case "senses":
      return <CapabilitiesPage kind="receptor" advanced={advanced} />;
    case "responses":
      return <CapabilitiesPage kind="actuator" advanced={advanced} />;
    case "toolops":
      return <CapabilitiesPage kind="tool-operation" advanced={advanced} />;
    // 更多：記憶與資料／活動紀錄／外觀與語言／備份與還原（舊 id 進到對應分頁）。
    case "more":
    case "memory":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="memory"
        />
      );
    case "activity":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="activity"
        />
      );
    case "settings":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="settings"
        />
      );
    case "backup":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="backup"
        />
      );
    // 隱藏的相容路由：角色與整合管理不再有分頁按鈕，舊書籤／深連結仍到得了。
    case "manage":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="manage"
        />
      );
    case "advanced-features":
      return (
        <MorePage
          refreshKey={refreshKey}
          events={events}
          advanced={advanced}
          onNavigate={onNavigate}
          onRerunOnboarding={onRerunOnboarding}
          initial="advanced-features"
        />
      );
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
      // 未知路由不得靜默空白：外部 daemon（supervisor mode "external"）版本可能領先
      // 前端，收件匣「前往」按鈕把後端給的 route 字串未經白名單直接餵進 goTo。
      // 不確定要說不確定，並留一條回得去的路（回到「現在」），不是一片空白。
      return (
        <div className="state-box state-error" role="alert">
          <p>找不到這個頁面。這個項目要開的頁面，這個版本的控制中心還不認得。</p>
          <button onClick={() => onNavigate("home")}>回到「現在」</button>
        </div>
      );
  }
}
