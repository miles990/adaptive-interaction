// 感測不靜默：只要有感測在跑，控制中心頂端就有這條橫幅。

import React from "react";
import type { SensorUse } from "../api";
import { sensorKindLabel, sensorStartedByLabel } from "../statusProjection";

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
  sensors: readonly SensorUse[];
  advanced: boolean;
  onStopAll: () => void;
}) {
  if (sensors.length === 0) return null;
  return (
    <div className="sensor-banner" role="status">
      {sensors.map((s) => (
        <span key={`${s.kind}#${s.startedBy}`}>
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
