// 連接與權限（v0.5 IA）：裝置與能力＋同意與安全集中在同一個入口。
// 權限地圖從首頁移到這裡 — 這裡是「AI 可以讀取或操作什麼」唯一的主人。
//
// 一般模式第一層只回答四件事（可以看見／可以回應／使用的裝置／需要你確認），
// 完整的能力卡片、掃描、配對與提供者清單收在第二層「全部能力與裝置」（既有 CapabilitiesHub）。
// 所有狀態標籤走 statusProjection；角色名稱走 useCharacterName；安全文字固定。

import React from "react";
import { api, HumanCard, Session } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Icon } from "../icons";
import { RISK_TIERS } from "../riskTier";
import {
  inboxItemTitle,
  inboxKindLabel,
  projectInboxStatus,
  projectProviderState,
} from "../statusProjection";
import { Badge, Section, useAsync } from "../ui";
import { CapabilitiesHub } from "./CapabilitiesHub";
import type { HubTab } from "./CapabilitiesHub";
import { SafetyPage } from "./SafetyPage";
import { PermissionMap } from "./HomePage";
import { CharacterAdaptersSection } from "./connect/CharacterAdaptersSection";

export type ConnectTab = "devices" | "safety";

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
 *  （徽章用的 pendingCount 是截斷前的全量；本頁可能一筆待決定都沒裝到）。 */
export function decisionPage(
  inbox: Record<string, unknown> | null | undefined,
  shownLimit: number
): { shown: Record<string, unknown>[]; notShown: number; pendingCount: number } {
  const items = ((inbox?.items as Record<string, unknown>[] | undefined) ?? []).filter(
    (item) => item && typeof item === "object" && item.needsDecision === true
  );
  const reported = inbox?.pendingCount;
  const pendingCount =
    typeof reported === "number" && Number.isFinite(reported) && reported >= 0
      ? Math.max(Math.floor(reported), items.length)
      : items.length;
  const shown = items.slice(0, shownLimit);
  return { shown, notShown: Math.max(0, pendingCount - shown.length), pendingCount };
}

/** 桌面角色的呈現層 provider：角色區已經列了，四區摘要不重複。 */
const COMPANION_PROVIDER_ID = "provider.companion.desktop";

