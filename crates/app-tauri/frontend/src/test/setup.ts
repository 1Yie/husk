import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import { installDomMocks, resetDomMocks } from "./dom";
import { resetTauri } from "./tauri";

vi.mock("@tauri-apps/api/core", async () => {
  const { tauri } = await import("./tauri");
  return { invoke: tauri.invoke, convertFileSrc: (path: string) => `asset://localhost/${path}` };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { tauri } = await import("./tauri");
  return { listen: tauri.listen };
});
vi.mock("@tauri-apps/api/window", async () => {
  const { tauri } = await import("./tauri");
  return { getCurrentWindow: () => tauri.window };
});

beforeEach(() => {
  resetTauri();
  installDomMocks();
  localStorage.clear();
  sessionStorage.clear();
});
afterEach(() => {
  cleanup();
  resetDomMocks();
  resetTauri();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});
