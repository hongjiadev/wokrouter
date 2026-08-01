import {
  closeSync,
  copyFileSync,
  existsSync,
  fstatSync,
  mkdirSync,
  openSync,
  readSync,
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
const wokCoreOfflineContracts = new Map([
  ["x86_64-pc-windows-msvc", ["Windows", "x86_64", "zip", true]],
  ["aarch64-pc-windows-msvc", ["Windows", "arm64", "zip", false]],
  ["x86_64-apple-darwin", ["macOS", "x86_64", "tar.gz", true]],
  ["aarch64-apple-darwin", ["macOS", "arm64", "tar.gz", true]],
  ["x86_64-unknown-linux-gnu", ["Linux", "x86_64", "tar.gz", true]],
  ["aarch64-unknown-linux-gnu", ["Linux", "arm64", "tar.gz", true]],
]);
const canonicalSemver =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const maxWokCoreManifestBytes = 64 * 1024;
const maxWokCoreArtifactBytes = 512 * 1024 * 1024;
const wokCoreManifestKeys = [
  "schema_version",
  "product",
  "api_major",
  "version",
  "signing_key_id",
  "artifacts",
];
const wokCoreArtifactKeys = [
  "target",
  "file",
  "executable",
  "size",
  "sha256",
  "url",
];

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
    "--bin",
    "wokrouter",
    "--no-default-features",
  ];
}

function executableSuffix(targetTriple) {
  return targetTriple.endsWith("-pc-windows-msvc") ? ".exe" : "";
}

export function wokCoreArtifactName({
  schemaVersion,
  targetTriple,
  version,
}) {
  const contract = wokCoreOfflineContracts.get(targetTriple);
  if (!contract) {
    throw new Error(`Unsupported target triple: ${targetTriple}`);
  }
  if (
    typeof version !== "string" ||
    version.length > 128 ||
    !canonicalSemver.test(version)
  ) {
    throw new Error("Invalid WokCore version.");
  }
  const [system, architecture, extension, legacyV1] = contract;
  if (schemaVersion === 1) {
    if (!legacyV1) {
      throw new Error("WokCore v1 does not support this target.");
    }
    return `wokcore-v${version}-${targetTriple}.${extension}`;
  }
  if (schemaVersion !== 2) {
    throw new Error("Unsupported WokCore schema version.");
  }
  return system === "Windows"
    ? `WokCore-v${version}-${system}-${architecture}-Portable.zip`
    : `WokCore-v${version}-${system}-${architecture}.${extension}`;
}

function hasExactKeys(value, expectedKeys) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const keys = Object.keys(value);
  return (
    keys.length === expectedKeys.length &&
    expectedKeys.every((key) => Object.hasOwn(value, key))
  );
}

function invalidWokCoreManifestJson() {
  return new Error("WokCore manifest JSON is invalid.");
}

