import type { SupportedLocale } from "./i18n";

export type { SupportedLocale } from "./i18n";

export type DocumentDirection = "ltr" | "rtl";

export interface DocumentLocale {
  lang: SupportedLocale;
  dir: DocumentDirection;
}

export interface NavigatorLocaleSource {
  languages?: readonly string[];
  language?: string;
}

function matchCandidate(candidate: string): SupportedLocale {
  const value = candidate.trim().replaceAll("_", "-").toLowerCase();
  if (
    value === "zh" ||
    value === "zh-cn" ||
    value === "zh-hans" ||
    value.startsWith("zh-hans-")
  ) {
    return "zh-CN";
  }
  return "en";
}

export function resolveSupportedLocale(
  systemLocale: string | null | undefined,
  browserLocales: readonly string[],
): SupportedLocale {
  if (systemLocale?.trim()) {
    return matchCandidate(systemLocale);
  }
  const browser = browserLocales.find((candidate) => candidate.trim());
  return browser ? matchCandidate(browser) : "en";
}

export function browserLocaleCandidates(
  source: NavigatorLocaleSource,
): string[] {
  return [...new Set([...(source.languages ?? []), source.language])].filter(
    (candidate): candidate is string => typeof candidate === "string",
  );
}

export function initializeDocumentLocale(
  root: HTMLElement,
  locale: SupportedLocale,
): DocumentLocale {
  const documentLocale = { lang: locale, dir: "ltr" } as const;
  root.lang = documentLocale.lang;
  root.dir = documentLocale.dir;
  return documentLocale;
}
