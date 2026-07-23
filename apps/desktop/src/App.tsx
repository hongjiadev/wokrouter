import { DaemonHealth } from "./components/DaemonHealth";

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
        <DaemonHealth />
      </main>
      <footer className="app-footer">
        Desktop controls communicate with the daemon over local IPC.
      </footer>
    </div>
  );
}
