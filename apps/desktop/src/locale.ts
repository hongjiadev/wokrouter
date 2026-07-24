export type DocumentDirection = "ltr" | "rtl";

export interface DocumentLocale {
  lang: string;
  dir: DocumentDirection;
}

interface NavigatorLocaleSource {
  languages?: readonly string[];
  language?: string;
}

const rightToLeftLanguages = new Set(["ar", "fa", "ur"]);

export function resolveDocumentLocale(
  candidates: readonly unknown[],
): DocumentLocale {
  for (const candidate of candidates) {
    if (typeof candidate !== "string" || candidate.trim() === "") {
      continue;
    }
    try {
      const [lang] = Intl.getCanonicalLocales(
        candidate.trim().replaceAll("_", "-"),
      );
      if (lang) {
        const language = lang.split("-", 1)[0].toLowerCase();
        return {
          lang,
          dir: rightToLeftLanguages.has(language) ? "rtl" : "ltr",
        };
      }
    } catch {
      // Try the next operating-system locale candidate.
    }
  }
  return { lang: "en", dir: "ltr" };
}

export function initializeDocumentLocale(
  root: HTMLElement = document.documentElement,
  source: NavigatorLocaleSource = navigator,
): DocumentLocale {
  const locale = resolveDocumentLocale([
    ...(source.languages ?? []),
    source.language,
  ]);
  root.lang = locale.lang;
  root.dir = locale.dir;
  return locale;
}
