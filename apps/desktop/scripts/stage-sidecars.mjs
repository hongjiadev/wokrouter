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

const sidecarNames = ["wokrouter", "wokrouterd"];

export function sidecarFileName(binaryName, targetTriple, platform) {
  const extension = platform === "win32" ? ".exe" : "";
  return `${binaryName}-${targetTriple}${extension}`;
}

export function sidecarPaths({
  workspaceRoot,
  tauriDir,
  targetDir,
  binaryName,
  targetTriple,
  platform,
}) {
  const path = platform === "win32" ? win32 : posix;
  const extension = platform === "win32" ? ".exe" : "";
  const buildTargetDir = targetDir ?? path.join(workspaceRoot, "target");
  return {
    source: path.join(buildTargetDir, "release", `${binaryName}${extension}`),
    destination: path.join(
      tauriDir,
      "binaries",
      sidecarFileName(binaryName, targetTriple, platform),
    ),
  };
}

export function stageBuiltSidecars({
  workspaceRoot,
  tauriDir,
  targetDir,
  targetTriple,
  platform,
  fileSystem = { copyFileSync, existsSync, mkdirSync },
}) {
  const paths = sidecarNames.map((binaryName) =>
    sidecarPaths({
      workspaceRoot,
      tauriDir,
      targetDir,
      binaryName,
      targetTriple,
      platform,
    }),
  );
  for (const path of paths) {
    if (!fileSystem.existsSync(path.source)) {
      throw new Error(`Built sidecar is missing: ${path.source}`);
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
  const targetTriple = commandOutput(rustc, ["--print", "host-tuple"]);
  if (!targetTriple) {
    throw new Error("rustc did not return a host target triple");
  }

  execFileSync(
    cargo,
    [
      "build",
      "--locked",
      "--release",
      "-p",
      "wokrouter-cli",
      "-p",
      "wokrouter-daemon",
    ],
    { cwd: workspaceRoot, stdio: "inherit" },
  );
  const targetDir = process.env.CARGO_TARGET_DIR
    ? resolve(workspaceRoot, process.env.CARGO_TARGET_DIR)
    : join(workspaceRoot, "target");
  const staged = stageBuiltSidecars({
    workspaceRoot,
    tauriDir,
    targetDir,
    targetTriple,
    platform: process.platform,
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
