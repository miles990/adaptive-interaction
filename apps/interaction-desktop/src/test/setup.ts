import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// The Tauri bridge does not exist under jsdom; every test mocks `invoke`.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => {
    throw new Error("unmocked invoke — tests must stub api calls");
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
