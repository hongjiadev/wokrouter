import "i18next";
import type en from "./locales/en.json";

export type SupportedLocale = "en" | "zh-CN";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: {
      translation: typeof en;
    };
  }
}
