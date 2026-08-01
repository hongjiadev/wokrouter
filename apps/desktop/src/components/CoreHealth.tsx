import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import {
  coreStatusQueryKey,
  getCoreStatus,
  startCore,
  stopCore,
  type CoreStatus,
} from "../control";
import type { CoreUpdateCheck } from "../coreOperation";
import { isCoreUpdateEligible } from "../coreUpdateEligibility";

type CoreHealthProps = {
  updatesEnabled?: boolean;
  updateCheck?: CoreUpdateCheck;
  updateCheckFailed?: boolean;
  updateCheckPending?: boolean;
  onCheckForUpdates?: () => void;
  onUpgrade?: (trigger: HTMLButtonElement) => void;
};

const stateCopy: Record<
  CoreStatus["state"],
  {
    key:
      | "core.state.missing"
      | "core.state.stopped"
      | "core.state.starting"
      | "core.state.running"
      | "core.state.draining"
      | "core.state.authorizationRequired"
      | "core.state.incompatible"
      | "core.state.invalidRuntime";
    tone: "running" | "stopped" | "error";
  }
> = {
  missing: {
    key: "core.state.missing",
    tone: "error",
  },
  stopped: {
    key: "core.state.stopped",
    tone: "stopped",
  },
  starting: {
    key: "core.state.starting",
    tone: "stopped",
  },
  running: {
    key: "core.state.running",
    tone: "running",
  },
  draining: {
    key: "core.state.draining",
    tone: "stopped",
  },
  authorization_required: {
    key: "core.state.authorizationRequired",
    tone: "error",
  },
  incompatible: {
    key: "core.state.incompatible",
    tone: "error",
  },
  invalid_runtime: {
    key: "core.state.invalidRuntime",
    tone: "error",
  },
};

const runtimeChannelKeys: Record<
  CoreStatus["runtime_channel"],
  "core.runtimeChannel.development" | "core.runtimeChannel.production"
> = {
  development: "core.runtimeChannel.development",
  production: "core.runtimeChannel.production",
};

const phaseKeys: Record<
  NonNullable<CoreStatus["phase"]>,
  | "core.phase.starting"
  | "core.phase.running"
  | "core.phase.draining"
  | "core.phase.awaitingCancellation"
  | "core.phase.stopping"
> = {
  starting: "core.phase.starting",
  running: "core.phase.running",
  draining: "core.phase.draining",
  awaiting_cancellation: "core.phase.awaitingCancellation",
  stopping: "core.phase.stopping",
};

function LoadingHealth() {
  const { t } = useTranslation();

  return (
    <>
      <h1 id="core-health-heading" tabIndex={-1}>
        {t("core.heading")}
      </h1>
      <div className="health-skeleton" aria-hidden="true">
        <span className="skeleton skeleton--status" />
        <span className="skeleton skeleton--title" />
        <span className="skeleton skeleton--body" />
        <span className="skeleton skeleton--meta" />
      </div>
    </>
  );
}