function scanWokCoreManifestJson(text) {
  let index = 0;

  function skipWhitespace() {
    while (
      text[index] === " " ||
      text[index] === "\t" ||
      text[index] === "\r" ||
      text[index] === "\n"
    ) {
      index += 1;
    }
  }

  function scanString(decode) {
    const start = index;
    if (text[index] !== '"') {
      throw invalidWokCoreManifestJson();
    }
    index += 1;
    while (index < text.length) {
      const character = text[index];
      index += 1;
      if (character === '"') {
        if (!decode) {
          return undefined;
        }
        try {
          return JSON.parse(text.slice(start, index));
        } catch {
          throw invalidWokCoreManifestJson();
        }
      }
      if (character === "\\") {
        if (index >= text.length) {
          throw invalidWokCoreManifestJson();
        }
        index += 1;
      } else if (character.charCodeAt(0) < 0x20) {
        throw invalidWokCoreManifestJson();
      }
    }
    throw invalidWokCoreManifestJson();
  }

  function scanValue() {
    skipWhitespace();
    if (text[index] === "{") {
      scanObject();
      return;
    }
    if (text[index] === "[") {
      scanArray();
      return;
    }
    if (text[index] === '"') {
      scanString(false);
      return;
    }

    const start = index;
    while (
      index < text.length &&
      text[index] !== "," &&
      text[index] !== "]" &&
      text[index] !== "}" &&
      text[index] !== " " &&
      text[index] !== "\t" &&
      text[index] !== "\r" &&
      text[index] !== "\n"
    ) {
      index += 1;
    }
    if (index === start) {
      throw invalidWokCoreManifestJson();
    }
  }

  function scanObject() {
    index += 1;
    skipWhitespace();
    if (text[index] === "}") {
      index += 1;
      return;
    }

    const keys = new Set();
    while (index < text.length) {
      skipWhitespace();
      const key = scanString(true);
      if (keys.has(key)) {
        throw invalidWokCoreManifestJson();
      }
      keys.add(key);
      skipWhitespace();
      if (text[index] !== ":") {
        throw invalidWokCoreManifestJson();
      }
      index += 1;
      scanValue();
      skipWhitespace();
      if (text[index] === "}") {
        index += 1;
        return;
      }
      if (text[index] !== ",") {
        throw invalidWokCoreManifestJson();
      }
      index += 1;
    }
    throw invalidWokCoreManifestJson();
  }

  function scanArray() {
    index += 1;
    skipWhitespace();
    if (text[index] === "]") {
      index += 1;
      return;
    }

    while (index < text.length) {
      scanValue();
      skipWhitespace();
      if (text[index] === "]") {
        index += 1;
        return;
      }
      if (text[index] !== ",") {
        throw invalidWokCoreManifestJson();
      }
      index += 1;
    }
    throw invalidWokCoreManifestJson();
  }

  skipWhitespace();
  scanValue();
  skipWhitespace();
  if (index !== text.length) {
    throw invalidWokCoreManifestJson();
  }
}

function readWokCoreManifest(manifest, fileSystem) {
  let descriptor;
  let failed = false;
  try {
    descriptor = fileSystem.openSync(manifest, "r");
    const status = fileSystem.fstatSync(descriptor);
    if (!Number.isSafeInteger(status?.size) || status.size < 0) {
      throw new Error("WokCore manifest could not be read.");
    }
    if (status.size > maxWokCoreManifestBytes) {
      throw new Error("WokCore manifest is empty or oversized.");
    }

    const bytes = Buffer.allocUnsafe(maxWokCoreManifestBytes + 1);
    let length = 0;
    while (length < bytes.byteLength) {
      const remaining = bytes.byteLength - length;
      const bytesRead = fileSystem.readSync(
        descriptor,
        bytes,
        length,
        remaining,
        null,
      );
      if (
        !Number.isSafeInteger(bytesRead) ||
        bytesRead < 0 ||
        bytesRead > remaining
      ) {
        throw new Error("WokCore manifest could not be read.");
      }
      if (bytesRead === 0) {
        break;
      }
      length += bytesRead;
    }
    if (length === 0 || length > maxWokCoreManifestBytes) {
      throw new Error("WokCore manifest is empty or oversized.");
    }
    return bytes.subarray(0, length);
  } catch (error) {
    failed = true;
    if (
      error instanceof Error &&
      (error.message === "WokCore manifest could not be read." ||
        error.message === "WokCore manifest is empty or oversized.")
    ) {
      throw error;
    }
    throw new Error("WokCore manifest could not be read.");
  } finally {
    if (descriptor !== undefined) {
      try {
        fileSystem.closeSync(descriptor);
      } catch {
        if (!failed) {
          throw new Error("WokCore manifest could not be read.");
        }
      }
    }
  }
}

