// 連接與權限（v0.5 IA）：裝置與能力＋同意與安全集中在同一個入口。
// 權限地圖從首頁移到這裡 — 這裡是「AI 可以讀取或操作什麼」唯一的主人。
//
// 一般模式第一層以「裝置優先」回答五件事，順序固定：
//   1 已連接的裝置 2 系統可以看見什麼 3 系統可以做什麼 4 目前需要確認的權限 5 立即停止與撤銷。
// 完整的能力卡片、掃描、配對與來源清單收在第二層「全部能力與裝置」（既有 CapabilitiesHub）。
// 所有狀態標籤走 statusProjection；角色名稱走 useCharacterName；安全文字固定。
//
// 誠實階梯：停止感測只會照後端逐項回報說話（送出≠停止），撤回授權失敗要說「狀態未變」，
// 技術識別（裝置識別碼、埠號、線路狀態）只在進階模式出現。

import React from "react";
import {
  api,
  HumanCard,
  SensorStopDeviceReport,
  SensorStopReport,
  SensorUse,
  Session,
  SensorStopSourceReport,
} from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { ConfirmButton } from "../components/Dialog";
import { Icon } from "../icons";
import { RISK_TIERS } from "../riskTier";
import {
  characterSyncDeviceLine,
  inboxItemTitle,
  inboxKindLabel,
  isPendingCountExact,
  PENDING_INCOMPLETE_NOTE,
  projectInboxStatus,
  projectProviderState,
} from "../statusProjection";
import { Badge, Section, useAsync } from "../ui";
import { CapabilitiesHub } from "./CapabilitiesHub";
import type { HubTab } from "./CapabilitiesHub";
import { SafetyPage } from "./SafetyPage";
import { PermissionMap } from "./HomePage";
import { CharacterAdaptersSection } from "./connect/CharacterAdaptersSection";
import {
  isMobileProviderId,
  phoneCardModel,
  PhoneDeviceCard,
  phonePermissionAlerts,
  stopSensorsMessage,
} from "./connect/PhoneDeviceCard";

export type ConnectTab = "devices" | "safety";

/**
 * 深連結落點。`"providers"` 不是第三個分頁，而是「裝置與能力」＋第二層直接停在
 * 配對區（`CapabilitiesHub` 的「裝置與來源」）——角色同步卡的「連接手機／去重新確認／
 * 重新連接手機」要一步到得了那裡，而不是把人丟在第一層自己找（M3 §4.2）。
 */
export type ConnectInitial = ConnectTab | "providers";

/** 深連結落點 → 第一層分頁。 */
function connectTabOf(initial: ConnectInitial): ConnectTab {
  return initial === "providers" ? "devices" : initial;
}

// ---------------------------------------------------------------------------
// 待決定清單（通知中心與「需要你確認」共用）
// ---------------------------------------------------------------------------

/** 後端是否認得 `needsDecision` 篩選：null＝還沒探測；false＝舊 daemon 拒絕過，之後不再送。 */
let needsDecisionFilterSupported: boolean | null = null;

/** 測試用：清掉探測結果。 */
export function resetDecisionInboxProbeForTests(): void {
  needsDecisionFilterSupported = null;
}

/** serde deny_unknown_fields 的拒絕訊息（`unknown field \`needsDecision\``）。 */
function isUnknownFieldRejection(error: unknown): boolean {
  return /unknown field/i.test(String(error)) && /needsDecision/.test(String(error));
}

/**
 * 待你決定的清單：優先請後端只回 needsDecision 的項目（v0.5 `ActivityInboxFilter.needsDecision`），
 * 讓一頁就是「全部待決定」的前 N 筆。舊 daemon 的 filter 是 deny_unknown_fields、會整筆拒絕——
 * 這時退回不帶篩選的查詢；那一頁只是「最近 N 筆」，呼叫端必須用 `pendingCount`
 * 對照本頁的待決定數，不得把「這一頁沒有」說成「沒有待決定事項」。
 */
export async function loadDecisionInbox(limit = 20): Promise<Record<string, unknown>> {
  if (needsDecisionFilterSupported !== false) {
    try {
      const inbox = await api.activityInbox({ limit, needsDecision: true });
      needsDecisionFilterSupported = true;
      return inbox;
    } catch (error) {
      if (isUnknownFieldRejection(error)) needsDecisionFilterSupported = false;
      // 其它錯誤（暫時失聯等）也退回一次不帶篩選的查詢；再失敗就照實往上丟。
    }
  }
  return api.activityInbox({ limit });
}

