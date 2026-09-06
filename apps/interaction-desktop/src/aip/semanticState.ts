// A complete-state boundary; merge-patch nulls are interpreted before this runs.
import { canonicalJson } from "./canonical";
import { SEMANTIC_STATE_DOUBLE_PATHS } from "./generated";
import { SEMANTIC_STATE_SCHEMA, type SemanticStateWire } from "./semanticStateGenerated";

type Schema = { readonly [key: string]: unknown };
const schema: Schema = SEMANTIC_STATE_SCHEMA;
const limits = SEMANTIC_STATE_SCHEMA["x-limits"];
declare const checkedState: unique symbol;
/** The wire object is retained intact, including unknown keys and raw-number metadata. */
export type ValidatedSemanticState = Record<string, unknown> & SemanticStateWire & { readonly [checkedState]: true };
function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function bounded(value: unknown, depth = 1): boolean {
  if (depth > limits.maxDepth || value === null || value === undefined) return false;
  if (typeof value === "string") return Array.from(value).length <= limits.maxStringChars;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value === "boolean") return true;
  if (Array.isArray(value)) return value.every((child) => bounded(child, depth + 1));
  return object(value) && Object.entries(value).every(([key, child]) => Array.from(key).length <= limits.maxStringChars && bounded(child, depth + 1));
}
/** Same structural RFC3339 policy as chrono::DateTime::parse_from_rfc3339.
 * Seconds 60 are retained (no leap-table inference); no Date.parse normalization. */
function validTimestamp(value: string): boolean {
  const parts = /^([0-9]{4})-([0-9]{2})-([0-9]{2})[Tt ]([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.[0-9]+)?(?:[Zz]|([+−-])([0-9]{2}):([0-9]{2}))$/.exec(value);
  if (!parts || parts[0] !== value) return false;
  const [, y, m, d, h, min, sec, , offsetH, offsetM] = parts;
  const year = Number(y), month = Number(m), day = Number(d);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1]
    && Number(h) <= 23 && Number(min) <= 59 && Number(sec) <= 60
    && (offsetH === undefined || (Number(offsetH) <= 23 && Number(offsetM) <= 59));
}
function matches(value: unknown, rule: Schema): boolean {
  if (typeof rule["$ref"] === "string") {
    const name = rule["$ref"].split("/").pop() ?? "";
    const defs = schema["$defs"] as Record<string, Schema>;
    return !!defs[name] && matches(value, defs[name]);
  }
  if (Array.isArray(rule["oneOf"]) && rule["oneOf"].filter((branch) => matches(value, branch as Schema)).length !== 1) return false;
  if (Array.isArray(rule["anyOf"]) && !rule["anyOf"].some((branch) => matches(value, branch as Schema))) return false;
  if (object(rule["not"]) && matches(value, rule["not"])) return false;
  if ("const" in rule && rule["const"] !== value) return false;
  if (Array.isArray(rule["enum"]) && !rule["enum"].includes(value)) return false;
  switch (rule["type"]) {
    case "object": {
      if (!object(value)) return false;
      if (Array.isArray(rule["required"]) && rule["required"].some((key) => !Object.prototype.hasOwnProperty.call(value, String(key)))) return false;
      return !object(rule["properties"]) || Object.entries(rule["properties"]).every(([key, child]) => !Object.prototype.hasOwnProperty.call(value, key) || matches(value[key], child as Schema));
    }
    case "array": return Array.isArray(value) && (typeof rule["maxItems"] !== "number" || value.length <= rule["maxItems"]) && (!object(rule["items"]) || value.every((child) => matches(child, rule["items"] as Schema)));
    case "string": return typeof value === "string" && (rule["format"] !== "date-time" || validTimestamp(value));
    case "boolean": return typeof value === "boolean";
    case "number": case "integer": return typeof value === "number" && Number.isFinite(value) && (rule["type"] !== "integer" || Number.isInteger(value)) && (typeof rule["minimum"] !== "number" || value >= rule["minimum"]) && (typeof rule["maximum"] !== "number" || value <= rule["maximum"]) && !(rule["x-nonNegativeZero"] === true && (Object.is(value, -0) || value < 0));
    default: return true;
  }
}
/** Validation does not coerce, default, serialize a DTO, or discard unknown data. */
export function validateSemanticState(value: unknown): ValidatedSemanticState | null {
  if (!bounded(value) || !matches(value, schema)) return null;
  if (new TextEncoder().encode(canonicalJson(value, SEMANTIC_STATE_DOUBLE_PATHS)).length > limits.maxBytes) return null;
  return value as ValidatedSemanticState;
}

/** Renderer projection has no authority and is never used to compute a state hash. */
export function projectSemanticState(state: ValidatedSemanticState) {
  return { characterId: state.characterId, mood: state.mood.kind, moodIntensity: state.mood.intensity,
    activity: state.activity, truth: state.truth.state, reducedMotion: state.reducedMotion,
    members: state.members, lastInteractionKind: state.lastInteraction?.kind };
}