export function ConnectPage({
  refreshKey,
  advanced,
  onNavigate,
  initial = "devices",
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  initial?: ConnectTab;
}) {
  const [tab, setTab] = React.useState<ConnectTab>(initial);
  // 相容路由：App 對「work／automations」這類舊 id 都渲染同一個元件，React 會沿用
  // 已掛載的實例，useState(initial) 只在首次掛載生效——route 改變時必須同步分頁，
  // 否則 tray／深連結／全域搜尋／Inbox 切到舊 id 只會高亮導覽、內容不動。
  React.useEffect(() => {
    setTab(initial);
  }, [initial]);
  const { human } = useAppState();
  const [hubTab, setHubTab] = React.useState<HubTab>("senses");
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
          />
          <details className="connect-all" open ref={allRef}>
            <summary>全部能力與裝置</summary>
            <p className="muted small">
              完整清單：每一項能力都可以測試、啟用或停用；裝置可以掃描、配對與測試。
            </p>
            <CapabilitiesHub refreshKey={refreshKey} advanced={advanced} initial={hubTab} />
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
          <SafetyPage refreshKey={refreshKey} onNavigate={onNavigate} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 第一層四區
// ---------------------------------------------------------------------------

const MAX_AREA_ITEMS = 8;

function isAiProviderKind(kind: unknown): boolean {
  return typeof kind === "string" && kind.startsWith("ai-");
}

function ConnectOverview({
  refreshKey,
  advanced,
  onNavigate,
  onShowAll,
  onSafety,
}: {
  refreshKey: number;
  advanced: boolean;
  onNavigate: (tab: string) => void;
  onShowAll: (tab: HubTab) => void;
  onSafety: () => void;
}) {
  const { human, humanError, findCard } = useAppState();
  const { name } = useCharacterName();
  const [providers] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
  const [mobile] = useAsync(() => api.mobileStatus(), [refreshKey]);
  const [inbox] = useAsync(() => loadDecisionInbox(20), [refreshKey]);
  const [session] = useAsync(() => api.sessionGet(), [refreshKey]);
  const [status] = useAsync(() => api.status(), [refreshKey]);

  const seeing = (human?.receptors ?? []).filter((r) => r.availability === "available");
  const responding = (human?.actuators ?? []).filter((a) => a.availability === "available");

  const phones = ((mobile.data?.devices as Record<string, unknown>[] | undefined) ?? []).filter(
    (d) => d && typeof d === "object"
  );
  const deviceProviders = (providers.data ?? []).filter((p) => {
    const identity = (p.identity as Record<string, unknown> | undefined) ?? {};
    return identity.id !== COMPANION_PROVIDER_ID && !isAiProviderKind(identity.kind);
  });

  // 待決定：本頁列出的＋不在這一頁的數量（徽章用的全量 pendingCount 對照）。
  const decisions = decisionPage(inbox.data, MAX_AREA_ITEMS);
  const s = (session.data ?? null) as Session | null;
  const consents = (s?.consents ?? []).filter((c) => !c.revokedAt);
  const estop = status.data?.["emergencyStop"] === true;

  return (
    <div className="connect-areas" data-testid="connect-areas">
      {/* 可以看見 */}
      <section className="connect-area" data-testid="connect-area-see" aria-labelledby="connect-see">
        <h2 id="connect-see">
          <Icon name="scan-eye" size={16} /> 可以看見
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

      {/* 可以回應 */}
      <section
        className="connect-area"
        data-testid="connect-area-respond"
        aria-labelledby="connect-respond"
      >
        <h2 id="connect-respond">
          <Icon name="send" size={16} /> 可以回應
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

      {/* 使用的裝置 */}
      <section
        className="connect-area"
        data-testid="connect-area-devices"
        aria-labelledby="connect-devices"
      >
        <h2 id="connect-devices">
          <Icon name="plug" size={16} /> 使用的裝置
        </h2>
        <p className="muted small">
          接上這台電腦的手機、硬體與角色。連上不等於測過：只有真的測試過的才會標「已測試」。
        </p>
        {mobile.loading && providers.loading ? (
          <div className="state-box">載入中…</div>
        ) : null}
        {phones.length === 0 && deviceProviders.length === 0 && !mobile.loading && !providers.loading && (
          <p className="muted small">
            還沒有連接任何裝置。{mobile.error || providers.error ? "（狀態讀取失敗，稍後再試）" : ""}
          </p>
        )}
        {(phones.length > 0 || deviceProviders.length > 0) && (
          <ul className="connect-area-list">
            {phones.map((d) => (
              <li key={`phone-${String(d.deviceId)}`}>
                <Icon name="wifi" size={14} />
                <span>
                  {String(d.name ?? "iPhone")}
                  <span className="muted small">
                    　iPhone{d.model ? `・${String(d.model)}` : ""}
                  </span>
                </span>
                {d.connected === true ? (
                  <Badge kind="ok">已連線</Badge>
                ) : (
                  <Badge kind="bad">未連線（能力不可用）</Badge>
                )}
              </li>
            ))}
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

      {/* 需要你確認 */}
      <section className="connect-area" data-testid="connect-area-confirm" aria-labelledby="connect-confirm">
        <h2 id="connect-confirm">
          <Icon name="hand" size={16} /> 需要你確認
        </h2>
        {estop && (
          <p className="home-status-line bad">
            <Icon name="octagon-x" size={16} /> 緊急停止中
            <button className="estop-recover" onClick={onSafety}>
              前往解除
            </button>
          </p>
        )}
        {inbox.loading ? (
          <div className="state-box">載入中…</div>
        ) : inbox.error ? (
          <p className="muted small">待決定事項讀取失敗（稍後再試）。</p>
        ) : decisions.shown.length === 0 && decisions.notShown === 0 ? (
          <p className="muted small">現在沒有需要你決定的事。</p>
        ) : decisions.shown.length === 0 ? (
          // 誠實：後端說還有待決定，只是最近這一頁沒裝到——不得說「沒有」。
          <p className="muted small" role="status">
            還有 {decisions.notShown} 項待決定不在這一頁，
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
                …還有 {decisions.notShown} 項待決定不在這一頁，
                <button onClick={() => onNavigate("activity")}>前往活動歷史</button>
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
              .map((c) =>
                c.scope.kind === "actuator"
                  ? findCard("actuator", c.scope.id).name
                  : c.scope.kind === "receptor"
                    ? findCard("receptor", c.scope.id).name
                    : c.scope.kind === "toolOperation"
                      ? findCard("tool", c.scope.id).name
                      : "整個通道"
              )
              .join("、")}
            {consents.length > MAX_AREA_ITEMS ? "…" : ""}
          </p>
        )}
        <div className="connect-area-actions">
          <button onClick={onSafety}>查看同意與安全</button>
        </div>
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
