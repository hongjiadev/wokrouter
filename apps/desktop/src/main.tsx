import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { initializeI18n } from "./i18n";
import {
  browserLocaleCandidates,
  initializeDocumentLocale,
  resolveSupportedLocale,
} from "./locale";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: true,
      retry: 1,
      staleTime: 5_000,
    },
    mutations: { retry: false },
  },
});

export async function bootstrap(): Promise<void> {
  const systemLocale = await invoke<string>("system_locale").catch(
    () => undefined,
  );
  const locale = resolveSupportedLocale(
    systemLocale,
    browserLocaleCandidates(window.navigator),
  );
  await initializeI18n(locale);
  initializeDocumentLocale(document.documentElement, locale);

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("WokRouter desktop root is missing.");
  }

  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}

void bootstrap();