function parseWokCoreManifest(manifest, fileSystem) {
  const bytes = readWokCoreManifest(manifest, fileSystem);

  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error("WokCore manifest JSON is invalid.");
  }
  let document;
  try {
    scanWokCoreManifestJson(text);
    document = JSON.parse(text);
  } catch {
    throw invalidWokCoreManifestJson();
  }
  return document;
}

function validateWokCoreOfflineManifest({
  document,
  schemaVersion,
  targetTriple,
  archiveName,
}) {
  if (!hasExactKeys(document, wokCoreManifestKeys)) {
    throw new Error("WokCore manifest contract is invalid.");
  }
  if (document.schema_version !== schemaVersion) {
    throw new Error("WokCore manifest schema does not match its filename.");
  }
  if (
    document.product !== "wokcore" ||
    document.api_major !== 1 ||
    typeof document.version !== "string" ||
    document.version.length > 128 ||
    !canonicalSemver.test(document.version) ||
    typeof document.signing_key_id !== "string" ||
    !/^[0-9A-F]{16}$/.test(document.signing_key_id) ||
    !Array.isArray(document.artifacts)
  ) {
    throw new Error("WokCore manifest contract is invalid.");
  }

  const contracts = [...wokCoreOfflineContracts.entries()].filter(
    ([, [, , , legacyV1]]) => schemaVersion === 2 || legacyV1,
  );
  if (document.artifacts.length !== contracts.length) {
    throw new Error("WokCore manifest artifact contract is invalid.");
  }

  let selectedArtifact;
  for (let index = 0; index < contracts.length; index += 1) {
    const [target, [system]] = contracts[index];
    const artifact = document.artifacts[index];
    const expectedFile = wokCoreArtifactName({
      schemaVersion,
      targetTriple: target,
      version: document.version,
    });
    const expectedExecutable = system === "Windows" ? "wokcore.exe" : "wokcore";
    const expectedUrl =
      `https://github.com/hongjiadev/wokcore/releases/download/` +
      `v${document.version}/${expectedFile}`;
    if (
      !hasExactKeys(artifact, wokCoreArtifactKeys) ||
      artifact.target !== target ||
      artifact.file !== expectedFile ||
      artifact.executable !== expectedExecutable ||
      !Number.isSafeInteger(artifact.size) ||
      artifact.size <= 0 ||
      artifact.size > maxWokCoreArtifactBytes ||
      typeof artifact.sha256 !== "string" ||
      !/^[0-9a-f]{64}$/.test(artifact.sha256) ||
      artifact.url !== expectedUrl
    ) {
      throw new Error("WokCore manifest artifact contract is invalid.");
    }
    if (target === targetTriple) {
      selectedArtifact = artifact;
    }
  }
  if (!selectedArtifact) {
    throw new Error("WokCore manifest does not support this target.");
  }
  if (selectedArtifact.file !== archiveName) {
    throw new Error("WokCore manifest and artifact versions do not match.");
  }
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
  fileSystem = {
    closeSync,
    copyFileSync,
    existsSync,
    fstatSync,
    mkdirSync,
    openSync,
    readSync,
    rmSync,
  },
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

    const manifestName = path.basename(manifest);
    const schemaVersion =
      manifestName === "wokcore-update-v2.json"
        ? 2
        : manifestName === "wokcore-update-v1.json"
          ? 1
          : null;
    if (schemaVersion === null) {
      throw new Error(`Unsupported WokCore manifest: ${manifestName}`);
    }
    if (path.basename(signature) !== `${manifestName}.minisig`) {
      throw new Error("WokCore manifest and signature versions do not match.");
    }

    const archiveName = path.basename(artifact);
    const document = parseWokCoreManifest(manifest, fileSystem);
    validateWokCoreOfflineManifest({
      document,
      schemaVersion,
      targetTriple: supportedTarget,
      archiveName,
    });

    files.push(
      {
        source: manifest,
        destination: path.join(artifactDirectory, manifestName),
      },
      {
        source: signature,
        destination: path.join(artifactDirectory, `${manifestName}.minisig`),
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
