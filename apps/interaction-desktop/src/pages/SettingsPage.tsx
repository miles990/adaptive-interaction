// 設定：一般／進階模式切換（後端持久化）、重新執行首次設定精靈。

import React from "react";
import { useAppState } from "../appstate";
import { Section, Toggle } from "../ui";

export function SettingsPage({ onRerunOnboarding }: { onRerunOnboarding: () => void }) {
  const { prefs, setMode } = useAppState();
  const [error, setError] = React.useState<string | null>(null);

  return (
    <div>
      <Section title="顯示模式">
        <Toggle
          checked={prefs.mode === "advanced"}
          onChange={async (on) => {
            try {
              await setMode(on ? "advanced" : "simple");
              setError(null);
            } catch (e) {
              setError(String(e));
            }
          }}
          label="顯示進階功能"
        />
        <p className="muted small">
          進階模式會多出技術頁面（原始受器／動器／工具／配方 YAML／政策 JSON／時間軸），
          並在各頁顯示技術 ID 與原始資料。兩種模式使用相同的後端狀態與安全規則。
        </p>
        {error && <p className="cap-card-error" role="alert">{error}</p>}
      </Section>

      <Section title="首次設定">
        <p className="muted small">
          重新執行首次設定精靈，重新選擇感知來源、回應方式與互動偏好。已有的設定不會被清除，
          精靈套用時才會變更。
        </p>
        <button onClick={onRerunOnboarding}>重新執行首次設定</button>
      </Section>

      <Section title="關於名稱">
        <p className="muted small">
          你在各能力「詳情」中自訂的名稱只影響顯示，不影響行為或安全規則；
          目前有 {Object.keys(prefs.customNames).length} 個自訂名稱。
        </p>
      </Section>
    </div>
  );
}
