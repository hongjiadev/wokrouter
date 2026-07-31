import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const expectedTopLevelNamespaces = [
  "app",
  "common",
  "core",
  "errors",
  "management",
  "operation",
];
const expectedManagementNamespaces = [
  "diagnostics",
  "providers",
  "sessions",
  "usage",
];
const htmlElementMarkup =
  /<\/?[A-Za-z][A-Za-z0-9-]*(?:\s+[^<>]*?)?\s*\/?>/u;
const htmlCommentMarkup = /<!--[\s\S]*?-->/u;
const htmlDoctypeMarkup = /<!doctype(?:\s+[^<>]*?)?>/iu;
const htmlProcessingMarkup = /<\?[A-Za-z][\s\S]*?\?>/u;
const interpolationPlaceholder = /{{\s*([A-Za-z0-9_.-]+)\s*}}/g;

function containsHtmlMarkup(value) {
  return (
    htmlElementMarkup.test(value) ||
    htmlCommentMarkup.test(value) ||
    htmlDoctypeMarkup.test(value) ||
    htmlProcessingMarkup.test(value)
  );
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function requirePlainNamespace(value, locale, path) {
  if (!isPlainObject(value)) {
    throw new Error(
      `Catalog "${locale}" namespace "${path}" must be a plain object.`,
    );
  }
}

function requireExactNamespaces(catalog, locale, path, expected) {
  requirePlainNamespace(catalog, locale, path);
  const actual = Object.keys(catalog).sort();
  if (
    actual.length !== expected.length ||
    actual.some((key, index) => key !== expected[index])
  ) {
    throw new Error(
      `Catalog "${locale}" namespace "${path}" must contain exactly: ${expected.join(
        ", ",
      )}.`,
    );
  }
}

function flattenCatalog(catalog, locale, prefix = "", leaves = new Map()) {
  for (const key of Object.keys(catalog).sort()) {
    const namespace = prefix || "<root>";
    if (key === "") {
      throw new Error(
        `Catalog "${locale}" namespace "${namespace}" contains an empty key segment.`,
      );
    }
    if (key.includes(".")) {
      throw new Error(
        `Catalog "${locale}" namespace "${namespace}" contains dotted key segment "${key}".`,
      );
    }
    const value = catalog[key];
    const path = prefix ? `${prefix}.${key}` : key;
    if (isPlainObject(value)) {
      flattenCatalog(value, locale, path, leaves);
      continue;
    }
    if (typeof value !== "string" || value.trim() === "") {
      throw new Error(
        `Catalog "${locale}" key "${path}" must be a non-empty string.`,
      );
    }
    if (containsHtmlMarkup(value)) {
      throw new Error(
        `Catalog "${locale}" key "${path}" must not contain HTML markup.`,
      );
    }
    leaves.set(path, value);
  }
  return leaves;
}

function placeholders(value) {
  return [
    ...new Set(
      [...value.matchAll(interpolationPlaceholder)].map((match) => match[1]),
    ),
  ].sort();
}

function sameStrings(left, right) {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

export function validateCatalogs(english, simplifiedChinese) {
  requireExactNamespaces(
    english,
    "en",
    "<root>",
    expectedTopLevelNamespaces,
  );
  requireExactNamespaces(
    simplifiedChinese,
    "zh-CN",
    "<root>",
    expectedTopLevelNamespaces,
  );
  for (const namespace of expectedTopLevelNamespaces) {
    requirePlainNamespace(english[namespace], "en", namespace);
    requirePlainNamespace(simplifiedChinese[namespace], "zh-CN", namespace);
  }
  requireExactNamespaces(
    english.management,
    "en",
    "management",
    expectedManagementNamespaces,
  );
  requireExactNamespaces(
    simplifiedChinese.management,
    "zh-CN",
    "management",
    expectedManagementNamespaces,
  );
  for (const namespace of expectedManagementNamespaces) {
    const path = `management.${namespace}`;
    requirePlainNamespace(english.management[namespace], "en", path);
    requirePlainNamespace(
      simplifiedChinese.management[namespace],
      "zh-CN",
      path,
    );
  }

  const catalogs = [
    ["en", flattenCatalog(english, "en")],
    ["zh-CN", flattenCatalog(simplifiedChinese, "zh-CN")],
  ];
  const [englishLocale, englishLeaves] = catalogs[0];
  const [chineseLocale, chineseLeaves] = catalogs[1];
  const keys = [...new Set([...englishLeaves.keys(), ...chineseLeaves.keys()])]
    .sort();

  for (const key of keys) {
    if (!englishLeaves.has(key)) {
      throw new Error(`Catalog "${englishLocale}" is missing key "${key}".`);
    }
    if (!chineseLeaves.has(key)) {
      throw new Error(`Catalog "${chineseLocale}" is missing key "${key}".`);
    }
    const englishPlaceholders = placeholders(englishLeaves.get(key));
    const chinesePlaceholders = placeholders(chineseLeaves.get(key));
    if (!sameStrings(englishPlaceholders, chinesePlaceholders)) {
      throw new Error(
        `Catalog placeholder mismatch at "${key}": ` +
          `en has [${englishPlaceholders.join(", ")}], ` +
          `zh-CN has [${chinesePlaceholders.join(", ")}].`,
      );
    }
  }

  return keys.length;
}

async function readCatalog(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function resolveCatalogPaths(desktopDirectory, arguments_) {
  const paths = arguments_[0] === "--" ? arguments_.slice(1) : arguments_;
  if (paths.length === 0) {
    const localeDirectory = resolve(desktopDirectory, "src", "i18n", "locales");
    return [
      resolve(localeDirectory, "en.json"),
      resolve(localeDirectory, "zh-CN.json"),
    ];
  }
  if (paths.length !== 2) {
    throw new Error(
      "Usage: check-i18n-catalogs.mjs [<en.json> <zh-CN.json>]",
    );
  }
  return paths.map((path) => resolve(path));
}

async function main(arguments_ = process.argv.slice(2)) {
  const desktopDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const [englishPath, chinesePath] = resolveCatalogPaths(
    desktopDirectory,
    arguments_,
  );
  const english = await readCatalog(englishPath);
  const simplifiedChinese = await readCatalog(chinesePath);
  const keyCount = validateCatalogs(english, simplifiedChinese);
  process.stdout.write(`Translation catalogs match (${keyCount} keys).\n`);
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    process.stderr.write(
      `${error instanceof Error ? error.message : "Catalog validation failed."}\n`,
    );
    process.exitCode = 1;
  });
}
