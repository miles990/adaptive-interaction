// 陪伴方式摘要（M3 §4.1 首屏第二件事）：一句話說明現在是哪一個預設，外加三個一鍵檔位。
//
// 預設只是**既有欄位**的組合（見 `src/companion/presets.ts`）：套用只寫表現程度、勿擾與
// 主動對話模式，不覆蓋其它自訂值、不改費用上限、不啟用任何權限、不更換 AI 幫手。
// 不吻合任何檔位時顯示「自訂」並逐項列出有效值——不把三個語意壓成一個字。
//
// 套用是**兩段寫入**（桌面偏好 → 後端模式），所以這一列還要說得出交易的狀態
// （`src/companion/applyPresetPlan.ts`）：
//   - `applied`：整組生效，才可以高亮那個檔位；
//   - `partially-applied`：第一段寫進去、第二段沒送到 → 說出來並給「補送」；
//   - `recovering`：正在補送上一次沒完成的第二段；
//   - `custom-effective`：不吻合任何檔位 → 逐項有效值；
//   - `unverified`：讀不回有效值 → 明說無法確認，任何檔位都不高亮。
// 只有 `applied` 會高亮檔位：半套用、補送中、無法確認都不得讓使用者以為整組生效了。

import {
  COMPANION_PRESETS,
  presetDefinition,
  type CompanionPresetChoice,
  type CompanionPresetId,
} from "../../companion/presets";
import type { CompanionPresetStatus } from "../../companion/applyPresetPlan";

export function CompanionPresetRow({
  choice,
  effectiveLines,
  busy,
  status,
  pendingPresetId,
  onApply,
  onRetry,
}: {
  choice: CompanionPresetChoice;
  /** 「自訂」時要逐項顯示的有效值。 */
  effectiveLines: string[];
  busy: boolean;
  /** 兩段寫入的交易狀態（見 `applyPresetPlan.ts`）。 */
  status: CompanionPresetStatus;
  /** 半套用時，marker 指的是哪一個檔位（文案要說得出名字）。 */
  pendingPresetId?: string | null;
  onApply: (id: CompanionPresetId) => void;
  /** 補送第二段（只送 mode；冪等）。 */
  onRetry?: () => void;
}) {
  const def = choice === "custom" ? null : presetDefinition(choice);
  const pendingDef = pendingPresetId ? presetDefinition(pendingPresetId) : null;
  // 只有整組確認生效才高亮：其餘狀態一律不讓按鈕替不確定的事背書。
  const highlighted = status === "applied" ? choice : null;
  return (
    <>
      <div className="character-preset-row" role="group" aria-label="陪伴方式">
        {COMPANION_PRESETS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className={preset.id === highlighted ? "primary" : undefined}
            aria-pressed={preset.id === highlighted}
            disabled={busy}
            onClick={() => onApply(preset.id)}
          >
            {preset.label}
          </button>
        ))}
      </div>
      {status === "unverified" ? (
        <p className="small" data-testid="companion-preset-summary" role="status">
          <strong>無法確認目前生效值</strong>
          ——讀不到主動說話的設定，所以上面的檔位都不代表現在生效的狀態。
        </p>
      ) : (
        <p className="small" data-testid="companion-preset-summary">
          目前：<strong>{def ? def.label : "自訂"}</strong>
          {def ? `——${def.summary}` : `——${effectiveLines.join("・")}`}
        </p>
      )}
      {status === "recovering" && (
        <p className="small" role="status" data-testid="companion-preset-recovering">
          正在補送上次未完成的設定…
        </p>
      )}
      {status === "partially-applied" && (
        <div className="row wrap" data-testid="companion-preset-partial">
          <span className="small" role="status">
            上次套用「{pendingDef ? pendingDef.label : "陪伴方式"}」時，主動說話的設定沒送到，可以再按一次補送。
          </span>
          <button type="button" disabled={busy} onClick={() => onRetry?.()}>
            補送
          </button>
        </div>
      )}
      <p className="muted small">
        這三個檔位只改變表現程度、勿擾與主動說話的模式；不會改動費用或次數上限、不會啟用任何權限，
        也不會更換指定的 AI 幫手。
      </p>
    </>
  );
}
