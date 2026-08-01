import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getCoreStatus, startCore, stopCore } from "./control";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("desktop control bridge", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("accepts a valid WokCore status DTO", async () => {
    vi.mocked(invoke).mockResolvedValue({
      state: "running",
      runtime_channel: "development",
      version: "0.1.0",
      management_api_major: 1,
      capabilities: ["service.status"],
      phase: "running",
      active_requests: 2,
    });

    await expect(getCoreStatus()).resolves.toEqual({
      state: "running",
      runtime_channel: "development",
      version: "0.1.0",
      management_api_major: 1,
      capabilities: ["service.status"],
      phase: "running",
      active_requests: 2,
    });
    expect(invoke).toHaveBeenCalledWith("core_status");
  });

  it("rejects a malformed WokCore status DTO", async () => {
    vi.mocked(invoke).mockResolvedValue({
      state: "healthy",
      runtime_channel: "production",
      capabilities: "service.status",
    });

    await expect(getCoreStatus()).rejects.toThrow("Invalid WokCore status");
  });

  it.each([
    ["missing", undefined],
    ["unknown", "preview"],
  ])("rejects a %s runtime channel", async (_case, runtimeChannel) => {
    vi.mocked(invoke).mockResolvedValue({
      state: "stopped",
      runtime_channel: runtimeChannel,
      capabilities: [],
    });

    await expect(getCoreStatus()).rejects.toThrow("Invalid WokCore status");
  });

  it.each([
    ["pid", 41],
    ["path", "C:\\private\\wokcore.exe"],
    ["executable", "C:\\private\\wokcore.exe"],
  ])("rejects a status DTO exposing %s", async (field, value) => {
    vi.mocked(invoke).mockResolvedValue({
      state: "stopped",
      runtime_channel: "production",
      capabilities: [],
      [field]: value,
    });

    await expect(getCoreStatus()).rejects.toThrow("Invalid WokCore status");
  });

  it("delegates lifecycle actions to narrow Tauri commands", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await expect(startCore()).resolves.toBeUndefined();
    await expect(stopCore()).resolves.toBeUndefined();
    expect(invoke).toHaveBeenNthCalledWith(1, "start_core");
    expect(invoke).toHaveBeenNthCalledWith(2, "stop_core");
  });
});
