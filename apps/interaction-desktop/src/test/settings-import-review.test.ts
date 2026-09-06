import { describe, expect, it } from "vitest";
import "../character/adapters";
import { parseCompanionSettingsImport } from "../companion/settingsTransfer";

describe("N4 review: corrupted nested backup fields", () => {
  it.each([
    ["id", 42],
    ["name", 42],
    ["name", {}],
    ["palette", ["maid-classic"]],
  ])("rejects malformed familiar %s before producing a settings patch", (key, value) => {
    const backup = {
      kind: "companion-settings", schemaVersion: 1, companionPack: "shu-maid",
      companionFamiliars: [{ id: "f1", name: "小夥伴", palette: "maid-classic", [key as string]: value }],
    };
    expect(() => parseCompanionSettingsImport(backup, {
      knownCharacterIds: ["shu-maid"], entrypointFor: () => "shu-rig",
    })).toThrow();
  });
});
