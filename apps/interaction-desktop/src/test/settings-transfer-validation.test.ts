import {describe, expect, it} from "vitest";
import "../character/adapters";
import {parseCompanionSettingsImport} from "../companion/settingsTransfer";
const base = {kind:"companion-settings",schemaVersion:1,companionPack:"plain-text",characterId:"plain-text"};
const options = {knownCharacterIds:["plain-text"],entrypointFor:() => "text"};
describe("settings backup schema at the production import boundary", () => {
  it.each([
    ["companionName",5], ["companionPack",5], ["characterId",false],
    ["companionPersona",{}], ["companionExpressiveness",5], ["companionScene",[]],
    ["companionPlay","false"], ["companionCursorPlay",1], ["companionApproach",null],
    ["companionDeskMove",{}], ["companionFamiliars",{}],
  ])("rejects a present %s of the wrong type before any patch can be applied", (key,value) => {
    expect(() => parseCompanionSettingsImport({...base,[key as string]:value},options)).toThrow();
  });
  it("restores an explicitly blank name rather than silently retaining a later name", () => {
    expect(parseCompanionSettingsImport({...base,companionName:""},options)).toHaveProperty("companionName","");
  });
});
