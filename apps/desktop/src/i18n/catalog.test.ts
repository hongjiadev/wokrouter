import { describe, expect, it } from "vitest";

import packageManifest from "../../package.json";
// @ts-expect-error The standalone checker is native ESM without declarations.
import { validateCatalogs } from "../../scripts/check-i18n-catalogs.mjs";
import { i18n, initializeI18n } from "./index";
import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";

type Catalog = Record<string, unknown>;

const topLevelNamespaces = [
  "app",
  "common",
  "core",
  "errors",
  "management",
  "operation",
];
const managementNamespaces = [
  "diagnostics",
  "providers",
  "sessions",
  "usage",
];

function flattenKeys(catalog: Catalog, prefix = ""): string[] {
  return Object.entries(catalog)
    .flatMap(([key, value]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return value !== null && typeof value === "object" && !Array.isArray(value)
        ? flattenKeys(value as Catalog, path)
        : [path];
    })
    .sort();
}

function readKey(catalog: Catalog, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>(
      (value, key) =>
        value !== null && typeof value === "object"
          ? (value as Catalog)[key]
          : undefined,
      catalog,
    );
}

function placeholders(value: unknown): string[] {
  if (typeof value !== "string") return [];
  return [
    ...new Set(
      [...value.matchAll(/{{\s*([A-Za-z0-9_.-]+)\s*}}/g)].map(
        (match) => match[1],
      ),
    ),
  ].sort();
}

function fixture(common: Catalog): Catalog {
  return {
    app: {},
    core: {},
    operation: {},
    management: {
      providers: {},
      sessions: {},
      usage: {},
      diagnostics: {},
    },
    errors: {},
    common,
  };
}

function assertCatalogKeyTypes() {
  i18n.t("common.retry");
  // @ts-expect-error English catalog keys are the compile-time source of truth.
  i18n.t("common.notRegistered");
}

void assertCatalogKeyTypes;

describe("desktop translation catalogs", () => {
  it("initializes and changes one explicit two-locale i18next singleton", async () => {
    const singleton = i18n;

    await initializeI18n("en");
    expect(i18n).toBe(singleton);
    expect(i18n.resolvedLanguage).toBe("en");
    expect(i18n.t("common.retry")).toBe("Try again");
    expect(i18n.options.fallbackLng).toEqual(["en"]);
    const supportedLngs = i18n.options.supportedLngs;
    expect(Array.isArray(supportedLngs)).toBe(true);
    if (!Array.isArray(supportedLngs)) {
      throw new TypeError("i18next supportedLngs must be an array.");
    }
    expect(supportedLngs.filter((locale) => locale !== "cimode")).toEqual([
      "en",
      "zh-CN",
    ]);
    expect(i18n.options.nonExplicitSupportedLngs).toBe(false);
    expect(i18n.options.interpolation?.escapeValue).toBe(false);

    await initializeI18n("zh-CN");
    expect(i18n).toBe(singleton);
    expect(i18n.resolvedLanguage).toBe("zh-CN");
    expect(i18n.t("common.retry")).toBe("重试");
  });

  it("keeps the exact planned namespace skeleton", () => {
    expect(Object.keys(en).sort()).toEqual(topLevelNamespaces);
    expect(Object.keys(zhCN).sort()).toEqual(topLevelNamespaces);
    expect(Object.keys(en.management).sort()).toEqual(managementNamespaces);
    expect(Object.keys(zhCN.management).sort()).toEqual(managementNamespaces);
  });

  it("keeps exact sorted leaf keys and placeholder sets in both locales", () => {
    expect(flattenKeys(zhCN)).toEqual(flattenKeys(en));
    for (const key of flattenKeys(en)) {
      expect(placeholders(readKey(zhCN, key)), key).toEqual(
        placeholders(readKey(en, key)),
      );
    }
  });

  it("accepts every current leaf through the standalone catalog rules", () => {
    expect(validateCatalogs(en, zhCN)).toBe(7);
  });

  it("contains the exact initial English and Simplified Chinese copy", () => {
    expect(en).toMatchObject({
      app: {
        localDesktopControl: "Local desktop control",
        loopbackFooter:
          "Desktop controls communicate with WokCore over loopback HTTP.",
      },
      common: {
        retry: "Try again",
        cancel: "Cancel",
        confirm: "Confirm",
        unavailable: "Unavailable",
        loading: "Loading…",
      },
    });
    expect(zhCN).toMatchObject({
      app: {
        localDesktopControl: "本地桌面控制",
        loopbackFooter: "桌面控制通过环回 HTTP 与 WokCore 通信。",
      },
      common: {
        retry: "重试",
        cancel: "取消",
        confirm: "确认",
        unavailable: "不可用",
        loading: "正在加载…",
      },
    });
  });

  it.each([
    [
      "a missing key",
      fixture({ retry: "Try again" }),
      fixture({}),
      'Catalog "zh-CN" is missing key "common.retry".',
    ],
    [
      "a mismatched placeholder",
      fixture({ greeting: "Hello {{name}}" }),
      fixture({ greeting: "你好 {{username}}" }),
      'Catalog placeholder mismatch at "common.greeting"',
    ],
    [
      "an empty value",
      fixture({ retry: "Try again" }),
      fixture({ retry: " " }),
      'Catalog "zh-CN" key "common.retry" must be a non-empty string.',
    ],
    [
      "a non-string leaf",
      fixture({ retry: "Try again" }),
      fixture({ retry: 1 }),
      'Catalog "zh-CN" key "common.retry" must be a non-empty string.',
    ],
    [
      "HTML markup",
      fixture({ retry: "Try again" }),
      fixture({ retry: "<strong>重试</strong>" }),
      'Catalog "zh-CN" key "common.retry" must not contain HTML markup.',
    ],
  ])("rejects %s", (_name, english, chinese, message) => {
    expect(() => validateCatalogs(english, chinese)).toThrow(message);
  });

  it("registers the standalone catalog check command", () => {
    expect(packageManifest.scripts["i18n:check"]).toBe(
      "node scripts/check-i18n-catalogs.mjs",
    );
  });
});
