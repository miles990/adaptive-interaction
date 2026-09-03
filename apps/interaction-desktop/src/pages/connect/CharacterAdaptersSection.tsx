// 角色 adapter（連接與權限 → 使用的裝置）：把「角色是怎麼接上系統的」用人話列出來。
// 資料只來自 /v1/character/instances 與 /v1/character/adapters；介面拿不到的欄位誠實寫
// 「未回報」，「已測試」只在 Runtime 旗標為 true（真的跑過 連上→演出→回報 一整圈）時出現。
// 一般模式不出現 hello／receipt／manifest／UUID 這類字；原始 id 只在進階模式一行帶過。

import React from "react";
import { api, CharacterAdapterView, CharacterInstanceView } from "../../api";
import { capabilitySummary } from "../../character/registry";
import type { CharacterManifest, LocalizedText } from "../../character/protocol";
import { useCharacterName } from "../../characterName";
import { ConfirmButton } from "../../components/Dialog";
import { Icon } from "../../icons";
import { Badge, Section, useAsync } from "../../ui";

/** 桌面角色視窗的固定實例 id（與 companion/gatewayWiring.ts 的 PRIMARY_INSTANCE_ID 一致）。 */
export const PRIMARY_INSTANCE_ID = "desktop-companion";

export type AdapterOrigin = "builtin" | "imported" | "external" | "unknown";
export type AdapterLocation = "local" | "external" | "unknown";
export type Tri = boolean | "unknown";

export interface AdapterRow {
  key: string;
  instanceId: string | null;
  name: string;
  characterId: string;
  origin: AdapterOrigin;
  adapterKind: string | null;
  location: AdapterLocation;
  executable: Tri;
  network: Tri;
  /** 只有 Runtime 旗標為 true 才是 true——連上、協商完成都不算。 */
  tested: boolean;
  connected: boolean;
  lifecycle: string | null;
  role: string | null;
  adapterId: string | null;
  revoked: boolean;
  createdAt: string | null;
  /** Runtime 回報的 input capability id（沒有就 null，畫面誠實寫「未回報」）。 */
  inputCapabilities: string[] | null;
}

export const ORIGIN_LABEL: Record<AdapterOrigin, string> = {
  builtin: "內建",
  imported: "匯入（第三方）",
  external: "外部（第三方）",
  unknown: "來源不確定",
};

export const LOCATION_LABEL: Record<AdapterLocation, string> = {
  local: "本機",
  external: "外部",
  unknown: "位置不確定",
};

const ADAPTER_KIND_DETAIL: Record<string, string> = {
  "in-process": "在這個視窗內執行，沒有獨立程式。",
  web: "本機模組，只有你啟用後才會載入。",
  "external-process": "獨立程式，永遠不會自動啟動，需要你明確安裝與授權。",
  "remote-device": "遠端裝置或另一台機器，永遠不會自動連線，需要配對。",
};

const LIFECYCLE_LABEL: Record<string, string> = {
  discovered: "已發現",
  loading: "載入中",
  validated: "已驗證",
  initializing: "初始化中",
  negotiating: "協商中",
  ready: "就緒",
  shown: "顯示中",
  hidden: "已隱藏",
  suspended: "已暫停",
  resumed: "已恢復",
  reconfiguring: "重新設定中",
  disposed: "已卸載",
  crashed: "已當機（結果不確定，已切回可信文字）",
  reconnecting: "重新連線中",
};

const ROLE_LABEL: Record<string, string> = {
  "primary-companion": "主要角色",
  familiar: "小夥伴",
  worker: "工作角色",
  observer: "只看不動",
  "notification-only": "只收通知",
};

