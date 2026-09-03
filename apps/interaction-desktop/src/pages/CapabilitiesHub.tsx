// 能力與裝置（spec §16-1.E）：內建能力（感知／回應／工具）＋裝置與來源。
// 掃描文案誠實：只宣稱「已偵測到目前可用」，不宣稱找到所有硬體。
// 一般模式不外洩技術識別（線路名稱、埠號、識別碼、原始能力 id）——那些只在進階模式出現。

import React from "react";
import { api, HardwareScanReport, ProviderTested, SensorUse } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Badge, Section, StateView, useAsync } from "../ui";
import { CapabilitiesPage } from "./CapabilitiesPage";
import { CharacterAdaptersSection } from "./connect/CharacterAdaptersSection";
import {
  isMobileProviderId,
  phoneCardModel,
  PhoneDeviceCard,
} from "./connect/PhoneDeviceCard";

export type HubTab = "senses" | "responses" | "toolops" | "providers";

/** 桌面角色的呈現層 provider id（Runtime character.rs COMPANION_PROVIDER_ID）。 */
const COMPANION_PROVIDER_ID = "provider.companion.desktop";

export function CapabilitiesHub({
  refreshKey,
  advanced,
  initial = "senses",
}: {
  refreshKey: number;
  advanced: boolean;
  /** 上層（連接與權限四區的「管理…」按鈕）指定分類；改變時同步切換。 */
  initial?: HubTab;
}) {
  const [tab, setTab] = React.useState<HubTab>(initial);
  React.useEffect(() => {
    setTab(initial);
  }, [initial]);
  return (
    <div>
      <div className="hub-tabs" role="tablist" aria-label="能力分類">
        {(
          [
            ["senses", "感知來源"],
            ["responses", "回應方式"],
            ["toolops", "工具操作"],
            ["providers", "裝置與來源"],
          ] as [HubTab, string][]
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
      {tab === "senses" && <CapabilitiesPage kind="receptor" advanced={advanced} />}
      {tab === "responses" && <CapabilitiesPage kind="actuator" advanced={advanced} />}
      {tab === "toolops" && <CapabilitiesPage kind="tool-operation" advanced={advanced} />}
      {tab === "providers" && <ProvidersSection refreshKey={refreshKey} advanced={advanced} />}
    </div>
  );
}

const PROVIDER_STATE_LABEL: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  available: { text: "可用", kind: "ok" },
  busy: { text: "忙碌中", kind: "ok" },
  degraded: { text: "部分可用", kind: "warn" },
  discovered: { text: "已發現（未配對）", kind: "pending" },
  unpaired: { text: "未配對", kind: "pending" },
  paired: { text: "已配對（未安裝）", kind: "pending" },
  installed: { text: "已安裝（未啟用）", kind: "pending" },
  disabled: { text: "已停用", kind: "warn" },
  disconnected: { text: "未連線", kind: "bad" },
  expired: { text: "已過期", kind: "bad" },
  revoked: { text: "已撤銷", kind: "bad" },
  closed: { text: "已關閉", kind: "bad" },
};

const HARDWARE_CLASS_LABEL: Record<string, string> = {
  camera: "攝影機與影像來源",
  microphone: "麥克風",
  "audio-input": "音訊輸入",
  "audio-output": "音訊輸出／喇叭",
  keyboard: "鍵盤",
  mouse: "滑鼠",
  touchpad: "觸控板",
  tablet: "手寫板",
  "game-controller": "遊戲控制器",
  midi: "MIDI",
  "usb-serial": "USB 接線裝置（用線接上的自製硬體，例如 ESP32）",
  "bluetooth-le": "低功耗藍牙裝置（不用接線的小型感測器與燈具）",
  display: "螢幕呈現",
  "system-notification": "系統通知",
  "os-sensor": "作業系統感測器",
  "mdns-device": "同一個 Wi-Fi 裡自動找到的裝置（裝置自己報名字，不用輸入位址）",
  "esp32-declaration": "ESP32 自製裝置（用設定檔描述它能感測什麼、能做什麼）",
};

/** 進階模式才附上的技術名稱（一般模式只說人話）。 */
const HARDWARE_CLASS_TECHNICAL: Record<string, string> = {
  "usb-serial": "USB Serial",
  "bluetooth-le": "Bluetooth LE",
  "mdns-device": "mDNS",
};

/** 來源類型的人話（Runtime ProviderKind，kebab-case）。 */
const PROVIDER_KIND_LABEL: Record<string, string> = {
  local: "內建本機能力",
  device: "外接裝置",
  service: "外部服務",
  application: "本機的其他程式",
  "ai-provider": "AI 服務",
  "ai-agent": "AI 幫手",
  "ai-session": "AI 工作階段",
};

/** 信任程度的人話（Runtime TrustLevel）。 */
const TRUST_LABEL: Record<string, string> = {
  untrusted: "未信任",
  discovered: "只發現（身分未驗證）",
  paired: "已配對",
  verified: "每次連線都驗證身分",
  builtin: "內建",
};

function providerKindLabel(raw: unknown): string {
  const key = String(raw ?? "");
  return Object.prototype.hasOwnProperty.call(PROVIDER_KIND_LABEL, key)
    ? PROVIDER_KIND_LABEL[key]
    : "來源類型不確定";
}

function trustLabel(raw: unknown): string {
  const key = String(raw ?? "");
  return Object.prototype.hasOwnProperty.call(TRUST_LABEL, key) ? TRUST_LABEL[key] : "不確定";
}

const HARDWARE_AVAILABILITY: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  available: { text: "目前可見", kind: "ok" },
  "permission-required": { text: "需要權限", kind: "pending" },
  busy: { text: "被占用", kind: "warn" },
  unavailable: { text: "目前不可用", kind: "bad" },
  unsupported: { text: "此平台尚未支援", kind: "warn" },
  unknown: { text: "結果不確定", kind: "pending" },
};