/** 收件匣頁面裡的待決定項目，以及「不在這一頁」的待決定數
 *  （徽章用的 pendingCount 是截斷前的全量；本頁可能一筆待決定都沒裝到）。
 *  `exact` 直接來自後端的 `pendingCountExact`：`false` 代表 pendingCount 只是
 *  下限（掃描上限內沒撈完），介面必須改口「至少 N 項」，而且**即使本頁與
 *  pendingCount 都是 0，也不得宣稱「沒有待決定事項」**。 */
export function decisionPage(
  inbox: Record<string, unknown> | null | undefined,
  shownLimit: number
): {
  shown: Record<string, unknown>[];
  notShown: number;
  pendingCount: number;
  exact: boolean;
} {
  const items = ((inbox?.items as Record<string, unknown>[] | undefined) ?? []).filter(
    (item) => item && typeof item === "object" && item.needsDecision === true
  );
  const reported = inbox?.pendingCount;
  const pendingCount =
    typeof reported === "number" && Number.isFinite(reported) && reported >= 0
      ? Math.max(Math.floor(reported), items.length)
      : items.length;
  const shown = items.slice(0, shownLimit);
  return {
    shown,
    notShown: Math.max(0, pendingCount - shown.length),
    pendingCount,
    // 讀不到收件匣時不做「精確」的宣稱也沒有意義（呼叫端另有 loading／error 分支）。
    exact: isPendingCountExact(inbox),
  };
}

/** 桌面角色的呈現層 provider：角色區已經列了，四區摘要不重複。 */
const COMPANION_PROVIDER_ID = "provider.companion.desktop";

