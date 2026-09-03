// 單元測試（ia-settings-012）：canonical quiet-hours builder。
// 防止未來任一呼叫點（Onboarding、CompanionPage 或其他）改壞這份共用清單，
// 讓桌面角色又被靜音回內建預設。

import { describe, expect, it } from "vitest";
import { buildQuietHoursPatch, QUIET_SILENCED_CHANNELS } from "../quietHours";

describe("quietHours", () => {
  it("QUIET_SILENCED_CHANNELS 不含 desktop-pet，且非空", () => {
    expect(QUIET_SILENCED_CHANNELS.length).toBeGreaterThan(0);
    expect(QUIET_SILENCED_CHANNELS).not.toContain("desktop-pet");
  });

  it("buildQuietHoursPatch 帶入 start/end，回傳明確的靜音清單（不是空陣列）", () => {
    const patch = buildQuietHoursPatch("22:00", "08:00");
    expect(patch).toEqual({
      start: "22:00",
      end: "08:00",
      silencedChannels: ["audio", "haptic", "notification", "light"],
    });
    expect(patch.silencedChannels.length).toBeGreaterThan(0);
    expect(patch.silencedChannels).not.toContain("desktop-pet");
  });

  it("每次呼叫回傳新陣列（不共享同一個底層陣列參考），避免呼叫端互相污染", () => {
    const a = buildQuietHoursPatch("22:00", "08:00");
    const b = buildQuietHoursPatch("23:00", "07:00");
    expect(a.silencedChannels).not.toBe(b.silencedChannels);
    expect(a.silencedChannels).not.toBe(QUIET_SILENCED_CHANNELS);
  });
});
