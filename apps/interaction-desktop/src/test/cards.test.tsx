// 能力卡片與權限地圖的元件測試：人類名稱、保守 fallback、徽章、三區分佈。

import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { CapabilityCard } from "../components/CapabilityCard";
import { PermissionMap } from "../pages/HomePage";
import { HumanCard } from "../api";

function card(overrides: Partial<HumanCard>): HumanCard {
  return {
    id: "test.cap",
    kind: "actuator",
    displayName: "測試能力",
    nameSource: "catalog",
    shortDescription: "一句說明。",
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

describe("CapabilityCard", () => {
  it("shows the human name and description, not the technical id", () => {
    render(
      <CapabilityCard
        card={card({ displayName: "桌面通知", shortDescription: "在這台電腦上顯示一則通知。" })}
        advanced={false}
        onChanged={() => {}}
      />
    );
    expect(screen.getByText("桌面通知")).toBeInTheDocument();
    expect(screen.getByText("在這台電腦上顯示一則通知。")).toBeInTheDocument();
    // 一般模式卡片不顯示技術 id 與 hash。
    expect(screen.queryByText("test.cap")).not.toBeInTheDocument();
    expect(screen.queryByText("0123456789abcdef")).not.toBeInTheDocument();
  });

  it("undescribed capability shows the conservative notice", () => {
    render(
      <CapabilityCard
        card={card({
          shortDescription: undefined,
          undescribed: true,
          conservativeNotice:
            "提供者尚未提供完整的資料與影響說明。在你確認前，系統不會自動使用這項能力。",
        })}
        advanced={false}
        onChanged={() => {}}
      />
    );
    expect(screen.getByText(/尚未提供完整的資料與影響說明/)).toBeInTheDocument();
  });

  it("actuator card is honest about the confirmation ceiling", () => {
    render(
      <CapabilityCard
        card={card({
          effect: {
            externalSideEffect: false,
            physicalEffect: false,
            interruptiveness: "medium",
            reversible: true,
            confirmationLevel: "delivered",
          },
        })}
        advanced={false}
        onChanged={() => {}}
      />
    );
    expect(screen.getByText(/已送達/)).toBeInTheDocument();
    expect(screen.getByText(/無法確認你是否已經看見/)).toBeInTheDocument();
  });

  it("renders backend-resolved badges verbatim", () => {
    render(
      <CapabilityCard
        card={card({
          badges: [
            { key: "local-only", label: "僅限本機", tone: "ok" },
            { key: "physical", label: "實體效果", tone: "warn" },
          ],
        })}
        advanced={false}
        onChanged={() => {}}
      />
    );
    expect(screen.getByText("僅限本機")).toBeInTheDocument();
    expect(screen.getByText("實體效果")).toBeInTheDocument();
  });
});

describe("PermissionMap", () => {
  it("distributes capabilities into know / do / must-ask zones", () => {
    const receptors = [card({ kind: "receptor", id: "r1", displayName: "任務狀態", availability: "available" })];
    const actuators = [
      card({ id: "a1", displayName: "對話訊息", availability: "available" }),
      card({
        id: "a2",
        displayName: "震動",
        availability: "disabled",
        consent: { required: true },
      }),
    ];
    render(<PermissionMap receptors={receptors} actuators={actuators} tools={[]} />);
    const know = screen.getByText("AI 可以知道").closest(".perm-zone")!;
    const doZone = screen.getByText("AI 可以做").closest(".perm-zone")!;
    const ask = screen.getByText("AI 必須先問").closest(".perm-zone")!;
    expect(know.textContent).toContain("任務狀態");
    expect(doZone.textContent).toContain("對話訊息");
    expect(ask.textContent).toContain("震動");
    // 需同意的能力絕不出現在「可以做」。
    expect(doZone.textContent).not.toContain("震動");
  });
});
