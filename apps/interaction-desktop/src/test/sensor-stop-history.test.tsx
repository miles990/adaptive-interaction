// N3-UI-STOP-HISTORY: exercise the actual App -> Shell -> SensorBanner stop
// handler. The status changes only after stop; initial render cannot supply the
// history/health that the command must reread. HomePage has its own wiring test.
import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

vi.mock("../api", async (importOriginal) => ({
  ...await importOriginal<typeof import("../api")>(),
  onRuntimeReady: async (ready: () => void) => { ready(); return () => {}; },
  onRuntimeError: async () => () => {},
  onRuntimeEvent: async () => () => {},
}));
vi.mock("../desktop", async (importOriginal) => ({
  ...await importOriginal<typeof import("../desktop")>(),
  bootstrapSupervisor: async () => null,
  onCloseRequested: async () => () => {},
  onNavigate: async () => () => {},
  onSupervisorState: async () => () => {},
  onTrayActionError: async () => () => {},
}));
vi.mock("../pages/HomePage", () => ({ HomePage: () => <div>Home content</div> }));

import App from "../App";
import { api } from "../api";
import { resetCharacterNameForTests } from "../characterName";

afterEach(() => vi.restoreAllMocks());

describe("N3-UI-STOP-HISTORY: App sensor stop wiring", () => {
  it.each([
    ["historical capture", { unresolvedStops: [{ sourceId: "old", generation: 1, sensors: ["microphone"] }] }],
    ["recovery health", { unresolvedStopHealth: { recoveryUnknown: true } }],
  ])("uses fresh %s and preserves its reminder", async (_name, evidence) => {
    resetCharacterNameForTests();
    let stopped = false;
    vi.spyOn(api, "status").mockImplementation(async () => ({
      onboardingCompleted: true,
      activeSensors: stopped ? [] : [{ kind: "microphone", startedAt: "2026-09-06T00:00:00Z" }],
      ...(stopped ? evidence : {}),
    }));
    vi.spyOn(api, "sensorsStop").mockImplementation(async () => {
      stopped = true;
      return { stopped: true, uncertain: false, devices: [] };
    });
    vi.spyOn(api, "uiPrefsGet").mockResolvedValue({ mode: "simple", locale: "zh-TW", customNames: {}, schemaVersion: "1.0" });
    vi.spyOn(api, "pauseGet").mockResolvedValue({ paused: false });
    vi.spyOn(api, "capabilitiesHuman").mockResolvedValue({ receptors: [], actuators: [], toolOperations: [] } as never);
    vi.spyOn(api, "characterManifest").mockResolvedValue({ characterId: "text", displayName: { "zh-TW": "角色" } } as never);
    vi.spyOn(api, "activityInbox").mockResolvedValue({ items: [], count: 0, pendingCount: 0 });
    vi.spyOn(api, "eventsRecent").mockResolvedValue([]);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "立即停止" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("尚未確認");
    expect(screen.queryByText("已停止感測。")).not.toBeInTheDocument();
    expect(screen.getByTestId("unresolved-stops-summary")).toBeInTheDocument();
  });
});
