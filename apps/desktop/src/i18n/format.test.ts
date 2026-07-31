import { describe, expect, it } from "vitest";

import { formatBytes, formatLocalTime, formatNumber } from "./format";

describe("localized management formatters", () => {
  it("formats timestamps with the selected catalog locale and preserves invalid values", () => {
    expect(formatLocalTime("2026-07-27T08:01:00Z", "en")).toContain("Jul");
    expect(formatLocalTime("2026-07-27T08:01:00Z", "zh-CN")).toContain(
      "2026年",
    );
    expect(formatLocalTime("not-a-timestamp", "zh-CN")).toBe(
      "not-a-timestamp",
    );
  });

  it("formats large numbers without using the WebView default locale", () => {
    expect(formatNumber(1_234_567.89, "en")).toBe("1,234,567.89");
  });

  it("formats bytes with localized numbers and Latin byte units", () => {
    expect(formatBytes(1_536, "en")).toBe("1.5 KiB");
    expect(formatBytes(1_572_864, "zh-CN")).toBe("1.5 MiB");
  });
});
