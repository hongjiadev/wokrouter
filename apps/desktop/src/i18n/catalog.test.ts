import { describe, expect, it } from "vitest";

// @ts-expect-error Node types are intentionally not a direct desktop dependency.
import { spawnSync } from "node:child_process";
// @ts-expect-error Node types are intentionally not a direct desktop dependency.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
// @ts-expect-error Node types are intentionally not a direct desktop dependency.
import { tmpdir } from "node:os";
// @ts-expect-error Node types are intentionally not a direct desktop dependency.
import { join } from "node:path";
import packageManifest from "../../package.json";
// @ts-expect-error The standalone checker is native ESM without declarations.
import { validateCatalogs } from "../../scripts/check-i18n-catalogs.mjs";
import { i18n, initializeI18n } from "./index";
import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";

type Catalog = Record<string, unknown>;
type CatalogPair = readonly [Catalog, Catalog];

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
      if (key === "" || key.includes(".")) {
        throw new Error(`Invalid catalog key segment: ${JSON.stringify(key)}`);
      }
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

function dottedCollision(): CatalogPair {
  return [
    fixture({
      a: { b: "Nested {{nested}}" },
      "a.b": "Literal {{kept}}",
    }),
    fixture({
      a: { b: "嵌套 {{wrong}}" },
      "a.b": "字面 {{kept}}",
    }),
  ];
}

function collapsedRootNamespace(value: unknown): CatalogPair {
  const english = fixture({ retry: "Try again" });
  const chinese = fixture({ retry: "重试" });
  english.core = value;
  chinese.core = value;
  return [english, chinese];
}

function collapsedManagementNamespace(value: unknown): CatalogPair {
  const english = fixture({ retry: "Try again" });
  const chinese = fixture({ retry: "重试" });
  (english.management as Catalog).providers = value;
  (chinese.management as Catalog).providers = value;
  return [english, chinese];
}

function markup(value: string): CatalogPair {
  return [fixture({ retry: value }), fixture({ retry: value })];
}

function runCatalogChecker([english, chinese]: CatalogPair) {
  const directory = mkdtempSync(join(tmpdir(), "wokrouter-i18n-"));
  try {
    const englishPath = join(directory, "en.json");
    const chinesePath = join(directory, "zh-CN.json");
    writeFileSync(englishPath, JSON.stringify(english), "utf8");
    writeFileSync(chinesePath, JSON.stringify(chinese), "utf8");
    const runtime = (
      globalThis as unknown as { process: { cwd(): string; platform: string } }
    ).process;
    const desktopDirectory = runtime.cwd();
    const platform = runtime.platform;
    const packageArguments = [
      "--dir",
      desktopDirectory,
      "i18n:check",
      "--",
      englishPath,
      chinesePath,
    ];
    return platform === "win32"
      ? spawnSync(
          "cmd.exe",
          ["/d", "/s", "/c", "pnpm.cmd", ...packageArguments],
          { encoding: "utf8" },
        )
      : spawnSync(
          "pnpm",
          packageArguments,
          { encoding: "utf8" },
        );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
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

  it("accepts every current leaf without pinning future catalog growth", () => {
    expect(validateCatalogs(en, zhCN)).toBe(flattenKeys(en).length);
  });

  it("defines matching active-request plural variants", () => {
    expect(en.operation.result).toMatchObject({
      activeRequests_one:
        "{{formattedCount}} active request remains. WokCore is still serving it; try the update again later.",
      activeRequests_other:
        "{{formattedCount}} active requests remain. WokCore is still serving them; try the update again later.",
    });
    expect(zhCN.operation.result).toMatchObject({
      activeRequests_one: "仍有 {{formattedCount}} 个活动请求正在处理中。请稍后重试更新。",
      activeRequests_other: "仍有 {{formattedCount}} 个活动请求正在处理中。请稍后重试更新。",
    });
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

  it("rejects a dotted key segment before it can hide placeholder drift", () => {
    const [english, chinese] = dottedCollision();
    expect(() => validateCatalogs(english, chinese)).toThrow(
      'Catalog "en" namespace "common" contains dotted key segment "a.b".',
    );
  });

  it("rejects an empty key segment", () => {
    expect(() =>
      validateCatalogs(fixture({ "": "Empty" }), fixture({ "": "空" })),
    ).toThrow(
      'Catalog "en" namespace "common" contains an empty key segment.',
    );
  });

  it.each([
    ["root string", collapsedRootNamespace("not a namespace"), "core"],
    ["root array", collapsedRootNamespace([]), "core"],
    [
      "management string",
      collapsedManagementNamespace("not a namespace"),
      "management.providers",
    ],
    [
      "management array",
      collapsedManagementNamespace([]),
      "management.providers",
    ],
  ])("rejects a collapsed %s namespace", (_name, [english, chinese], path) => {
    expect(() => validateCatalogs(english, chinese)).toThrow(
      `Catalog "en" namespace "${path}" must be a plain object.`,
    );
  });

  it.each([
    "<strong>Retry</strong>",
    "</strong>",
    "<br />",
    '<custom-element data-state="ready">',
    "<!-- translator note -->",
    "<!doctype html>",
    "<?catalog note?>",
  ])(
    "rejects complete HTML-like markup %s",
    (value) => {
      const [english, chinese] = markup(value);
      expect(() => validateCatalogs(english, chinese)).toThrow(
        'Catalog "en" key "common.retry" must not contain HTML markup.',
      );
    },
  );

  it("allows ordinary i18next interpolation", () => {
    expect(
      validateCatalogs(
        fixture({ greeting: "Hello {{name}}" }),
        fixture({ greeting: "你好 {{name}}" }),
      ),
    ).toBe(1);
  });

  it.each([
    "A<B",
    "Run wokrouter <input.json",
    "<input.json>",
    "<!-- unfinished",
    "<!doctype html",
    "<?catalog note",
  ])("allows non-markup technical angle text %s", (value) => {
    const [english, chinese] = markup(value);
    expect(validateCatalogs(english, chinese)).toBe(1);
  });

  it.each([
    "A<B",
    "Run wokrouter <input.json",
    "<input.json>",
    "<!-- unfinished",
    "<!doctype html",
    "<?catalog note",
  ])("allows the real CLI technical angle text: %s", (value) => {
    const result = runCatalogChecker(markup(value));
    expect(result.status).toBe(0);
    expect(result.stdout).toContain("Translation catalogs match (1 keys).");
  });

  it.each([
    ["dotted-key collision", dottedCollision(), /dotted key segment/],
    [
      "collapsed root namespace",
      collapsedRootNamespace("not a namespace"),
      /namespace "core" must be a plain object/,
    ],
    [
      "collapsed management namespace",
      collapsedManagementNamespace("not a namespace"),
      /namespace "management\.providers" must be a plain object/,
    ],
    ["HTML comment", markup("<!-- translator note -->"), /HTML markup/],
    ["HTML doctype", markup("<!doctype html>"), /HTML markup/],
    ["processing-like markup", markup("<?catalog note?>"), /HTML markup/],
  ])("rejects the real CLI mutation: %s", (_name, catalogs, error) => {
    const result = runCatalogChecker(catalogs);
    expect(result.status).toBe(1);
    expect(result.stderr).toMatch(error);
  });

  it("registers the standalone catalog check command", () => {
    expect(packageManifest.scripts["i18n:check"]).toBe(
      "node scripts/check-i18n-catalogs.mjs",
    );
  });
});
