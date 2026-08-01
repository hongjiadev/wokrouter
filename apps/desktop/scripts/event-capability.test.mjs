import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const capabilityPath = resolve(
  process.cwd(),
  "src-tauri/capabilities/main.json",
);

describe("desktop event capability", () => {
  it("grants only event listen and unlisten to the main window", () => {
    const capability = JSON.parse(readFileSync(capabilityPath, "utf8"));

    expect(capability).toEqual({
      $schema: "../gen/schemas/desktop-schema.json",
      identifier: "main-event-listener",
      description: "Allows the main window to monitor WokCore operations.",
      windows: ["main"],
      permissions: [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
      ],
    });
  });
});
