import "./types";

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";
import type { SupportedLocale } from "./types";

export type { SupportedLocale } from "./types";

export async function initializeI18n(locale: SupportedLocale): Promise<void> {
  if (i18n.isInitialized) {
    await i18n.changeLanguage(locale);
    return;
  }
  await i18n.use(initReactI18next).init({
    lng: locale,
    fallbackLng: "en",
    supportedLngs: ["en", "zh-CN"],
    nonExplicitSupportedLngs: false,
    interpolation: { escapeValue: false },
    resources: {
      en: { translation: en },
      "zh-CN": { translation: zhCN },
    },
  });
}

export { i18n };
