// 能力與裝置（spec §16-1.E）：內建能力（感知／回應／工具）＋裝置與提供者。
// 掃描文案誠實：只宣稱「已偵測到目前可用」，不宣稱找到所有硬體。

import React from "react";
import { api, HardwareScanReport, ProviderTested } from "../api";
import { useAppState } from "../appstate";
import { useCharacterName } from "../characterName";
import { Badge, Section, StateView, useAsync } from "../ui";
import { CapabilitiesPage } from "./CapabilitiesPage";
import { CharacterAdaptersSection } from "./connect/CharacterAdaptersSection";

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
            ["providers", "裝置與提供者"],
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
  "usb-serial": "USB 接線裝置（USB Serial：用線接上的自製硬體，例如 ESP32）",
  "bluetooth-le": "低功耗藍牙裝置（Bluetooth LE：不用接線的小型感測器與燈具）",
  display: "螢幕呈現",
  "system-notification": "系統通知",
  "os-sensor": "作業系統感測器",
  "mdns-device": "同一個 Wi-Fi 裡自動找到的裝置（mDNS：裝置自己報名字，不用輸入位址）",
  "esp32-declaration": "ESP32 自製裝置（用設定檔描述它能感測什麼、能做什麼）",
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
  return {
    note,
    tested: {
      at: String(t.at ?? ""),
      how: String(t.how ?? ""),
      ok: t.ok,
      note: typeof t.note === "string" ? t.note : undefined,
    },
  };
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
                    <strong>{HARDWARE_CLASS_LABEL[device.class] ?? device.displayName}</strong>
                    <Badge kind={availability.kind}>{availability.text}</Badge>
                  </div>
                  <div>{device.displayName}</div>
                  <div className="muted small">{device.detail}</div>
                  <div className="muted small">識別依據：{device.identityBasis}</div>
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
                          <strong>{capability.id}</strong> — {capability.scope}
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
        <StateView state={providers} empty="尚未發現任何提供者。">
          {(list) => (
            <div className="provider-list">
              {list.map((p) => {
                const identity = p.identity as Record<string, unknown>;
                const state = String(p.state ?? "");
                const id = String(identity?.id ?? "");
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
// iPhone Mobile Provider（v0.5 Phase 6）：配對、狀態、撤銷。
// 誠實：配對碼 5 分鐘一段；斷線＝能力不可用；感測狀態由手機自報。
// ---------------------------------------------------------------------------

/** 手機自報的 iOS 系統權限（桌面 Consent 不能取代，誠實照抄手機的回報）。 */
const MOBILE_PERMISSION_LABEL: Record<string, string> = {
  microphone: "麥克風",
  location: "位置",
  bluetooth: "藍牙",
};
const MOBILE_PERMISSION_STATE: Record<string, string> = {
  granted: "已授權",
  denied: "已拒絕",
  notDetermined: "未詢問",
};
/** 手機自報的感測開關（開＝手機端真的在感測）。 */
const MOBILE_SENSOR_LABEL: Record<string, string> = {
  motion: "動作",
  battery: "電量",
  micLevel: "麥克風音量",
  location: "位置",
  bleGateway: "BLE 閘道",
};

export function MobileSection({
  refreshKey,
  advanced = false,
}: {
  refreshKey: number;
  advanced?: boolean;
}) {
  const [status] = useAsync(() => api.mobileStatus(), [refreshKey]);
  const [pairing, setPairing] = React.useState<Record<string, unknown> | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const devices = ((status.data?.devices as Record<string, unknown>[] | undefined) ?? []);
  const bonjour = (status.data?.bonjour as Record<string, unknown> | undefined) ?? null;

  return (
    <Section title="iPhone">
      <p className="muted small">
        iPhone 需要安裝配套的手機 App（Interaction Companion）。配對用一次性配對碼，連線全程加密並確認是同一台電腦；
        每台 iPhone 各自一把鑰匙，隨時可以撤銷。桌面上的同意不能取代 iOS 系統權限；感測有沒有開，以手機畫面為準。
      </p>
      {status.error && <div className="state-box state-error">無法讀取狀態：{status.error}</div>}
      {bonjour && (
        <div className="muted small">
          {bonjour.advertised === true ? (
            <>
              同一個 Wi-Fi 裡的 iPhone 可以自動找到這台電腦（Bonjour）。
              {advanced
                ? `　服務：${String(bonjour.service ?? "")}${
                    bonjour.instance ? `／${String(bonjour.instance)}` : ""
                  }`
                : ""}
            </>
          ) : (
            <>
              iPhone 無法自動找到這台電腦（Bonjour 未啟用
              {bonjour.error ? `：${String(bonjour.error)}` : ""}）——請掃 QR，或在手機上手動輸入電腦位址與埠號配對。
            </>
          )}
        </div>
      )}
      {devices.length === 0 ? (
        <p className="muted small">還沒有配對的 iPhone。</p>
      ) : (
        <div className="provider-list">
          {devices.map((d) => (
            <div className="provider-card" key={String(d.deviceId)}>
              <div className="row space-between">
                <strong>{String(d.name)}</strong>
                {d.connected === true ? (
                  <Badge kind="ok">已連線</Badge>
                ) : (
                  <Badge kind="bad">未連線（能力不可用）</Badge>
                )}
              </div>
              <div className="muted small">
                {String(d.model || "")}・配對於 {new Date(String(d.pairedAt)).toLocaleString("zh-TW")}
              </div>
              {d.connected === true && d.sensors ? (
                <div className="muted small">
                  手機自報感測：
                  {Object.entries((d.sensors as Record<string, unknown>) ?? {})
                    .map(
                      ([k, v]) =>
                        `${MOBILE_SENSOR_LABEL[k] ?? k}：${v === true ? "開" : "關"}`,
                    )
                    .join("、")}
                </div>
              ) : null}
              {d.connected === true && d.permissions ? (
                <div className="muted small">
                  iOS 系統權限（手機自報，桌面授權不能取代）：
                  {Object.entries((d.permissions as Record<string, unknown>) ?? {})
                    .map(
                      ([k, v]) =>
                        `${MOBILE_PERMISSION_LABEL[k] ?? k}：${
                          MOBILE_PERMISSION_STATE[String(v)] ?? String(v)
                        }`,
                    )
                    .join("、")}
                </div>
              ) : null}
              {d.connected === true && !d.permissions ? (
                <div className="muted small">iOS 系統權限：手機尚未回報（未知）。</div>
              ) : null}
              <button
                className="danger"
                onClick={async () => {
                  try {
                    await api.mobileRevoke(String(d.deviceId));
                    setError(null);
                  } catch (e) {
                    setError(String(e));
                  }
                }}
              >
                撤銷配對（立即斷線）
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="row wrap" style={{ marginTop: 8 }}>
        <button
          onClick={async () => {
            try {
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
      {pairing && (
        <div className="notice-box" role="status">
          <p>
            在 iPhone App 掃描 QR 或輸入配對碼：<strong>{String(pairing.code)}</strong>
            （電腦埠號 {String(pairing.port)}；手機會核對這台電腦的識別碼{" "}
            {String(pairing.fingerprint).slice(0, 16)}…）
          </p>
          {typeof pairing.qrSvg === "string" && pairing.qrSvg.length > 0 && (
            <div
              aria-label="配對 QR Code"
              // 後端 qrcode crate 產生的 SVG（本機生成，非外部內容）。
              dangerouslySetInnerHTML={{ __html: pairing.qrSvg }}
            />
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
