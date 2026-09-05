// 陪伴預設按鈕群的可及性（M5 §6）。
//
// 為什麼需要單獨一支：這一列**只在 Tauri 控制中心存在**（瀏覽器模式看到的是誠實降級
// 文字），所以 Playwright 的 a11y spec 驗不到它——瀏覽器 e2e 永遠不會渲染出這三顆
// 按鈕。它的可及性只能在這裡（jsdom）釘住。
//
// 釘住的事實：
//   1. 按鈕群有可及名稱「陪伴方式」，三顆按鈕各自有非空的可及名稱。
//   2. **每一顆**按鈕都一直帶著 `aria-pressed`（true／false）——不是只有被選中的那顆
//      才有。少了 false，螢幕閱讀器使用者只會聽到「按鈕 安靜」，聽不出它沒被選中。
//   3. 只有 `applied` 會有一顆 `aria-pressed="true"`。半套用／補送中／無法確認／自訂
//      一律**零**顆——視覺高亮（`primary`）與 ARIA 必須說同一件事，不得一邊亮著一邊
//      對輔助科技說「沒有選中」（誠實階梯的可及性版本）。
//   4. 狀態是用**文字**講的，不是只靠顏色或動畫：五種狀態各自有一句讀得出來的話，
//      而且在 Reduced Motion 下一字不變。
//   5. 這一列用 `aria-pressed` 表達選中狀態，**不**同時掛 `aria-current`：兩套並存會
//      讓輔助科技聽到兩個互相矛盾的狀態；切換按鈕的正確 ARIA 是 `aria-pressed`。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { CompanionPresetRow } from "../pages/companion/CompanionPresets";
import { COMPANION_PRESETS } from "../companion/presets";
import type { CompanionPresetStatus } from "../companion/applyPresetPlan";

afterEach(() => {
  cleanup();
  document.documentElement.classList.remove("reduce-motion");
});

const EFFECTIVE_LINES = ["表現程度：自然", "勿擾：關", "主動說話：必要時"];

function renderRow(overrides: {
  status: CompanionPresetStatus;
  choice?: "quiet" | "natural" | "lively" | "custom";
  busy?: boolean;
  pendingPresetId?: string | null;
}) {
  return render(
    <CompanionPresetRow
      choice={overrides.choice ?? "quiet"}
      effectiveLines={EFFECTIVE_LINES}
      busy={overrides.busy ?? false}
      status={overrides.status}
      pendingPresetId={overrides.pendingPresetId ?? null}
      onApply={vi.fn()}
      onRetry={vi.fn()}
    />
  );
}

function group(): HTMLElement {
  return screen.getByRole("group", { name: "陪伴方式" });
}

function buttons(): HTMLElement[] {
  return within(group()).getAllByRole("button");
}

/** 目前 `aria-pressed="true"` 的按鈕名字（可能是空陣列）。 */
function pressedNames(): string[] {
  return buttons()
    .filter((b) => b.getAttribute("aria-pressed") === "true")
    .map((b) => (b.textContent ?? "").trim());
}

const ALL_STATUSES: CompanionPresetStatus[] = [
  "applied",
  "partially-applied",
  "recovering",
  "custom-effective",
  "unverified",
];