/** 已停用／不可用狀態的人話說明（角色名稱由呼叫端帶入，不寫死）。 */
function providerStoppedHint(state: string, characterName: string): string | undefined {
  const table: Record<string, string> = {
    disabled: `已停用——連線已關閉，${characterName}不會用它做任何事。`,
    disconnected: "連不上裝置：拔線、被占用、位址不對或裝置沒開。",
    expired: "授權已過期——需要重新配對／啟用。",
    revoked: "已撤銷——能力立即停用，且不會自己回來。",
    closed: "已關閉。",
  };
  return Object.prototype.hasOwnProperty.call(table, state) ? table[state] : undefined;
}

/** 證據來源的人話。 */
const TESTED_HOW_LABEL: Record<string, string> = {
  handshake: "裝置連線握手（裝置報上身分並完成配對）",
  capability: "能力實際運作（讀到資料或裝置回 ack）",
  human: "人為測試",
};

export type ProviderStage =
  | "discovered"
  | "unpaired"
  | "paired"
  | "installed"
  | "connected"
  | "tested"
  | "enabled"
  | "stopped";

export interface ProviderProgress {
  stage: ProviderStage;
  label: string;
  kind: "ok" | "warn" | "bad" | "pending";
  hint: string;
}

/**
 * `detail` 可能是純文字註記，也可能是帶「已測試」證據的 JSON 物件。
 * 兩種都要看得懂，且看不懂時一律當純文字（不臆造證據）。
 */
export function parseProviderDetail(detail: unknown): { note?: string; tested?: ProviderTested } {
  if (typeof detail !== "string" || !detail.trim()) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(detail);
  } catch {
    return { note: detail };
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return { note: detail };
  const obj = parsed as Record<string, unknown>;
  const note = typeof obj.note === "string" ? obj.note : undefined;
  const raw = obj.tested;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return { note: note ?? detail };
  const t = raw as Record<string, unknown>;
  if (typeof t.ok !== "boolean") return { note: note ?? detail };
  const tested: ProviderTested = {
    at: String(t.at ?? ""),
    how: String(t.how ?? ""),
    ok: t.ok,
    note: typeof t.note === "string" ? t.note : undefined,
  };
  // 只在 Runtime 真的標了旗標時才帶上：缺席的舊記錄維持原本的形狀，
  // 不憑空長出「未驗證」（也不讓既有比較／快照被這個鍵改變）。
  if (t.pairingUnverified === true) tested.pairingUnverified = true;
  return { note, tested };
}

