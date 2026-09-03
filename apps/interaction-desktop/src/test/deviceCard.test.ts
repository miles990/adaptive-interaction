// 手機卡片的純函式（phoneCardModel 與兩句誠實回報）。
// 這裡不渲染任何東西：只驗證「三份真實狀態 → 一張卡」的推導不會發明資料，
// 以及「送出≠停止」「有回應≠功能可用」這兩條誠實階梯落在文案上。

import { describe, expect, it } from "vitest";
import type { HumanCapabilities, HumanCard, SensorUse } from "../api";
import {
  isMobileProviderId,
  phoneCardModel,
  phonePermissionAlerts,
  stopSensorsMessage,
  stripPhonePrefix,
  testMessage,
} from "../pages/connect/PhoneDeviceCard";

function card(overrides: Partial<HumanCard>): HumanCard {
  return {
    id: "test.cap",
    kind: "actuator",
    displayName: "測試能力",
    nameSource: "catalog",
    descriptionSource: "catalog",
    icon: "bell",
    colorRole: "output",
    category: "notification",
    beginnerRecommended: false,
    badges: [],
    consent: { required: false },
    undescribed: false,
    availability: "available",
    requiresConsent: false,
    manifestHash: "0123456789abcdef",
    ...overrides,
  };
}

function human(overrides: Partial<HumanCapabilities> = {}): HumanCapabilities {
  return {
    locale: "zh-TW",
    catalogVersion: 1,
    capabilityVersion: 1,
    generatedAt: "2026-09-01T00:00:00Z",
    constraints: [],
    receptors: [
      card({ id: "iphone.battery", kind: "receptor", displayName: "iPhone 電量與前景" }),
      card({
        id: "iphone.mic-level",
        kind: "receptor",
        displayName: "iPhone 環境音量",
        availability: "disabled",
        requiresConsent: true,
        consent: { required: true },
      }),
      card({ id: "time.now", kind: "receptor", displayName: "系統時間" }),
    ],
    actuators: [
      card({ id: "iphone.haptic", displayName: "iPhone 觸覺回饋" }),
      card({ id: "notify.desktop", displayName: "桌面通知" }),
    ],
    toolOperations: [],
    ...overrides,
  };
}

const DEVICE = {
  deviceId: "d1",
  name: "Alex 的 iPhone",
  model: "iPhone 15",
  pairedAt: "2026-08-01T00:00:00Z",
  connected: true,
};

function sensor(overrides: Partial<SensorUse>): SensorUse {
  return {
    kind: "iphone.mic-level",
    startedAt: "2026-09-01T00:00:00Z",
    startedBy: "iphone:d1",
    purpose: "iPhone 麥克風音量（僅音量值）",
    ...overrides,
  };
}

