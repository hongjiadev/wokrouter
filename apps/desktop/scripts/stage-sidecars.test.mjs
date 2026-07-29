import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

import * as sidecars from "./stage-sidecars.mjs";

const {
  bundleArtifactName,
  cargoBuildArguments,
  resolveBundleKind,
  resolveTargetTriple,
  resolveTargetTripleFromEnvironment,
  sidecarFileName,
  sidecarPaths,
  stageBundleArtifact,
  stageBuiltSidecars,
  tauriTargetTriple,
  wokCoreArtifactName,
} = sidecars;

const supportedTauriTargets = [
  ["windows", "x86_64", "x86_64-pc-windows-msvc"],
  ["windows", "aarch64", "aarch64-pc-windows-msvc"],
  ["darwin", "x86_64", "x86_64-apple-darwin"],
  ["darwin", "aarch64", "aarch64-apple-darwin"],
  ["linux", "x86_64", "x86_64-unknown-linux-gnu"],
  ["linux", "aarch64", "aarch64-unknown-linux-gnu"],
];
const supportedTargetTriples = supportedTauriTargets.map(([, , triple]) => triple);
const v1ManifestText = readFileSync(
  resolve(
    process.cwd(),
    "../../crates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v1.json",
  ),
  "utf8",
);
const v2ManifestText = readFileSync(
  resolve(
    process.cwd(),
    "../../crates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v2.json",
  ),
  "utf8",
);

function canonicalManifest(document) {
  return `${JSON.stringify(document)}\n`;
}

function offlineFileSystem(manifestText) {
  const manifestBytes = Buffer.from(manifestText);
  let readOffset = 0;
  return {
    closeSync: vi.fn(),
    copyFileSync: vi.fn(),
    existsSync: () => true,
    fstatSync: vi.fn(() => ({ size: manifestBytes.byteLength })),
    mkdirSync: vi.fn(),
    openSync: vi.fn(() => {
      readOffset = 0;
      return 17;
    }),
    readSync: vi.fn((_descriptor, buffer, offset, length) => {
      const bytesRead = Math.min(length, manifestBytes.byteLength - readOffset);
      manifestBytes.copy(
        buffer,
        offset,
        readOffset,
        readOffset + bytesRead,
      );
      readOffset += bytesRead;
      return bytesRead;
    }),
    rmSync: vi.fn(),
  };
}

function windowsArm64OfflineBundle(
  manifestText,
  fileSystem = offlineFileSystem(manifestText),
) {
  return {
    kind: "offline",
    targetDir: "/work/wokrouter/target",
    targetTriple: "aarch64-pc-windows-msvc",
    hostPlatform: "linux",
    stagedSidecars: [
      {
        source:
          "/work/wokrouter/target/aarch64-pc-windows-msvc/release/wokrouter.exe",
        destination:
          "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-aarch64-pc-windows-msvc.exe",
      },
    ],
    offlineWokCore: {
      manifest: "/release/wokcore-update-v2.json",
      signature: "/release/wokcore-update-v2.json.minisig",
      artifact: "/release/WokCore-v1.2.3-Windows-arm64-Portable.zip",
    },
    fileSystem,
  };
}

