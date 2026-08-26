// 首次設定精靈：歡迎頁保證文字、預選只含低風險本機能力、草稿保存。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const mockApi = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(async () => ({
    mode: "simple",
    locale: "zh-TW",
    customNames: {},
    schemaVersion: "1.0",
  })),
  uiPrefsPatch: vi.fn(),
  pauseGet: vi.fn(async () => ({ paused: false })),
  capabilitiesHuman: vi.fn(async () => ({
    locale: "zh-TW",
    catalogVersion: 1,
    capabilityVersion: 1,
    generatedAt: "",
    constraints: [],
    receptors: [
      {
        id: "task.lifecycle",
        kind: "receptor",
        displayName: "任務狀態",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "list-checks",
        colorRole: "input",
        category: "task",
        beginnerRecommended: true,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h1",
        data: { personalData: false, sensitivity: "none", source: "local", leavesDevice: false, retention: "session" },
      },
      {
        id: "camera.main",
        kind: "receptor",
        displayName: "攝影機",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "video",
        colorRole: "input",
        category: "sensor",
        beginnerRecommended: false,
        badges: [],
        consent: { required: true },
        undescribed: false,
        availability: "disabled",
        requiresConsent: true,
        manifestHash: "h2",
        data: { personalData: true, sensitivity: "high", source: "device", leavesDevice: "unknown", retention: "unknown" },
      },
    ],
    actuators: [
      {
        id: "conversation",
        kind: "actuator",
        displayName: "對話訊息",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "message-square",
        colorRole: "output",
        category: "message",
        beginnerRecommended: true,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h3",
        effect: { externalSideEffect: false, physicalEffect: false, interruptiveness: "low", reversible: true, confirmationLevel: "delivered" },
      },
      {
        id: "webhook.output",
        kind: "actuator",
        displayName: "Webhook 傳送",
        nameSource: "catalog",
        shortDescription: "x",
        descriptionSource: "catalog",
        icon: "cloud-upload",
        colorRole: "output",
        category: "integration",
        beginnerRecommended: false,
        badges: [],
        consent: { required: false },
        undescribed: false,
        availability: "available",
        requiresConsent: false,
        manifestHash: "h4",
        effect: { externalSideEffect: true, physicalEffect: false, interruptiveness: "none", reversible: false, confirmationLevel: "acknowledged" },
      },
    ],
    toolOperations: [],
  })),
  onboardingGet: vi.fn(async () => ({
    completed: false,
    draft: null,
    starterRecipes: [
      { id: "starter-task-complete", title: "任務完成時，用最低干擾方式回應" },
      { id: "starter-quiet-log", title: "安靜時段只記錄、不打擾" },
    ],
  })),
  onboardingDraft: vi.fn(async () => ({})),
  onboardingCommit: vi.fn(async () => ({ completed: true })),
}));

vi.mock("../api", async (importOriginal) => {
  const original = await importOriginal<Record<string, unknown>>();
  return { ...original, api: mockApi };
});

import { AppStateProvider } from "../appstate";
import { Onboarding } from "../pages/Onboarding";

function renderWizard() {
  return render(
    <AppStateProvider ready={true} refreshKey={0}>
      <Onboarding onDone={() => {}} onSkip={() => {}} />
    </AppStateProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("Onboarding", () => {
  it("welcome step states the core guarantees", async () => {
    renderWizard();
    await screen.findByText("歡迎使用自適應互動");
    expect(screen.getByText(/能力存在不等於 AI 自動獲得權限/)).toBeInTheDocument();
    expect(screen.getByText(/緊急停止/)).toBeInTheDocument();
  });

  it("pre-selects only low-risk local capabilities; camera and external writes stay off", async () => {
    renderWizard();
    await screen.findByText("歡迎使用自適應互動");
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));

    // 感知來源步驟：任務狀態被預選；攝影機（需同意）根本不在清單。
    await screen.findByText("AI 可以知道什麼？");
    const task = screen.getByRole("checkbox", { name: /任務狀態/ });
    expect(task).toBeChecked();
    expect(screen.queryByText("攝影機")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await screen.findByText("AI 可以怎麼回應？");
    const conv = screen.getByRole("checkbox", { name: /對話訊息/ });
    expect(conv).toBeChecked();
    // 對外寫入能力出現在清單，但不預選。
    const webhook = screen.getByRole("checkbox", { name: /Webhook 傳送/ });
    expect(webhook).not.toBeChecked();
  });

  it("saves a draft as steps advance", async () => {
    renderWizard();
    await screen.findByText("歡迎使用自適應互動");
    await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    await waitFor(() => expect(mockApi.onboardingDraft).toHaveBeenCalled());
    const draft = mockApi.onboardingDraft.mock.calls.at(-1)![0] as { step: number };
    expect(draft.step).toBe(1);
  });

  it("commit sends enable lists and policy through the backend", async () => {
    renderWizard();
    await screen.findByText("歡迎使用自適應互動");
    // 一路到最後一步。
    for (let i = 0; i < 6; i++) {
      await userEvent.click(screen.getByRole("button", { name: "下一步" }));
    }
    await screen.findByRole("heading", { name: "確認" });
    await userEvent.click(screen.getByRole("button", { name: "完成設定" }));
    await waitFor(() => expect(mockApi.onboardingCommit).toHaveBeenCalled());
    const commit = mockApi.onboardingCommit.mock.calls[0][0] as Record<string, unknown>;
    expect(commit["enableReceptors"]).toContain("task.lifecycle");
    // 對外寫入未被啟用。
    expect(commit["enableActuators"]).not.toContain("webhook.output");
    expect((commit["policyPatch"] as Record<string, unknown>)["initiative"]).toBe("suggest");
  });
});
