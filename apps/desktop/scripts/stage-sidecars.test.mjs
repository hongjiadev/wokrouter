import { describe, expect, it, vi } from "vitest";

import * as sidecars from "./stage-sidecars.mjs";

const {
  cargoBuildArguments,
  resolveTargetTriple,
  sidecarFileName,
  sidecarPaths,
  stageBuiltSidecars,
  tauriTargetTriple,
} = sidecars;

const supportedTauriTargets = [
  ["windows", "x86_64", "x86_64-pc-windows-msvc"],
  ["macos", "x86_64", "x86_64-apple-darwin"],
  ["macos", "aarch64", "aarch64-apple-darwin"],
  ["linux", "x86_64", "x86_64-unknown-linux-gnu"],
  ["linux", "aarch64", "aarch64-unknown-linux-gnu"],
];

describe("target resolution", () => {
  it.each(supportedTauriTargets)(
    "maps Tauri %s/%s to %s",
    (platform, arch, expected) => {
      expect(tauriTargetTriple({ platform, arch })).toBe(expected);
    },
  );

  it("fails fast for an unsupported Tauri platform and architecture", () => {
    expect(() =>
      tauriTargetTriple({ platform: "windows", arch: "aarch64" }),
    ).toThrow("Unsupported Tauri target: windows/aarch64");
  });

  it("prefers explicit, Cargo, and Tauri targets before the native fallback", () => {
    expect(
      resolveTargetTriple({
        explicitTarget: "aarch64-apple-darwin",
        cargoBuildTarget: "x86_64-unknown-linux-gnu",
        tauriPlatform: "windows",
        tauriArch: "x86_64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("aarch64-apple-darwin");
    expect(
      resolveTargetTriple({
        cargoBuildTarget: "x86_64-unknown-linux-gnu",
        tauriPlatform: "windows",
        tauriArch: "x86_64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("x86_64-unknown-linux-gnu");
    expect(
      resolveTargetTriple({
        tauriPlatform: "macos",
        tauriArch: "aarch64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("aarch64-apple-darwin");
    expect(
      resolveTargetTriple({
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("x86_64-pc-windows-msvc");
  });

  it("rejects unsupported explicit, Cargo, and native target triples", () => {
    expect(() =>
      resolveTargetTriple({
        explicitTarget: "aarch64-pc-windows-msvc",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toThrow("Unsupported target triple: aarch64-pc-windows-msvc");
    expect(() =>
      resolveTargetTriple({
        cargoBuildTarget: "wasm32-unknown-unknown",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toThrow("Unsupported target triple: wasm32-unknown-unknown");
    expect(() =>
      resolveTargetTriple({ hostTargetTriple: "i686-pc-windows-msvc" }),
    ).toThrow("Unsupported target triple: i686-pc-windows-msvc");
  });
});

describe("sidecar staging paths", () => {
  it("derives the executable extension from the target triple", () => {
    expect(
      sidecarFileName("wokrouter", "x86_64-pc-windows-msvc"),
    ).toBe("wokrouter-x86_64-pc-windows-msvc.exe");
    expect(
      sidecarFileName("wokrouterd", "aarch64-apple-darwin"),
    ).toBe("wokrouterd-aarch64-apple-darwin");
  });

  it("reads the target-specific Cargo release directory", () => {
    expect(
      sidecarPaths({
        workspaceRoot: "C:/work/wokrouter",
        tauriDir: "C:/work/wokrouter/apps/desktop/src-tauri",
        binaryName: "wokrouter",
        targetTriple: "x86_64-pc-windows-msvc",
        hostPlatform: "win32",
      }),
    ).toEqual({
      source:
        "C:\\work\\wokrouter\\target\\x86_64-pc-windows-msvc\\release\\wokrouter.exe",
      destination:
        "C:\\work\\wokrouter\\apps\\desktop\\src-tauri\\binaries\\wokrouter-x86_64-pc-windows-msvc.exe",
    });
  });

  it("uses the target extension on a different host platform", () => {
    expect(
      sidecarPaths({
        workspaceRoot: "/work/wokrouter",
        tauriDir: "/work/wokrouter/apps/desktop/src-tauri",
        binaryName: "wokrouter",
        targetTriple: "x86_64-pc-windows-msvc",
        hostPlatform: "linux",
      }),
    ).toEqual({
      source:
        "/work/wokrouter/target/x86_64-pc-windows-msvc/release/wokrouter.exe",
      destination:
        "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-x86_64-pc-windows-msvc.exe",
    });
  });

  it("builds both lifecycle binaries for the resolved target", () => {
    expect(cargoBuildArguments("aarch64-apple-darwin")).toEqual([
      "build",
      "--locked",
      "--release",
      "--target",
      "aarch64-apple-darwin",
      "-p",
      "wokrouter-cli",
      "-p",
      "wokrouter-daemon",
    ]);
  });

  it("fails before copying when a built lifecycle binary is missing", () => {
    const copyFileSync = vi.fn();

    expect(() =>
      stageBuiltSidecars({
        workspaceRoot: "/work/wokrouter",
        tauriDir: "/work/wokrouter/apps/desktop/src-tauri",
        targetTriple: "x86_64-unknown-linux-gnu",
        hostPlatform: "linux",
        fileSystem: {
          copyFileSync,
          existsSync: () => false,
          mkdirSync: vi.fn(),
        },
      }),
    ).toThrow(
      "Built sidecar is missing for x86_64-unknown-linux-gnu: /work/wokrouter/target/x86_64-unknown-linux-gnu/release/wokrouter",
    );
    expect(copyFileSync).not.toHaveBeenCalled();
  });
});
