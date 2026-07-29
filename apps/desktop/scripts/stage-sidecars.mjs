import {
  copyFileSync,
  existsSync,
  mkdirSync,
  rmSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import {
  dirname,
  join,
  posix,
  resolve,
  win32,
} from "node:path";
import { fileURLToPath } from "node:url";

const sidecarNames = ["wokrouter"];
const bundleKinds = new Set(["online", "offline"]);
const supportedTargetTriples = new Set([
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
]);
const tauriTargets = new Map([
  ["windows/x86_64", "x86_64-pc-windows-msvc"],
  ["windows/aarch64", "aarch64-pc-windows-msvc"],
  ["darwin/x86_64", "x86_64-apple-darwin"],
  ["darwin/aarch64", "aarch64-apple-darwin"],
  ["linux/x86_64", "x86_64-unknown-linux-gnu"],
  ["linux/aarch64", "aarch64-unknown-linux-gnu"],
]);

function normalized(value) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function resolveBundleKind(value) {
  const bundleKind = normalized(value) ?? "online";
  if (!bundleKinds.has(bundleKind)) {
    throw new Error(`Unsupported bundle kind: ${bundleKind}`);
  }
  return bundleKind;
}

export function bundleArtifactName({ kind, targetTriple }) {
  return `wokrouter-${resolveBundleKind(kind)}-${supportedTargetTriple(targetTriple)}`;
}

function supportedTargetTriple(targetTriple) {
  if (!supportedTargetTriples.has(targetTriple)) {
    throw new Error(`Unsupported target triple: ${targetTriple}`);
  }
  return targetTriple;
}

export function tauriTargetTriple({ platform, arch }) {
  const key = `${normalized(platform)}/${normalized(arch)}`;
  const targetTriple = tauriTargets.get(key);
  if (!targetTriple) {
    throw new Error(`Unsupported Tauri target: ${key}`);
  }
  return targetTriple;
}

export function resolveTargetTriple({
  explicitTarget,
  cargoBuildTarget,
  tauriTargetTriple: directTauriTarget,
  tauriPlatform,
  tauriArch,
  hostTargetTriple,
}) {
  const configuredTarget =
    normalized(explicitTarget) ??
    normalized(cargoBuildTarget) ??
    normalized(directTauriTarget);
  if (configuredTarget) {
    return supportedTargetTriple(configuredTarget);
  }
  if (normalized(tauriPlatform) || normalized(tauriArch)) {
    return tauriTargetTriple({ platform: tauriPlatform, arch: tauriArch });
  }
  const nativeTarget = normalized(hostTargetTriple);
  if (!nativeTarget) {
    throw new Error("Unable to resolve a sidecar target triple");
  }
  return supportedTargetTriple(nativeTarget);
}

export function resolveTargetTripleFromEnvironment({
  environment,
  readHostTargetTriple,
}) {
  const targetSources = [
    environment.WOKROUTER_TARGET_TRIPLE,
    environment.CARGO_BUILD_TARGET,
    environment.TAURI_ENV_TARGET_TRIPLE,
    environment.TAURI_ENV_PLATFORM,
    environment.TAURI_ENV_ARCH,
  ];
  const hostTargetTriple = targetSources.some((source) => normalized(source))
    ? undefined
    : readHostTargetTriple();
  return resolveTargetTriple({
    explicitTarget: environment.WOKROUTER_TARGET_TRIPLE,
    cargoBuildTarget: environment.CARGO_BUILD_TARGET,
    tauriTargetTriple: environment.TAURI_ENV_TARGET_TRIPLE,
    tauriPlatform: environment.TAURI_ENV_PLATFORM,
    tauriArch: environment.TAURI_ENV_ARCH,
    hostTargetTriple,
  });
}

export function cargoBuildArguments(targetTriple) {
  return [
    "build",
    "--locked",
    "--release",
    "--target",
    supportedTargetTriple(targetTriple),
    "-p",
    "wokrouter-cli",
  ];
}

function executableSuffix(targetTriple) {
  return targetTriple.endsWith("-pc-windows-msvc") ? ".exe" : "";
}

export function sidecarFileName(binaryName, targetTriple) {
  const extension = executableSuffix(targetTriple);
  return `${binaryName}-${targetTriple}${extension}`;
}

export function sidecarPaths({
  workspaceRoot,
  tauriDir,
  targetDir,
  binaryName,
  targetTriple,
  hostPlatform,
}) {
  const path = hostPlatform === "win32" ? win32 : posix;
  const extension = executableSuffix(targetTriple);
  const buildTargetDir = targetDir ?? path.join(workspaceRoot, "target");
  return {
    source: path.join(
      buildTargetDir,
      targetTriple,
      "release",
      `${binaryName}${extension}`,
    ),
    destination: path.join(
      tauriDir,
      "binaries",
      sidecarFileName(binaryName, targetTriple),
    ),
  };
}

export function stageBuiltSidecars({
  workspaceRoot,
  tauriDir,
  targetDir,
  targetTriple,
  hostPlatform,
  fileSystem = { copyFileSync, existsSync, mkdirSync },
}) {
  const paths = sidecarNames.map((binaryName) =>
    sidecarPaths({
      workspaceRoot,
      tauriDir,
      targetDir,
      binaryName,
      targetTriple,
      hostPlatform,
    }),
  );
  for (const path of paths) {
    if (!fileSystem.existsSync(path.source)) {
      throw new Error(
        `Built sidecar is missing for ${targetTriple}: ${path.source}`,
      );
    }
  }

  fileSystem.mkdirSync(join(tauriDir, "binaries"), { recursive: true });
  for (const path of paths) {
    fileSystem.copyFileSync(path.source, path.destination);
  }
  return paths;
}

export function stageBundleArtifact({
  kind,
  targetDir,
  targetTriple,
  hostPlatform,
  stagedSidecars,
  offlineWokCore,
  fileSystem = { copyFileSync, existsSync, mkdirSync, rmSync },
}) {
  const bundleKind = resolveBundleKind(kind);
  const supportedTarget = supportedTargetTriple(targetTriple);
  const path = hostPlatform === "win32" ? win32 : posix;
  const artifactName = bundleArtifactName({
    kind: bundleKind,
    targetTriple: supportedTarget,
  });
  const artifactDirectory = path.join(
    targetDir,
    "wokrouter-bundles",
    artifactName,
  );
  const expectedSidecarName = sidecarFileName("wokrouter", supportedTarget);
  if (
    stagedSidecars?.length !== 1 ||
    path.basename(stagedSidecars[0].destination) !== expectedSidecarName
  ) {
    throw new Error("Exactly one target-specific WokRouter sidecar is required");
  }

  const files = [
    {
      source: stagedSidecars[0].source,
      destination: path.join(artifactDirectory, expectedSidecarName),
    },
  ];
  if (bundleKind === "offline") {
    const manifest = normalized(offlineWokCore?.manifest);
    const signature = normalized(offlineWokCore?.signature);
    const artifact = normalized(offlineWokCore?.artifact);
    if (!manifest || !signature || !artifact) {
      throw new Error("Offline WokCore inputs are required");
    }

    const archiveExtension =
      supportedTarget === "x86_64-pc-windows-msvc" ? "zip" : "tar.gz";
    const archiveName = path.basename(artifact);
    const escapedTarget = supportedTarget.replace(
      /[.*+?^${}()|[\]\\]/g,
      "\\$&",
    );
    const archivePattern = new RegExp(
      `^wokcore-v[^/\\\\]+-${escapedTarget}\\.${archiveExtension.replace(".", "\\.")}$`,
    );
    if (
      path.basename(manifest) !== "wokcore-update-v1.json" ||
      path.basename(signature) !== "wokcore-update-v1.json.minisig" ||
      !archivePattern.test(archiveName)
    ) {
      throw new Error("Offline WokCore inputs do not match the target contract");
    }

    files.push(
      {
        source: manifest,
        destination: path.join(
          artifactDirectory,
          "wokcore-update-v1.json",
        ),
      },
      {
        source: signature,
        destination: path.join(
          artifactDirectory,
          "wokcore-update-v1.json.minisig",
        ),
      },
      {
        source: artifact,
        destination: path.join(artifactDirectory, archiveName),
      },
    );
  }

  if (files.some(({ source }) => !fileSystem.existsSync(source))) {
    throw new Error("Bundle input is missing");
  }
  fileSystem.rmSync(artifactDirectory, { recursive: true, force: true });
  fileSystem.mkdirSync(artifactDirectory, { recursive: true });
  for (const file of files) {
    fileSystem.copyFileSync(file.source, file.destination);
  }
  return { artifactName, artifactDirectory, files };
}

function commandOutput(command, arguments_, options = {}) {
  return execFileSync(command, arguments_, {
    encoding: "utf8",
    ...options,
  }).trim();
}

function main() {
  const scriptDirectory = dirname(fileURLToPath(import.meta.url));
  const desktopDirectory = resolve(scriptDirectory, "..");
  const workspaceRoot = resolve(desktopDirectory, "..", "..");
  const tauriDir = join(desktopDirectory, "src-tauri");
  const rustc = process.env.RUSTC || "rustc";
  const cargo = process.env.CARGO || "cargo";
  const rustVersion = commandOutput(rustc, ["--version"]);
  if (!rustVersion.startsWith("rustc 1.97.1 ")) {
    throw new Error(`Rust 1.97.1 is required; found ${rustVersion}`);
  }
  const targetTriple = resolveTargetTripleFromEnvironment({
    environment: process.env,
    readHostTargetTriple: () =>
      commandOutput(rustc, ["--print", "host-tuple"]),
  });
  const bundleKind = resolveBundleKind(process.env.WOKROUTER_BUNDLE_KIND);
  const artifactName = bundleArtifactName({ kind: bundleKind, targetTriple });

  process.stdout.write(`Building ${artifactName}\n`);
  try {
    execFileSync(cargo, cargoBuildArguments(targetTriple), {
      cwd: workspaceRoot,
      stdio: "inherit",
    });
  } catch (error) {
    throw new Error(`Failed to build sidecars for ${targetTriple}`, {
      cause: error,
    });
  }
  const targetDir = process.env.CARGO_TARGET_DIR
    ? resolve(workspaceRoot, process.env.CARGO_TARGET_DIR)
    : join(workspaceRoot, "target");
  const staged = stageBuiltSidecars({
    workspaceRoot,
    tauriDir,
    targetDir,
    targetTriple,
    hostPlatform: process.platform,
  });
  for (const path of staged) {
    process.stdout.write(`Staged ${path.destination}\n`);
  }
  const bundle = stageBundleArtifact({
    kind: bundleKind,
    targetDir,
    targetTriple,
    hostPlatform: process.platform,
    stagedSidecars: staged,
    offlineWokCore: {
      manifest: process.env.WOKCORE_OFFLINE_MANIFEST,
      signature: process.env.WOKCORE_OFFLINE_SIGNATURE,
      artifact: process.env.WOKCORE_OFFLINE_ARTIFACT,
    },
  });
  process.stdout.write(`Prepared ${bundle.artifactDirectory}\n`);
}

if (import.meta.url.startsWith("file:") && process.argv[1]) {
  const invokedPath = resolve(process.argv[1]);
  if (invokedPath === fileURLToPath(import.meta.url)) {
    main();
  }
}
