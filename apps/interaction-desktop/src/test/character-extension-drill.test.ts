// N5 uses the normal manifest validator, factory and renderer; no new adapter.
import { expect, it } from "vitest";
import raw from "../../../../examples/characters/drill-text/manifest.json";
import "../character/adapters";
import { validateImportedManifestText } from "../character/registry";
import { createBuiltinAdapter, registeredBuiltinAdapterIds, FALLBACK_ADAPTER_ID } from "../character/adapterRegistry";
import type { AdapterReceipt } from "../character/adapter";
import { TextCharacterAdapter } from "../character/adapters/text";
import { boundValues } from "../pages/character/preferences";
import { FIXED_SAFETY_LINES } from "../companion/packs";

it("N5 新角色包沿用文字 adapter：取得獨立身份、保留呈現偏好及固定安全語句", async () => {
  const before = [...registeredBuiltinAdapterIds()];
  const valid = validateImportedManifestText(JSON.stringify(raw));
  expect(valid.ok).toBe(true);
  if (!valid.ok) throw new Error(valid.errors.join(","));
  const container = document.createElement("div");
  const built = await createBuiltinAdapter("text", { characterId: valid.manifest.characterId, manifest: valid.manifest, textHost: container });
  expect(built.adapter).toBeInstanceOf(TextCharacterAdapter);
  expect(built.adapter.manifest.characterId).toBe("drill-text");
  await built.adapter.initialize({ now: () => 1000, reducedMotion: () => true, locale: "zh-TW", log: () => {} });
  built.adapter.show();
  const prefs = boundValues(valid.manifest.preferencesSchema, { motto: "保存這句話", unknown: true });
  expect(prefs).toEqual({ motto: "保存這句話" });
  built.adapter.reconfigure(prefs);
  const receipts: AdapterReceipt[] = [];
  built.adapter.perform({
    protocolVersion: "1.0", messageId: "drill-emergency", characterInstanceId: "drill-instance",
    timestamp: "2026-09-06T00:00:00Z", intent: "emergency", truthState: "emergency",
    priority: 100, interruptPolicy: "preempt", resumePolicy: "none", privacyClass: "internal",
    presentationHints: { message: "不能覆盖安全語句" },
  }, (receipt) => receipts.push(receipt));
  expect(container.textContent).toContain(FIXED_SAFETY_LINES.emergency);
  expect(receipts.map(r => r.status)).toEqual(["accepted", "started", "completed"]);
  built.adapter.dispose();
  const fallback = await createBuiltinAdapter(FALLBACK_ADAPTER_ID, { textHost: container });
  expect(fallback.adapter.manifest.characterId).toBe("plain-text");
  expect(registeredBuiltinAdapterIds()).toEqual(before);
  fallback.adapter.dispose();
});
