// 角色頁的「同步」卡：手機上的角色與這台電腦是不是同一個狀態。
//
// 契約：`docs/aip/character-session.md` §11（一般模式文案）＋
// `docs/aip/transport-bindings.md` §2（四條路由）。UI 只做投影：
//
//   * 每一句都來自 Runtime 的真實回應（權威快照＋已配對裝置清單＋來源狀態），
//     沒有示範資料、沒有樂觀預設。
//   * 讀不到就說讀不到；連續讀不到才升級成「無法恢復，請重新連接」（契約 §7.5）。
//   * 空狀態不像成功：一台裝置都沒有時是中性的「尚未連接 iPhone」。
//   * 一般模式看不到任何技術數字；進階模式才展開「連接診斷」。
//   * 模擬 iPhone（fixture）的名稱自帶標籤，原樣顯示，不得被寫成真機。
//   * 緊急停止中的固定安全句由這裡（可信 host 介面）顯示，角色無法覆寫。

import React from "react";
import { api, type CharacterSessionDiagnostics } from "../api";
import {
  characterSyncLastInteraction,
  characterSyncMemberDeviceIds,
  characterSyncMembers,
  characterSyncPresenceLabel,
  characterSyncSafetyNote,
  projectCharacterSession,
  type CharacterSyncMember,
  type CharacterSyncSignals,
  type ProjectedCharacterSync,
} from "../statusProjection";
import { Badge } from "../ui";

/** 後端把角色同步整個關掉時的穩定錯誤碼（HTTP 503，AIP §12）。 */
const SESSION_DISABLED = "session-disabled";
/** 同一件事在 Runtime 內嵌模式的固定訊息（`SESSION_DISABLED_MESSAGE`）。 */
const SESSION_DISABLED_MESSAGE = "character session is turned off";
/** 手機在來源清單裡的 id 前綴（Runtime `mobile.rs` 的 `provider.mobile.<id>`）。 */
const MOBILE_PROVIDER_PREFIX = "provider.mobile.";

