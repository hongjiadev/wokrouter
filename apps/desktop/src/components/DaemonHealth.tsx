import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getDaemonStatus, startDaemon } from "../control";

const daemonStatusQueryKey = ["daemon-status"] as const;

function LoadingHealth() {
  return (
    <section className="health-panel" aria-labelledby="daemon-health-heading">
      <h1 id="daemon-health-heading">Daemon health</h1>
      <div
        className="health-skeleton"
        role="status"
        aria-label="Checking daemon status"
      >
        <span className="skeleton skeleton--status" aria-hidden="true" />
        <span className="skeleton skeleton--title" aria-hidden="true" />
        <span className="skeleton skeleton--body" aria-hidden="true" />
        <span className="skeleton skeleton--meta" aria-hidden="true" />
      </div>
    </section>
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

  if (status.isPending) {
    return <LoadingHealth />;
  }

  if (status.isError) {
    return (
      <section className="health-panel" aria-labelledby="daemon-health-heading">
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
      </section>
    );
  }

  const isRunning = status.data.state === "running";

  return (
    <section className="health-panel" aria-labelledby="daemon-health-heading">
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
            <p className="recovery-error" role="alert">
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
    </section>
  );
}