describe("phoneCardModel：三份真實狀態合成一張卡", () => {
  it("可以提供／可以執行只取手機的能力，其他能力不混進來；可用狀態照抄", () => {
    const model = phoneCardModel(DEVICE, human(), []);
    expect(model.provides.map((p) => p.id)).toEqual(["iphone.battery", "iphone.mic-level"]);
    expect(model.performs.map((p) => p.id)).toEqual(["iphone.haptic"]);
    // 卡片本身已經說了是哪一台手機，能力名稱不再重複「iPhone 」。
    expect(model.provides.map((p) => p.name)).toEqual(["電量與前景", "環境音量"]);
    expect(model.performs[0].name).toBe("觸覺回饋");
    // 可用狀態照抄能力卡，不改寫。
    expect(model.provides[1].availability).toBe("disabled");
    expect(model.provides[1].requiresConsent).toBe(true);
  });

  it("能力清單還沒載入時是空的，不編造能力", () => {
    const model = phoneCardModel(DEVICE, null, []);
    expect(model.provides).toEqual([]);
    expect(model.performs).toEqual([]);
  });

  it("目前使用中的感測：startedBy 精確比對這一台，別台的不算", () => {
    const other = phoneCardModel(DEVICE, human(), [sensor({ startedBy: "iphone:d2" })]);
    expect(other.activeSensing).toEqual([]);
    const mine = phoneCardModel(DEVICE, human(), [sensor({})]);
    expect(mine.activeSensing).toEqual(["麥克風音量"]);
  });

  it("手機自報為「開」的感測也算使用中；重複的只列一次，關著的不列", () => {
    const model = phoneCardModel(
      { ...DEVICE, sensors: { micLevel: true, motion: true, battery: false } },
      human(),
      [sensor({})]
    );
    expect(model.activeSensing).toEqual(["麥克風音量", "動作"]);
  });

  it("介面不認得的自報感測鍵原樣顯示，不發明名稱", () => {
    const model = phoneCardModel({ ...DEVICE, sensors: { heartRate: true } }, human(), []);
    expect(model.activeSensing).toEqual(["heartRate"]);
  });

  it("手機沒回報 iOS 權限時是 null（未知），不當成已授權", () => {
    expect(phoneCardModel(DEVICE, human(), []).permissions).toBeNull();
    expect(phonePermissionAlerts(phoneCardModel(DEVICE, human(), []))).toEqual([
      "Alex 的 iPhone：手機尚未回報 iPhone 上的權限（未知）",
    ]);
    // 未連線的手機現在確認不了什麼，不列進「需要你確認」（卡片上仍寫未連線／未回報）。
    expect(
      phonePermissionAlerts(phoneCardModel({ ...DEVICE, connected: false }, human(), []))
    ).toEqual([]);
  });

  it("回報了就照抄，未允許的列進「需要你確認」", () => {
    const model = phoneCardModel(
      { ...DEVICE, permissions: { microphone: "denied", location: "granted" } },
      human(),
      []
    );
    expect(model.permissions).toEqual([
      { key: "microphone", label: "麥克風", state: "已拒絕", needsAttention: true },
      { key: "location", label: "位置", state: "已授權", needsAttention: false },
    ]);
    expect(phonePermissionAlerts(model)).toEqual(["在 Alex 的 iPhone 上尚未允許：麥克風（已拒絕）"]);
  });

  it("進階模式用的原始診斷欄位：連線狀態與感測旗標照抄，不經過人話翻譯", () => {
    const model = phoneCardModel(
      { ...DEVICE, connected: true, sensors: { motion: true, battery: false } },
      human(),
      []
    );
    expect(model.connectedRaw).toBe(true);
    expect(model.sensorFlagsRaw).toEqual({ motion: true, battery: false });
  });

  it("手機沒回報感測旗標時，原始值是 null（不是空物件，不假裝知道）", () => {
    const model = phoneCardModel(DEVICE, human(), []);
    expect(model.sensorFlagsRaw).toBeNull();
    expect(model.connectedRaw).toBe(true);
  });

  it("沒有名字的手機不留空白；未連線照實回報", () => {
    const model = phoneCardModel({ deviceId: "d9", connected: false }, human(), []);
    expect(model.name).toBe("iPhone");
    expect(model.connected).toBe(false);
    expect(model.model).toBeNull();
    expect(model.pairedAt).toBeNull();
  });

  it("stripPhonePrefix：只去掉前綴；整個名字就是 iPhone 時保留原名", () => {
    expect(stripPhonePrefix("iPhone 通知")).toBe("通知");
    expect(stripPhonePrefix("iPhone")).toBe("iPhone");
    expect(stripPhonePrefix("桌面通知")).toBe("桌面通知");
  });

  it("手機在來源清單裡的那一列認得出來（不重複列）", () => {
    expect(isMobileProviderId("provider.mobile.d1")).toBe(true);
    expect(isMobileProviderId("provider.esp32.desk")).toBe(false);
    expect(isMobileProviderId(undefined)).toBe(false);
  });
});

describe("誠實文案：送出≠停止、有回應≠功能可用", () => {
  it("停止感測：送出後只說「已要求停止」，不得說已停止", () => {
    const line = stopSensorsMessage("Alex 的 iPhone", {
      deviceId: "d1",
      requested: true,
      connected: true,
      outcome: "unknown",
    });
    expect(line).toContain("已要求停止（以手機回報為準）");
    expect(line).toContain("結果不確定");
    expect(line).not.toContain("已停止感測");
  });

  it("停止感測：手機回報 stopped 才可以說已停止；送不到就說未送達", () => {
    expect(
      stopSensorsMessage("我的手機", {
        deviceId: "d1",
        requested: true,
        connected: true,
        outcome: "stopped",
      })
    ).toBe("我的手機：已停止（手機回報已停止）。");
    expect(
      stopSensorsMessage("我的手機", {
        deviceId: "d1",
        requested: false,
        connected: false,
        outcome: "unreachable",
      })
    ).toBe("我的手機：未送達（手機未連線），感測狀態未變。");
  });

  it("測試連接：有回應要附上往返時間並說清楚它只證明連線還在", () => {
    const line = testMessage({ deviceId: "d1", ok: true, connected: true, latencyMs: 42 });
    expect(line).toContain("有回應（42 ms）");
    expect(line).toContain("不代表手機 App 的功能都能用");
    expect(line).not.toContain("已測試");
  });

  it("測試連接：沒有回應是「結果不確定」，不是失敗也不是成功", () => {
    const line = testMessage({
      deviceId: "d1",
      ok: false,
      connected: true,
      uncertain: true,
      reason: "3 秒內沒有回覆",
    });
    expect(line).toContain("沒有回應（結果不確定）");
    expect(line).toContain("3 秒內沒有回覆");
    expect(line).not.toContain("已測試");
    expect(line).not.toContain("失敗");
  });
});
