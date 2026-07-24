import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const configPath = resolve(process.cwd(), "src-tauri/tauri.conf.json");
const config = JSON.parse(readFileSync(configPath, "utf8"));
const packagePath = resolve(process.cwd(), "package.json");
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));

describe("desktop bundle contract", () => {
  it("bundles both lifecycle sidecars", () => {
    expect(config.bundle.active).toBe(true);
    expect(config.bundle.externalBin).toEqual([
      "binaries/wokrouter",
      "binaries/wokrouterd",
    ]);
    expect(config.build.beforeBuildCommand).toBe("pnpm build:bundle");
    expect(config.build.beforeBundleCommand).toBe("pnpm stage:sidecars");
    expect(packageJson.scripts["stage:sidecars"]).toBe(
      "node scripts/stage-sidecars.mjs",
    );
  });

  it("uses the committed cross-platform desktop icons", () => {
    expect(config.bundle.icon).toEqual([
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico",
    ]);
  });
});
