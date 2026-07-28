import { describe, expect, it, vi } from "vitest";

import * as sidecars from "./stage-sidecars.mjs";

const {
  cargoBuildArguments,
  resolveTargetTriple,
  resolveTargetTripleFromEnvironment,
  sidecarFileName,
  sidecarPaths,
  stageBuiltSidecars,
  tauriTargetTriple,
} = sidecars;

const supportedTauriTargets = [
  ["windows", "x86_64", "x86_64-pc-windows-msvc"],
  ["darwin", "x86_64", "x86_64-apple-darwin"],
  ["darwin", "aarch64", "aarch64-apple-darwin"],
  ["linux", "x86_64", "x86_64-unknown-linux-gnu"],
  ["linux", "aarch64", "aarch64-unknown-linux-gnu"],
];
const supportedTargetTriples = supportedTauriTargets.map(([, , triple]) => triple);

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

  it("prefers explicit, Cargo, direct Tauri, and platform targets before the native fallback", () => {
    expect(
      resolveTargetTriple({
        explicitTarget: "aarch64-apple-darwin",
        cargoBuildTarget: "x86_64-unknown-linux-gnu",
        tauriTargetTriple: "x86_64-apple-darwin",
        tauriPlatform: "windows",
        tauriArch: "x86_64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("aarch64-apple-darwin");
    expect(
      resolveTargetTriple({
        cargoBuildTarget: "x86_64-unknown-linux-gnu",
        tauriTargetTriple: "x86_64-apple-darwin",
        tauriPlatform: "windows",
        tauriArch: "x86_64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("x86_64-unknown-linux-gnu");
    expect(
      resolveTargetTriple({
        tauriTargetTriple: "aarch64-apple-darwin",
        tauriPlatform: "windows",
        tauriArch: "x86_64",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("aarch64-apple-darwin");
    expect(
      resolveTargetTriple({
        tauriPlatform: "darwin",
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

  it.each(supportedTargetTriples)(
    "accepts direct Tauri target triple %s",
    (tauriTargetTriple) => {
      expect(
        resolveTargetTriple({
          tauriTargetTriple,
          hostTargetTriple: "i686-pc-windows-msvc",
        }),
      ).toBe(tauriTargetTriple);
    },
  );

  it("rejects unsupported explicit, Cargo, direct Tauri, and native target triples", () => {
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
      resolveTargetTriple({
        tauriTargetTriple: "universal-apple-darwin",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toThrow("Unsupported target triple: universal-apple-darwin");
    expect(() =>
      resolveTargetTriple({ hostTargetTriple: "i686-pc-windows-msvc" }),
    ).toThrow("Unsupported target triple: i686-pc-windows-msvc");
  });
});

describe("target environment resolution", () => {
  it.each([
    [
      "WOKROUTER_TARGET_TRIPLE",
      { WOKROUTER_TARGET_TRIPLE: "aarch64-apple-darwin" },
      "aarch64-apple-darwin",
    ],
    [
      "CARGO_BUILD_TARGET",
      { CARGO_BUILD_TARGET: "x86_64-unknown-linux-gnu" },
      "x86_64-unknown-linux-gnu",
    ],
    [
      "TAURI_ENV_TARGET_TRIPLE",
      { TAURI_ENV_TARGET_TRIPLE: "x86_64-apple-darwin" },
      "x86_64-apple-darwin",
    ],
    [
      "TAURI_ENV_PLATFORM and TAURI_ENV_ARCH",
      { TAURI_ENV_PLATFORM: "darwin", TAURI_ENV_ARCH: "aarch64" },
      "aarch64-apple-darwin",
    ],
  ])("does not read the rustc host when %s is configured", (_, environment, expected) => {
    const readHostTargetTriple = vi.fn(() => "x86_64-pc-windows-msvc");

    expect(
      resolveTargetTripleFromEnvironment({
        environment,
        readHostTargetTriple,
      }),
    ).toBe(expected);
    expect(readHostTargetTriple).not.toHaveBeenCalled();
  });

  it("fails fast for an unsupported direct Tauri triple without reading the rustc host", () => {
    const readHostTargetTriple = vi.fn(() => "x86_64-pc-windows-msvc");

    expect(() =>
      resolveTargetTripleFromEnvironment({
        environment: { TAURI_ENV_TARGET_TRIPLE: "universal-apple-darwin" },
        readHostTargetTriple,
      }),
    ).toThrow("Unsupported target triple: universal-apple-darwin");
    expect(readHostTargetTriple).not.toHaveBeenCalled();
  });

  it("reads the rustc host only when no target source is configured", () => {
    const readHostTargetTriple = vi.fn(() => "x86_64-pc-windows-msvc");

    expect(
      resolveTargetTripleFromEnvironment({
        environment: {},
        readHostTargetTriple,
      }),
    ).toBe("x86_64-pc-windows-msvc");
    expect(readHostTargetTriple).toHaveBeenCalledOnce();
  });
});

describe("sidecar staging paths", () => {
  it("derives the executable extension from the target triple", () => {
    expect(
      sidecarFileName("wokrouter", "x86_64-pc-windows-msvc"),
    ).toBe("wokrouter-x86_64-pc-windows-msvc.exe");
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

  it("builds only the WokRouter lifecycle binary for the resolved target", () => {
    expect(cargoBuildArguments("aarch64-apple-darwin")).toEqual([
      "build",
      "--locked",
      "--release",
      "--target",
      "aarch64-apple-darwin",
      "-p",
      "wokrouter-cli",
    ]);
  });

  it("stages only the WokRouter sidecar", () => {
    const copyFileSync = vi.fn();
    const staged = stageBuiltSidecars({
      workspaceRoot: "/work/wokrouter",
      tauriDir: "/work/wokrouter/apps/desktop/src-tauri",
      targetTriple: "x86_64-unknown-linux-gnu",
      hostPlatform: "linux",
      fileSystem: {
        copyFileSync,
        existsSync: () => true,
        mkdirSync: vi.fn(),
      },
    });

    expect(staged).toHaveLength(1);
    expect(staged[0].destination).toBe(
      "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-x86_64-unknown-linux-gnu",
    );
    expect(copyFileSync).toHaveBeenCalledOnce();
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
