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
import {
  inboxItemTitle,
  isPendingCountExact,
  PENDING_INCOMPLETE_NOTE,
  pendingCountLabel,
  projectInboxStatus,
  projectSensorStop,
  sensorKindLabel,
  sensorStartedByLabel,
} from "./statusProjection";
import { ConfirmButton, Dialog, useFocusTrap } from "./components/Dialog";
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
import { ConnectPage, decisionPage, loadDecisionInbox } from "./pages/ConnectPage";
import { MorePage } from "./pages/MorePage";
import { ProvidersAdvancedPage } from "./pages/ProvidersAdvanced";
import { KnowledgeAdvancedPage } from "./pages/KnowledgeAdvanced";
import { GlobalSearch } from "./components/GlobalSearch";
import {
  characterNameFallback,
  NEUTRAL_CHARACTER_ICON,
  refreshCharacterName,
  useCharacterName,
} from "./characterName";

export type Tab = string;

export interface NavEntry {
  id: Tab;
  label: string;
  icon: string;
}

// v0.5 資訊架構：5 個一級入口（現在／角色／工作／連接與權限／更多）。
// 第二項的 label 與 icon 是目前角色（useCharacterName：prefs 名字＞manifest
// displayName＞「角色」），由 simpleNavFor 在執行期代入；這份靜態表只放中立值。
// 舊 tab id 全部保留可用（tray 深連結、Inbox route、書籤），由
// navAnchorFor 折疊到新家；內容走 PageBody 的相容路由。
export const SIMPLE_NAV: NavEntry[] = [
  { id: "home", label: "現在", icon: "house" },
  { id: "companion", label: characterNameFallback, icon: NEUTRAL_CHARACTER_ICON },
  { id: "work", label: "工作", icon: "bot" },
  { id: "connect", label: "連接與權限", icon: "plug" },
  { id: "more", label: "更多", icon: "menu" },
];

/** 一級導覽的執行期版本：第二項換成目前角色的名字與 icon（其餘不變、仍恰 5 項）。 */
export function simpleNavFor(character: { name: string; icon: string }): NavEntry[] {
  return SIMPLE_NAV.map((t) =>
    t.id === "companion" ? { ...t, label: character.name, icon: character.icon } : t
  );
}

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

// 相容 tab id → 新一級入口的折疊表。key 是舊 id（tray 深連結、
// Runtime Inbox route、舊書籤、GlobalSearch），value 是導覽高亮／標題的新家。
export const LEGACY_ANCHORS: Record<string, string> = {
  ai: "work",
  automations: "work",
  capabilities: "connect",
  senses: "connect",
  responses: "connect",
  toolops: "connect",
  safety: "connect",
  memory: "more",
  activity: "more",
  settings: "more",
  // v0.5 一般模式「更多」的新分頁：備份與還原／進階模式。
  backup: "more",
  // 相容保留：角色與整合管理不再是「更多」的分頁按鈕，但舊書籤／深連結仍要到得了。
  manage: "more",
  "advanced-features": "more",
};

/** 收件匣狀態的人話：走共用的狀態投影（statusProjection.ts），與 AiPage／
 *  HomePage／收件匣／全域搜尋同一份文案。未知狀態不回原始字串，
 *  投影成「結果不確定」——不假裝看得懂，也不把 enum 外洩到一般模式。 */
export function inboxStatusLabel(status: string): string {
  return projectInboxStatus(status).label;
}

/** 導覽高亮／標題所對應的 nav id（相容 tab 折疊到新 5 入口）。 */
export function navAnchorFor(tab: string): string {
  return LEGACY_ANCHORS[tab] ?? tab;
}

/** topbar 標題：相容 tab 也必須有標題，不得渲染空字串。
 *  角色頁的標題是目前角色的名字（傳入 characterName）；沒傳就是中立的「角色」。 */
export function titleFor(tab: string, characterName?: string): string {
  const anchor = navAnchorFor(tab);
  if (anchor === "companion" && characterName) return characterName;
  return (
    SIMPLE_NAV.find((t) => t.id === anchor)?.label ??
    ADVANCED_NAV.find((t) => t.id === anchor)?.label ??
    ""
  );
}

/** 感測器種類的人話（橫幅用）。未知種類不猜、也不外洩原始 id：走共用投影
 *  （statusProjection.ts）說「其他感測器」，與「現在」頁、角色一句話同一份文案。 */
export { sensorKindLabel };

/** 感測倒數：介面上顯示的「N 秒後自動停止」必須真的走。
 *  interval 只在此元件掛載期間存在（感測結束、banner 消失即清除），有界。 */
