import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getDaemonStatus, startDaemon } from "./control";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("desktop control bridge", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("accepts a valid daemon status DTO", async () => {
    vi.mocked(invoke).mockResolvedValue({ state: "running", version: "0.1.0" });

    await expect(getDaemonStatus()).resolves.toEqual({
      state: "running",
      version: "0.1.0",
    });
    expect(invoke).toHaveBeenCalledWith("daemon_status");
  });

  it("rejects a malformed daemon status DTO", async () => {
    vi.mocked(invoke).mockResolvedValue({ state: "healthy", version: 1 });

    await expect(getDaemonStatus()).rejects.toThrow("Invalid daemon status");
  });

  it("delegates start to the narrow Tauri command", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await expect(startDaemon()).resolves.toBeUndefined();
    expect(invoke).toHaveBeenCalledWith("start_daemon");
  });
});
