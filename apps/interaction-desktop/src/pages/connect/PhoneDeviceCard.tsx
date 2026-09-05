// 已連接的手機（連接與權限 →「已連接的裝置」第一張卡；第二層的 iPhone 區共用同一份）。
//
// 卡片內容全部來自 Runtime 的三份真實狀態，沒有任何示範資料：
//   1. `mobile_status` 的裝置列 —— 連線與否、手機自報的感測開關與 iOS 系統權限；
//   2. `capabilities/human` 的能力卡 —— 可以提供／可以執行（照抄能力卡的可用狀態）；
//   3. `status.activeSensors` —— 真的正在感測的項目（`startedBy` 精確比對這台手機）。
//
// 誠實階梯：
//   * 「停止感測」送出後只說「已要求停止（以手機回報為準）」；只有後端回報
//     `outcome:"stopped"` 才可以說「已停止」，送不到就說「未送達」。
//   * 「測試連接」有回應只證明連線還在，不證明手機 App 的功能可用；沒有回應一律
//     「結果不確定」，不得說成失敗或成功，也不會寫成「已測試」。
//   * 手機自報的感測鍵介面不認得時原樣顯示，不發明名稱。

import React from "react";
import {
  api,
  HumanCapabilities,
  HumanCard,
  MobileSensorsStopResult,
  MobileTestResult,
  SensorUse,
} from "../../api";
import { availabilityLabel } from "../../appstate";
import { ConfirmButton } from "../../components/Dialog";
import { Icon } from "../../icons";
import { Badge } from "../../ui";

/** 手機自報的 iOS 系統權限（桌面的同意不能取代，照抄手機的回報）。 */
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

/** 還沒在 iPhone 上允許的狀態（列進「目前需要確認的權限」）。 */
const PERMISSION_NEEDS_ATTENTION = new Set(["denied", "notDetermined"]);

/** 手機自報的感測開關（開＝手機端真的在感測）。 */
const MOBILE_SENSOR_LABEL: Record<string, string> = {
  motion: "動作",
  battery: "電量",
  micLevel: "麥克風音量",
  location: "位置",
  bleGateway: "藍牙轉接",
};

/** Runtime `activeSensors.kind` → 人話（未知種類原樣顯示，不猜）。 */
const MOBILE_SENSOR_KIND_LABEL: Record<string, string> = {
  "iphone.mic-level": "麥克風音量",
  "iphone.motion": "動作",
  "iphone.battery": "電量",
  "iphone.touch": "角色觸碰",
};

/** 手機能力的 id 前綴（Runtime `MOBILE_RECEPTOR_SPECS`／`MOBILE_ACTUATORS`）。 */
const PHONE_CAPABILITY_PREFIX = "iphone.";
/** 手機在來源清單裡的 id 前綴（Runtime `mobile.rs` 的 `provider.mobile.<id>`）。 */
const MOBILE_PROVIDER_PREFIX = "provider.mobile.";

/** 這一列是不是手機？手機已經有自己的卡片，來源清單要排掉才不會同一台列兩次。 */
export function isMobileProviderId(id: unknown): boolean {
  return typeof id === "string" && id.startsWith(MOBILE_PROVIDER_PREFIX);
}
/** 能力卡若帶 driver，手機能力的 driver 是這個（沒帶就只認 id 前綴，不猜）。 */
const PHONE_DRIVER = "mobile.iphone";

export interface PhoneCapabilityView {
  id: string;
  /** 已去掉「iPhone 」前綴的短名（卡片本身已經說了是哪一台手機）。 */
  name: string;
  /** 照抄能力卡的可用狀態，不改寫。 */
  availability: string;
  requiresConsent: boolean;
}

export interface PhonePermissionView {
  key: string;
  label: string;
  state: string;
  /** true＝還要你在 iPhone 上處理（已拒絕／未詢問）。 */
  needsAttention: boolean;
}