describe("target resolution", () => {
  it.each(supportedTauriTargets)(
    "maps Tauri %s/%s to %s",
    (platform, arch, expected) => {
      expect(tauriTargetTriple({ platform, arch })).toBe(expected);
    },
  );

  it("fails fast for an unsupported Tauri platform and architecture", () => {
    expect(() =>
      tauriTargetTriple({ platform: "windows", arch: "armv7" }),
    ).toThrow("Unsupported Tauri target: windows/armv7");
  });

  it("accepts Windows ARM64 as an explicit target", () => {
    expect(
      resolveTargetTriple({
        explicitTarget: "aarch64-pc-windows-msvc",
        hostTargetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("aarch64-pc-windows-msvc");
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
  it("derives exact WokCore v1 and v2 portable artifact names", () => {
    expect(
      wokCoreArtifactName({
        schemaVersion: 1,
        targetTriple: "x86_64-pc-windows-msvc",
        version: "1.2.3",
      }),
    ).toBe("wokcore-v1.2.3-x86_64-pc-windows-msvc.zip");
    expect(
      wokCoreArtifactName({
        schemaVersion: 2,
        targetTriple: "aarch64-pc-windows-msvc",
        version: "1.2.3",
      }),
    ).toBe("WokCore-v1.2.3-Windows-arm64-Portable.zip");
    expect(
      wokCoreArtifactName({
        schemaVersion: 2,
        targetTriple: "aarch64-unknown-linux-gnu",
        version: "1.2.3",
      }),
    ).toBe("WokCore-v1.2.3-Linux-arm64.tar.gz");
    expect(
      wokCoreArtifactName({
        schemaVersion: 2,
        targetTriple: "x86_64-apple-darwin",
        version: "1.2.3-1a+build.7",
      }),
    ).toBe("WokCore-v1.2.3-1a+build.7-macOS-x86_64.tar.gz");
    expect(() =>
      wokCoreArtifactName({
        schemaVersion: 2,
        targetTriple: "x86_64-apple-darwin",
        version: "01.2.3",
      }),
    ).toThrow("Invalid WokCore version.");
    expect(() =>
      wokCoreArtifactName({
        schemaVersion: 1,
        targetTriple: "aarch64-pc-windows-msvc",
        version: "1.2.3",
      }),
    ).toThrow("WokCore v1 does not support this target.");
  });

  it("uses distinct target-specific online and offline artifact names", () => {
    expect(resolveBundleKind(undefined)).toBe("online");
    expect(
      bundleArtifactName({
        targetTriple: "x86_64-pc-windows-msvc",
      }),
    ).toBe("wokrouter-online-x86_64-pc-windows-msvc");
    expect(
      bundleArtifactName({
        kind: "offline",
        targetTriple: "aarch64-apple-darwin",
      }),
    ).toBe("wokrouter-offline-aarch64-apple-darwin");
    expect(() => resolveBundleKind("combined")).toThrow(
      "Unsupported bundle kind: combined",
    );
  });

  it("writes online bundle inputs to the target-specific artifact directory", () => {
    const copyFileSync = vi.fn();
    const result = stageBundleArtifact({
      kind: "online",
      targetDir: "/work/wokrouter/target",
      targetTriple: "x86_64-unknown-linux-gnu",
      hostPlatform: "linux",
      stagedSidecars: [
        {
          source:
            "/work/wokrouter/target/x86_64-unknown-linux-gnu/release/wokrouter",
          destination:
            "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-x86_64-unknown-linux-gnu",
        },
      ],
      fileSystem: {
        copyFileSync,
        existsSync: () => true,
        mkdirSync: vi.fn(),
        rmSync: vi.fn(),
      },
    });

    expect(result.artifactName).toBe(
      "wokrouter-online-x86_64-unknown-linux-gnu",
    );
    expect(result.artifactDirectory).toBe(
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-online-x86_64-unknown-linux-gnu",
    );
    expect(result.files.map(({ destination }) => destination)).toEqual([
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-online-x86_64-unknown-linux-gnu/wokrouter-x86_64-unknown-linux-gnu",
    ]);
    expect(copyFileSync).toHaveBeenCalledOnce();
  });

  it("requires and stages signed WokCore inputs only for an offline artifact", () => {
    const fileSystem = offlineFileSystem(v1ManifestText);
    const common = {
      kind: "offline",
      targetDir: "/work/wokrouter/target",
      targetTriple: "aarch64-apple-darwin",
      hostPlatform: "linux",
      stagedSidecars: [
        {
          source:
            "/work/wokrouter/target/aarch64-apple-darwin/release/wokrouter",
          destination:
            "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-aarch64-apple-darwin",
        },
      ],
      fileSystem,
    };

    expect(() => stageBundleArtifact(common)).toThrow(
      "Offline WokCore inputs are required",
    );

    const result = stageBundleArtifact({
      ...common,
      offlineWokCore: {
        manifest: "/release/wokcore-update-v1.json",
        signature: "/release/wokcore-update-v1.json.minisig",
        artifact:
          "/release/wokcore-v1.2.3-aarch64-apple-darwin.tar.gz",
      },
    });

    expect(result.artifactName).toBe(
      "wokrouter-offline-aarch64-apple-darwin",
    );
    expect(result.files.map(({ destination }) => destination)).toEqual([
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-apple-darwin/wokrouter-aarch64-apple-darwin",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-apple-darwin/wokcore-update-v1.json",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-apple-darwin/wokcore-update-v1.json.minisig",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-apple-darwin/wokcore-v1.2.3-aarch64-apple-darwin.tar.gz",
    ]);
    expect(fileSystem.copyFileSync).toHaveBeenCalledTimes(4);
  });

  it("stages a complete v2 WokCore set and rejects mixed manifest versions", () => {
    const fileSystem = offlineFileSystem(v2ManifestText);
    const common = {
      kind: "offline",
      targetDir: "/work/wokrouter/target",
      targetTriple: "aarch64-pc-windows-msvc",
      hostPlatform: "linux",
      stagedSidecars: [
        {
          source:
            "/work/wokrouter/target/aarch64-pc-windows-msvc/release/wokrouter.exe",
          destination:
            "/work/wokrouter/apps/desktop/src-tauri/binaries/wokrouter-aarch64-pc-windows-msvc.exe",
        },
      ],
      fileSystem,
    };

    const result = stageBundleArtifact({
      ...common,
      offlineWokCore: {
        manifest: "/release/wokcore-update-v2.json",
        signature: "/release/wokcore-update-v2.json.minisig",
        artifact: "/release/WokCore-v1.2.3-Windows-arm64-Portable.zip",
      },
    });

    expect(result.files.map(({ destination }) => destination)).toEqual([
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-pc-windows-msvc/wokrouter-aarch64-pc-windows-msvc.exe",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-pc-windows-msvc/wokcore-update-v2.json",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-pc-windows-msvc/wokcore-update-v2.json.minisig",
      "/work/wokrouter/target/wokrouter-bundles/wokrouter-offline-aarch64-pc-windows-msvc/WokCore-v1.2.3-Windows-arm64-Portable.zip",
    ]);

    expect(() =>
      stageBundleArtifact({
        ...common,
        offlineWokCore: {
          manifest: "/release/wokcore-update-v2.json",
          signature: "/release/wokcore-update-v1.json.minisig",
          artifact:
            "/release/WokCore-v1.2.3-Windows-arm64-Portable.zip",
        },
      }),
    ).toThrow("WokCore manifest and signature versions do not match.");
    expect(() =>
      stageBundleArtifact({
        ...common,
        offlineWokCore: {
          manifest: "/release/wokcore-update-v2.json",
          signature: "/release/wokcore-update-v2.json.minisig",
          artifact:
            "/release/wokcore-v1.2.3-aarch64-pc-windows-msvc.zip",
        },
      }),
    ).toThrow("WokCore manifest and artifact versions do not match.");
  });

  it("rejects offline manifest body schema, version, and selected artifact mismatches", () => {
    expect(() =>
      stageBundleArtifact(windowsArm64OfflineBundle(v1ManifestText)),
    ).toThrow("WokCore manifest schema does not match its filename.");

    const wrongVersion = JSON.parse(v2ManifestText);
    wrongVersion.version = "1.2.4";
    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(canonicalManifest(wrongVersion)),
      ),
    ).toThrow("WokCore manifest artifact contract is invalid.");

    const wrongFile = JSON.parse(v2ManifestText);
    wrongFile.artifacts[1].file =
      "WokCore-v1.2.3-Windows-x86_64-Portable.zip";
    expect(() =>
      stageBundleArtifact(windowsArm64OfflineBundle(canonicalManifest(wrongFile))),
    ).toThrow("WokCore manifest artifact contract is invalid.");
  });

  it("accepts schema-valid object member reordering while keeping artifact order strict", () => {
    const original = JSON.parse(v2ManifestText);
    const reordered = {
      artifacts: original.artifacts.map((artifact) => ({
        sha256: artifact.sha256,
        target: artifact.target,
        url: artifact.url,
        size: artifact.size,
        executable: artifact.executable,
        file: artifact.file,
      })),
      signing_key_id: original.signing_key_id,
      version: original.version,
      api_major: original.api_major,
      product: original.product,
      schema_version: original.schema_version,
    };

    const result = stageBundleArtifact(
      windowsArm64OfflineBundle(canonicalManifest(reordered)),
    );

    expect(result.files).toHaveLength(4);
  });

  it("rejects missing, duplicate, extra, reordered, and unknown manifest members", () => {
    const mutations = [
      [
        "missing target",
        (document) => {
          document.artifacts.splice(1, 1);
        },
      ],
      [
        "duplicate target",
        (document) => {
          document.artifacts[1] = structuredClone(document.artifacts[0]);
        },
      ],
      [
        "extra target",
        (document) => {
          document.artifacts.push(structuredClone(document.artifacts[0]));
        },
      ],
      [
        "wrong order",
        (document) => {
          [document.artifacts[0], document.artifacts[1]] = [
            document.artifacts[1],
            document.artifacts[0],
          ];
        },
      ],
      [
        "unknown root member",
        (document) => {
          document.extra = true;
        },
      ],
      [
        "unknown artifact member",
        (document) => {
          document.artifacts[1].extra = true;
        },
      ],
    ];

    for (const [name, mutate] of mutations) {
      const document = JSON.parse(v2ManifestText);
      mutate(document);
      expect(
        () =>
          stageBundleArtifact(
            windowsArm64OfflineBundle(canonicalManifest(document)),
          ),
        name,
      ).toThrow("WokCore manifest");
    }
  });

  it("validates every selected artifact field and bounds manifest reads", () => {
    const mutations = [
      ["executable", "wokcore"],
      ["url", "https://example.com/WokCore.zip"],
      ["size", 0],
      ["sha256", "A".repeat(64)],
    ];
    for (const [field, value] of mutations) {
      const document = JSON.parse(v2ManifestText);
      document.artifacts[1][field] = value;
      expect(
        () =>
          stageBundleArtifact(
            windowsArm64OfflineBundle(canonicalManifest(document)),
          ),
        field,
      ).toThrow("WokCore manifest artifact contract is invalid.");
    }

    expect(() =>
      stageBundleArtifact(windowsArm64OfflineBundle("{not-json}\n")),
    ).toThrow("WokCore manifest JSON is invalid.");
    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(" ".repeat(64 * 1024 + 1)),
      ),
    ).toThrow("WokCore manifest is empty or oversized.");
  });

  it("bounds physical manifest reads for oversized and growing files", () => {
    const maximumBytes = 64 * 1024;
    const oversizedFileSystem = offlineFileSystem(v2ManifestText);
    oversizedFileSystem.fstatSync.mockReturnValue({
      size: maximumBytes + 1,
    });

    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(v2ManifestText, oversizedFileSystem),
      ),
    ).toThrow("WokCore manifest is empty or oversized.");
    expect(oversizedFileSystem.readSync).not.toHaveBeenCalled();
    expect(oversizedFileSystem.closeSync).toHaveBeenCalledOnce();

    const growingFileSystem = offlineFileSystem(" ".repeat(maximumBytes + 1));
    growingFileSystem.fstatSync.mockReturnValue({ size: v2ManifestText.length });

    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(v2ManifestText, growingFileSystem),
      ),
    ).toThrow("WokCore manifest is empty or oversized.");
    expect(
      growingFileSystem.readSync.mock.calls.reduce(
        (total, call) => total + call[3],
        0,
      ),
    ).toBe(maximumBytes + 1);
    expect(
      growingFileSystem.readSync.mock.calls.every(
        (call) => call[1].byteLength === maximumBytes + 1,
      ),
    ).toBe(true);
    expect(growingFileSystem.closeSync).toHaveBeenCalledOnce();
  });

  it("continues bounded manifest reads after short reads", () => {
    const fileSystem = offlineFileSystem(v2ManifestText);
    const readSync = fileSystem.readSync.getMockImplementation();
    fileSystem.readSync.mockImplementation(
      (descriptor, buffer, offset, length, position) =>
        readSync(descriptor, buffer, offset, Math.min(length, 7), position),
    );

    const result = stageBundleArtifact(
      windowsArm64OfflineBundle(v2ManifestText, fileSystem),
    );

    expect(result.files).toHaveLength(4);
    expect(fileSystem.readSync.mock.calls.length).toBeGreaterThan(1);
    expect(fileSystem.closeSync).toHaveBeenCalledExactlyOnceWith(17);
  });

  it("closes opened manifests on read errors without closing failed opens", () => {
    const readFailure = offlineFileSystem(v2ManifestText);
    readFailure.readSync.mockImplementation(() => {
      throw new Error("disk failure");
    });
    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(v2ManifestText, readFailure),
      ),
    ).toThrow("WokCore manifest could not be read.");
    expect(readFailure.closeSync).toHaveBeenCalledExactlyOnceWith(17);

    const openFailure = offlineFileSystem(v2ManifestText);
    openFailure.openSync.mockImplementation(() => {
      throw new Error("open failure");
    });
    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(v2ManifestText, openFailure),
      ),
    ).toThrow("WokCore manifest could not be read.");
    expect(openFailure.closeSync).not.toHaveBeenCalled();
  });

  it("surfaces close failures after otherwise successful bounded reads", () => {
    const fileSystem = offlineFileSystem(v2ManifestText);
    fileSystem.closeSync.mockImplementation(() => {
      throw new Error("close failure");
    });

    expect(() =>
      stageBundleArtifact(
        windowsArm64OfflineBundle(v2ManifestText, fileSystem),
      ),
    ).toThrow("WokCore manifest could not be read.");
    expect(fileSystem.closeSync).toHaveBeenCalledExactlyOnceWith(17);
  });

  it("decodes escaped object keys and rejects their duplicates", () => {
    const escapedRoot = v2ManifestText.replace(
      '"schema_version":2',
      '"schema_\\u0076ersion":2',
    );
    expect(
      stageBundleArtifact(windowsArm64OfflineBundle(escapedRoot)).files,
    ).toHaveLength(4);

    const duplicateRoot = v2ManifestText.replace(
      '"schema_version":2',
      '"schema_version":2,"schema_\\u0076ersion":2',
    );
    expect(() =>
      stageBundleArtifact(windowsArm64OfflineBundle(duplicateRoot)),
    ).toThrow("WokCore manifest JSON is invalid.");

    const duplicateArtifact = v2ManifestText.replace(
      '"target":"x86_64-pc-windows-msvc"',
      '"target":"x86_64-pc-windows-msvc","ta\\u0072get":"x86_64-pc-windows-msvc"',
    );
    expect(() =>
      stageBundleArtifact(windowsArm64OfflineBundle(duplicateArtifact)),
    ).toThrow("WokCore manifest JSON is invalid.");
  });

  it("derives the executable extension from the target triple", () => {
    expect(
      sidecarFileName("wokrouter", "x86_64-pc-windows-msvc"),
    ).toBe("wokrouter-x86_64-pc-windows-msvc.exe");
    expect(
      sidecarFileName("wokrouter", "aarch64-pc-windows-msvc"),
    ).toBe("wokrouter-aarch64-pc-windows-msvc.exe");
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
    expect(staged.map(({ destination }) => destination)).not.toEqual(
      expect.arrayContaining([
        expect.stringContaining("wokrouterd"),
        expect.stringContaining("wokcore"),
        expect.stringContaining("provider-sim"),
        expect.stringContaining("loadgen"),
      ]),
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
