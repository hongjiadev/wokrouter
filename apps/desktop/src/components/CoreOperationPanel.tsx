import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { CoreStatus } from "../control";
import type { CoreOperation } from "../coreOperation";

type CoreOperationPanelProps = {
  operation: CoreOperation;
  onRetry: (trigger?: HTMLButtonElement) => void;
  diagnosticsAvailable?: boolean;
  onOpenDiagnostics?: () => void;
  coreState?: CoreStatus["state"];
  onCheckCoreStatus?: () =>
    | CoreStatus["state"]
    | Promise<CoreStatus["state"]>;
};

type OfflineStatusCheck =
  | { state: "idle" | "checking" | "failed" }
  | { state: "completed"; coreState: CoreStatus["state"] };

const coreStateTitleKeys: Record<
  CoreStatus["state"],
  | "core.state.missing.title"
  | "core.state.stopped.title"
  | "core.state.starting.title"
  | "core.state.running.title"
  | "core.state.draining.title"
  | "core.state.authorizationRequired.title"
  | "core.state.incompatible.title"
  | "core.state.invalidRuntime.title"
> = {
  missing: "core.state.missing.title",
  stopped: "core.state.stopped.title",
  starting: "core.state.starting.title",
  running: "core.state.running.title",
  draining: "core.state.draining.title",
  authorization_required: "core.state.authorizationRequired.title",
  incompatible: "core.state.incompatible.title",
  invalid_runtime: "core.state.invalidRuntime.title",
};

const phaseKeys = {
  checking_release: "operation.phase.checkingRelease",
  downloading: "operation.phase.downloading",
  verifying: "operation.phase.verifying",
  installing: "operation.phase.installing",
  preparing_service: "operation.phase.preparingService",
  draining: "operation.phase.draining",
  stopping: "operation.phase.stopping",
  starting: "operation.phase.starting",
  authorizing: "operation.phase.authorizing",
  verifying_runtime: "operation.phase.verifyingRuntime",
  rolling_back: "operation.phase.rollingBack",
  completed: "operation.phase.completed",
} as const satisfies Record<CoreOperation["phase"], string>;

const progressPhaseKeys = {
  checking_release: "operation.progress.ariaPhase.checkingRelease",
  downloading: "operation.phase.downloading",
  verifying: "operation.progress.ariaPhase.verifying",
  installing: "operation.progress.ariaPhase.installing",
  preparing_service: "operation.progress.ariaPhase.preparingService",
  draining: "operation.progress.ariaPhase.draining",
  stopping: "operation.progress.ariaPhase.stopping",
  starting: "operation.progress.ariaPhase.starting",
  authorizing: "operation.progress.ariaPhase.authorizing",
  verifying_runtime: "operation.progress.ariaPhase.verifyingRuntime",
  rolling_back: "operation.progress.ariaPhase.rollingBack",
  completed: "operation.progress.ariaPhase.completed",
} as const satisfies Record<CoreOperation["phase"], string>;

function errorTranslationKey(errorCode: string | undefined) {
  switch (errorCode) {
    case "download_failed":
      return "errors.downloadFailed" as const;
    case "invalid_install_state":
      return "errors.invalidInstallState" as const;
    case "invalid_manifest":
      return "errors.invalidManifest" as const;
    case "invalid_signature":
      return "errors.invalidSignature" as const;
    case "incompatible_manifest":
      return "errors.incompatibleManifest" as const;
    case "artifact_size_mismatch":
      return "errors.artifactSizeMismatch" as const;
    case "artifact_hash_mismatch":
      return "errors.artifactHashMismatch" as const;
    case "invalid_archive":
      return "errors.invalidArchive" as const;
    case "unsafe_install_location":
      return "errors.unsafeInstallLocation" as const;
    case "install_in_progress":
      return "errors.installInProgress" as const;
    case "install_failed":
      return "errors.installFailed" as const;
    case "install_record_failed":
      return "errors.installRecordFailed" as const;
    case "start_failed":
      return "errors.startFailed" as const;
    case "authorization_failed":
      return "errors.authorizationFailed" as const;
    case "update_unavailable":
      return "errors.updateUnavailable" as const;
    case "update_verification_failed":
      return "errors.updateVerificationFailed" as const;
    case "active_requests_remain":
      return "errors.activeRequestsRemain" as const;
    case "rolled_back":
      return "errors.rolledBack" as const;
    case "update_install_failed":
      return "errors.updateInstallFailed" as const;
    case "recovery_required":
      return "errors.recoveryRequired" as const;
    case "operation_in_progress":
      return "errors.operationInProgress" as const;
    case "invalid_progress":
      return "errors.invalidProgress" as const;
    default:
      return "errors.unknown" as const;
  }
}