export interface PhoneCardModel {
  deviceId: string;
  name: string;
  model: string | null;
  connected: boolean;
  pairedAt: string | null;
  /** 這台手機可以提供的感知來源（能力由 Runtime 共用註冊，不是逐台宣告）。 */
  provides: PhoneCapabilityView[];
  /** 這台手機可以執行的回應方式。 */
  performs: PhoneCapabilityView[];
  /** 目前真的在感測的項目：Runtime 的 activeSensors ∪ 手機自報為「開」的感測。 */
  activeSensing: string[];
  /** null＝手機尚未回報 iOS 系統權限（未知，不當成已授權）。 */
  permissions: PhonePermissionView[] | null;
  /** 連線狀態原始值——只給進階模式「連接診斷」看；一般模式一律用已翻譯的 Badge。 */
  connectedRaw: unknown;
  /** 手機自報感測旗標原始值——只給進階模式「連接診斷」看；一般模式看 activeSensing 的人話。
   *  null＝手機還沒回報過（不是「都關著」）。 */
  sensorFlagsRaw: Record<string, unknown> | null;
}

function text(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** 「iPhone 電量與前景」→「電量與前景」；去掉後會變空就保留原名。 */
export function stripPhonePrefix(name: string): string {
  const stripped = name.replace(/^iPhone[\s　]*/, "").trim();
  return stripped.length > 0 ? stripped : name;
}

function isPhoneCard(card: HumanCard): boolean {
  return card.driver === PHONE_DRIVER || String(card.id ?? "").startsWith(PHONE_CAPABILITY_PREFIX);
}

function capabilityView(card: HumanCard): PhoneCapabilityView {
  return {
    id: card.id,
    name: stripPhonePrefix(card.displayName ?? card.id),
    availability: card.availability,
    requiresConsent: card.requiresConsent === true || card.consent?.required === true,
  };
}

/** Runtime 感測種類的人話；不認得就原樣（不發明名稱）。 */
export function phoneSensorKindLabel(kind: string): string {
  return Object.prototype.hasOwnProperty.call(MOBILE_SENSOR_KIND_LABEL, kind)
    ? MOBILE_SENSOR_KIND_LABEL[kind]
    : kind;
}

/** 手機自報感測鍵的人話；不認得就原樣（不發明名稱）。 */
export function phoneSelfReportSensorLabel(key: string): string {
  return Object.prototype.hasOwnProperty.call(MOBILE_SENSOR_LABEL, key)
    ? MOBILE_SENSOR_LABEL[key]
    : key;
}

/**
 * 一台手機的卡片資料。純函式：三份真實狀態進去，一張卡出來，沒有任何猜測。
 *
 * `human` 的手機能力卡是 Runtime 共用註冊的（不分機台），因此多台手機看到的
 * 「可以提供／可以執行」相同；能不能用要看每張卡自己的可用狀態。
 */
export function phoneCardModel(
  device: Record<string, unknown>,
  human: HumanCapabilities | null | undefined,
  activeSensors: SensorUse[] | null | undefined
): PhoneCardModel {
  const deviceId = String(device.deviceId ?? "");
  const sensors = (device.sensors as Record<string, unknown> | null | undefined) ?? null;
  const permissionsRaw =
    (device.permissions as Record<string, unknown> | null | undefined) ?? null;

  const fromRuntime = (activeSensors ?? [])
    .filter((s) => s && s.startedBy === `iphone:${deviceId}`)
    .map((s) => phoneSensorKindLabel(String(s.kind ?? "")));
  const fromPhone = Object.entries(sensors ?? {})
    .filter(([, on]) => on === true)
    .map(([key]) => phoneSelfReportSensorLabel(key));
  const activeSensing: string[] = [];
  for (const label of [...fromRuntime, ...fromPhone]) {
    if (label && !activeSensing.includes(label)) activeSensing.push(label);
  }

  const permissions =
    permissionsRaw === null
      ? null
      : Object.entries(permissionsRaw).map(([key, value]) => {
          const state = String(value);
          return {
            key,
            label: MOBILE_PERMISSION_LABEL[key] ?? key,
            state: MOBILE_PERMISSION_STATE[state] ?? state,
            needsAttention: PERMISSION_NEEDS_ATTENTION.has(state),
          };
        });

  return {
    deviceId,
    name: text(device.name) ?? "iPhone",
    model: text(device.model),
    connected: device.connected === true,
    pairedAt: text(device.pairedAt),
    provides: (human?.receptors ?? []).filter(isPhoneCard).map(capabilityView),
    performs: (human?.actuators ?? []).filter(isPhoneCard).map(capabilityView),
    activeSensing,
    permissions,
    connectedRaw: device.connected,
    sensorFlagsRaw: sensors,
  };
}

/** 手機沒回報 iOS 權限，或有權限還沒允許 → 需要你在手機上確認的項目。
 *  未連線的手機現在確認不了什麼，不列（卡片上仍會誠實寫「未連線」與「未回報」）。 */
export function phonePermissionAlerts(model: PhoneCardModel): string[] {
  if (!model.connected) return [];
  if (model.permissions === null) {
    return [`${model.name}：手機尚未回報 iPhone 上的權限（未知）`];
  }
  return model.permissions
    .filter((p) => p.needsAttention)
    .map((p) => `在 ${model.name} 上尚未允許：${p.label}（${p.state}）`);
}

function capabilityText(caps: PhoneCapabilityView[]): string {
  return caps
    .map((c) => (c.availability === "available" ? c.name : `${c.name}（${availabilityLabel(c.availability)}）`))
    .join("、");
}

// ---------------------------------------------------------------------------

/** 停止感測的回應 → 一句誠實的話。送出≠停止，只有手機回報 stopped 才算停止。 */
export function stopSensorsMessage(name: string, result: MobileSensorsStopResult): string {
  const outcome = String(result.outcome ?? "");
  if (result.requested !== true || outcome === "unreachable") {
    return `${name}：未送達（手機未連線），感測狀態未變。`;
  }
  if (outcome === "stopped") return `${name}：已停止（手機回報已停止）。`;
  return `${name}：已要求停止（以手機回報為準）；手機還沒回報，結果不確定。`;
}

/** 測試連接的回應 → 一句誠實的話。有回應只代表連線還在。 */
export function testMessage(result: MobileTestResult): string {
  if (result.ok === true) {
    const latency = result.latencyMs;
    const ms =
      typeof latency === "number" && Number.isFinite(latency) ? `（${Math.round(latency)} ms）` : "";
    return `有回應${ms}——只代表連線還在，不代表手機 App 的功能都能用。`;
  }
  const reason = text(result.reason);
  return `沒有回應（結果不確定）${reason ? `：${reason}` : ""}`;
}

export function PhoneDeviceCard({
  model,
  advanced = false,
  onChanged,
  onManagePermissions,
  onRepair,
  syncLine,
  focused = false,
}: {
  model: PhoneCardModel;
  advanced?: boolean;
  /**
   * 深連結指名的就是這一台（角色同步卡的「去重新確認」）：把卡片標出來並捲到它。
   * 純呈現：不改任何狀態、不代替使用者按任何按鈕，也不會影響卡片上寫的事實。
   */
  focused?: boolean;
  /** 角色同步的一行人話（`statusProjection.characterSyncDeviceLine`）。
   *  沒給就不顯示這一行——不猜、也不用「已同步」當預設。 */
  syncLine?: string | null;
  /** 撤銷成功／失敗後重新讀取裝置清單。 */
  onChanged?: () => void;
  /** 「管理權限」：帶去同意與安全。沒給就不顯示這顆按鈕。 */
  onManagePermissions?: () => void;
  /** 「重新配對」：卡片不自己實作配對流程，交給呼叫端導到既有的 iPhone 配對區
   *（ConnectPage 的 CapabilitiesHub「providers」分類）。沒給就不顯示這顆按鈕。 */
  onRepair?: () => void;
}) {
  const [message, setMessage] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState<string | null>(null);
  const cardRef = React.useRef<HTMLDivElement>(null);

  // 被指名時捲到自己身上（jsdom／舊 WebView 沒有 scrollIntoView 就什麼都不做——
  // 標示本身已經夠用，不為了捲動而擲例外）。
  React.useEffect(() => {
    if (!focused) return;
    const node = cardRef.current;
    if (node && typeof node.scrollIntoView === "function") {
      node.scrollIntoView({ block: "nearest" });
    }
  }, [focused]);

  async function run(kind: string, work: () => Promise<string>) {
    setBusy(kind);
    setMessage(null);
    try {
      setMessage(await work());
    } catch (e) {
      setMessage(`${kind}沒有完成：${e}（結果不確定）。`);
    } finally {
      setBusy(null);
    }
  }

  const offlineReason = model.connected ? null : "手機未連線時送不出任何指令。";

  return (
    <div
      ref={cardRef}
      className={focused ? "provider-card phone-card focused" : "provider-card phone-card"}
      data-testid={`phone-card-${model.deviceId}`}
      {...(focused ? { "data-focused": "true", "aria-current": "true" as const } : {})}
    >
      <div className="row space-between wrap">
        <strong>
          <Icon name="wifi" size={16} /> 我的 iPhone（
          <span className="phone-name">{model.name}</span>）
        </strong>
        {model.connected ? (
          <Badge kind="ok">已連線</Badge>
        ) : (
          <Badge kind="bad">未連線（能力不可用）</Badge>
        )}
      </div>
      {(model.model || model.pairedAt) && (
        <div className="muted small">
          {model.model ?? ""}
          {model.model && model.pairedAt ? "・" : ""}
          {model.pairedAt ? `配對於 ${new Date(model.pairedAt).toLocaleString("zh-TW")}` : ""}
        </div>
      )}
      <div className="small">
        可以提供：
        {model.provides.length === 0
          ? "尚未回報能力（手機連上這台電腦後才會出現）"
          : capabilityText(model.provides)}
      </div>
      <div className="small">
        可以執行：
        {model.performs.length === 0
          ? "尚未回報能力（手機連上這台電腦後才會出現）"
          : `${capabilityText(model.performs)}（使用前會先問你）`}
      </div>
      <div className="small">
        目前使用中的感測：
        {model.activeSensing.length === 0 ? "無" : model.activeSensing.join("、")}
      </div>
      {/* 角色同步（AIP Character Session §11）：連上 ≠ 同步，兩件事分開說。 */}
      {syncLine && <div className="small">{syncLine}</div>}
      {model.permissions === null ? (
        <div className="muted small">手機上的權限：手機尚未回報（未知）。</div>
      ) : (
        <div className="muted small">
          手機上的權限（手機自報，這台電腦上的同意不能取代）：
          {model.permissions.map((p) => `${p.label}：${p.state}`).join("、")}
        </div>
      )}
      {advanced && (
        <div
          className="muted small phone-diagnostics"
          data-testid={`phone-diagnostics-${model.deviceId}`}
        >
          <div className="phone-diagnostics-title">連接診斷</div>
          <div>
            原始：{model.deviceId}・{model.pairedAt ?? "—"}
          </div>
          <div>連線狀態原始值：{String(model.connectedRaw)}</div>
          <div>
            手機自報感測旗標原始值：
            {model.sensorFlagsRaw
              ? Object.entries(model.sensorFlagsRaw)
                  .map(([key, value]) => `${key}=${String(value)}`)
                  .join("、")
              : "（手機尚未回報）"}
          </div>
        </div>
      )}
      <div className="row wrap">
        {onManagePermissions && <button onClick={onManagePermissions}>管理權限</button>}
        {onRepair && <button onClick={onRepair}>重新配對</button>}
        <button
          disabled={!model.connected || busy !== null}
          onClick={() =>
            void run("測試連接", async () => testMessage(await api.mobileTest(model.deviceId)))
          }
        >
          測試連接
        </button>
        <button
          disabled={!model.connected || busy !== null}
          onClick={() =>
            void run("停止感測", async () =>
              stopSensorsMessage(model.name, await api.mobileSensorsStop(model.deviceId))
            )
          }
        >
          停止感測
        </button>
        <ConfirmButton
          className="danger"
          label="移除此手機"
          confirmLabel="確定移除？（立即斷線，要再用必須重新配對）"
          disabled={busy !== null}
          onConfirm={() => {
            void (async () => {
              setBusy("移除此手機");
              setMessage(null);
              try {
                await api.mobileRevoke(model.deviceId);
                setMessage("已移除這台手機的鑰匙並斷線；要再用必須重新配對。");
              } catch (e) {
                setMessage(`移除失敗：${e}。配對狀態未變，請重試。`);
              } finally {
                setBusy(null);
                onChanged?.();
              }
            })();
          }}
        />
      </div>
      {offlineReason && (
        <>
          <div className="muted small">{offlineReason}</div>
          {/* 真機限制：iPhone 是用桌面目前的網路位址配對的，桌面換了 IP／Wi-Fi
              就連不上，必須重新配對才會恢復——未連線時這句提醒最有用。 */}
          <div className="muted small">若桌面的網路位址變了，需要重新配對。</div>
        </>
      )}
      {message && (
        <p className="notice-box small" role="status">
          {message}
        </p>
      )}
    </div>
  );
}
