import { describe, expect, it, vi } from "vitest";

import {
  formatBytes,
  formatLocalDate,
  formatLocalTime,
  formatNumber,
} from "./format";

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

  it("formats day labels with the selected catalog locale", () => {
    expect(formatLocalDate("2026-07-27T00:00:00Z", "en")).toContain("Jul");
    expect(formatLocalDate("2026-07-27T00:00:00Z", "zh-CN")).toBe(
      "2026年7月27日",
    );
  });

  it("formats large numbers without using the WebView default locale", () => {
    expect(formatNumber(1_234_567.89, "en")).toBe("1,234,567.89");
  });

  it("formats bytes with localized numbers and Latin byte units", () => {
    expect(formatBytes(1_536, "en")).toBe("1.5 KiB");
    expect(formatBytes(1_572_864, "zh-CN")).toBe("1.5 MiB");
  });

  it("passes the selected catalog locale to every Intl formatter", () => {
    const originalDateTimeFormat = Intl.DateTimeFormat;
    const originalNumberFormat = Intl.NumberFormat;
    const dateTimeFormat = vi
      .spyOn(Intl, "DateTimeFormat")
      .mockImplementation(
        function dateTimeFormatConstructor(locales, options) {
          return new originalDateTimeFormat(locales, options);
        },
      );
    const numberFormat = vi
      .spyOn(Intl, "NumberFormat")
      .mockImplementation(
        function numberFormatConstructor(locales, options) {
          return new originalNumberFormat(locales, options);
        },
      );

    formatLocalTime("2026-07-27T08:01:00Z", "zh-CN");
    formatLocalDate("2026-07-27T00:00:00Z", "zh-CN");
    formatNumber(1_234_567.89, "zh-CN");
    formatBytes(1_536, "zh-CN");

    expect(dateTimeFormat.mock.calls.map(([locale]) => locale)).toEqual([
      "zh-CN",
      "zh-CN",
    ]);
    expect(numberFormat.mock.calls.map(([locale]) => locale)).toEqual([
      "zh-CN",
      "zh-CN",
    ]);
  });
});