describe("陪伴預設按鈕群：可及名稱與選中狀態", () => {
  it("按鈕群有名字，三顆按鈕都有非空的可及名稱", () => {
    renderRow({ status: "applied" });
    const names = buttons().map((b) => (b.textContent ?? "").trim());
    expect(names).toEqual(COMPANION_PRESETS.map((p) => p.label));
    for (const name of names) expect(name.length).toBeGreaterThan(0);
  });

  it("每一顆按鈕在每一種狀態下都帶著 aria-pressed（沒被選中要說得出來）", () => {
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({ status, choice: status === "custom-effective" ? "custom" : "quiet" });
      for (const b of buttons()) {
        expect(
          b.getAttribute("aria-pressed"),
          `狀態 ${status}：按鈕「${b.textContent}」少了 aria-pressed`
        ).toMatch(/^(true|false)$/);
      }
    }
  });

  it("只有 applied 會有一顆被按下；其餘四種狀態一顆都不得高亮", () => {
    cleanup();
    renderRow({ status: "applied", choice: "quiet" });
    expect(pressedNames()).toEqual(["安靜"]);

    for (const status of ALL_STATUSES.filter((s) => s !== "applied")) {
      cleanup();
      // 刻意仍然傳一個吻合的檔位：不確定的時候，**檔位吻合也不准高亮**。
      renderRow({ status, choice: "quiet", pendingPresetId: "quiet" });
      expect(pressedNames(), `狀態 ${status} 不得高亮任何檔位`).toEqual([]);
    }
  });

  it("視覺高亮（primary）與 aria-pressed 永遠說同一件事", () => {
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({ status, choice: status === "custom-effective" ? "custom" : "natural" });
      for (const b of buttons()) {
        const highlighted = b.classList.contains("primary");
        const pressed = b.getAttribute("aria-pressed") === "true";
        expect(
          highlighted,
          `狀態 ${status}：按鈕「${b.textContent}」的視覺高亮與 aria-pressed 不一致`
        ).toBe(pressed);
      }
    }
  });

  it("不混用 aria-current：切換按鈕的狀態只由 aria-pressed 表達", () => {
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({ status });
      for (const b of buttons()) {
        expect(
          b.hasAttribute("aria-current"),
          `狀態 ${status}：按鈕「${b.textContent}」同時掛了 aria-current`
        ).toBe(false);
      }
    }
  });

  it("交易進行中：按鈕被停用時，停用狀態本身是可讀的（disabled 而非只是變灰）", () => {
    renderRow({ status: "recovering", busy: true });
    for (const b of buttons()) expect(b).toBeDisabled();
    // 而且畫面上要有一句話說明為什麼現在不能按。
    expect(screen.getByTestId("companion-preset-recovering")).toHaveAttribute("role", "status");
  });
});

describe("陪伴預設按鈕群：四態文案讀得到，且在 Reduced Motion 下一字不變", () => {
  /** 每一種狀態必須在畫面上讀得到的那一句（不是靠顏色或動畫傳達）。 */
  const EXPECTED_TEXT: Record<CompanionPresetStatus, RegExp> = {
    applied: /目前：/,
    "partially-applied": /主動說話的設定沒送到/,
    recovering: /正在補送上次未完成的設定/,
    "custom-effective": /自訂/,
    unverified: /無法確認目前生效值/,
  };

  it("五種狀態各自有一句讀得出來的話", () => {
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({
        status,
        choice: status === "custom-effective" ? "custom" : "quiet",
        pendingPresetId: "lively",
      });
      const body = document.body.textContent ?? "";
      expect(body, `狀態 ${status} 沒有可讀的說明`).toMatch(EXPECTED_TEXT[status]);
    }
  });

  it("不確定的三種狀態（半套用／補送中／無法確認）都用 role=status 播報", () => {
    cleanup();
    renderRow({ status: "unverified" });
    expect(screen.getByTestId("companion-preset-summary")).toHaveAttribute("role", "status");

    cleanup();
    renderRow({ status: "recovering" });
    expect(screen.getByTestId("companion-preset-recovering")).toHaveAttribute("role", "status");

    cleanup();
    renderRow({ status: "partially-applied", pendingPresetId: "quiet" });
    const partial = screen.getByTestId("companion-preset-partial");
    expect(within(partial).getByRole("status")).toBeInTheDocument();
    // 半套用要說得出是**哪一個**檔位沒完成，並且給得出補送的按鈕。
    expect(partial.textContent ?? "").toContain("安靜");
    expect(within(partial).getByRole("button", { name: "補送" })).toBeInTheDocument();
  });

  it("Reduced Motion 下，五種狀態的文字一字不變", () => {
    const withoutReduce: Record<string, string> = {};
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({
        status,
        choice: status === "custom-effective" ? "custom" : "quiet",
        pendingPresetId: "lively",
      });
      withoutReduce[status] = document.body.textContent ?? "";
      expect(withoutReduce[status].trim().length).toBeGreaterThan(0);
    }

    document.documentElement.classList.add("reduce-motion");
    for (const status of ALL_STATUSES) {
      cleanup();
      renderRow({
        status,
        choice: status === "custom-effective" ? "custom" : "quiet",
        pendingPresetId: "lively",
      });
      expect(
        document.body.textContent ?? "",
        `減少動態改變了狀態 ${status} 說的話`
      ).toBe(withoutReduce[status]);
    }
  });
});
