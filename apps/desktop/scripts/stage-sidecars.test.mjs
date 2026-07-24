import { describe, expect, it, vi } from "vitest";

import {
  sidecarFileName,
  sidecarPaths,
  stageBuiltSidecars,
} from "./stage-sidecars.mjs";

describe("sidecar staging paths", () => {
  it("adds the Windows extension after the target triple", () => {
    expect(
      sidecarFileName("wokrouter", "x86_64-pc-windows-msvc", "win32"),
    ).toBe("wokrouter-x86_64-pc-windows-msvc.exe");
    expect(
      sidecarFileName("wokrouterd", "aarch64-apple-darwin", "darwin"),
    ).toBe("wokrouterd-aarch64-apple-darwin");
  });

  it("maps release binaries into Tauri’s target-suffixed directory", () => {
    expect(
      sidecarPaths({
        workspaceRoot: "C:/work/wokrouter",
        tauriDir: "C:/work/wokrouter/apps/desktop/src-tauri",
        binaryName: "wokrouter",
        targetTriple: "x86_64-pc-windows-msvc",
        platform: "win32",
      }),
    ).toEqual({
      source: "C:\\work\\wokrouter\\target\\release\\wokrouter.exe",
      destination:
        "C:\\work\\wokrouter\\apps\\desktop\\src-tauri\\binaries\\wokrouter-x86_64-pc-windows-msvc.exe",
    });
  });

  it("fails before copying when a built lifecycle binary is missing", () => {
    const copyFileSync = vi.fn();

    expect(() =>
      stageBuiltSidecars({
        workspaceRoot: "/work/wokrouter",
        tauriDir: "/work/wokrouter/apps/desktop/src-tauri",
        targetTriple: "x86_64-unknown-linux-gnu",
        platform: "linux",
        fileSystem: {
          copyFileSync,
          existsSync: () => false,
          mkdirSync: vi.fn(),
        },
      }),
    ).toThrow("Built sidecar is missing: /work/wokrouter/target/release/wokrouter");
    expect(copyFileSync).not.toHaveBeenCalled();
  });
});