/** LocalizedText → 顯示名：zh-TW → 其他 zh → en → 第一個 → fallback。純字串直接用。 */
export function localizedName(
  text: LocalizedText | string | null | undefined,
  fallback = "角色"
): string {
  if (typeof text === "string") return text.trim() || fallback;
  if (!text || typeof text !== "object") return fallback;
  const entries = Object.entries(text).filter(
    ([, v]) => typeof v === "string" && v.trim().length > 0
  ) as [string, string][];
  if (entries.length === 0) return fallback;
  const pick = (pred: (k: string) => boolean) => entries.find(([k]) => pred(k))?.[1];
  return (
    pick((k) => k === "zh-TW") ??
    pick((k) => k.toLowerCase().startsWith("zh")) ??
    pick((k) => k === "en") ??
    entries[0][1]
  ).trim();
}

export function originOf(raw: unknown): AdapterOrigin {
  return raw === "builtin" || raw === "imported" || raw === "external" ? raw : "unknown";
}

export function locationOf(adapterKind: unknown): AdapterLocation {
  if (adapterKind === "in-process" || adapterKind === "web") return "local";
  if (adapterKind === "external-process" || adapterKind === "remote-device") return "external";
  return "unknown";
}

export function lifecycleLabel(raw: string): string {
  return Object.prototype.hasOwnProperty.call(LIFECYCLE_LABEL, raw)
    ? LIFECYCLE_LABEL[raw]
    : "狀態不確定";
}

export function roleLabel(raw: string): string {
  return Object.prototype.hasOwnProperty.call(ROLE_LABEL, raw) ? ROLE_LABEL[raw] : "角色";
}

function rank(row: AdapterRow): number {
  if (row.instanceId === PRIMARY_INSTANCE_ID) return 0;
  if (row.instanceId) return 1;
  return 2;
}

/**
 * 實例清單＋adapter 登記合併成一列一列。純函式，方便測試。
 * 有實例的 adapter 用實例的旗標；只登記、沒連線的 adapter 沒有 manifest 資料，
 * 可執行／網路一律「未回報」，不猜。
 */
export function adapterRows(
  instances: CharacterInstanceView[],
  adapters: CharacterAdapterView[]
): AdapterRow[] {
  const byAdapter = new Map(adapters.map((a) => [a.adapterId, a]));
  const seen = new Set<string>();
  const rows: AdapterRow[] = instances.map((inst) => {
    const record = inst.adapterId ? byAdapter.get(inst.adapterId) : undefined;
    if (inst.adapterId) seen.add(inst.adapterId);
    return {
      key: `instance:${inst.instanceId}`,
      instanceId: inst.instanceId,
      name: localizedName(inst.displayName, record?.displayName || "角色"),
      characterId: String(inst.characterId ?? ""),
      origin: originOf(inst.origin),
      adapterKind: typeof inst.adapterKind === "string" ? inst.adapterKind : null,
      location: locationOf(inst.adapterKind),
      executable: inst.executable === true,
      network: inst.network === true,
      tested: inst.tested === true,
      connected: inst.connected === true,
      lifecycle: typeof inst.lifecycle === "string" ? inst.lifecycle : null,
      role: typeof inst.role === "string" ? inst.role : null,
      adapterId: inst.adapterId ?? null,
      revoked: record?.revoked === true,
      createdAt: record?.createdAt ?? null,
      inputCapabilities: inst.inputCapabilities ?? record?.inputCapabilities ?? null,
    };
  });
  for (const a of adapters) {
    if (seen.has(a.adapterId)) continue;
    rows.push({
      key: `adapter:${a.adapterId}`,
      instanceId: null,
      name: localizedName(a.displayName, "角色"),
      characterId: String(a.characterId ?? ""),
      origin: "external",
      adapterKind: typeof a.adapterKind === "string" ? a.adapterKind : null,
      location: locationOf(a.adapterKind ?? "external-process"),
      executable: typeof a.executable === "boolean" ? a.executable : "unknown",
      network: typeof a.network === "boolean" ? a.network : "unknown",
      tested: false,
      connected: a.connected === true,
      lifecycle: null,
      role: null,
      adapterId: a.adapterId,
      revoked: a.revoked === true,
      createdAt: a.createdAt ?? null,
      inputCapabilities: a.inputCapabilities ?? null,
    });
  }
  return rows.sort((x, y) => Number(x.revoked) - Number(y.revoked) || rank(x) - rank(y));
}

