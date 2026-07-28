import { CoreHealth } from "./components/CoreHealth";
import { ManagementPanel } from "./components/ManagementPanel";

export function App() {
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            W
          </span>
          <span className="brand-name">WokRouter</span>
        </div>
        <p className="surface-name">Local desktop control</p>
      </header>
      <main className="app-main">
        <CoreHealth />
        <ManagementPanel />
      </main>
      <footer className="app-footer">
        Desktop controls communicate with WokCore over loopback HTTP.
      </footer>
    </div>
  );
}
