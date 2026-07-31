import type { SupportedLocale } from "./types";

export function formatLocalTime(value: string, locale: SupportedLocale): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    timeZoneName: "short",
  }).format(date);
}

export function formatNumber(value: number, locale: SupportedLocale): string {
  return new Intl.NumberFormat(locale).format(value);
}

export function formatBytes(value: number, locale: SupportedLocale): string {
  const numberFormat = new Intl.NumberFormat(locale, {
    minimumFractionDigits: value >= 1024 ? 1 : 0,
    maximumFractionDigits: 1,
  });
  if (value < 1024) {
    return `${numberFormat.format(value)} B`;
  }
  if (value < 1024 * 1024) {
    return `${numberFormat.format(value / 1024)} KiB`;
  }
  return `${numberFormat.format(value / (1024 * 1024))} MiB`;
}
