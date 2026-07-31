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
const htmlMarkup = /<\/?[A-Za-z][^>]*>/u;
const interpolationPlaceholder = /{{\s*([A-Za-z0-9_.-]+)\s*}}/g;

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireExactNamespaces(catalog, locale, path, expected) {
  if (!isPlainObject(catalog)) {
    throw new Error(`Catalog "${locale}" namespace "${path}" must be an object.`);
  }
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
    if (htmlMarkup.test(value)) {
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

async function main() {
  const desktopDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const localeDirectory = resolve(desktopDirectory, "src", "i18n", "locales");
  const english = await readCatalog(resolve(localeDirectory, "en.json"));
  const simplifiedChinese = await readCatalog(
    resolve(localeDirectory, "zh-CN.json"),
  );
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
