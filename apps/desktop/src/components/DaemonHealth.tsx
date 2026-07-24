import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { getDaemonStatus, startDaemon } from "../control";

const daemonStatusQueryKey = ["daemon-status"] as const;

function LoadingHealth() {
  return (
    <>
      <h1 id="daemon-health-heading">Daemon health</h1>
      <div className="health-skeleton" aria-hidden="true">
        <span className="skeleton skeleton--status" />
        <span className="skeleton skeleton--title" />
        <span className="skeleton skeleton--body" />
        <span className="skeleton skeleton--meta" />
      </div>
    </>
  );
}

export function DaemonHealth() {
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: daemonStatusQueryKey,
    queryFn: getDaemonStatus,
  });
  const start = useMutation({
    mutationFn: startDaemon,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: daemonStatusQueryKey });
    },
  });

  let announcement: string;
  let content: ReactNode;
  if (status.isPending) {
    announcement = "Checking daemon status";
    content = <LoadingHealth />;
  } else if (status.isError) {
    announcement = status.isFetching
      ? "Checking daemon status"
      : "Daemon status unavailable. Check again.";
    content = (
      <>
        <p className="section-label">Runtime status</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="daemon-health-heading">Daemon status unavailable</h1>
        </div>
        <p className="health-summary">
          WokRouter couldn’t confirm whether the local daemon is available. Your
          configuration has not been changed.
        </p>
        <button
          className="button button--primary"
          type="button"
          disabled={status.isFetching}
          onClick={() => void status.refetch()}
        >
          {status.isFetching ? "Checking…" : "Check again"}
        </button>
      </>
    );
  } else {
    const isRunning = status.data.state === "running";
    if (isRunning) {
      announcement = `Daemon running. Version ${status.data.version}.`;
    } else if (start.isPending) {
      announcement = "Starting WokRouter.";
    } else if (start.isError) {
      announcement = "WokRouter couldn’t start. Try starting again.";
    } else {
      announcement = `Daemon stopped. Version ${status.data.version}. Start WokRouter is available.`;
    }
    content = (
      <>
        <p className="section-label">Runtime status</p>
        <div
          className={`status-line status-line--${isRunning ? "running" : "stopped"}`}
        >
          <span className="status-mark" aria-hidden="true">
            {isRunning ? "✓" : "—"}
          </span>
          <h1 id="daemon-health-heading">
            {isRunning ? "Daemon running" : "Daemon stopped"}
          </h1>
        </div>
        <p className="health-summary">
          {isRunning
            ? "WokRouter is ready to accept local client traffic."
            : "The desktop app is available, but routing stays offline until the daemon starts."}
        </p>
        <dl className="health-meta">
          <div>
            <dt>Version</dt>
            <dd dir="ltr">
              <code>{status.data.version}</code>
            </dd>
          </div>
          <div>
            <dt>Connection</dt>
            <dd>{isRunning ? "Local control IPC" : "Not connected"}</dd>
          </div>
        </dl>
        {!isRunning && (
          <div className="recovery">
            {start.isError && (
              <p className="recovery-error">
                <strong>WokRouter couldn’t start</strong>
                <span>
                  The daemon did not become ready. Nothing else was changed; you
                  can safely try again.
                </span>
              </p>
            )}
            <button
              className="button button--primary"
              type="button"
              disabled={start.isPending}
              onClick={() => start.mutate()}
            >
              {start.isPending
                ? "Starting WokRouter…"
                : start.isError
                  ? "Try starting again"
                  : "Start WokRouter"}
            </button>
            <p className="action-note">
              Closing this window never stops the daemon.
            </p>
          </div>
        )}
      </>
    );
  }

  return (
    <section className="health-panel" aria-labelledby="daemon-health-heading">
      <p
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
        aria-label={announcement}
      >
        {announcement}
      </p>
      {content}
    </section>
  );
}