/**
 * 四階誠實階梯（spec §9.3）：只發現 → 已配對 → 已測試 → 已啟用。
 * 掃描到 metadata 不等於連線完成；狀態變成 available 也不等於測過。
 * 純函式，方便回歸測試。
 */
export function providerProgress(
  input: {
    state: string;
    tested?: ProviderTested;
    enabledCapabilities: number;
  },
  characterName = "角色"
): ProviderProgress {
  const { state, tested } = input;
  const enabled = input.enabledCapabilities > 0;
  const failedReason = tested?.note ?? "原因未知";
  // 「裝置宣稱不需配對」不等於「配對已驗證」：Runtime 標了這個旗標時，
  // 這次的身分證據只有裝置自報的 deviceId，不得與真配對同樣顯示成綠燈。
  const pairingUnverified = tested?.pairingUnverified === true;
  const pairingHint =
    "配對碼未經比對（裝置說它不需要配對），身分證據只有裝置自報的 deviceId" +
    "——只能確定「有一台自稱是它的裝置回應了」。";
  switch (state) {
    case "discovered":
      return {
        stage: "discovered",
        label: "只發現",
        kind: "pending",
        hint: `只是掃描時看見（名稱、型號這類 metadata），還沒配對，也還沒連上——${characterName}還不能用它做任何事。`,
      };
    case "unpaired":
      return {
        stage: "unpaired",
        label: "只發現（未配對）",
        kind: "pending",
        hint: "尚未配對——需要先完成配對與身分驗證，之後才談得上連線與測試。",
      };
    case "paired":
      return {
        stage: "paired",
        label: "已配對",
        kind: "pending",
        hint: "已完成配對，但能力還沒安裝進系統，也還沒測試過。",
      };
    case "installed":
      return {
        stage: "installed",
        label: "已安裝設定，尚未連線",
        kind: "pending",
        hint: "設定檔已存在，但還沒連上裝置，也還沒測試過——設定檔存在不等於連線完成。",
      };
    case "available":
    case "busy":
    case "degraded": {
      if (enabled && tested?.ok === true) {
        if (pairingUnverified) {
          return {
            stage: "enabled",
            label: "已啟用（配對碼未驗證）",
            kind: "warn",
            hint: `有 ${input.enabledCapabilities} 項能力真的開著，測試也通過了，但${pairingHint}`,
          };
        }
        return {
          stage: "enabled",
          label: "已啟用",
          kind: "ok",
          hint: `已測試通過，而且有 ${input.enabledCapabilities} 項能力真的開著——${characterName}此刻真的能用它。`,
        };
      }
      if (enabled && tested?.ok === false) {
        return {
          stage: "enabled",
          label: "已啟用（上次測試沒過）",
          kind: "warn",
          hint: `有 ${input.enabledCapabilities} 項能力開著，但最近一次測試沒過：${failedReason}`,
        };
      }
      if (enabled) {
        return {
          stage: "enabled",
          label: "已啟用（尚未測試）",
          kind: "warn",
          hint: `有 ${input.enabledCapabilities} 項能力開著，但從來沒測試過——沒測過就無法確定它真的讀得到或做得到。`,
        };
      }
      if (tested?.ok === true) {
        if (pairingUnverified) {
          return {
            stage: "tested",
            label: "已測試（配對碼未驗證）",
            kind: "warn",
            hint: `測試通過（真的有回應），但${pairingHint}`,
          };
        }
        return {
          stage: "tested",
          label: "已測試",
          kind: "ok",
          hint: `測試通過（真的讀到資料），但能力還沒啟用——啟用後${characterName}才會真的用它。`,
        };
      }
      if (tested?.ok === false) {
        return {
          stage: "connected",
          label: "已連線，測試沒過",
          kind: "warn",
          hint: `最近一次測試沒過：${failedReason}`,
        };
      }
      return {
        stage: "connected",
        label: "已連線，尚未測試",
        kind: "pending",
        hint: "連線設定已就緒，但還沒測試過——按「測試裝置」做一次唯讀測試，確認真的讀得到。",
      };
    }
    default: {
      // 介面不認得的狀態：不把原始字串當標籤（一般模式不外洩 enum）。
      const base = PROVIDER_STATE_LABEL[state] ?? { text: "狀態不確定", kind: "pending" as const };
      return {
        stage: "stopped",
        label: base.text,
        kind: base.kind,
        hint: providerStoppedHint(state, characterName) ?? "目前不能用。",
      };
    }
  }
}

