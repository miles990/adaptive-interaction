// 感測不靜默的另一半：離開了「使用中」清單、卻沒有任何人確認它停了的擷取。
//
// 它不是「還在感測」（那是 `SensorBanner` 的事），也不是「已經停了」——所以這一
// 行只說「有幾筆沒有人確認」，逐筆與人為確認在「連接與權限」。狀態列（tray）有
// 同一句的 Rust 版本（`src-tauri/src/host_safety.rs` 的 `unresolved_text`）。

export function UnresolvedStopsBanner({
  summary,
  onOpen,
}: {
  /** `projectUnresolvedStops(status).summary`；`null` ＝沒有這一類紀錄。 */
  summary: string | null;
  onOpen: () => void;
}) {
  if (!summary) return null;
  return (
    <div className="sensor-banner" role="status" data-testid="unresolved-stops-summary">
      {summary}，到「連接與權限」逐筆看。
      <button style={{ marginLeft: 8 }} onClick={onOpen}>
        前往查看
      </button>
    </div>
  );
}
