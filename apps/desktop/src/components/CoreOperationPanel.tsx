import { useMemo } from "react";

import type { CoreOperation } from "../coreOperation";

type CoreOperationPanelProps = {
  operation: CoreOperation;
  onRetry: () => void;
};

const phaseCopy: Record<CoreOperation["phase"], string> = {
  checking_release: "Checking for a WokCore release",
  downloading: "Downloading WokCore",
  verifying: "Verifying WokCore",
  installing: "Installing WokCore",
  preparing_service: "Preparing the WokCore service",
  draining: "Waiting for active requests",
  stopping: "Stopping WokCore",
  starting: "Starting WokCore",
  authorizing: "Authorizing WokRouter",
  verifying_runtime: "Verifying the WokCore runtime",
  rolling_back: "Restoring the previous WokCore version",
  completed: "WokCore operation complete",
};

const progressLabel: Record<CoreOperation["phase"], string> = {
  checking_release: "WokCore release check progress",
  downloading: "Download WokCore progress",
  verifying: "Verify WokCore progress",
  installing: "Install WokCore progress",
  preparing_service: "Prepare WokCore service progress",
  draining: "Drain WokCore requests progress",
  stopping: "Stop WokCore progress",
  starting: "Start WokCore progress",
  authorizing: "Authorize WokRouter progress",
  verifying_runtime: "Verify WokCore runtime progress",
  rolling_back: "Restore WokCore progress",
  completed: "Complete WokCore operation progress",
};

const errorCopy: Record<string, string> = {
  download_failed:
    "WokCore could not be downloaded. Check the network and try again.",
  invalid_install_state:
    "The configured WokCore installation could not be trusted. Review the installation before retrying.",
  invalid_manifest:
    "The WokCore release information was invalid. Nothing was installed.",
  invalid_signature:
    "The WokCore signature could not be verified. Nothing untrusted was installed.",
  incompatible_manifest:
    "This WokCore release is not compatible with the current system.",
  artifact_size_mismatch:
    "The downloaded WokCore package was incomplete. Nothing was installed.",
  artifact_hash_mismatch:
    "The downloaded WokCore package did not pass verification. Nothing was installed.",
  invalid_archive:
    "The WokCore package could not be opened safely. Nothing was installed.",
  unsafe_install_location:
    "WokCore cannot be installed in the configured location safely.",
  install_in_progress:
    "Another WokCore installation is still in progress. Try again after it finishes.",
  install_failed:
    "WokCore could not be installed. The current configuration was left unchanged.",
  install_record_failed:
    "WokCore was installed, but its trusted installation record could not be saved.",
  start_failed:
    "WokCore was installed but could not be started. You can safely try again.",
  authorization_failed:
    "WokRouter could not be authorized to manage WokCore. You can safely try again.",
  update_unavailable: "No verified WokCore update is currently available.",
  update_verification_failed:
    "The WokCore update could not be verified. No untrusted update was installed.",
  update_install_failed:
    "The WokCore update could not be completed safely. Review diagnostics and try again.",
  active_requests_remain:
    "WokCore is still serving active requests. Try the update again later.",
  rolled_back:
    "The WokCore update failed and the previous version was restored.",
  recovery_required:
    "WokCore could not recover automatically. Review diagnostics before retrying.",
  operation_in_progress:
    "Another WokCore operation is still in progress. Wait for it to finish.",
  invalid_progress:
    "WokRouter could not verify the operation progress. Check WokCore status before retrying.",
};

function useByteFormatter(): (bytes: number) => string {
  const numberFormat = useMemo(
    () =>
      new Intl.NumberFormat(undefined, {
        maximumFractionDigits: 1,
      }),
    [],
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

function versionRows(operation: CoreOperation) {
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
          <dt>Current version</dt>
          <dd>
            <code dir="ltr">{operation.currentVersion}</code>
          </dd>
        </div>
      )}
      {operation.targetVersion !== undefined && (
        <div>
          <dt>Target version</dt>
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
}: CoreOperationPanelProps) {
  const formatBytes = useByteFormatter();
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
  const announcement = failed
    ? "WokCore setup did not finish"
    : succeeded
      ? "WokCore is ready"
      : phaseCopy[operation.phase];

  return (
    <section
      className="health-panel core-operation-panel"
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
          ? "WokCore setup"
          : "WokCore update"}
      </p>
      <h1 id="core-operation-heading">{announcement}</h1>
      <p className="health-summary">
        {failed
          ? (errorCopy[operation.errorCode ?? ""] ??
            "WokRouter could not complete the operation safely. Check WokCore status and try again.")
          : succeeded
            ? "The verified WokCore operation completed successfully."
            : phaseCopy[operation.phase]}
      </p>

      {operation.state === "running" && (
        <>
          <div
            className="core-progress"
            role="progressbar"
            aria-label={progressLabel[operation.phase]}
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
              {formatBytes(operation.bytesCompleted!)} /{" "}
              {formatBytes(operation.bytesTotal!)}
            </p>
          )}
        </>
      )}

      {versionRows(operation)}

      {failed && (
        <div className="recovery">
          <button
            className="button button--primary"
            type="button"
            onClick={onRetry}
          >
            Try again
          </button>
          <p className="action-note">
            Closing this window never cancels a WokCore operation.
          </p>
        </div>
      )}
    </section>
  );
}
