// 能力與裝置（spec §16-1.E）：內建能力（感知／回應／工具）＋裝置與提供者。
// 掃描文案誠實：只宣稱「已偵測到目前可用」，不宣稱找到所有硬體。

import React from "react";
import { api } from "../api";
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

/** 平台尚未支援的能力：誠實列具體原因，不用灰色按鈕冒充。 */
const UNSUPPORTED: { name: string; reason: string }[] = [
  { name: "攝影機", reason: "尚未實作影像擷取層（刻意誠實未支援；不偽裝存在）" },
  { name: "HID（鍵盤／滑鼠／手寫板／遊戲控制器）", reason: "作業系統層列舉 adapter 尚未實作" },
  { name: "MIDI", reason: "MIDI adapter 尚未實作" },
  { name: "USB Serial／Bluetooth LE", reason: "序列／BLE 傳輸層尚未實作（宣告式 adapter 只支援 HTTP/SSE）" },
  { name: "mDNS 自動探索", reason: "網路探索尚未實作；外部裝置以宣告式 YAML 手動加入" },
];

function ProvidersSection({ refreshKey }: { refreshKey: number }) {
  const [providers] = useAsync(
    () => api.providersList() as Promise<Record<string, unknown>[]>,
    [refreshKey]
  );
  const [scanning, setScanning] = React.useState(false);
  const [scanNote, setScanNote] = React.useState<string | null>(null);

  async function scan() {
    setScanning(true);
    try {
      await api.agentsRefresh();
      setScanNote("已重新偵測目前可用的裝置與本機 AI agent。偵測不到不代表不存在：驅動、權限、沙盒、未配對或裝置被占用都可能讓設備看不見。");
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
      <Section title="此平台尚未支援的能力">
        <p className="muted small">以下能力目前誠實地「不存在」，並附具體原因（不是灰色按鈕）：</p>
        <ul className="plain-list">
          {UNSUPPORTED.map((u) => (
            <li key={u.name}>
              <strong>{u.name}</strong> — <span className="muted">{u.reason}</span>
            </li>
          ))}
        </ul>
      </Section>
    </div>
  );
}
