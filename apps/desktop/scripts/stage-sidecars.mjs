import {
  copyFileSync,
  existsSync,
  mkdirSync,
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
const supportedTargetTriples = new Set([
  "x86_64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
]);
const tauriTargets = new Map([
  ["windows/x86_64", "x86_64-pc-windows-msvc"],
  ["darwin/x86_64", "x86_64-apple-darwin"],
  ["darwin/aarch64", "aarch64-apple-darwin"],
  ["linux/x86_64", "x86_64-unknown-linux-gnu"],
  ["linux/aarch64", "aarch64-unknown-linux-gnu"],
]);

function normalized(value) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
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

function targetExtension(targetTriple) {
  return targetTriple === "x86_64-pc-windows-msvc" ? ".exe" : "";
}

export function sidecarFileName(binaryName, targetTriple) {
  const extension = targetExtension(targetTriple);
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
  const extension = targetExtension(targetTriple);
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

  process.stdout.write(`Building sidecars for ${targetTriple}\n`);
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
}

if (import.meta.url.startsWith("file:") && process.argv[1]) {
  const invokedPath = resolve(process.argv[1]);
  if (invokedPath === fileURLToPath(import.meta.url)) {
    main();
  }
}
