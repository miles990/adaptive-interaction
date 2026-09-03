// 「停止所有感測」的誠實投影（statusProjection.projectSensorStop）。
//
// 誠實階梯：送出 ≠ 已停止；裝置沒回覆是「結果不確定」，不是成功也不是失敗；
// 讀不到狀態就說讀不到。只有「重讀到的 activeSensors 是空的」而且回報沒有不確定，
// 才可以說「已停止感測」。認不得的感測種類不外洩原始 id。

import { describe, expect, it } from "vitest";
import { projectSensorStop, sensorKindLabel, sensorStartedByLabel } from "../statusProjection";

describe("sensorKindLabel", () => {
  it("認得的種類翻成人話", () => {
    expect(sensorKindLabel("microphone")).toBe("麥克風");
    expect(sensorKindLabel("iphone.mic-level")).toBe("麥克風");
    expect(sensorKindLabel("camera.main")).toBe("攝影機");
    expect(sensorKindLabel("location")).toBe("定位");
  });

  it("認不得的種類不猜、也不外洩原始 id", () => {
    expect(sensorKindLabel("iphone.motion")).toBe("其他感測器");
    expect(sensorKindLabel("")).toBe("其他感測器");
  });
});

// regression（App.tsx 感測橫幅）：一般模式曾經把 `startedBy` 原樣印進
// 「（由 … 啟動…）」，使用者看到的是 `iphone:iphone-87b4…` 這種內部裝置 id。
describe("sensorStartedByLabel", () => {
  it("iPhone：不外洩裝置 id；知道名字就用名字", () => {
    expect(sensorStartedByLabel("iphone:iphone-87b4c1d2")).toBe("你的 iPhone");
    expect(sensorStartedByLabel("iphone:d1", "阿明的 iPhone")).toBe("阿明的 iPhone");
    // 空白名字不算名字。
    expect(sensorStartedByLabel("iphone:d1", "   ")).toBe("你的 iPhone");
    expect(sensorStartedByLabel("iphone:d1", null)).toBe("你的 iPhone");
  });

  it("本機來源：user 是「你」，desktop／api／cli 是「這台電腦」", () => {
    expect(sensorStartedByLabel("user")).toBe("你");
    expect(sensorStartedByLabel("desktop")).toBe("這台電腦");
    expect(sensorStartedByLabel("api")).toBe("這台電腦");
    expect(sensorStartedByLabel("cli")).toBe("這台電腦");
  });

  it("認不得的來源說「系統」——不猜，更不得冒充成使用者自己開的", () => {
    expect(sensorStartedByLabel("recipe:auto-listen")).toBe("系統");
    expect(sensorStartedByLabel("")).toBe("系統");
    expect(sensorStartedByLabel(undefined)).toBe("系統");
    expect(sensorStartedByLabel(null)).toBe("系統");
    // 任何形狀都不得把原始 id 漏出去。
    for (const raw of ["iphone-87b4c1d2", "provider.mobile.d1", "  "]) {
      expect(sensorStartedByLabel(raw)).toBe("系統");
    }
  });
});

describe("projectSensorStop", () => {
  it("全部停了才算成功", () => {
    expect(projectSensorStop({ stopped: true, uncertain: false, devices: [] }, [])).toEqual({
      ok: true,
      message: "已停止感測。",
    });
  });

  it("舊 daemon 的 {stopped:true}：以重讀到的空清單為準", () => {
    expect(projectSensorStop({ stopped: true }, []).ok).toBe(true);
  });

  it("重讀仍有感測在用：不算成功，且只說人話種類", () => {
    const out = projectSensorStop({ stopped: true }, [{ kind: "iphone.mic-level" }]);
    expect(out.ok).toBe(false);
    expect(out.message).toContain("仍在使用中");
    expect(out.message).toContain("麥克風");
    expect(out.message).not.toContain("iphone.mic-level");
    expect(out.message).not.toContain("已停止感測");
  });

  it("重複的種類只說一次", () => {
    const out = projectSensorStop({ stopped: true }, [
      { kind: "microphone" },
      { kind: "iphone.mic-level" },
    ]);
    expect(out.message).toContain("麥克風");
    expect(out.message.match(/麥克風/g)).toHaveLength(1);
  });

  it("裝置沒回覆：結果不確定（帶裝置名稱）", () => {
    const out = projectSensorStop(
      {
        stopped: true,
        uncertain: true,
        local: true,
        devices: [{ deviceId: "d1", name: "iPhone", outcome: "unreachable", waitedMs: 3000 }],
      },
      []
    );
    expect(out.ok).toBe(false);
    expect(out.message).toContain("結果不確定");
    expect(out.message).toContain("iPhone");
  });

  it("uncertain 旗標缺席但有裝置沒回報 stopped，一樣是不確定", () => {
    const out = projectSensorStop({ stopped: true, devices: [{ outcome: "unknown" }] }, []);
    expect(out.ok).toBe(false);
    expect(out.message).toContain("結果不確定");
    expect(out.message).toContain("某台裝置");
  });

  it("後端明說沒停成功：不算成功", () => {
    expect(projectSensorStop({ stopped: false }, []).ok).toBe(false);
  });

  it("讀不到狀態：說無法確認，不猜成功也不猜失敗", () => {
    const out = projectSensorStop({ stopped: true }, null);
    expect(out.ok).toBe(false);
    expect(out.message).toContain("無法確認感測狀態");
    expect(out.message).not.toContain("已停止感測");
  });

  it("回報形狀不可信也不會爆（undefined／字串／陣列）", () => {
    for (const bogus of [undefined, null, "ok", 42, []]) {
      expect(() => projectSensorStop(bogus, [])).not.toThrow();
    }
    // 重讀清單仍在使用中時，形狀再怪也不得說成功。
    expect(projectSensorStop(undefined, [{ kind: "microphone" }]).ok).toBe(false);
  });
});
