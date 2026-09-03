// PhoneDeviceCard 的「重新配對」與「連接診斷」。
//
// 重新配對：卡片不自己實作配對流程——只透過可選的 onRepair 把動作交給呼叫端
// （ConnectPage 導到既有的 iPhone 配對區）。未連線時尤其需要它，因為桌面的網路
// 位址一變，這台手機就必須重新配對（真機限制），所以離線原因旁要多一句提醒。
//
// 連接診斷：進階模式才把「原始：deviceId・pairedAt」升級成一個有標題的小區塊，
// 一併給連線狀態與手機自報感測旗標的原始值；一般模式完全不顯示這個區塊，
// 也不能在主要文案裡出現 Provider／Adapter／Manifest／Lease／UUID／GATT／
// MQTT／Serial／YAML／Token 這類技術黑話。

import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { PhoneCardModel, PhoneDeviceCard } from "../pages/connect/PhoneDeviceCard";

function model(overrides: Partial<PhoneCardModel> = {}): PhoneCardModel {
  return {
    deviceId: "d1",
    name: "Alex 的 iPhone",
    model: "iPhone 15",
    connected: true,
    pairedAt: "2026-08-01T00:00:00Z",
    provides: [],
    performs: [],
    activeSensing: [],
    permissions: null,
    connectedRaw: true,
    sensorFlagsRaw: { motion: true, battery: false },
    ...overrides,
  };
}

describe("PhoneDeviceCard：「重新配對」不自己實作配對，交給呼叫端", () => {
  it("主要操作有「重新配對」按鈕；點下去只呼叫 onRepair，卡片本身不跳出配對流程", () => {
    const onRepair = vi.fn();
    render(<PhoneDeviceCard model={model()} onRepair={onRepair} />);
    const card = screen.getByTestId("phone-card-d1");
    within(card).getByRole("button", { name: "重新配對" }).click();
    expect(onRepair).toHaveBeenCalledTimes(1);
  });

  it("沒傳 onRepair 就不顯示「重新配對」（不留死按鈕）", () => {
    render(<PhoneDeviceCard model={model()} />);
    expect(screen.queryByRole("button", { name: "重新配對" })).not.toBeInTheDocument();
  });

  it("未連線時：離線原因旁多一句「網路位址變了要重新配對」的提醒", () => {
    render(<PhoneDeviceCard model={model({ connected: false })} onRepair={vi.fn()} />);
    const card = screen.getByTestId("phone-card-d1");
    expect(within(card).getByText("手機未連線時送不出任何指令。")).toBeInTheDocument();
    expect(within(card).getByText("若桌面的網路位址變了，需要重新配對。")).toBeInTheDocument();
  });

  it("已連線時不顯示這句離線提醒", () => {
    render(<PhoneDeviceCard model={model({ connected: true })} onRepair={vi.fn()} />);
    const card = screen.getByTestId("phone-card-d1");
    expect(
      within(card).queryByText("若桌面的網路位址變了，需要重新配對。")
    ).not.toBeInTheDocument();
  });
});

describe("PhoneDeviceCard：「連接診斷」只在進階模式出現", () => {
  it("進階模式：標題「連接診斷」＋deviceId、pairedAt、連線狀態原始值、感測旗標原始值", () => {
    render(<PhoneDeviceCard model={model()} advanced />);
    const card = screen.getByTestId("phone-card-d1");
    expect(within(card).getByText("連接診斷")).toBeInTheDocument();
    const diagnostics = screen.getByTestId("phone-diagnostics-d1");
    const text = diagnostics.textContent ?? "";
    expect(text).toContain("d1");
    expect(text).toContain("2026-08-01T00:00:00Z".slice(0, 4)); // pairedAt 的年份有出現即可，不比對時區格式化
    expect(text).toContain("true"); // 連線狀態原始值（未經翻譯成「已連線」）
    expect(text).toContain("motion");
    expect(text).toContain("battery");
  });

  it("一般模式：完全不顯示「連接診斷」，也不外洩 deviceId", () => {
    render(<PhoneDeviceCard model={model()} advanced={false} />);
    const card = screen.getByTestId("phone-card-d1");
    expect(within(card).queryByText("連接診斷")).not.toBeInTheDocument();
    expect(screen.queryByTestId("phone-diagnostics-d1")).not.toBeInTheDocument();
    expect(card.textContent ?? "").not.toContain("d1");
  });

  it("一般模式主要文案不得出現技術黑話", () => {
    render(<PhoneDeviceCard model={model()} advanced={false} />);
    const card = screen.getByTestId("phone-card-d1");
    const text = card.textContent ?? "";
    for (const word of [
      "Provider",
      "Adapter",
      "Manifest",
      "Lease",
      "UUID",
      "GATT",
      "MQTT",
      "Serial",
      "YAML",
      "Token",
    ]) {
      expect(text, `一般模式不得出現「${word}」`).not.toContain(word);
    }
  });
});