function triText(v: Tri, yes: string, no: string): string {
  if (v === "unknown") return "未回報（還沒連上，沒有資料）";
  return v ? yes : no;
}

const INPUT_CAPABILITY_LABEL: Record<string, string> = {
  "input.click": "點擊",
  "input.hover": "滑鼠靠近",
  "input.drag": "拖曳",
  "input.drop": "放下",
  "input.pointerProximity": "游標接近",
  "input.text": "文字",
  "input.fileDrop": "檔案拖放（只有檔名與大小）",
};

/** Runtime 回報的 input capability id → 「可以接收：…」；沒有清單回 null（不猜）。 */
export function receiveLineFromInputs(ids: string[] | null | undefined): string | null {
  if (!Array.isArray(ids)) return null;
  const labels = ids
    .filter((id): id is string => typeof id === "string")
    .map((id) => INPUT_CAPABILITY_LABEL[id] ?? (id.startsWith("input.") ? id.slice("input.".length) : id));
  if (labels.length === 0) return "可以接收：不接收任何互動（只演出）";
  return `可以接收：${labels.join("、")}`;
}

/** 「可以接收：…」那一行，直接用角色模組的共用摘要（同一份文案）。拿不到就回 null。 */
export function receiveLine(manifest: CharacterManifest | null | undefined): string | null {
  if (!manifest) return null;
  try {
    return capabilitySummary(manifest, "zh-TW").find((line) => line.startsWith("可以接收")) ?? null;
  } catch {
    return null;
  }
}

