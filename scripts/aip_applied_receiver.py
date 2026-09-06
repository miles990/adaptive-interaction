"""Bounded semantic receiver used by the Serial pty simulator (never real hardware).

Consumes the committed consumer schema, preserves numeric wire literals for the
canonical hash, and commits a candidate only after all checks have succeeded.
"""
import hashlib
import json
import math
from pathlib import Path

PROFILE = "aip.applied/1"

class WireFloat(float):
    def __new__(cls, literal):
        value = super().__new__(cls, literal)
        value.literal = literal
        return value

class WireInt(int):
    def __new__(cls, literal):
        value = super().__new__(cls, literal)
        value.literal = literal
        return value

def loads(text):
    return json.loads(text, parse_float=WireFloat, parse_int=WireInt,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite")))

def canonical(value):
    if isinstance(value, dict):
        return "{" + ",".join(canonical(k) + ":" + canonical(value[k]) for k in sorted(value)) + "}"
    if isinstance(value, list):
        return "[" + ",".join(canonical(v) for v in value) + "]"
    if hasattr(value, "literal"):
        return value.literal
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)

def merge(base, patch):
    if not isinstance(patch, dict):
        return patch
    out = dict(base) if isinstance(base, dict) else {}
    for key, value in patch.items():
        if value is None:
            out.pop(key, None)
        else:
            out[key] = merge(out.get(key), value)
    return out

def matches(value, schema, root):
    if schema is True: return True
    if schema is False: return False
    if "$ref" in schema:
        target = root
        for part in schema["$ref"].split("/")[1:]:
            target = target[part.replace("~1", "/").replace("~0", "~")]
        if not matches(value, target, root): return False
    if "not" in schema and matches(value, schema["not"], root): return False
    if "const" in schema and value != schema["const"]: return False
    if "enum" in schema and value not in schema["enum"]: return False
    for keyword, predicate in (("anyOf", any), ("allOf", all)):
        if keyword in schema and not predicate(matches(value, s, root) for s in schema[keyword]): return False
    if "oneOf" in schema and sum(matches(value, s, root) for s in schema["oneOf"]) != 1: return False
    kind = schema.get("type")
    types = kind if isinstance(kind, list) else [kind]
    def has_type(name):
        return {"object": isinstance(value, dict), "array": isinstance(value, list),
                "string": isinstance(value, str), "boolean": isinstance(value, bool),
                "number": isinstance(value, (int, float)) and not isinstance(value, bool),
                "integer": isinstance(value, int) and not isinstance(value, bool),
                "null": value is None, None: True}.get(name, False)
    if not any(has_type(t) for t in types): return False
    if isinstance(value, dict):
        if any(key not in value for key in schema.get("required", [])): return False
        properties = schema.get("properties", {})
        for key, item in value.items():
            if key in properties:
                if not matches(item, properties[key], root): return False
            elif not matches(item, schema.get("additionalProperties", True), root): return False
    if isinstance(value, list):
        if len(value) > schema.get("maxItems", 32768): return False
        if "items" in schema and not all(matches(v, schema["items"], root) for v in value): return False
    if isinstance(value, str):
        if not schema.get("minLength", 0) <= len(value) <= schema.get("maxLength", 32768): return False
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if not math.isfinite(value): return False
        if not schema.get("minimum", -math.inf) <= value <= schema.get("maximum", math.inf): return False
        if schema.get("x-nonNegativeZero") and math.copysign(1, value) < 0: return False
    return True

class Receiver:
    def __init__(self):
        self.schema = json.loads((Path(__file__).resolve().parents[1] / "schemas/semantic-state-1.0.schema.json").read_text())
        self.local = None

    def valid(self, value):
        def bounded(item, depth=0):
            if item is None or depth > 7: return False
            if isinstance(item, str): return len(item) <= 2000
            if isinstance(item, dict): return all(len(k) <= 2000 and bounded(v, depth + 1) for k, v in item.items())
            if isinstance(item, list): return all(bounded(v, depth + 1) for v in item)
            return not isinstance(item, float) or math.isfinite(item)
        return bounded(value) and len(canonical(value).encode()) <= 32768 and matches(value, self.schema, self.schema)

    def apply(self, envelope):
        context = envelope.get("stateApplied")
        if not isinstance(context, dict) or context.get("profile") != PROFILE: return None
        if envelope.get("source", {}).get("kind") != "runtime": return None
        if envelope.get("sessionId") != context.get("sessionId") or envelope.get("messageId") != context.get("messageId"): return None
        body = envelope.get("payload", {})
        items = body.get("patches", []) if body.get("kind") == "patches" else [body]
        if not isinstance(items, list) or len(items) > 512: return None
        candidate = self.local
        for item in items:
            if not isinstance(item, dict): return None
            kind = item.get("kind", "patch")
            epoch, revision = item.get("sessionEpoch"), item.get("revision")
            if any(isinstance(v, bool) or not isinstance(v, int) or not 0 <= v <= 2**53 - 1 for v in [epoch, revision]): return None
            if candidate and candidate["sessionId"] != context["sessionId"]: return None
            if candidate and epoch == candidate["epoch"] and revision <= candidate["revision"] and item.get("reason") != "recovery":
                continue
            if kind == "snapshot":
                if candidate and epoch != candidate["epoch"] and item.get("reason") != "session-reset": return None
                state = item.get("state")
            elif kind == "patch":
                if not candidate or epoch != candidate["epoch"] or item.get("baseRevision", envelope.get("baseRevision")) != candidate["revision"]: return None
                state = merge(candidate["state"], item.get("patch"))
            else: return None
            if not self.valid(state): return None
            digest = hashlib.sha256(canonical(state).encode()).hexdigest()
            if digest != item.get("hash"): return None
            candidate = dict(state=state, sessionId=context["sessionId"], epoch=epoch, revision=revision, hash=digest)
        if not candidate or any(candidate[key] != context.get(key) for key in ["sessionId", "epoch", "revision", "hash"]): return None
        self.local = candidate
        return context
