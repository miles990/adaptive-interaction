// manifest.preferencesSchema → bounded 表單（boolean→開關、number/integer→滑桿或數字、
// string enum→下拉、string→文字＋maxLength）。只渲染 schema 宣告的屬性；不顯示任何
// rig 參數、通道或 manifest 原文。

import type { PreferencePropertySchema, PreferencesSchema } from "../../character/protocol";
import { Toggle } from "../../ui";
import { boundValue, MAX_PREFERENCE_PROPERTIES, type PreferenceValue, type PreferenceValues } from "./preferences";

function labelOf(key: string, prop: PreferencePropertySchema): string {
  return typeof prop.title === "string" && prop.title.trim().length > 0 ? prop.title.trim().slice(0, 48) : key;
}

function PreferenceField({
  id,
  prop,
  value,
  disabled,
  onChange,
}: {
  id: string;
  prop: PreferencePropertySchema;
  value: PreferenceValue;
  disabled: boolean;
  onChange: (value: PreferenceValue) => void;
}) {
  const label = labelOf(id, prop);
  const description =
    typeof prop.description === "string" && prop.description.trim().length > 0 ? (
      <span className="muted small">{prop.description.trim().slice(0, 200)}</span>
    ) : null;

  if (prop.type === "boolean") {
    return (
      <div className="character-pref-field" data-pref={id}>
        <Toggle checked={value === true} onChange={(on) => onChange(on)} label={label} />
        {description}
      </div>
    );
  }

  if (prop.type === "number" || prop.type === "integer") {
    const hasRange = typeof prop.minimum === "number" && typeof prop.maximum === "number";
    const step = prop.type === "integer" ? 1 : undefined;
    const numeric = typeof value === "number" ? value : Number(boundValue(prop, value));
    return (
      <label className="field-label character-pref-field" data-pref={id}>
        <span>
          {label}：{Number.isInteger(numeric) ? numeric : numeric.toFixed(2)}
        </span>
        {hasRange ? (
          <input
            type="range"
            min={prop.minimum}
            max={prop.maximum}
            step={step ?? (Math.abs((prop.maximum ?? 1) - (prop.minimum ?? 0)) <= 2 ? 0.05 : 1)}
            value={numeric}
            disabled={disabled}
            aria-label={label}
            onChange={(e) => onChange(boundValue(prop, e.target.value))}
          />
        ) : (
          <input
            type="number"
            min={prop.minimum}
            max={prop.maximum}
            step={step}
            value={numeric}
            disabled={disabled}
            aria-label={label}
            onChange={(e) => onChange(boundValue(prop, e.target.value))}
          />
        )}
        {description}
      </label>
    );
  }

  if (Array.isArray(prop.enum) && prop.enum.length > 0) {
    return (
      <label className="field-label character-pref-field" data-pref={id}>
        {label}
        <select
          value={String(value)}
          disabled={disabled}
          aria-label={label}
          onChange={(e) => onChange(boundValue(prop, e.target.value))}
        >
          {prop.enum.slice(0, 16).map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
        {description}
      </label>
    );
  }

  const maxLength = Math.min(typeof prop.maxLength === "number" ? prop.maxLength : 200, 200);
  return (
    <label className="field-label character-pref-field" data-pref={id}>
      {label}
      <input
        type="text"
        value={String(value)}
        maxLength={maxLength}
        disabled={disabled}
        aria-label={label}
        onChange={(e) => onChange(boundValue(prop, e.target.value))}
      />
      {description}
    </label>
  );
}

export function PreferencesForm({
  schema,
  values,
  disabled = false,
  onChange,
}: {
  schema: PreferencesSchema | undefined;
  values: PreferenceValues;
  disabled?: boolean;
  onChange: (key: string, value: PreferenceValue) => void;
}) {
  const entries = Object.entries(schema?.properties ?? {}).slice(0, MAX_PREFERENCE_PROPERTIES);
  if (entries.length === 0) {
    return <p className="muted small">這個角色沒有提供可調整的偏好。</p>;
  }
  return (
    <div className="character-pref-form" role="group" aria-label="角色偏好">
      {entries.map(([key, prop]) => (
        <PreferenceField
          key={key}
          id={key}
          prop={prop}
          value={values[key] ?? boundValue(prop, undefined)}
          disabled={disabled}
          onChange={(value) => onChange(key, value)}
        />
      ))}
    </div>
  );
}