export function SensorCountdown({ autoStopAt }: { autoStopAt: string }) {
  const remaining = React.useCallback(
    () => Math.max(0, Math.round((new Date(autoStopAt).getTime() - Date.now()) / 1000)),
    [autoStopAt]
  );
  const [secs, setSecs] = React.useState(remaining);
  React.useEffect(() => {
    setSecs(remaining());
    const t = setInterval(() => setSecs(remaining()), 1000);
    return () => clearInterval(t);
  }, [remaining]);
  return <>{`・${secs} 秒後自動停止`}</>;
}

/**
 * 感測不靜默：只要有感測在跑就一定有這條橫幅（種類、誰啟動的、用途、狀態、倒數、
 * 立即停止）。
 *
 * 「誰啟動的」走 `sensorStartedByLabel`：一般模式說人話，**不得**把 runtime 的內部
 * 身分字串（`iphone:iphone-87b4…` 這種裝置 id）原樣印給使用者看；原始值只在進階模式
 * 以 `title` 補上，所以透明度沒有變少、只是不再外洩實作細節。
 */
export function SensorBanner({
  sensors,
  advanced,
  onStopAll,
}: {
  sensors: readonly import("./api").SensorUse[];
  advanced: boolean;
  onStopAll: () => void;
}) {
  if (sensors.length === 0) return null;
  return (
    <div className="sensor-banner" role="status">
      {sensors.map((s) => (
        <span key={s.kind}>
          {s.kind === "microphone" ? "🎙 正在使用麥克風" : `感測使用中：${sensorKindLabel(s.kind)}`}
          （由{" "}
          <span title={advanced ? s.startedBy : undefined}>{sensorStartedByLabel(s.startedBy)}</span>{" "}
          啟動・{s.purpose}
          {s.state !== undefined && s.state !== "active" ? "・狀態未確認" : ""}
          {s.autoStopAt ? <SensorCountdown autoStopAt={s.autoStopAt} /> : ""}
          ）
        </span>
      ))}
      {/* 停止結果不得靜默吞掉：成功／仍在使用／不確定都會落到同一條回報列。 */}
      <button style={{ marginLeft: 8 }} onClick={onStopAll}>
        立即停止
      </button>
    </div>
  );
}

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
      onNavigate((t) => setTab(t)),
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
        onNavigate={(t) => setTab(t)}
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
              onClick={() => setTab(t.id)}
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
                  onClick={() => setTab(t.id)}
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
        {notificationOpen && (
          <NotificationPanel
            inbox={inbox}
            onClose={() => setNotificationOpen(false)}
            onNavigate={(t) => {
              setTab(t);
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
          setTab(t);
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

/** 右上角徽章的數字。後端說 `pendingCountExact: false` 時 pendingCount 只是
 *  下限，徽章要說「至少 N」——不得讓使用者以為那就是全部。 */
export function inboxBadgeText(inbox: Record<string, unknown> | null): string {
  const raw = inbox?.pendingCount;
  const count = typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : 0;
  return isPendingCountExact(inbox) ? String(count) : `至少 ${count}`;
}

/** 徽章的螢幕閱讀器說明（同一份真相，含「至少」）。 */
export function inboxBadgeLabel(inbox: Record<string, unknown> | null): string {
  if (!inbox) return "未知 項";
  const raw = inbox.pendingCount;
  const count = typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : 0;
  return pendingCountLabel(count, isPendingCountExact(inbox));
}

/** 右上角通知中心：與 Dialog 共用同一個焦點陷阱（Escape 關閉並還原焦點、
 *  Tab 在面板內循環），不是只能用滑鼠點的浮層。 */
export function NotificationPanel({
  inbox,
  onClose,
  onNavigate,
}: {
  inbox: Record<string, unknown> | null;
  onClose: () => void;
  onNavigate: (tab: string) => void;
}) {
  const { ref, onKeyDown } = useFocusTrap(onClose);
  // 徽章用的是截斷前的全量 pendingCount；本頁（最多 10 筆）裝不下的要照實說「還有 N 項」。
  const decisions = decisionPage(inbox, 10);
  return (
    <div
      className="notification-panel"
      role="dialog"
      aria-modal="true"
      aria-label="通知中心"
      tabIndex={-1}
      ref={ref}
      onKeyDown={onKeyDown}
    >
      <div className="row space-between">
        <strong>待你決定</strong>
        <button onClick={onClose}>關閉</button>
      </div>
      {!inbox ? (
        <div className="state-box state-error">目前無法確認通知狀態。</div>
      ) : decisions.shown.length === 0 && decisions.notShown === 0 && !decisions.exact ? (
        // 後端說 pendingCount 只是下限：這一頁空的不代表沒有待決定。
        <div className="state-box" role="status">
          {PENDING_INCOMPLETE_NOTE}。
        </div>
      ) : decisions.shown.length === 0 && decisions.notShown === 0 ? (
        <div className="state-box">目前沒有待決定事項。</div>
      ) : (
        <>
          {decisions.shown.length > 0 && (
            <ul className="plain-list">
              {decisions.shown.map((item) => (
                <li key={`${String(item.kind)}-${String(item.itemId)}`} className="row space-between">
                  <span>
                    <Badge kind="warn">{inboxStatusLabel(String(item.status))}</Badge>{" "}
                    {inboxItemTitle(item)}
                  </span>
                  <button onClick={() => onNavigate(String(item.route))}>前往</button>
                </li>
              ))}
            </ul>
          )}
          {decisions.notShown > 0 && (
            // 誠實：徽章數來自全量，這一頁裝不下（或舊 daemon 只給最近 20 筆）——
            // 不得宣稱「沒有待決定事項」。
            <div className="state-box" role="status">
              {decisions.exact ? "還有" : "至少還有"} {decisions.notShown}{" "}
              項待決定不在這一頁，前往活動歷史。
            </div>
          )}
          {decisions.notShown === 0 && !decisions.exact && (
            <div className="state-box" role="status">
              {PENDING_INCOMPLETE_NOTE}。
            </div>
          )}
        </>
      )}
      <button onClick={() => onNavigate("activity")}>查看完整活動歷史</button>
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
        {external && "（目前連線到外部系統：完全結束只會關閉這個視窗，不會停止那個系統。）"}
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

/** 窄視窗（<700px）底部導覽：4 個主要入口＋「更多」選單。
 *  所有頁面都可抵達、鍵盤可操作、永遠有文字標籤（不只靠 Icon）。 */
const NARROW_PRIMARY: string[] = ["home", "companion", "work", "connect"];

/** 窄視窗「更多」選單的細項（寬視窗時這些是 MorePage 的分頁）。
 *  與 MORE_TABS 同一組 id／文案；`manage` 是隱藏的相容路由，不列在這裡。 */
export const NARROW_MORE_ITEMS: NavEntry[] = [
  { id: "memory", label: "記憶與資料", icon: "book-open" },
  { id: "activity", label: "活動紀錄", icon: "history" },
  { id: "settings", label: "外觀與語言", icon: "settings" },
  { id: "backup", label: "備份與還原", icon: "cloud-download" },
  { id: "advanced-features", label: "進階模式", icon: "code2" },
];

/** 「更多」選單裡目前所在的細項 id。傳進來的是**未折疊**的路由（settings／memory…）；
 *  裸的 `more` 對應 PageBody 的預設分頁（記憶與資料），與寬視窗 MorePage 的高亮一致。 */
export function moreSheetCurrent(tab: Tab): Tab {
  return tab === "more" ? "memory" : tab;
}

export function NarrowNav({
  tab,
  nav,
  onNavigate,
  advanced,
  statusBadge,
}: {
  /** 未折疊的目前路由。一級入口的高亮走 navAnchorFor（相容 id 也會亮對），
   *  「更多」選單的細項則要用原始路由比對，否則永遠沒有細項會亮。 */
  tab: Tab;
  /** 執行期一級導覽（第二項已換成目前角色）。 */
  nav: NavEntry[];
  onNavigate: (tab: Tab) => void;
  advanced: boolean;
  statusBadge: React.ReactNode;
}) {
  const [moreOpen, setMoreOpen] = React.useState(false);
  const primary = nav.filter((t) => NARROW_PRIMARY.includes(t.id));
  const secondary = NARROW_MORE_ITEMS;
  const anchor = navAnchorFor(tab);
  const current = moreSheetCurrent(tab);
  const moreActive = !NARROW_PRIMARY.includes(anchor);
  return (
    <>
      <nav className="bottom-nav" aria-label="主要導覽（窄視窗）">
        {primary.map((t) => (
          <button
            key={t.id}
            className={anchor === t.id ? "bottom-nav-item active" : "bottom-nav-item"}
            onClick={() => onNavigate(t.id)}
            aria-current={anchor === t.id ? "page" : undefined}
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
                className={current === t.id ? "more-item active" : "more-item"}
                aria-current={current === t.id ? "page" : undefined}
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
                    className={current === t.id ? "more-item active" : "more-item"}
                    aria-current={current === t.id ? "page" : undefined}
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

export function PageBody({
  tab,
  refreshKey,
  events,
  advanced,
  onNavigate,
  onRerunOnboarding,
  estopped = false,
  onEstop,
}: {
  tab: Tab;
  refreshKey: number;
  events: RuntimeEvent[];
  advanced: boolean;
  onNavigate: (tab: Tab) => void;
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
      return <CompanionPage refreshKey={refreshKey} />;
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
      return (
        <ConnectPage
          refreshKey={refreshKey}
          advanced={advanced}
          onNavigate={onNavigate}
          initial="devices"
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
      return null;
  }
}
