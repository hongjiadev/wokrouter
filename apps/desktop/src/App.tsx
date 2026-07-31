import { useTranslation } from "react-i18next";

import { CoreLifecycle } from "./components/CoreLifecycle";

export function App() {
  const { t } = useTranslation();

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            W
          </span>
          <span className="brand-name">WokRouter</span>
        </div>
        <p className="surface-name">{t("app.localDesktopControl")}</p>
      </header>
      <main className="app-main">
        <CoreLifecycle />
      </main>
      <footer className="app-footer">{t("app.loopbackFooter")}</footer>
    </div>
  );
}