export function CoreHealth({
  updatesEnabled = true,
  updateCheck,
  updateCheckFailed = false,
  updateCheckPending = false,
  onCheckForUpdates,
  onUpgrade,
}: CoreHealthProps = {}) {
  const { i18n, t } = useTranslation();
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(i18n.resolvedLanguage),
    [i18n.resolvedLanguage],
  );
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
    announcement = t("core.announcement.checking");
    content = <LoadingHealth />;
  } else if (status.isError) {
    announcement = status.isFetching
      ? t("core.announcement.checking")
      : t("core.statusUnavailable.announcement");
    content = (
      <>
        <p className="section-label">{t("core.runtimeStatus")}</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="core-health-heading" tabIndex={-1}>
            {t("core.statusUnavailable.title")}
          </h1>
        </div>
        <p className="health-summary">{t("core.statusUnavailable.summary")}</p>
        <button
          className="button button--primary"
          type="button"
          disabled={status.isFetching}
          onClick={() => void status.refetch()}
        >
          {status.isFetching
            ? t("core.action.checking")
            : t("core.action.checkAgain")}
        </button>
      </>
    );
  } else {
    const copy = stateCopy[status.data.state];
    const isRunning = status.data.state === "running";
    const isDevelopment = status.data.runtime_channel === "development";
    const updateEligible = isCoreUpdateEligible(status.data);
    const canStart =
      !isDevelopment &&
      (status.data.state === "stopped" ||
        status.data.state === "authorization_required");
    const canRetry =
      status.data.state === "incompatible" ||
      status.data.state === "invalid_runtime";
    const updateAvailable =
      updatesEnabled &&
      updateEligible &&
      updateCheck?.code === "update_available" &&
      updateCheck.targetVersion !== undefined;
    const canRetryUpdateCheck =
      updatesEnabled &&
      updateEligible &&
      updateCheckFailed &&
      onCheckForUpdates !== undefined;
    const actionErrorKey = start.isError
      ? "core.actionFailure.start"
      : stop.isError
        ? "core.actionFailure.stop"
        : undefined;
    const actionError = actionErrorKey ? t(actionErrorKey) : undefined;
    const title = t(`${copy.key}.title`);

    if (start.isPending) {
      announcement = t("core.announcement.starting");
    } else if (stop.isPending) {
      announcement = t("core.announcement.stopping");
    } else if (actionError) {
      announcement = t("core.actionFailure.safeRetry", {
        error: actionError,
      });
    } else {
      announcement = status.data.version
        ? t("core.announcement.version", {
            title,
            version: status.data.version,
          })
        : t("core.announcement.state", { title });
    }

    content = (
      <>
        <p className="section-label">{t("core.runtimeStatus")}</p>
        <div className={`status-line status-line--${copy.tone}`}>
          <span className="status-mark" aria-hidden="true">
            {copy.tone === "running"
              ? "✓"
              : copy.tone === "error"
                ? "!"
                : "–"}
          </span>
          <h1 id="core-health-heading" tabIndex={-1}>
            {title}
          </h1>
        </div>
        <p className="health-summary">{t(`${copy.key}.summary`)}</p>
        <dl className="health-meta">
          <div>
            <dt>{t("core.field.runtimeChannel")}</dt>
            <dd>{t(runtimeChannelKeys[status.data.runtime_channel])}</dd>
          </div>
          <div>
            <dt>{t("core.field.version")}</dt>
            <dd dir="ltr">
              {status.data.version ? (
                <code>{status.data.version}</code>
              ) : (
                t("common.unavailable")
              )}
            </dd>
          </div>
          <div>
            <dt>{t("core.field.connection")}</dt>
            <dd>
              {isRunning
                ? t("core.connection.loopbackHttp")
                : t("core.connection.notConnected")}
            </dd>
          </div>
          {status.data.phase && (
            <div>
              <dt>{t("core.field.phase")}</dt>
              <dd>{t(phaseKeys[status.data.phase])}</dd>
            </div>
          )}
          {status.data.active_requests !== undefined && (
            <div>
              <dt>{t("core.field.activeRequests")}</dt>
              <dd>{numberFormatter.format(status.data.active_requests)}</dd>
            </div>
          )}
        </dl>
        <div className="recovery">
          {actionError && (
            <p className="recovery-error">
              <strong>{actionError}</strong>
              <span>{t("core.actionFailure.summary")}</span>
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
                ? t("core.action.starting")
                : status.data.state === "authorization_required"
                  ? t("core.action.authorize")
                  : start.isError
                    ? t("core.action.retryStart")
                    : t("core.action.start")}
            </button>
          )}
          {isRunning && !isDevelopment && (
            <button
              className="button button--secondary"
              type="button"
              disabled={start.isPending || stop.isPending}
              onClick={() => stop.mutate()}
            >
              {stop.isPending
                ? t("core.action.stopping")
                : stop.isError
                  ? t("core.action.retryStop")
                  : t("core.action.stop")}
            </button>
          )}
          {canRetry && (
            <button
              className="button button--primary"
              type="button"
              disabled={status.isFetching}
              onClick={() => void status.refetch()}
            >
              {status.isFetching
                ? t("core.action.checking")
                : t("core.action.checkAgain")}
            </button>
          )}
          {updateAvailable && onUpgrade && (
            <button
              className="button button--primary"
              type="button"
              onClick={(event) => onUpgrade(event.currentTarget)}
            >
              {t("operation.update.trigger")}
            </button>
          )}
          {canRetryUpdateCheck && (
            <div className="recovery-error">
              <strong>{t("operation.update.checkUnavailable")}</strong>
              <span>{t("operation.update.checkUnavailableSummary")}</span>
              <button
                className="button button--secondary"
                type="button"
                disabled={updateCheckPending}
                onClick={onCheckForUpdates}
              >
                {updateCheckPending
                  ? t("operation.update.checking")
                  : t("operation.update.check")}
              </button>
            </div>
          )}
          <p className="action-note">
            {isDevelopment
              ? t("core.developmentActionNote")
              : t("core.actionNote")}
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