function useByteFormatter(locale: string | undefined): (bytes: number) => string {
  const numberFormat = useMemo(
    () =>
      new Intl.NumberFormat(locale, {
        maximumFractionDigits: 1,
      }),
    [locale],
  );
  return (bytes) => {
    const units = ["B", "KB", "MB", "GB", "TB"] as const;
    let value = bytes;
    let unitIndex = 0;
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }
    return `${numberFormat.format(value)} ${units[unitIndex]}`;
  };
}

function versionRows(
  operation: CoreOperation,
  currentLabel: string,
  targetLabel: string,
) {
  if (
    operation.currentVersion === undefined &&
    operation.targetVersion === undefined
  ) {
    return null;
  }
  return (
    <dl className="core-operation__versions">
      {operation.currentVersion !== undefined && (
        <div>
          <dt>{currentLabel}</dt>
          <dd>
            <code dir="ltr">{operation.currentVersion}</code>
          </dd>
        </div>
      )}
      {operation.targetVersion !== undefined && (
        <div>
          <dt>{targetLabel}</dt>
          <dd>
            <code dir="ltr">{operation.targetVersion}</code>
          </dd>
        </div>
      )}
    </dl>
  );
}

export function CoreOperationPanel({
  operation,
  onRetry,
  diagnosticsAvailable = false,
  onOpenDiagnostics,
  coreState,
  onCheckCoreStatus,
}: CoreOperationPanelProps) {
  const { i18n, t } = useTranslation();
  const [offlineRecoveryOpen, setOfflineRecoveryOpen] = useState(false);
  const [offlineStatusCheck, setOfflineStatusCheck] =
    useState<OfflineStatusCheck>({ state: "idle" });
  const formatBytes = useByteFormatter(i18n.resolvedLanguage);
  const countFormatter = useMemo(
    () => new Intl.NumberFormat(i18n.resolvedLanguage),
    [i18n.resolvedLanguage],
  );
  const determinate =
    operation.phase === "downloading" &&
    operation.bytesTotal !== undefined &&
    operation.bytesTotal > 0 &&
    operation.bytesCompleted !== undefined;
  const percent = determinate
    ? Math.min(
        100,
        Math.floor(
          (operation.bytesCompleted! / operation.bytesTotal!) * 100,
        ),
      )
    : undefined;
  const failed = operation.state === "failed";
  const succeeded = operation.state === "succeeded";
  const isUpdate = operation.operation === "update";
  const recoveryRequired =
    isUpdate && operation.errorCode === "recovery_required";
  const retryAvailable =
    !recoveryRequired || coreState === undefined || coreState === "running";
  const updateIsCurrent =
    isUpdate && succeeded && operation.targetVersion === undefined;
  const announcement = recoveryRequired
    ? t("operation.result.recoveryRequired")
    : failed
      ? isUpdate
        ? t("operation.result.updateDidNotFinish")
        : t("operation.result.setupDidNotFinish")
      : succeeded
        ? isUpdate
          ? updateIsCurrent
            ? t("operation.result.alreadyCurrent")
            : t("operation.result.updatedHeading")
          : t("operation.result.ready")
        : t(phaseKeys[operation.phase]);
  const failureCopy =
    operation.errorCode === "active_requests_remain" &&
    operation.activeRequests !== undefined
      ? t("operation.result.activeRequests", {
          count: operation.activeRequests,
          formattedCount: countFormatter.format(operation.activeRequests),
        })
      : t(errorTranslationKey(operation.errorCode));
  const successCopy = isUpdate
    ? updateIsCurrent
      ? t("operation.result.current", {
          currentVersion: operation.currentVersion ?? "",
        })
      : t("operation.result.updated", {
          targetVersion: operation.targetVersion ?? "",
        })
    : t("operation.result.installed");
  const progressLabel = determinate
    ? t("operation.progress.downloadAria")
    : t("operation.progress.indeterminateAria", {
        phase: t(progressPhaseKeys[operation.phase]),
      });
  const offlineCoreStatus =
    coreState === undefined
      ? t("common.unavailable")
      : t(coreStateTitleKeys[coreState]);

  async function checkOfflineCoreStatus() {
    if (
      onCheckCoreStatus === undefined ||
      offlineStatusCheck.state === "checking"
    ) {
      return;
    }
    setOfflineStatusCheck({ state: "checking" });
    try {
      const refreshedCoreState = await onCheckCoreStatus();
      setOfflineStatusCheck({
        state: "completed",
        coreState: refreshedCoreState,
      });
    } catch {
      setOfflineStatusCheck({ state: "failed" });
    }
  }

  const offlineStatusAnnouncement =
    offlineStatusCheck.state === "checking"
      ? t("operation.recovery.checkingStatus")
      : offlineStatusCheck.state === "completed"
        ? t("operation.recovery.checkCompleted", {
            status: t(coreStateTitleKeys[offlineStatusCheck.coreState]),
          })
        : offlineStatusCheck.state === "failed"
          ? t("operation.recovery.checkFailed")
          : "";

  return (
    <section
      className={`health-panel core-operation-panel${recoveryRequired ? " core-operation-panel--urgent" : ""}`}
      aria-labelledby="core-operation-heading"
    >
      <p
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {announcement}
      </p>
      <p className="section-label">
        {operation.operation === "install"
          ? t("operation.install.title")
          : t("operation.update.title")}
      </p>
      <h1 id="core-operation-heading" tabIndex={-1}>
        {announcement}
      </h1>
      <p className="health-summary">
        {failed
          ? failureCopy
          : succeeded
            ? successCopy
            : t(phaseKeys[operation.phase])}
      </p>

      {operation.state === "running" && (
        <>
          <div
            className="core-progress"
            role="progressbar"
            aria-label={progressLabel}
            {...(percent === undefined
              ? {}
              : {
                  "aria-valuemin": 0,
                  "aria-valuemax": 100,
                  "aria-valuenow": percent,
                })}
          >
            <span
              className={
                percent === undefined
                  ? "core-progress__bar core-progress__bar--indeterminate"
                  : "core-progress__bar"
              }
              style={
                percent === undefined ? undefined : { width: `${percent}%` }
              }
            />
          </div>
          {determinate && (
            <p className="core-progress__bytes">
              {t("operation.progress.bytes", {
                completed: formatBytes(operation.bytesCompleted!),
                total: formatBytes(operation.bytesTotal!),
              })}
            </p>
          )}
        </>
      )}

      {versionRows(
        operation,
        t("operation.version.current"),
        t("operation.version.target"),
      )}

      {failed && (
        <div className="recovery">
          {retryAvailable && (
            <button
              className="button button--primary"
              type="button"
              onClick={(event) => onRetry(event.currentTarget)}
            >
              {isUpdate && operation.errorCode === "active_requests_remain"
                ? t("operation.recovery.retryUpdateLater")
                : isUpdate
                  ? t("operation.recovery.retryUpdate")
                  : t("operation.install.retry")}
            </button>
          )}
          {recoveryRequired &&
            (diagnosticsAvailable && onOpenDiagnostics ? (
              <button
                className="button button--secondary"
                type="button"
                onClick={onOpenDiagnostics}
              >
                {t("operation.recovery.openDiagnostics")}
              </button>
            ) : (
              <>
                <button
                  className="button button--secondary"
                  type="button"
                  aria-expanded={offlineRecoveryOpen}
                  aria-controls="offline-recovery-guide"
                  onClick={() =>
                    setOfflineRecoveryOpen((isOpen) => !isOpen)
                  }
                >
                  {offlineRecoveryOpen
                    ? t("operation.recovery.hideOfflineRecovery")
                    : t("operation.recovery.viewOfflineRecovery")}
                </button>
                <p className="action-note">
                  {t("operation.recovery.diagnosticsUnavailable")}
                </p>
                {offlineRecoveryOpen && (
                  <section
                    id="offline-recovery-guide"
                    className="offline-recovery"
                    aria-labelledby="offline-recovery-heading"
                  >
                    <h2 id="offline-recovery-heading">
                      {t("operation.recovery.offlineTitle")}
                    </h2>
                    <p>{t("operation.recovery.offlineDescription")}</p>
                    <dl className="offline-recovery__facts">
                      <div>
                        <dt>{t("operation.recovery.recoveryCode")}</dt>
                        <dd>
                          <code dir="ltr">{operation.errorCode}</code>
                        </dd>
                      </div>
                      <div>
                        <dt>{t("operation.recovery.operationState")}</dt>
                        <dd>{t("operation.recovery.failedState")}</dd>
                      </div>
                      <div>
                        <dt>{t("operation.recovery.coreStatus")}</dt>
                        <dd>{offlineCoreStatus}</dd>
                      </div>
                    </dl>
                    <div className="offline-recovery__steps">
                      <h3>{t("operation.recovery.stepsTitle")}</h3>
                      <ol>
                        <li>{t("operation.recovery.stepCheck")}</li>
                        <li>{t("operation.recovery.stepRestart")}</li>
                        <li>{t("operation.recovery.stepRetry")}</li>
                      </ol>
                    </div>
                    {onCheckCoreStatus !== undefined && (
                      <button
                        className="button button--secondary"
                        type="button"
                        disabled={offlineStatusCheck.state === "checking"}
                        onClick={() => void checkOfflineCoreStatus()}
                      >
                        {offlineStatusCheck.state === "checking"
                          ? t("operation.recovery.checkingStatus")
                          : t("operation.recovery.checkStatus")}
                      </button>
                    )}
                  </section>
                )}
              </>
            ))}
          {recoveryRequired && (
            <p
              className="sr-only"
              role="status"
              aria-live="polite"
              aria-atomic="true"
            >
              {offlineStatusAnnouncement}
            </p>
          )}
          <p className="action-note">{t("operation.recovery.actionNote")}</p>
        </div>
      )}
    </section>
  );
}
