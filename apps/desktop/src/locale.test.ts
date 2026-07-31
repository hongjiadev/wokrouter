import { describe, expect, it } from "vitest";

import {
  browserLocaleCandidates,
  initializeDocumentLocale,
  resolveSupportedLocale,
} from "./locale";

describe("supported locale resolution", () => {
  it.each([
    ["zh-CN", [], "zh-CN"],
    ["zh_CN", ["en-US"], "zh-CN"],
    ["ZH_cn", ["en-US"], "zh-CN"],
    ["zh", ["en-US"], "zh-CN"],
    ["zh-Hans", ["en-US"], "zh-CN"],
    ["zh_hans_sg", ["en-US"], "zh-CN"],
    ["zh-TW", ["zh-CN"], "en"],
    ["zh-HK", ["zh-CN"], "en"],
    ["zh-Hant", ["zh-CN"], "en"],
    ["zh-MO", ["zh-CN"], "en"],
    ["fr-FR", ["zh-CN"], "en"],
    [undefined, ["zh-CN", "en-US"], "zh-CN"],
    [undefined, ["zh-TW", "zh-CN"], "en"],
    [undefined, ["", "zh-Hans"], "zh-CN"],
    ["   ", ["zh-CN"], "zh-CN"],
    [undefined, ["not a locale", "zh-CN"], "en"],
    [undefined, [], "en"],
  ] as const)("resolves %s and %j to %s", (system, browser, expected) => {
    expect(resolveSupportedLocale(system, browser)).toBe(expected);
  });

  it("orders navigator.languages before navigator.language and removes exact duplicates", () => {
    expect(
      browserLocaleCandidates({
        languages: ["zh-TW", "zh-CN", "zh-TW"],
        language: "zh-CN",
      }),
    ).toEqual(["zh-TW", "zh-CN"]);
  });

  it("appends a distinct navigator.language after navigator.languages", () => {
    expect(
      browserLocaleCandidates({
        languages: [""],
        language: "zh-Hans",
      }),
    ).toEqual(["", "zh-Hans"]);
  });

  it.each(["en", "zh-CN"] as const)(
    "initializes the document from the selected %s catalog",
    (locale) => {
      document.documentElement.lang = "raw-system-candidate";
      document.documentElement.dir = "rtl";

      expect(initializeDocumentLocale(document.documentElement, locale)).toEqual({
        lang: locale,
        dir: "ltr",
      });
      expect(document.documentElement.lang).toBe(locale);
      expect(document.documentElement.dir).toBe("ltr");
    },
  );
});
