// 能力與裝置（spec §16-1.E）：內建能力（感知／回應／工具）＋裝置與提供者。
// 掃描文案誠實：只宣稱「已偵測到目前可用」，不宣稱找到所有硬體。

import React from "react";
import { api, HardwareScanReport } from "../api";
import { Badge, Section, StateView, useAsync } from "../ui";
import { CapabilitiesPage } from "./CapabilitiesPage";

type HubTab = "senses" | "responses" | "toolops" | "providers";

export function CapabilitiesHub({
  refreshKey,
  advanced,
}: {
  refreshKey: number;
  advanced: boolean;
}) {
  const [tab, setTab] = React.useState<HubTab>("senses");
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
      {tab === "providers" && <ProvidersSection refreshKey={refreshKey} />}
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
  "usb-serial": "USB Serial",
  "bluetooth-le": "Bluetooth LE",
  display: "螢幕呈現",
  "system-notification": "系統通知",
  "os-sensor": "作業系統感測器",
  "mdns-device": "mDNS 本機網路裝置",
  "esp32-declaration": "ESP32 宣告式裝置",
};

const HARDWARE_AVAILABILITY: Record<string, { text: string; kind: "ok" | "warn" | "bad" | "pending" }> = {
  available: { text: "目前可見", kind: "ok" },
  "permission-required": { text: "需要權限", kind: "pending" },
  busy: { text: "被占用", kind: "warn" },
  unavailable: { text: "目前不可用", kind: "bad" },
  unsupported: { text: "此平台尚未支援", kind: "warn" },
  unknown: { text: "結果未知", kind: "pending" },
};

function ProvidersSection({ refreshKey }: { refreshKey: number }) {
  const [providers] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
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
                text: "結果未知",
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
                    <div className="muted small">穩定識別：{device.stableId}</div>
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
      <Section title="提供者（Provider）">
        <StateView state={providers} empty="尚未發現任何提供者。">
          {(list) => (
            <div className="provider-list">
              {list.map((p) => {
                const identity = p.identity as Record<string, unknown>;
                const state = String(p.state ?? "");
                const label = PROVIDER_STATE_LABEL[state] ?? {
                  text: state,
                  kind: "pending" as const,
                };
                const id = String(identity?.id ?? "");
                return (
                  <div className="provider-card" key={id}>
                    <div className="row space-between">
                      <strong>{String(identity?.displayName ?? id)}</strong>
                      <Badge kind={label.kind}>{label.text}</Badge>
                    </div>
                    <div className="muted small">
                      {String(identity?.kind ?? "")}・信任：{String(identity?.trustLevel ?? "")}
                      {p.detail ? `・${String(p.detail)}` : ""}
                    </div>
                    <div className="muted small">
                      能力：受器 {(p.receptors as unknown[] | undefined)?.length ?? 0}・動器{" "}
                      {(p.actuators as unknown[] | undefined)?.length ?? 0}・工具{" "}
                      {(p.toolOperations as unknown[] | undefined)?.length ?? 0}
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
