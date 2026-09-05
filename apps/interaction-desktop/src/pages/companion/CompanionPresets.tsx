// 陪伴方式摘要（M3 §4.1 首屏第二件事）：一句話說明現在是哪一個預設，外加三個一鍵檔位。
//
// 預設只是**既有欄位**的組合（見 `src/companion/presets.ts`）：套用只寫表現程度、勿擾與
// 主動對話模式，不覆蓋其它自訂值、不改費用上限、不啟用任何權限、不更換 AI 幫手。
// 不吻合任何檔位時顯示「自訂」並逐項列出有效值——不把三個語意壓成一個字。

import {
  COMPANION_PRESETS,
  presetDefinition,
  type CompanionPresetChoice,
  type CompanionPresetId,
} from "../../companion/presets";

export function CompanionPresetRow({
  choice,
  effectiveLines,
  busy,
  onApply,
}: {
  choice: CompanionPresetChoice;
  /** 「自訂」時要逐項顯示的有效值。 */
  effectiveLines: string[];
  busy: boolean;
  onApply: (id: CompanionPresetId) => void;
}) {
  const def = choice === "custom" ? null : presetDefinition(choice);
  return (
    <>
      <div className="character-preset-row" role="group" aria-label="陪伴方式">
        {COMPANION_PRESETS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className={preset.id === choice ? "primary" : undefined}
            aria-pressed={preset.id === choice}
            disabled={busy}
            onClick={() => onApply(preset.id)}
          >
            {preset.label}
          </button>
        ))}
      </div>
      <p className="small" data-testid="companion-preset-summary">
        目前：<strong>{def ? def.label : "自訂"}</strong>
        {def ? `——${def.summary}` : `——${effectiveLines.join("・")}`}
      </p>
      <p className="muted small">
        這三個檔位只改變表現程度、勿擾與主動說話的模式；不會改動費用或次數上限、不會啟用任何權限，
        也不會更換指定的 AI 幫手。
      </p>
    </>
  );
}
