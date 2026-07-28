import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import {
  coreStatusQueryKey,
  getCoreStatus,
  startCore,
  stopCore,
  type CoreStatus,
} from "../control";

const stateCopy: Record<
  CoreStatus["state"],
  { title: string; summary: string; tone: "running" | "stopped" | "error" }
> = {
  missing: {
    title: "WokCore not installed",
    summary:
      "Install WokCore or register its trusted installation before starting local routing.",
    tone: "error",
  },
  stopped: {
    title: "WokCore stopped",
    summary:
      "The desktop app is available, but local routing stays offline until WokCore starts.",
    tone: "stopped",
  },
  starting: {
    title: "WokCore starting",
    summary: "WokCore is preparing its local management service.",
    tone: "stopped",
  },
  running: {
    title: "WokCore running",
    summary: "WokCore is ready to accept local client traffic.",
    tone: "running",
  },
  draining: {
    title: "WokCore draining",
    summary:
      "Existing requests are finishing. New work remains offline until draining completes.",
    tone: "stopped",
  },
  authorization_required: {
    title: "WokRouter authorization required",
    summary:
      "Authorize this desktop client before it can inspect or control the WokCore service.",
    tone: "error",
  },
  incompatible: {
    title: "WokCore update required",
    summary:
      "The installed WokCore management API is not compatible with this WokRouter version.",
    tone: "error",
  },
  invalid_runtime: {
    title: "WokCore runtime invalid",
    summary:
      "WokRouter could not verify the configured WokCore installation or runtime response.",
    tone: "error",
  },
};

function formatPhase(phase: NonNullable<CoreStatus["phase"]>): string {
  return phase
    .split("_")
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function LoadingHealth() {
  return (
    <>
      <h1 id="core-health-heading">WokCore health</h1>
      <div className="health-skeleton" aria-hidden="true">
        <span className="skeleton skeleton--status" />
        <span className="skeleton skeleton--title" />
        <span className="skeleton skeleton--body" />
        <span className="skeleton skeleton--meta" />
      </div>
    </>
  );
}

export function CoreHealth() {
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: coreStatusQueryKey,
    queryFn: getCoreStatus,
  });
  const refreshStatus = async () => {
    await queryClient.invalidateQueries({ queryKey: coreStatusQueryKey });
  };
  const start = useMutation({
    mutationFn: startCore,
    onSuccess: refreshStatus,
  });
  const stop = useMutation({
    mutationFn: stopCore,
    onSuccess: refreshStatus,
  });

  let announcement: string;
  let content: ReactNode;
  if (status.isPending) {
    announcement = "Checking WokCore status";
    content = <LoadingHealth />;
  } else if (status.isError) {
    announcement = status.isFetching
      ? "Checking WokCore status"
      : "WokCore status unavailable. Check again.";
    content = (
      <>
        <p className="section-label">Runtime status</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="core-health-heading">WokCore status unavailable</h1>
        </div>
        <p className="health-summary">
          WokRouter could not confirm whether WokCore is available. Your
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
    const copy = stateCopy[status.data.state];
    const isRunning = status.data.state === "running";
    const canStart =
      status.data.state === "stopped" ||
      status.data.state === "authorization_required";
    const canRetry =
      status.data.state === "missing" ||
      status.data.state === "incompatible" ||
      status.data.state === "invalid_runtime";
    const actionError = start.isError
      ? "WokCore could not start"
      : stop.isError
        ? "WokCore could not stop"
        : undefined;

    if (start.isPending) {
      announcement = "Starting WokCore.";
    } else if (stop.isPending) {
      announcement = "Stopping WokCore.";
    } else if (actionError) {
      announcement = `${actionError}. You can safely try again.`;
    } else {
      announcement = `${copy.title}.${status.data.version ? ` Version ${status.data.version}.` : ""}`;
    }

    content = (
      <>
        <p className="section-label">Runtime status</p>
        <div className={`status-line status-line--${copy.tone}`}>
          <span className="status-mark" aria-hidden="true">
            {copy.tone === "running"
              ? "✓"
              : copy.tone === "error"
                ? "!"
                : "–"}
          </span>
          <h1 id="core-health-heading">{copy.title}</h1>
        </div>
        <p className="health-summary">{copy.summary}</p>
        <dl className="health-meta">
          <div>
            <dt>Version</dt>
            <dd dir="ltr">
              {status.data.version ? (
                <code>{status.data.version}</code>
              ) : (
                "Unavailable"
              )}
            </dd>
          </div>
          <div>
            <dt>Connection</dt>
            <dd>{isRunning ? "Loopback HTTP" : "Not connected"}</dd>
          </div>
          {status.data.phase && (
            <div>
              <dt>Phase</dt>
              <dd>{formatPhase(status.data.phase)}</dd>
            </div>
          )}
          {status.data.active_requests !== undefined && (
            <div>
              <dt>Active requests</dt>
              <dd>{status.data.active_requests}</dd>
            </div>
          )}
        </dl>
        <div className="recovery">
          {actionError && (
            <p className="recovery-error">
              <strong>{actionError}</strong>
              <span>
                The service state was not assumed to have changed. Check its
                status or safely try the action again.
              </span>
            </p>
          )}
          {canStart && (
            <button
              className="button button--primary"
              type="button"
              disabled={start.isPending || stop.isPending}
              onClick={() => start.mutate()}
            >
              {start.isPending
                ? "Starting WokCore…"
                : status.data.state === "authorization_required"
                  ? "Authorize WokRouter"
                  : start.isError
                    ? "Try starting again"
                    : "Start WokCore"}
            </button>
          )}
          {isRunning && (
            <button
              className="button button--secondary"
              type="button"
              disabled={start.isPending || stop.isPending}
              onClick={() => stop.mutate()}
            >
              {stop.isPending
                ? "Stopping WokCore…"
                : stop.isError
                  ? "Try stopping again"
                  : "Stop WokCore"}
            </button>
          )}
          {canRetry && (
            <button
              className="button button--primary"
              type="button"
              disabled={status.isFetching}
              onClick={() => void status.refetch()}
            >
              {status.isFetching ? "Checking…" : "Check again"}
            </button>
          )}
          <p className="action-note">
            Closing this window never stops WokCore.
          </p>
        </div>
      </>
    );
  }

  return (
    <section className="health-panel" aria-labelledby="core-health-heading">
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
