import { describe, expect, it } from "vitest";

import { initializeDocumentLocale, resolveDocumentLocale } from "./locale";

describe("document locale resolution", () => {
  it.each([
    ["ar-SA", "ar-SA"],
    ["fa", "fa"],
    ["ur", "ur"],
  ])("sets %s to right-to-left", (candidate, expected) => {
    expect(resolveDocumentLocale([candidate])).toEqual({
      lang: expected,
      dir: "rtl",
    });
  });

  it("keeps zh-CN left-to-right", () => {
    expect(resolveDocumentLocale(["zh-CN"])).toEqual({
      lang: "zh-CN",
      dir: "ltr",
    });
  });

  it.each([[[]], [["", "??"]], [[null, 42, "_"]]])(
    "falls back malformed candidates to English",
    (candidates) => {
      expect(resolveDocumentLocale(candidates)).toEqual({
        lang: "en",
        dir: "ltr",
      });
    },
  );

  it("initializes lang and dir from navigator languages before render", () => {
    document.documentElement.lang = "";
    document.documentElement.dir = "";

    initializeDocumentLocale(document.documentElement, {
      languages: ["", "ar-SA"],
      language: "en-US",
    });

    expect(document.documentElement.lang).toBe("ar-SA");
    expect(document.documentElement.dir).toBe("rtl");
  });
});