/** 「最近測試」那一行人話（沒測過就說沒測過，不留白）。 */
export function testedSummary(tested?: ProviderTested): string {
  if (!tested) return "還沒測試過——測過才算數，掃描到或設定好都不算。";
  const at = tested.at ? new Date(tested.at).toLocaleString("zh-TW") : "時間未知";
  const how = TESTED_HOW_LABEL[tested.how] ?? tested.how;
  const verdict = tested.ok ? "成功" : "失敗";
  return `最近測試：${at}・${verdict}（${how}）${tested.note ? `——${tested.note}` : ""}`;
}

function ProvidersSection({
  refreshKey,
  advanced = false,
}: {
  refreshKey: number;
  advanced?: boolean;
}) {
  const { findCard, human } = useAppState();
  const { name: characterName } = useCharacterName();
  const [providers, reloadProviders] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
  const [testing, setTesting] = React.useState<string | null>(null);
  const [testResult, setTestResult] = React.useState<Record<string, string>>({});
  /** 這個能力此刻真的開著嗎（以能力清單的可用性為準，不用 provider 狀態猜）。 */
  const capabilityAvailable = React.useCallback(
    (kind: "receptor" | "actuator", id: string) => {
      const list = kind === "receptor" ? human?.receptors : human?.actuators;
      return list?.find((c) => c.id === id)?.availability === "available";
    },
    [human]
  );

  async function testProvider(id: string) {
    setTesting(id);
    try {
      const report = await api.providerTest(id);
      setTestResult((prev) => ({
        ...prev,
        [id]: report.ok
          ? `測試成功：讀到了「${
              report.receptorId ? findCard("receptor", report.receptorId).name : "感知來源"
            }」的資料。`
          : `測試沒過：${report.reason ?? "原因未知"}`,
      }));
      reloadProviders();
    } catch (e) {
      setTestResult((prev) => ({ ...prev, [id]: `測試失敗：${e}` }));
    } finally {
      setTesting(null);
    }
  }
  const [scanning, setScanning] = React.useState(false);
  const [scanNote, setScanNote] = React.useState<string | null>(null);
  const [hardware, setHardware] = React.useState<HardwareScanReport | null>(null);

  async function scan() {
    setScanning(true);
    try {
      const [report] = await Promise.all([api.hardwareScan(), api.agentsRefresh()]);
      setHardware(report);
      setScanNote(
        `已偵測到目前可用裝置與能力，共 ${report.devices.length} 筆結果；感測器啟動：${
          report.sensorActivationAttempted ? "曾嘗試（異常）" : "否"
        }。偵測不到不代表不存在：驅動、權限、沙盒、未配對或裝置被占用都可能讓設備看不見。`
      );
    } catch (e) {
      setScanNote(`掃描失敗：${e}`);
    } finally {
      setScanning(false);
    }
  }

  return (
    <div>
      <Section title="掃描目前可用的互動能力">
        <p className="muted small">
          掃描只讀取名稱、類型、識別與可用狀態，<strong>不會</strong>開啟攝影機、麥克風或任何感測。
          結果代表「已偵測到目前可用」，不代表找到了所有硬體。
        </p>
        <button onClick={scan} disabled={scanning}>
          {scanning ? "掃描中…" : "重新掃描"}
        </button>
        {scanNote && <p className="muted small">{scanNote}</p>}
        {hardware && (
          <div className="provider-list" style={{ marginTop: 12 }} aria-live="polite">
            {hardware.devices.map((device, index) => {
              const availability = HARDWARE_AVAILABILITY[device.availability] ?? {
                text: "結果不確定",
                kind: "pending" as const,
              };
              return (
                <div className="provider-card" key={`${device.class}-${device.stableId ?? index}`}>
                  <div className="row space-between">
                    <strong>
                      {HARDWARE_CLASS_LABEL[device.class] ?? device.displayName}
                      {advanced && HARDWARE_CLASS_TECHNICAL[device.class]
                        ? `（${HARDWARE_CLASS_TECHNICAL[device.class]}）`
                        : ""}
                    </strong>
                    <Badge kind={availability.kind}>{availability.text}</Badge>
                  </div>
                  <div>{device.displayName}</div>
                  <div className="muted small">{device.detail}</div>
                  {advanced && (
                    <div className="muted small">識別依據：{device.identityBasis}</div>
                  )}
                  {device.stableId ? (
                    <div className="muted small">
                      {advanced
                        ? `穩定識別：${device.stableId}`
                        : "有可以安全保存的穩定識別（可以配對或安裝）。"}
                    </div>
                  ) : (
                    <div className="muted small">沒有可安全保存的穩定識別，不會直接配對或安裝。</div>
                  )}
                  {device.permissionRequirements.map((requirement) => (
                    <div className="muted small" key={requirement}>使用前：{requirement}</div>
                  ))}
                  {device.capabilities.length > 0 && (
                    <ul className="plain-list small">
                      {device.capabilities.map((capability) => (
                        <li key={capability.id}>
                          {/* 一般模式只說這項能力會做什麼；原始識別留給進階模式。 */}
                          <strong>
                            {advanced
                              ? capability.id
                              : capability.kind === "receptor"
                                ? "可以感知"
                                : capability.kind === "actuator"
                                  ? "可以執行"
                                  : "能力"}
                          </strong>{" "}
                          — {capability.scope}
                          {capability.requiresConsent ? "（必須先取得使用授權）" : ""}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              );
            })}
            {hardware.limitations.map((limitation) => (
              <div className="state-box state-warning" key={limitation}>{limitation}</div>
            ))}
          </div>
        )}
      </Section>
      <MobileSection refreshKey={refreshKey} advanced={advanced} />
      <CharacterAdaptersSection refreshKey={refreshKey} advanced={advanced} standalone />
      <Section title="已連接的裝置與來源">
        <StateView state={providers} empty="還沒有任何裝置或來源。">
          {(list) => (
            <div className="provider-list">
              {list.map((p) => {
                const identity = p.identity as Record<string, unknown>;
                const state = String(p.state ?? "");
                const id = String(identity?.id ?? "");
                // 手機已經有自己的卡片（上方 iPhone 區），這裡不再重複列一次。
                if (isMobileProviderId(id)) return null;
                const receptorIds = (p.receptors as string[] | undefined) ?? [];
                const actuatorIds = (p.actuators as string[] | undefined) ?? [];
                const { note, tested } = parseProviderDetail(p.detail);
                const enabledCapabilities =
                  receptorIds.filter((rid) => capabilityAvailable("receptor", rid)).length +
                  actuatorIds.filter((aid) => capabilityAvailable("actuator", aid)).length;
                const progress = providerProgress(
                  { state, tested, enabledCapabilities },
                  characterName
                );
                const stateLabel = PROVIDER_STATE_LABEL[state] ?? {
                  text: "狀態不確定",
                  kind: "pending" as const,
                };
                return (
                  <div className="provider-card" key={id}>
                    <div className="row space-between">
                      <strong>{String(identity?.displayName ?? id)}</strong>
                      <Badge kind={progress.kind}>{progress.label}</Badge>
                    </div>
                    <div className="muted small">
                      {providerKindLabel(identity?.kind)}・信任：{trustLabel(identity?.trustLevel)}
                      ・狀態：{stateLabel.text}
                      {note ? `・${note}` : ""}
                    </div>
                    {id === COMPANION_PROVIDER_ID && (
                      <div className="muted small">
                        這是桌面角色的呈現層：只負責演出，不持有任何權限。
                      </div>
                    )}
                    {advanced && (
                      <div className="muted small">
                        原始：{id}・{String(identity?.kind ?? "")}・{String(identity?.trustLevel ?? "")}・
                        {state}
                      </div>
                    )}
                    <div className="muted small">{progress.hint}</div>
                    <div className="muted small">{testedSummary(tested)}</div>
                    <div className="row wrap">
                      <button onClick={() => testProvider(id)} disabled={testing === id}>
                        {testing === id ? "測試中…" : "測試裝置"}
                      </button>
                      <span className="muted small">
                        只讀一次目前開著的感知來源，不會觸發任何動作，也不會替你打開被停用的感測器。
                      </span>
                    </div>
                    {testResult[id] && (
                      <div className="muted small" role="status">
                        {testResult[id]}
                      </div>
                    )}
                    {/* 裝置導向（spec §9.3）：以「<角色>可以知道／可以做」呈現，
                        不先丟 receptor/actuator 技術詞；角色名稱來自 useCharacterName。 */}
                    {receptorIds.length > 0 && (
                      <div className="small">
                        {characterName}可以知道：
                        {receptorIds
                          .slice(0, 6)
                          .map((rid) => findCard("receptor", rid).name)
                          .join("、")}
                        {receptorIds.length > 6 ? "…" : ""}
                      </div>
                    )}
                    {actuatorIds.length > 0 && (
                      <div className="small">
                        {characterName}可以做：
                        {actuatorIds
                          .slice(0, 6)
                          .map((aid) => findCard("actuator", aid).name)
                          .join("、")}
                        {actuatorIds.length > 6 ? "…" : ""}
                      </div>
                    )}
                    <div className="muted small">
                      能力：感知來源 {receptorIds.length}・回應方式 {actuatorIds.length}・工具操作{" "}
                      {(p.toolOperations as unknown[] | undefined)?.length ?? 0}・此刻真的開著{" "}
                      {enabledCapabilities} 項
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </StateView>
      </Section>
      {!hardware && (
        <Section title="尚未掃描">
          <p className="muted small">
            按下「重新掃描」後，這裡會逐類顯示目前可見、需要權限、未知、不可用或此平台尚未支援；不以灰色按鈕或假裝置代替結果。
          </p>
        </Section>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// iPhone（v0.5 Phase 6）：配對、狀態、撤銷。
// 誠實：配對碼 5 分鐘一段；斷線＝能力不可用；感測狀態由手機自報。
// 每一台手機的卡片與第一層共用同一個元件（PhoneDeviceCard），只有一份真相。
// ---------------------------------------------------------------------------

/** 配對載荷（QR 內容＝iOS「手動貼上」欄位吃的同一份）裡的主機位址。 */
export function pairingHostPort(payload: unknown): string | null {
  if (typeof payload !== "string" || payload.length === 0) return null;
  try {
    const parsed = JSON.parse(payload) as Record<string, unknown>;
    const host = typeof parsed.host === "string" ? parsed.host : null;
    if (!host) return null;
    const port = typeof parsed.port === "number" ? parsed.port : null;
    return port ? `${host}:${port}` : host;
  } catch {
    return null;
  }
}

/** 這一段配對期為什麼不能用了（`null`＝還有效）。 */
export function pairingInvalidReason(
  expiresAt: unknown,
  live: { active: boolean; burnedAt: string | null } | null,
  now: number
): string | null {
  // runtime 在每段配對期開始時把 pairingBurnedAt 歸零，所以這裡看到值就是
  // 「這一段被燒掉了」——區網上任何未認證 peer 送一則錯的回應就會發生。
  if (live?.burnedAt) {
    return "這段配對期已經作廢：有別的裝置試過配對（配對碼一次只能用一次）。請重新開始配對。";
  }
  const deadline = Date.parse(String(expiresAt ?? ""));
  if (Number.isFinite(deadline) && deadline <= now) {
    return "這段配對期已經過期。請重新開始配對。";
  }
  // 已經去問過 runtime，而它說沒有配對期在進行中（被用掉或被清掉）。
  if (live && !live.active) {
    return "這段配對期已經結束（配對碼只能用一次）。請重新開始配對。";
  }
  return null;
}

export function MobileSection({
  refreshKey,
  advanced = false,
}: {
  refreshKey: number;
  advanced?: boolean;
}) {
  const { human } = useAppState();
  const [status, reloadStatus] = useAsync(() => api.mobileStatus(), [refreshKey]);
  const [runtimeStatus] = useAsync(() => api.status(), [refreshKey]);
  const [pairing, setPairing] = React.useState<Record<string, unknown> | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState(false);
  // 配對面板是一次性快照，runtime 才是真相：這一段配對期可能已經被區網上的
  // peer 燒掉、被用掉或過期（status.pairingBurnedAt／pairingActive）。畫面不
  // 得繼續顯示配對碼與「有效至 …」——那是在宣稱一件已經不成立的事。
  const [pairingLive, setPairingLive] = React.useState<{
    active: boolean;
    burnedAt: string | null;
  } | null>(null);
  const [now, setNow] = React.useState(() => Date.now());
  const reloadStatusRef = React.useRef(reloadStatus);
  reloadStatusRef.current = reloadStatus;
  const invalidPairing = pairing
    ? pairingInvalidReason(pairing.expiresAt, pairingLive, now)
    : null;

  React.useEffect(() => {
    if (!pairing || invalidPairing) return;
    let alive = true;
    const check = async () => {
      try {
        const s = await api.mobileStatus();
        if (!alive) return;
        setPairingLive({
          active: s["pairingActive"] === true,
          burnedAt: (s["pairingBurnedAt"] as string | null) ?? null,
        });
      } catch {
        // 問不到就不臆測：維持上一次已知的狀態（不假裝有效，也不假裝失效）。
      }
    };
    void check();
    const timer = setInterval(() => {
      setNow(Date.now());
      void check();
    }, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [pairing, invalidPairing, refreshKey]);

  // 配對期收掉的同時，裝置清單可能已經多了一台（別人用掉了這組碼）。
  React.useEffect(() => {
    if (invalidPairing) reloadStatusRef.current();
  }, [invalidPairing]);

  const activeSensors = (runtimeStatus.data?.["activeSensors"] as SensorUse[] | undefined) ?? [];
  const devices = ((status.data?.devices as Record<string, unknown>[] | undefined) ?? []).map((d) =>
    phoneCardModel(d, human, activeSensors)
  );
  const bonjour = (status.data?.bonjour as Record<string, unknown> | undefined) ?? null;
  // runtime 讀不到／解析不了配對清單時的誠實訊號（`devicesUnknown`）。
  const devicesUnknown = status.data?.["devicesUnknown"] === true;
  const devicesError = (status.data?.["devicesError"] as string | null) ?? null;

  return (
    <Section title="iPhone">
      <p className="muted small">
        iPhone 需要安裝配套的手機 App（Interaction Companion）。配對用一次性配對碼，連線全程加密並確認是同一台電腦；
        每台 iPhone 各自一把鑰匙，隨時可以撤銷。桌面上的同意不能取代 iOS 系統權限；感測有沒有開，以手機畫面為準。
      </p>
      {status.error && <div className="state-box state-error">無法讀取狀態：{status.error}</div>}
      {bonjour && (
        <div className="muted small" data-testid="mobile-bonjour">
          {bonjour.advertised === true ? (
            <>
              同一個 Wi-Fi 裡的 iPhone 可以自動找到這台電腦。
              {advanced
                ? `　Bonjour 服務：${String(bonjour.service ?? "")}${
                    bonjour.instance ? `／${String(bonjour.instance)}` : ""
                  }`
                : ""}
            </>
          ) : (
            <>
              iPhone 無法自動找到這台電腦（自動尋找未啟用
              {advanced && bonjour.error ? `：${String(bonjour.error)}` : ""}
              ）——請掃 QR；不能掃（相機被占用、沒授權或機型不支援）時，開始配對後把下面那段
              「配對資料」複製到 iPhone App 手動貼上。
            </>
          )}
        </div>
      )}
      {devicesUnknown ? (
        // 誠實階梯：狀態檔讀不到＝**未知**，不得演成「還沒有配對的 iPhone」
        // ——那會讓已配對的手機在畫面上無聲消失。
        <div className="state-box state-error" role="alert" data-testid="mobile-devices-unknown">
          讀不到已配對的 iPhone 清單（狀態檔讀取或解析失敗），所以現在無法確定有哪些手機配對過
          ——這不等於「沒有配對過」。已配對的手機可能因此連不上；重新配對可以重建這份清單。
          {advanced && devicesError ? `　原因：${devicesError}` : ""}
        </div>
      ) : devices.length === 0 ? (
        <p className="muted small">還沒有配對的 iPhone。</p>
      ) : (
        <div className="provider-list">
          {devices.map((d) => (
            <PhoneDeviceCard
              key={d.deviceId}
              model={d}
              advanced={advanced}
              onChanged={reloadStatus}
            />
          ))}
        </div>
      )}
      <div className="row wrap" style={{ marginTop: 8 }}>
        <button
          onClick={async () => {
            try {
              setPairingLive(null);
              setCopied(false);
              setNow(Date.now());
              setPairing(await api.mobilePairingBegin());
              setError(null);
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          開始配對（5 分鐘內有效）
        </button>
      </div>
      {pairing && invalidPairing && (
        <div className="notice-box state-error" role="alert" data-testid="pairing-invalid">
          <p>{invalidPairing}</p>
        </div>
      )}
      {pairing && !invalidPairing && (
        <div className="notice-box" role="status">
          <p>
            在 iPhone App 掃描 QR 或輸入配對碼：<strong>{String(pairing.code)}</strong>
            （手機會核對這台電腦的配對安全碼前 6 碼{" "}
            {String(pairing.fingerprint ?? "").slice(0, 6)}）
            {advanced ? `　電腦埠號 ${String(pairing.port)}・識別碼 ${String(pairing.fingerprint ?? "")}` : ""}
          </p>
          {typeof pairing.qrSvg === "string" && pairing.qrSvg.length > 0 && (
            <div
              aria-label="配對 QR Code"
              // 後端 qrcode crate 產生的 SVG（本機生成，非外部內容）。
              dangerouslySetInnerHTML={{ __html: pairing.qrSvg }}
            />
          )}
          {typeof pairing.payload === "string" && pairing.payload.length > 0 && (
            <div className="pairing-manual">
              <p className="muted small">
                不能掃 QR 時：把下面這段「配對資料」整段複製，貼進 iPhone App 的手動配對欄位
                （手機需要的是這一整段，只有 6 位配對碼是不夠的）。
              </p>
              <textarea
                data-testid="pairing-payload"
                aria-label="配對資料（複製到 iPhone App 手動貼上）"
                readOnly
                rows={3}
                value={String(pairing.payload)}
                onFocus={(e) => e.currentTarget.select()}
              />
              <div className="row wrap">
                <button
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(String(pairing.payload));
                      setCopied(true);
                    } catch {
                      // 沒有剪貼簿權限就誠實不宣稱複製成功：內容本來就看得到、選得起來。
                      setCopied(false);
                    }
                  }}
                >
                  複製配對資料
                </button>
                {copied && <span className="muted small">已複製到剪貼簿。</span>}
              </div>
              {pairingHostPort(pairing.payload) && (
                <p className="muted small" data-testid="pairing-host">
                  這台電腦的位址：{pairingHostPort(pairing.payload)}
                </p>
              )}
            </div>
          )}
          <p className="muted small">
            有效至 {new Date(String(pairing.expiresAt)).toLocaleTimeString("zh-TW")}；配對碼只能用一次。
          </p>
        </div>
      )}
      {error && <p className="cap-card-error" role="alert">{error}</p>}
    </Section>
  );
}