export function CharacterAdaptersSection({
  refreshKey,
  advanced = false,
  standalone = false,
}: {
  refreshKey: number;
  advanced?: boolean;
  /** true＝自己包一個 Section（裝置與提供者分頁）；false＝嵌在四區的「使用的裝置」裡。 */
  standalone?: boolean;
}) {
  const { name } = useCharacterName();
  const [instances, reloadInstances] = useAsync(() => api.characterInstances(), [refreshKey]);
  const [adapters, reloadAdapters] = useAsync(() => api.characterAdapters(), [refreshKey]);
  // 桌面角色還沒連線時後端回 404／Err：這是正常狀態，不是錯誤。
  const [manifest] = useAsync(
    () => api.characterManifest().catch(() => null as CharacterManifest | null),
    [refreshKey]
  );
  const [message, setMessage] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState<string | null>(null);

  const rows = React.useMemo(
    () => adapterRows(instances.data?.instances ?? [], adapters.data?.adapters ?? []),
    [instances.data, adapters.data]
  );
  const primaryReceive = React.useMemo(() => receiveLine(manifest.data ?? null), [manifest.data]);

  async function revoke(adapterId: string) {
    setBusy(adapterId);
    setMessage(null);
    try {
      const result = await api.characterAdapterRevoke(adapterId);
      setMessage(
        result.disconnected
          ? "已撤銷並斷線。這個 adapter 的鑰匙已失效，不會自己回來；要再用必須重新登記。"
          : "已撤銷。這個 adapter 的鑰匙已失效（當時沒有連線）；要再用必須重新登記。"
      );
    } catch (e) {
      setMessage(`撤銷失敗：${e}。登記狀態未變，請重試。`);
    } finally {
      setBusy(null);
      reloadInstances();
      reloadAdapters();
    }
  }

  const loading = instances.loading || adapters.loading;
  const error = instances.error ?? adapters.error;

  const body = (
    <div className="connect-adapters">
      <p className="muted small">
        這裡列出{name}與其他角色是怎麼接上系統的。第三方角色只會收到「該演什麼」，看不到你的對話內容，
        也改不了任何安全設定；安全提示（緊急停止中、被阻擋、結果不確定、感測使用中）永遠由系統顯示，角色不能改寫。
      </p>
      {loading && rows.length === 0 && <div className="state-box">載入中…</div>}
      {error && rows.length === 0 && (
        <div className="state-box state-error">無法讀取角色連線狀態：{error}</div>
      )}
      {!loading && !error && rows.length === 0 && (
        <p className="muted small">目前沒有任何角色接上系統（桌面角色視窗還沒連線）。</p>
      )}
      {rows.length > 0 && (
        <div className="provider-list">
          {rows.map((row) => {
            const receive =
              row.instanceId === PRIMARY_INSTANCE_ID && primaryReceive
                ? primaryReceive
                : receiveLineFromInputs(row.inputCapabilities) ?? "可以接收：介面沒有拿到清單（未回報）";
            return (
              <div
                className="provider-card connect-adapter-row"
                key={row.key}
                data-testid={`adapter-row-${row.key}`}
              >
                <div className="row space-between wrap">
                  <strong>
                    <Icon name="cat" size={16} /> {row.name}
                  </strong>
                  <span className="row wrap">
                    {row.revoked ? (
                      <Badge kind="bad">已撤銷</Badge>
                    ) : row.connected ? (
                      <Badge kind="ok">已連線</Badge>
                    ) : (
                      <Badge kind="muted">未連線</Badge>
                    )}
                    {row.tested ? (
                      <Badge kind="ok">已測試</Badge>
                    ) : (
                      <Badge kind="pending">未測試</Badge>
                    )}
                  </span>
                </div>
                <div className="connect-adapter-flags">
                  <Badge kind={row.origin === "builtin" ? "muted" : "warn"}>
                    {ORIGIN_LABEL[row.origin]}
                  </Badge>
                  <Badge kind={row.location === "local" ? "muted" : "warn"}>
                    {LOCATION_LABEL[row.location]}
                  </Badge>
                  {row.role && <Badge kind="info">{roleLabel(row.role)}</Badge>}
                </div>
                {row.adapterKind && ADAPTER_KIND_DETAIL[row.adapterKind] && (
                  <div className="muted small">{ADAPTER_KIND_DETAIL[row.adapterKind]}</div>
                )}
                <div className="small">
                  有可執行程式：{triText(row.executable, "是（只記錄，不會自動執行）", "否（純資料）")}
                </div>
                <div className="small">需要網路：{triText(row.network, "是", "否")}</div>
                <div className="small">{receive}</div>
                <div className="muted small">
                  {row.tested
                    ? "已測試：真的跑過一次完整回合（連上→演出→回報結果）。"
                    : "未測試：還沒跑過完整回合——連上或協商完成都不等於測過。"}
                </div>
                {row.lifecycle && (
                  <div className="muted small">目前狀態：{lifecycleLabel(row.lifecycle)}</div>
                )}
                {row.createdAt && (
                  <div className="muted small">
                    登記於 {new Date(row.createdAt).toLocaleString("zh-TW")}
                  </div>
                )}
                {advanced && (
                  <div className="muted small">
                    原始：instance {row.instanceId ?? "—"}・adapter {row.adapterId ?? "—"}・
                    {row.characterId || "—"}・{row.adapterKind ?? "—"}・{row.lifecycle ?? "—"}
                  </div>
                )}
                {row.adapterId && !row.revoked && (
                  <div className="row wrap">
                    <ConfirmButton
                      className="danger"
                      label="撤銷"
                      confirmLabel="確定撤銷？（立即斷線，不會自己回來）"
                      disabled={busy === row.adapterId}
                      onConfirm={() => {
                        void revoke(row.adapterId!);
                      }}
                    />
                    <span className="muted small">
                      撤銷後這個角色的鑰匙立即失效；要再用必須重新登記。
                    </span>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      {message && (
        <p className="notice-box" role="status">
          {message}
        </p>
      )}
    </div>
  );

  return standalone ? <Section title="角色如何接上系統">{body}</Section> : body;
}