interface SyncView {
  projection: ProjectedCharacterSync;
  members: CharacterSyncMember[];
  lastInteraction: string | null;
  safetyNote: string | null;
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** 已配對裝置清單 → 「裝置識別碼 → 這台電腦上的顯示名稱」。 */
export function deviceNames(mobile: unknown): Record<string, string> {
  const devices = record(mobile)?.["devices"];
  const names: Record<string, string> = {};
  if (!Array.isArray(devices)) return names;
  for (const entry of devices) {
    const device = record(entry);
    const id = typeof device?.["deviceId"] === "string" ? String(device["deviceId"]) : "";
    const name = typeof device?.["name"] === "string" ? String(device["name"]).trim() : "";
    if (id) names[id] = name.length > 0 ? name : "一台裝置";
  }
  return names;
}

/** 連著這台電腦的手機（不是「配對過」，是現在真的連著）。 */
function connectedDeviceIds(mobile: unknown): string[] {
  const devices = record(mobile)?.["devices"];
  if (!Array.isArray(devices)) return [];
  return devices
    .map((entry) => record(entry))
    .filter((device) => device?.["connected"] === true)
    .map((device) => String(device?.["deviceId"] ?? ""))
    .filter((id) => id.length > 0);
}

/** 有沒有裝置的授權被撤銷過（撤銷之後要重新確認才會再同步）。 */
export function hasRevokedDevice(providers: unknown): boolean {
  if (!Array.isArray(providers)) return false;
  return providers.some((entry) => {
    const provider = record(entry);
    const id = String(record(provider?.["identity"])?.["id"] ?? "");
    return id.startsWith(MOBILE_PROVIDER_PREFIX) && provider?.["state"] === "revoked";
  });
}

/** 後端說「角色同步關閉」了嗎（只認這兩個穩定字串，其他失敗一律當成「讀不到」）。 */
function isSessionDisabled(error: unknown): boolean {
  const text = String(error);
  return text.includes(SESSION_DISABLED) || text.includes(SESSION_DISABLED_MESSAGE);
}

export function CharacterSyncCard({
  refreshKey,
  advanced = false,
}: {
  refreshKey: number;
  /** 進階模式才顯示「連接診斷」（revision／sequence／計數）。 */
  advanced?: boolean;
}) {
  const [view, setView] = React.useState<SyncView | null>(null);
  const [diagnostics, setDiagnostics] = React.useState<CharacterSessionDiagnostics | null>(null);
  const [reload, setReload] = React.useState(0);
  // 連續讀不到權威狀態的次數：達 3 次才升級成「無法恢復」（契約 §7.5）。
  const failedReads = React.useRef(0);

  React.useEffect(() => {
    let alive = true;
    void (async () => {
      let snapshot: unknown = null;
      let enabled = true;
      try {
        snapshot = await api.characterSessionSnapshot();
        failedReads.current = 0;
      } catch (e) {
        // 後端明說關閉時不算「讀失敗」——那是一個確定的事實，不是不確定。
        if (isSessionDisabled(e)) enabled = false;
        else failedReads.current += 1;
      }
      let mobile: unknown = null;
      let providers: unknown = null;
      try {
        mobile = await api.mobileStatus();
      } catch {
        /* 裝置清單讀不到：名字就查不到，投影會用中性稱呼 */
      }
      try {
        providers = await api.providersList();
      } catch {
        /* 來源清單讀不到：不假設沒有被撤銷過的裝置，也不假設有 */
      }
      if (!alive) return;
      const names = deviceNames(mobile);
      const members = characterSyncMembers(snapshot, names);
      // 連著、但不在成員名單裡的手機＝還沒重新確認過（送不出互動，也收不到狀態）。
      const synced = new Set(characterSyncMemberDeviceIds(snapshot));
      const signals: CharacterSyncSignals = {
        enabled,
        failedReads: failedReads.current,
        revokedDevice: hasRevokedDevice(providers),
        connectedButNotSynced: connectedDeviceIds(mobile).some((id) => !synced.has(id)),
      };
      setView({
        projection: projectCharacterSession(snapshot, members, signals),
        members,
        lastInteraction: characterSyncLastInteraction(snapshot, names),
        safetyNote: characterSyncSafetyNote(snapshot),
      });
      if (!advanced) {
        setDiagnostics(null);
        return;
      }
      try {
        const value = await api.characterSessionDiagnostics();
        if (alive) setDiagnostics(value);
      } catch {
        if (alive) setDiagnostics(null);
      }
    })();
    return () => {
      alive = false;
    };
  }, [refreshKey, reload, advanced]);

  const projection = view?.projection ?? null;
  const remote = (view?.members ?? []).filter((m) => m.remote);
  // 安全狀態壓過同步狀態：緊急停止中，同步「技術上」也許還好好的，但綠色徽章
  // 讀起來就是「一切正常」，會和正下方的固定安全句互相矛盾。有安全訊息時一律
  // 不給綠色（句子本身照實不改：已同步就是已同步）。
  const tone = view?.safetyNote ? "bad" : (projection?.tone ?? "muted");

  return (
    <div className="character-sync" data-testid="character-sync" role="region" aria-label="角色同步">
      <div className="row space-between wrap">
        <strong>同步</strong>
        {projection ? (
          <Badge kind={tone}>{projection.headline}</Badge>
        ) : (
          <Badge kind="muted">正在讀取</Badge>
        )}
      </div>
      <p className="muted small">
        {projection ? projection.detail : "正在讀取角色目前的同步狀態…"}
      </p>
      {view?.safetyNote && (
        <p className="cap-card-error" role="alert">
          {view.safetyNote}
        </p>
      )}
      {remote.length > 0 && (
        <ul className="plain-list small" aria-label="同步中的裝置">
          {remote.map((m, index) => (
            <li key={`${m.name}-${index}`}>
              <span>{m.name}</span>
              <span className="muted">：</span>
              <span>{characterSyncPresenceLabel(m.presence)}</span>
            </li>
          ))}
        </ul>
      )}
      {view?.lastInteraction && (
        <p className="small" role="status">
          最近互動：{view.lastInteraction}
        </p>
      )}
      <div className="connect-area-actions">
        <button onClick={() => setReload((n) => n + 1)}>重新檢查</button>
      </div>
      {advanced && (
        <details className="tech-details">
          <summary>連接診斷</summary>
          {diagnostics ? (
            <ul className="plain-list small">
              <li>revision {diagnostics.revision}</li>
              <li>sequence {diagnostics.sequence}</li>
              <li>sessionEpoch {diagnostics.sessionEpoch}</li>
              <li>
                eventLog {diagnostics.eventLog.len}/{diagnostics.eventLog.cap}
              </li>
              {Object.entries(diagnostics.counters).map(([key, value]) => (
                <li key={key}>
                  {key} {value}
                </li>
              ))}
              {diagnostics.storeNote && <li>storeNote {diagnostics.storeNote}</li>}
            </ul>
          ) : (
            <p className="muted small">目前讀不到連接診斷。</p>
          )}
        </details>
      )}
    </div>
  );
}