export function ConnectPage({
  refreshKey,
  advanced,
  onNavigate,
  initial = "devices",
  focusDeviceId,
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  initial?: ConnectInitial;
  /**
   * 深連結指名的裝置（角色同步卡的「去重新確認」帶來的 deviceId）。落點是第二層的
   * 配對區（iPhone 區）：把那一台的卡片標出來並捲到它，不改變任何狀態、
   * 不代替使用者按任何東西。第一層的裝置區用的是同一個卡片元件，同樣標出來
   *（兩層同時在畫面上，只標一邊會變成「同一台手機兩種說法」）。
   */
  focusDeviceId?: string;
}) {
  const [tab, setTab] = React.useState<ConnectTab>(() => connectTabOf(initial));
  const { human } = useAppState();
  const [hubTab, setHubTab] = React.useState<HubTab>(() =>
    initial === "providers" ? "providers" : "senses"
  );
  // 相容路由：App 對「work／automations」這類舊 id 都渲染同一個元件，React 會沿用
  // 已掛載的實例，useState(initial) 只在首次掛載生效——route 改變時必須同步分頁，
  // 否則 tray／深連結／全域搜尋／Inbox 切到舊 id 只會高亮導覽、內容不動。
  React.useEffect(() => {
    setTab(connectTabOf(initial));
    // 深連結指名配對區時，第二層也要跟著到位（同樣的理由：已掛載的實例不會自己動）。
    if (initial === "providers") setHubTab("providers");
  }, [initial]);
  const allRef = React.useRef<HTMLDetailsElement>(null);

  /** 四區的「管理…」按鈕：展開第二層並切到對應分類。 */
  function showAll(next: HubTab) {
    setHubTab(next);
    const el = allRef.current;
    if (el) {
      el.open = true;
      el.scrollIntoView?.({ block: "start", behavior: "smooth" });
    }
  }

  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="連接與權限分類">
        {(
          [
            ["devices", "裝置與能力"],
            ["safety", "同意與安全"],
          ] as [ConnectTab, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={tab === id ? "hub-tab active" : "hub-tab"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>
      {tab === "devices" && (
        <div>
          <ConnectOverview
            refreshKey={refreshKey}
            advanced={advanced}
            onNavigate={onNavigate}
            onShowAll={showAll}
            onSafety={() => setTab("safety")}
            {...(focusDeviceId ? { focusDeviceId } : {})}
          />
          <details className="connect-all" open ref={allRef}>
            <summary>全部能力與裝置</summary>
            <p className="muted small">
              完整清單：每一項能力都可以測試、啟用或停用；裝置可以掃描、配對與測試。
            </p>
            <CapabilitiesHub
              refreshKey={refreshKey}
              advanced={advanced}
              initial={hubTab}
              {...(focusDeviceId ? { focusDeviceId } : {})}
            />
          </details>
        </div>
      )}
      {tab === "safety" && (
        <div>
          <Section title="權限地圖 — AI 現在可以做什麼？">
            {human ? (
              <PermissionMap
                receptors={human.receptors}
                actuators={human.actuators}
                tools={human.toolOperations}
              />
            ) : (
              <div className="state-box">載入中…</div>
            )}
            <p className="muted small">
              這張地圖來自目前的啟用狀態、安全規則與同意設定，是全 App 唯一的完整權限總覽。
            </p>
          </Section>
          <Section title="風險分級 — 什麼時候會問你">
            <p className="muted small">
              每一項能力都有固定的分級；分級決定「預設開不開」與「多常問你」。
              分級只是說明，實際強制永遠由系統的安全規則執行。
            </p>
            <ul className="plain-list risk-tier-list">
              {RISK_TIERS.map((t) => (
                <li key={t.label}>
                  <Badge
                    kind={t.tier >= 4 ? "bad" : t.tier === 3 ? "warn" : t.tier === 2 ? "info" : "muted"}
                  >
                    {t.label}
                  </Badge>{" "}
                  <span className="muted small">{t.policy}</span>
                  {t.hardLimits && <div className="muted small">{t.hardLimits}</div>}
                </li>
              ))}
            </ul>
          </Section>
          <SafetyPage refreshKey={refreshKey} advanced={advanced} onNavigate={onNavigate} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 第一層五區（裝置優先）
// ---------------------------------------------------------------------------

const MAX_AREA_ITEMS = 8;

function isAiProviderKind(kind: unknown): boolean {
  return typeof kind === "string" && kind.startsWith("ai-");
}

/** 「停止所有感測」裡「這台電腦」那一行：後端回報什麼就說什麼，缺欄位一律結果不確定。
 *  後端的 `local` 是物件 `{microphone: "stopped"|"idle"}`——`stopped` ＝本來在擷取、
 *  現在停了；`idle` ＝本來就沒有在擷取（兩者都不是「沒停成功」）。
 *  舊 daemon 曾回布林值，這裡一併容忍。 */
export function localStopLine(report: SensorStopReport | null): string {
  if (!report) return "這台電腦：沒有完成，結果不確定（請看狀態列是否還在感測）。";
  const local: unknown = report.local;
  if (local === true) return "這台電腦：已停止本機感測。";
  if (local === false) return "這台電腦：本機感測沒有停止（結果不確定）。";
  if (local && typeof local === "object") {
    const mic = (local as { microphone?: unknown }).microphone;
    if (mic === "stopped") return "這台電腦：已停止本機感測（麥克風）。";
    if (mic === "idle") return "這台電腦：本機本來就沒有在感測。";
    return "這台電腦：本機感測狀態未確認，結果不確定。";
  }
  return "這台電腦：已要求停止；這個版本沒有逐項回報，請看狀態列是否還在感測。";
}

/** 後端逐台回報 → 誠實的一行；只有 stopped 才可以說「已停止」。 */
export function deviceStopLine(report: SensorStopDeviceReport, fallbackName: string): string {
  const name = report.name || fallbackName || "手機";
  const outcome = String(report.outcome ?? "");
  if (outcome === "stopped") return `${name}：已停止（手機回報已停止）。`;
  if (outcome === "unreachable") return `${name}：未送達（手機未連線），感測狀態未變。`;
  return `${name}：已要求停止（以手機回報為準）；手機還沒回報，結果不確定。`;
}

/** 非手機來源（`sources[]`）的一行：只有 stopped／already-stopped 可以說「沒在感測」；
 *  名稱只用 sourceLabel，沒有就說「某個裝置」——sourceId 是內部 id，不丟給一般模式。 */
export function sourceStopLine(report: SensorStopSourceReport): string {
  const name =
    typeof report.sourceLabel === "string" && report.sourceLabel.trim()
      ? report.sourceLabel.trim()
      : "某個裝置";
  const outcome = String(report.outcome ?? "");
  if (outcome === "stopped") return `${name}：已停止（裝置回報已停止）。`;
  if (outcome === "already-stopped") return `${name}：本來就沒有在感測。`;
  if (outcome === "unreachable") return `${name}：未送達（裝置未連線），感測狀態未變。`;
  if (outcome === "refused") return `${name}：裝置拒絕停止，可能仍在感測。`;
  return `${name}：已要求停止；裝置還沒回報，結果不確定。`;
}

function ConnectOverview({
  refreshKey,
  advanced,
  onNavigate,
  onShowAll,
  onSafety,
  focusDeviceId,
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  onShowAll: (tab: HubTab) => void;
  onSafety: () => void;
  /** 深連結指名的裝置：這一層與配對區用的是同一個元件，標示也只有一份真相。 */
  focusDeviceId?: string;
}) {
  const { human, humanError, findCard } = useAppState();
  const { name } = useCharacterName();
  const [providers] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
  const [mobile, reloadMobile] = useAsync(() => api.mobileStatus(), [refreshKey]);
  // 角色同步（AIP Character Session）：手機卡上多一行「連上 ≠ 已同步」的人話。
  // 讀不到就是讀不到（投影會照實說），不用上一次的狀態冒充現在。
  const [characterSession] = useAsync(async () => {
    try {
      return (await api.characterSessionSnapshot()) as unknown;
    } catch {
      return null;
    }
  }, [refreshKey]);
  const [inbox] = useAsync(() => loadDecisionInbox(20), [refreshKey]);
  const [session, reloadSession] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [status] = useAsync(() => api.status(), [refreshKey]);
  const [stopLines, setStopLines] = React.useState<string[] | null>(null);
  const [stopBusy, setStopBusy] = React.useState(false);
  const [revokeMessage, setRevokeMessage] = React.useState<string | null>(null);
  const [revokeBusy, setRevokeBusy] = React.useState(false);

  const seeing = (human?.receptors ?? []).filter((r) => r.availability === "available");
  const responding = (human?.actuators ?? []).filter((a) => a.availability === "available");

  const activeSensors = (status.data?.["activeSensors"] as SensorUse[] | undefined) ?? [];
  const phones = ((mobile.data?.devices as Record<string, unknown>[] | undefined) ?? []).filter(
    (d) => d && typeof d === "object"
  );
  // 一台手機一張卡：能力（可以提供／可以執行）來自能力清單，正在使用中的感測來自
  // status.activeSensors ∪ 手機自報，全部是 Runtime 的真實狀態。
  const phoneCards = phones.map((d) => phoneCardModel(d, human, activeSensors));
  const connectedPhones = phoneCards.filter((p) => p.connected);
  // 手機在來源清單裡也有一列（provider.mobile.<id>）——它已經有自己的卡片，這裡排掉。
  const deviceProviders = (providers.data ?? []).filter((p) => {
    const identity = (p.identity as Record<string, unknown> | undefined) ?? {};
    return (
      identity.id !== COMPANION_PROVIDER_ID &&
      !isAiProviderKind(identity.kind) &&
      !isMobileProviderId(identity.id)
    );
  });

  // 待決定：本頁列出的＋不在這一頁的數量（徽章用的全量 pendingCount 對照）。
  const decisions = decisionPage(inbox.data, MAX_AREA_ITEMS);
  const s = (session.data ?? null) as Session | null;
  const consents = (s?.consents ?? []).filter((c) => !c.revokedAt);
  const estop = status.data?.["emergencyStop"] === true;
  // iOS 上還沒允許（或手機還沒回報）的權限：桌面的同意不能取代，列進「需要確認」。
  const permissionAlerts = phoneCards.flatMap(phonePermissionAlerts);

  /** 停止所有感測：本機＋每一台連線中的手機。逐項照後端回報說話，不做總結式的「已全部停止」。 */
  async function stopAllSensors() {
    setStopBusy(true);
    setStopLines(null);
    const lines: string[] = [];
    let report: SensorStopReport | null = null;
    try {
      report = (await api.sensorsStop()) as SensorStopReport | null;
      lines.push(localStopLine(report));
    } catch (e) {
      lines.push(`這台電腦：沒有完成（${e}），結果不確定。`);
    }
    const reported = report?.devices;
    // 非手機來源（宣告式 Serial／MQTT 裝置等）：一台一句，照 outcome 說。
    for (const src of Array.isArray(report?.sources) ? report.sources : []) {
      lines.push(sourceStopLine(src));
    }
    if (Array.isArray(reported) && reported.length > 0) {
      for (const d of reported) {
        const known = phoneCards.find((p) => p.deviceId === String(d.deviceId ?? ""));
        lines.push(deviceStopLine(d, known?.name ?? "手機"));
      }
    } else {
      // 後端沒有逐台回報：自己對每一台連線中的手機各送一次，一台一句照實說。
      const results = await Promise.allSettled(
        connectedPhones.map((p) => api.mobileSensorsStop(p.deviceId))
      );
      results.forEach((r, i) => {
        const phone = connectedPhones[i];
        lines.push(
          r.status === "fulfilled"
            ? stopSensorsMessage(phone.name, r.value)
            : `${phone.name}：沒有完成（${r.reason}），結果不確定。`
        );
      });
    }
    setStopLines(lines);
    setStopBusy(false);
  }

  /** 授權項目的人話名稱：能力清單裡查不到就照實說，不把原始識別碼丟給一般模式。 */
  function consentName(scope: { kind: string; id: string }): string {
    if (scope.kind === "channel") return "整個通道";
    const list =
      scope.kind === "actuator"
        ? human?.actuators
        : scope.kind === "receptor"
          ? human?.receptors
          : scope.kind === "toolOperation"
            ? human?.toolOperations
            : undefined;
    if (!list) return "（名稱載入中）";
    if (!list.some((c) => c.id === scope.id)) return "（介面沒有這項能力的名稱）";
    return findCard(
      scope.kind === "toolOperation" ? "tool" : (scope.kind as "actuator" | "receptor"),
      scope.id
    ).name;
  }

  /** 撤回全部授權：逐項呼叫，失敗的照實說「授權狀態未變」，不假裝全部成功。 */
  async function revokeAllConsents() {
    setRevokeBusy(true);
    setRevokeMessage(null);
    const targets = consents.map((c) => ({
      name: consentName(c.scope),
      scope: `${c.scope.kind === "toolOperation" ? "tool" : c.scope.kind}:${c.scope.id}`,
    }));
    const results = await Promise.allSettled(targets.map((t) => api.consentRevoke(t.scope)));
    const failed = targets.filter((_, i) => results[i].status === "rejected");
    setRevokeMessage(
      failed.length === 0
        ? "已撤回全部授權。新的動作會立即被阻止；進行中的動作已要求取消（無法取消的會標示「結果未知」）。"
        : `有 ${failed.length} 項沒有撤回成功（${failed
            .map((f) => f.name)
            .join("、")}）：這些授權狀態未變，請重試。`
    );
    setRevokeBusy(false);
    reloadSession();
  }

  return (
    <div className="connect-areas" data-testid="connect-areas">
      {/* 已連接的裝置 —— 裝置優先：先回答「有什麼接上來了」，再談能力。 */}
      <section
        className="connect-area"
        data-testid="connect-area-devices"
        aria-labelledby="connect-devices"
      >
        <h2 id="connect-devices">
          <Icon name="plug" size={16} /> 已連接的裝置
        </h2>
        <p className="muted small">
          接上這台電腦的手機、硬體與角色。連上不等於測過：只有真的測試過的才會標「已測試」。
        </p>
        {mobile.loading && providers.loading ? (
          <div className="state-box">載入中…</div>
        ) : null}
        {phoneCards.length === 0 &&
          deviceProviders.length === 0 &&
          !mobile.loading &&
          !providers.loading && (
            <p className="muted small">
              還沒有連接任何裝置。{mobile.error || providers.error ? "（狀態讀取失敗，稍後再試）" : ""}
            </p>
          )}
        {phoneCards.length > 0 && (
          <div className="provider-list">
            {phoneCards.map((m) => (
              <PhoneDeviceCard
                key={m.deviceId}
                model={m}
                advanced={advanced}
                focused={focusDeviceId === m.deviceId}
                syncLine={characterSyncDeviceLine(characterSession.data ?? null, m.deviceId)}
                onChanged={reloadMobile}
                onManagePermissions={onSafety}
                onRepair={() => onShowAll("providers")}
              />
            ))}
          </div>
        )}
        {deviceProviders.length > 0 && (
          <ul className="connect-area-list">
            {deviceProviders.map((p) => {
              const identity = (p.identity as Record<string, unknown> | undefined) ?? {};
              const id = String(identity.id ?? "");
              const state = projectProviderState(String(p.state ?? ""));
              return (
                <li key={`provider-${id}`}>
                  <Icon name="cpu" size={14} />
                  <span>{String(identity.displayName ?? "裝置")}</span>
                  <Badge kind={state.badge}>{state.label}</Badge>
                </li>
              );
            })}
          </ul>
        )}
        <h3 className="connect-area-subhead">角色</h3>
        <CharacterAdaptersSection refreshKey={refreshKey} advanced={advanced} />
        <div className="connect-area-actions">
          <button onClick={() => onShowAll("providers")}>加入或掃描裝置</button>
        </div>
      </section>

      {/* 系統可以看見什麼 */}
      <section className="connect-area" data-testid="connect-area-see" aria-labelledby="connect-see">
        <h2 id="connect-see">
          <Icon name="scan-eye" size={16} /> 系統可以看見什麼
        </h2>
        <p className="muted small">{name}和 AI 現在能接收的資訊。感測使用中時，這裡、狀態列與角色都會同時顯示。</p>
        <CapabilityList
          cards={seeing}
          loading={!human && !humanError}
          error={humanError}
          empty="目前沒有開啟任何感知來源。"
        />
        <div className="connect-area-actions">
          <button onClick={() => onShowAll("senses")}>管理感知來源</button>
        </div>
      </section>

      {/* 系統可以做什麼 */}
      <section
        className="connect-area"
        data-testid="connect-area-respond"
        aria-labelledby="connect-respond"
      >
        <h2 id="connect-respond">
          <Icon name="send" size={16} /> 系統可以做什麼
        </h2>
        <p className="muted small">{name}和 AI 現在能做的事。標了「使用前會先問你」的，每次用之前都會徵求同意。</p>
        <CapabilityList
          cards={responding}
          loading={!human && !humanError}
          error={humanError}
          empty="目前沒有可用的回應方式。"
          consentBadge
        />
        <div className="connect-area-actions">
          <button onClick={() => onShowAll("responses")}>管理回應方式</button>
          <button onClick={() => onShowAll("toolops")}>工具操作</button>
        </div>
      </section>

      {/* 目前需要確認的權限 */}
      <section className="connect-area" data-testid="connect-area-confirm" aria-labelledby="connect-confirm">
        <h2 id="connect-confirm">
          <Icon name="hand" size={16} /> 目前需要確認的權限
        </h2>
        {permissionAlerts.length > 0 && (
          <ul className="connect-area-list">
            {permissionAlerts.map((line) => (
              <li key={line}>
                <Icon name="triangle-alert" size={14} />
                <span>{line}</span>
                <Badge kind="warn">要在手機上處理</Badge>
              </li>
            ))}
          </ul>
        )}
        {inbox.loading ? (
          <div className="state-box">載入中…</div>
        ) : inbox.error ? (
          <p className="muted small">待決定事項讀取失敗（稍後再試）。</p>
        ) : decisions.shown.length === 0 && decisions.notShown === 0 && decisions.exact ? (
          <p className="muted small">現在沒有需要你決定的事。</p>
        ) : decisions.shown.length === 0 && decisions.notShown === 0 ? (
          // 後端說 pendingCount 只是下限（掃描上限內沒撈完）：本頁看起來乾淨，
          // 但不得宣稱「沒有需要你決定的事」。
          <p className="muted small" role="status">
            {PENDING_INCOMPLETE_NOTE}
            <button onClick={() => onNavigate("activity")}>前往活動紀錄</button>
          </p>
        ) : decisions.shown.length === 0 ? (
          // 誠實：後端說還有待決定，只是最近這一頁沒裝到——不得說「沒有」。
          <p className="muted small" role="status">
            {decisions.exact ? "還有" : "至少還有"} {decisions.notShown} 項待決定不在這一頁，
            <button onClick={() => onNavigate("activity")}>前往活動歷史</button>
          </p>
        ) : (
          <ul className="connect-area-list">
            {decisions.shown.map((item) => {
              const projected = projectInboxStatus(String(item.status ?? ""));
              const route = typeof item.route === "string" && item.route ? item.route : "activity";
              return (
                <li key={String(item.itemId ?? item.title)}>
                  <Icon name="circle-help" size={14} />
                  <span>
                    {inboxItemTitle(item) || inboxKindLabel(String(item.kind ?? ""))}
                    <span className="muted small">　{inboxKindLabel(String(item.kind ?? ""))}</span>
                  </span>
                  <Badge kind={projected.badge}>{projected.label}</Badge>
                  <button onClick={() => onNavigate(route)}>處理</button>
                </li>
              );
            })}
            {decisions.notShown > 0 && (
              <li className="muted small">
                …{decisions.exact ? "還有" : "至少還有"} {decisions.notShown} 項待決定不在這一頁，
                <button onClick={() => onNavigate("activity")}>前往活動歷史</button>
              </li>
            )}
            {decisions.notShown === 0 && !decisions.exact && (
              <li className="muted small" role="status">
                {PENDING_INCOMPLETE_NOTE}
                <button onClick={() => onNavigate("activity")}>前往活動紀錄</button>
              </li>
            )}
          </ul>
        )}
        <h3 className="connect-area-subhead">目前的同意</h3>
        {!s ? (
          <p className="muted small">還沒有工作階段，也就沒有額外授權。需要同意的能力在使用前都會先問你。</p>
        ) : consents.length === 0 ? (
          <p className="muted small">目前沒有額外授權。需要同意的能力在使用前都會先問你。</p>
        ) : (
          <p className="small">
            目前有 {consents.length} 項額外授權：
            {consents
              .slice(0, MAX_AREA_ITEMS)
              .map((c) => consentName(c.scope))
              .join("、")}
            {consents.length > MAX_AREA_ITEMS ? "…" : ""}
          </p>
        )}
        <div className="connect-area-actions">
          <button onClick={onSafety}>查看同意與安全</button>
        </div>
      </section>

      {/* 立即停止與撤銷 —— 一頁之內就找得到「全部停下來」。緊急停止的觸發鍵仍在右上角。 */}
      <section
        className="connect-area"
        data-testid="connect-area-stop"
        aria-labelledby="connect-stop"
      >
        <h2 id="connect-stop">
          <Icon name="octagon-x" size={16} /> 立即停止與撤銷
        </h2>
        {estop ? (
          <p className="home-status-line bad">
            <Icon name="octagon-x" size={16} /> 緊急停止中
            <button className="estop-recover" onClick={onSafety}>
              前往解除
            </button>
          </p>
        ) : (
          <p className="muted small">
            <Icon name="shield-check" size={16} /> 緊急停止未啟動。右上角的紅色按鈕隨時可以立即停止一切
            —— 它不經過任何佇列，也不依賴 AI。
          </p>
        )}
        <div className="row wrap">
          <button disabled={stopBusy} onClick={() => void stopAllSensors()}>
            停止所有感測
          </button>
          <ConfirmButton
            className="danger"
            label="撤回全部授權"
            confirmLabel="確定撤回全部授權？"
            disabled={revokeBusy || consents.length === 0}
            onConfirm={() => {
              void revokeAllConsents();
            }}
          />
          <button onClick={onSafety}>查看同意與安全</button>
        </div>
        <p className="muted small">
          停止感測只會停止正在擷取的來源，不會關閉能力；手機那邊有沒有真的停下來，以手機回報為準。
          撤回全部授權後，需要同意的能力在使用前會重新問你。
        </p>
        {stopLines && (
          <ul className="plain-list small" role="status">
            {stopLines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        )}
        {revokeMessage && (
          <p className="notice-box small" role="status">
            {revokeMessage}
          </p>
        )}
      </section>
    </div>
  );
}

function CapabilityList({
  cards,
  loading,
  error,
  empty,
  consentBadge = false,
}: {
  cards: HumanCard[];
  loading: boolean;
  error?: string;
  empty: string;
  consentBadge?: boolean;
}) {
  if (loading) return <div className="state-box">載入中…</div>;
  if (error) return <div className="state-box state-error">無法載入能力清單：{error}</div>;
  if (cards.length === 0) return <p className="muted small">{empty}</p>;
  const shown = cards.slice(0, MAX_AREA_ITEMS);
  return (
    <ul className="connect-area-list">
      {shown.map((c) => (
        <li key={`${c.kind}-${c.id}`}>
          <Icon name={c.icon} size={14} className={`icon-${c.colorRole}`} />
          <span>
            {c.displayName}
            {c.shortDescription && <span className="muted small">　{c.shortDescription}</span>}
          </span>
          {consentBadge && (c.consent.required === true || c.requiresConsent) && (
            <Badge kind="warn">使用前會先問你</Badge>
          )}
        </li>
      ))}
      {cards.length > shown.length && (
        <li className="muted small">…還有 {cards.length - shown.length} 項</li>
      )}
    </ul>
  );
}
