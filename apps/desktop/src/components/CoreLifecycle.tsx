import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";

import {
  coreStatusQueryKey,
  getCoreStatus,
  startCore,
  type CoreStatus,
} from "../control";
import {
  checkCoreUpdateOnce,
  getCoreOperation,
  installAndStartCore,
  installCoreUpdate,
  listenForCoreOperation,
  rememberCoreUpdateCompletion,
  retryCoreUpdateCheck,
  type CoreOperation,
  type CoreUpdateCheck,
} from "../coreOperation";
import { isCoreUpdateEligible } from "../coreUpdateEligibility";
import { RecentSuccesses } from "../recentSuccesses";
import { CoreHealth } from "./CoreHealth";
import { CoreOperationPanel } from "./CoreOperationPanel";
import {
  ManagementPanel,
  type ManagementArea,
} from "./ManagementPanel";

type SetupFailure = "bridge" | "install" | "status";

const lifecycleQueryKeys = [
  coreStatusQueryKey,
  ["provider-catalog"],
  ["provider-runtime"],
  ["provider-models"],
  ["sessions"],
  ["usage"],
  ["diagnostic-logs"],
] as const;
function reconcileOperation(
  current: CoreOperation | undefined,
  incoming: CoreOperation,
): CoreOperation {
  if (
    current?.operationId === incoming.operationId &&
    current.sequence >= incoming.sequence
  ) {
    return current;
  }
  return incoming;
}

function blocksUpdateInteraction(
  operation: CoreOperation | undefined,
): boolean {
  return (
    operation?.state === "running" ||
    operation?.state === "succeeded"
  );
}

export function CoreLifecycle() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: coreStatusQueryKey,
    queryFn: getCoreStatus,
  });
  const [operation, setOperation] = useState<CoreOperation>();
  const [bridgeReady, setBridgeReady] = useState(false);
  const [bridgeAttempt, setBridgeAttempt] = useState(0);
  const [setupFailure, setSetupFailure] = useState<SetupFailure>();
  const [statusRecoveryOperation, setStatusRecoveryOperation] =
    useState<CoreOperation["operation"]>();
  const [updateCheck, setUpdateCheck] = useState<CoreUpdateCheck>();
  const [updateCheckFailed, setUpdateCheckFailed] = useState(false);
  const [updateCheckPending, setUpdateCheckPending] = useState(false);
  const [updateStartFailed, setUpdateStartFailed] = useState(false);
  const [updateConfirmationOpen, setUpdateConfirmationOpen] =
    useState(false);
  const [requestedManagementArea, setRequestedManagementArea] =
    useState<ManagementArea>();
  const [
    requestedManagementAreaRequestId,
    setRequestedManagementAreaRequestId,
  ] = useState(0);
  const installRequested = useRef(false);
  const startupCheckConsumed = useRef(false);
  const startupCheckRevision = useRef(0);
  const nextUpdateCheckRequestId = useRef(0);
  const activeUpdateCheckRequestId = useRef<number | undefined>(
    undefined,
  );
  const updateRequested = useRef(false);
  const updateTrigger = useRef<HTMLButtonElement | null>(null);
  const confirmUpdateButton = useRef<HTMLButtonElement>(null);
  const mounted = useRef(false);
  const observedRuntimeChannel =
    useRef<CoreStatus["runtime_channel"] | undefined>(undefined);
  const latestStatus = useRef<CoreStatus | undefined>(status.data);
  const latestBridgeReady = useRef(bridgeReady);
  const latestOperation = useRef<CoreOperation | undefined>(operation);
  const processedSuccesses = useRef(new RecentSuccesses());
  const retryPending = useRef(false);
  if (status.data?.runtime_channel !== undefined) {
    observedRuntimeChannel.current = status.data.runtime_channel;
  }
  latestStatus.current = status.data;
  latestBridgeReady.current = bridgeReady;
  latestOperation.current = operation;
  const updateInterfaceReady =
    bridgeReady && isCoreUpdateEligible(status.data);
  const updateInteractionReady =
    updateInterfaceReady && !blocksUpdateInteraction(operation);
  const diagnosticsAvailable =
    status.data?.state === "running" &&
    status.data.capabilities.includes("diagnostics.events.v1");
  const waitsForAnotherProcess =
    status.data?.runtime_channel === "production" &&
    operation?.state === "failed" &&
    operation.errorCode === "install_in_progress";

  const acceptOperation = useCallback(
    (incoming: CoreOperation) => {
      if (!mounted.current) {
        return;
      }
      const terminalKey = `${incoming.operationId}:${incoming.sequence}`;
      if (
        incoming.state === "succeeded" &&
        processedSuccesses.current.has(terminalKey)
      ) {
        return;
      }
      const reconciled = reconcileOperation(
        latestOperation.current,
        incoming,
      );
      latestOperation.current = reconciled;
      if (blocksUpdateInteraction(reconciled)) {
        startupCheckRevision.current += 1;
        activeUpdateCheckRequestId.current = undefined;
      }
      if (
        incoming.operation === "update" &&
        incoming.state === "succeeded" &&
        incoming.phase === "completed"
      ) {
        rememberCoreUpdateCompletion(incoming);
        startupCheckRevision.current += 1;
      }
      setOperation((current) => reconcileOperation(current, incoming));
    },
    [],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (operation?.state !== "succeeded") {
      return;
    }
    const terminalKey = `${operation.operationId}:${operation.sequence}`;
    if (processedSuccesses.current.has(terminalKey)) {
      return;
    }
    processedSuccesses.current.remember(terminalKey);
    void (async () => {
      const refreshes = await Promise.allSettled(
        lifecycleQueryKeys.map((queryKey) =>
          Promise.resolve().then(() =>
            queryClient.invalidateQueries({ queryKey }),
          ),
        ),
      );
      if (!mounted.current) {
        return;
      }
      let refreshedStatus =
        queryClient.getQueryData<CoreStatus>(coreStatusQueryKey);
      const coreQueryState =
        queryClient.getQueryState<CoreStatus>(coreStatusQueryKey);
      const coreNeedsRecovery =
        refreshes[0]?.status === "rejected" ||
        refreshedStatus === undefined ||
        refreshedStatus.state === "missing" ||
        coreQueryState?.status === "error";
      let statusConfirmed = !coreNeedsRecovery;
      if (coreNeedsRecovery) {
        try {
          const result = await status.refetch();
          refreshedStatus = result.data;
          statusConfirmed =
            result.isSuccess &&
            result.data !== undefined &&
            result.data.state !== "missing";
        } catch {
          statusConfirmed = false;
        }
        if (!mounted.current) {
          return;
        }
        setSetupFailure(statusConfirmed ? undefined : "status");
        setStatusRecoveryOperation(
          statusConfirmed ||
            observedRuntimeChannel.current !== "production"
            ? undefined
            : operation.operation,
        );
      }
      if (
        operation.operation === "update" &&
        statusConfirmed &&
        refreshedStatus?.runtime_channel === "production" &&
        refreshedStatus?.state === "authorization_required"
      ) {
        try {
          await startCore();
          if (!mounted.current) {
            return;
          }
          await status.refetch();
        } catch {
          // CoreHealth retains its safe explicit authorization recovery.
        }
        if (!mounted.current) {
          return;
        }
      }
      if (operation.operation === "update") {
        setUpdateCheck(undefined);
        setUpdateCheckFailed(false);
      }
      setOperation((current) =>
        current?.operationId === operation.operationId &&
        current.sequence === operation.sequence
          ? undefined
          : current,
      );
    })();
  }, [
    operation,
    queryClient,
    status.data?.runtime_channel,
    status.refetch,
  ]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        const removeListener =
          await listenForCoreOperation(acceptOperation);
        if (!active) {
          removeListener();
          return;
        }
        unlisten = removeListener;
        const current = await getCoreOperation();
        if (!active) {
          return;
        }
        if (current) {
          acceptOperation(current);
        }
        setSetupFailure((failure) =>
          failure === "bridge" ? undefined : failure,
        );
        setBridgeReady(true);
      } catch {
        unlisten?.();
        unlisten = undefined;
        if (active) {
          latestOperation.current = undefined;
          setOperation(undefined);
          setBridgeReady(false);
          setSetupFailure((failure) => failure ?? "bridge");
        }
      }
    })();

    return () => {
      active = false;
      unlisten?.();
    };
  }, [acceptOperation, bridgeAttempt]);

  useEffect(() => {
    if (
      !bridgeReady ||
      startupCheckConsumed.current ||
      operation !== undefined ||
      !isCoreUpdateEligible(status.data)
    ) {
      return;
    }
    startupCheckConsumed.current = true;
    const revision = startupCheckRevision.current;
    void checkCoreUpdateOnce()
      .then((result) => {
        if (
          !mounted.current ||
          revision !== startupCheckRevision.current ||
          !latestBridgeReady.current ||
          blocksUpdateInteraction(latestOperation.current) ||
          !isCoreUpdateEligible(latestStatus.current)
        ) {
          return;
        }
        setUpdateCheck(result);
        setUpdateCheckFailed(false);
      })
      .catch(() => {
        if (
          mounted.current &&
          revision === startupCheckRevision.current &&
          latestBridgeReady.current &&
          !blocksUpdateInteraction(latestOperation.current) &&
          isCoreUpdateEligible(latestStatus.current)
        ) {
          setUpdateCheck(undefined);
          setUpdateCheckFailed(true);
        }
      });
  }, [bridgeReady, operation, status.data]);

  useEffect(() => {
    if (
      !bridgeReady ||
      installRequested.current ||
      status.data?.runtime_channel !== "production" ||
      status.data.state !== "missing" ||
      operation !== undefined
    ) {
      return;
    }
    installRequested.current = true;
    void installAndStartCore()
      .then(acceptOperation)
      .catch(() => {
        if (mounted.current) {
          setSetupFailure("install");
        }
      });
  }, [acceptOperation, bridgeReady, operation, status.data]);

  useEffect(() => {
    if (!waitsForAnotherProcess) {
      return;
    }
    if (status.data && status.data.state !== "missing") {
      setOperation(undefined);
      return;
    }
    const poll = window.setInterval(() => {
      void Promise.allSettled([getCoreOperation(), status.refetch()]).then(
        ([operationResult, statusResult]) => {
          if (!mounted.current) {
            return;
          }
          if (
            operationResult.status === "fulfilled" &&
            operationResult.value
          ) {
            acceptOperation(operationResult.value);
          }
          if (
            statusResult.status === "fulfilled" &&
            statusResult.value.data &&
            statusResult.value.data.state !== "missing"
          ) {
            setOperation(undefined);
          }
        }
      );
    }, 1_000);
    return () => window.clearInterval(poll);
  }, [acceptOperation, status.data, status.refetch, waitsForAnotherProcess]);

  const retryInstall = useCallback(() => {
    if (retryPending.current) {
      return;
    }
    retryPending.current = true;
    void (async () => {
      try {
        const current = await getCoreOperation();
        if (
          current?.state === "running" ||
          current?.state === "succeeded" ||
          (current?.state === "failed" &&
            current.errorCode === "install_in_progress")
        ) {
          acceptOperation(current);
          return;
        }
        const retry = await installAndStartCore();
        acceptOperation(retry);
        if (mounted.current) {
          setSetupFailure(undefined);
        }
      } catch {
        if (mounted.current) {
          setSetupFailure("install");
        }
      } finally {
        retryPending.current = false;
      }
    })();
  }, [acceptOperation]);

  const retryStatus = useCallback(() => {
    if (retryPending.current) {
      return;
    }
    retryPending.current = true;
    void (async () => {
      try {
        const result = await status.refetch();
        if (!mounted.current) {
          return;
        }
        if (
          result.data !== undefined &&
          result.data.state !== "missing"
        ) {
          setSetupFailure(undefined);
          setStatusRecoveryOperation(undefined);
        } else {
          setSetupFailure("status");
        }
      } catch {
        if (mounted.current) {
          setSetupFailure("status");
        }
      } finally {
        retryPending.current = false;
      }
    })();
  }, [status.refetch]);

  const retrySetup = useCallback(() => {
    if (setupFailure === "bridge") {
      setBridgeAttempt((attempt) => attempt + 1);
      return;
    }
    if (setupFailure === "status") {
      retryStatus();
      return;
    }
    retryInstall();
  }, [retryInstall, retryStatus, setupFailure]);

  const runManualUpdateCheck = useCallback(
    (openConfirmation: boolean, trigger?: HTMLButtonElement) => {
      if (
        activeUpdateCheckRequestId.current !== undefined ||
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current)
      ) {
        return;
      }
      nextUpdateCheckRequestId.current += 1;
      const requestId = nextUpdateCheckRequestId.current;
      activeUpdateCheckRequestId.current = requestId;
      startupCheckConsumed.current = true;
      setUpdateCheckPending(true);
      const revision = startupCheckRevision.current;
      void retryCoreUpdateCheck()
        .then((result) => {
          if (
            !mounted.current ||
            activeUpdateCheckRequestId.current !== requestId ||
            revision !== startupCheckRevision.current ||
            !latestBridgeReady.current ||
            blocksUpdateInteraction(latestOperation.current) ||
            !isCoreUpdateEligible(latestStatus.current)
          ) {
            return;
          }
          setUpdateCheck(result);
          setUpdateCheckFailed(false);
          setUpdateStartFailed(false);
          if (
            openConfirmation &&
            result.code === "update_available" &&
            result.targetVersion !== undefined
          ) {
            if (trigger) {
              updateTrigger.current = trigger;
            }
            setUpdateConfirmationOpen(true);
          } else if (openConfirmation && result.code === "current") {
            setOperation((current) =>
              current?.operation === "update" &&
              current.state === "failed"
                ? undefined
                : current,
            );
          }
        })
        .catch(() => {
          if (
            !mounted.current ||
            activeUpdateCheckRequestId.current !== requestId ||
            revision !== startupCheckRevision.current ||
            !latestBridgeReady.current ||
            blocksUpdateInteraction(latestOperation.current) ||
            !isCoreUpdateEligible(latestStatus.current)
          ) {
            return;
          }
          setUpdateCheck(undefined);
          setUpdateCheckFailed(true);
        })
        .finally(() => {
          if (activeUpdateCheckRequestId.current === requestId) {
            activeUpdateCheckRequestId.current = undefined;
          } else {
            return;
          }
          if (mounted.current) {
            setUpdateCheckPending(false);
          }
        });
    },
    [],
  );

  const checkForUpdates = useCallback(() => {
    runManualUpdateCheck(false);
  }, [runManualUpdateCheck]);

  const retryUpdate = useCallback(
    (trigger?: HTMLButtonElement) => {
      runManualUpdateCheck(true, trigger);
    },
    [runManualUpdateCheck],
  );

  const closeUpdateConfirmation = useCallback(() => {
    setUpdateConfirmationOpen(false);
    window.queueMicrotask(() => {
      if (mounted.current) {
        const trigger = updateTrigger.current;
        const target = trigger?.isConnected
          ? trigger
          : document.querySelector<HTMLElement>("h1");
        target?.focus({ preventScroll: true });
      }
    });
  }, []);

  useEffect(() => {
    if (updateInteractionReady) {
      return;
    }
    startupCheckRevision.current += 1;
    startupCheckConsumed.current = false;
    activeUpdateCheckRequestId.current = undefined;
    setUpdateCheck(undefined);
    setUpdateCheckFailed(false);
    setUpdateCheckPending(false);
    setUpdateStartFailed(false);
    if (updateConfirmationOpen) {
      closeUpdateConfirmation();
    }
  }, [
    closeUpdateConfirmation,
    updateConfirmationOpen,
    updateInteractionReady,
  ]);

  const requestUpdate = useCallback(
    (trigger?: HTMLButtonElement) => {
      if (
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current) ||
        updateCheck?.code !== "update_available" ||
        updateCheck.targetVersion === undefined ||
        updateRequested.current
      ) {
        return;
      }
      if (trigger) {
        updateTrigger.current = trigger;
      }
      setUpdateConfirmationOpen(true);
    },
    [updateCheck],
  );

  useEffect(() => {
    if (updateConfirmationOpen) {
      confirmUpdateButton.current?.focus();
    }
  }, [updateConfirmationOpen]);

  const handleDialogKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeUpdateConfirmation();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const controls = Array.from(
        event.currentTarget.querySelectorAll<HTMLButtonElement>(
          "button:not([disabled])",
        ),
      );
      if (controls.length === 0) {
        return;
      }
      const first = controls[0]!;
      const last = controls[controls.length - 1]!;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    },
    [closeUpdateConfirmation],
  );

  const confirmUpdate = useCallback(() => {
    const targetVersion = updateCheck?.targetVersion;
    if (
      updateRequested.current ||
      !latestBridgeReady.current ||
      blocksUpdateInteraction(latestOperation.current) ||
      !isCoreUpdateEligible(latestStatus.current) ||
      updateCheck?.code !== "update_available" ||
      targetVersion === undefined
    ) {
      return;
    }
    updateRequested.current = true;
    setUpdateStartFailed(false);
    closeUpdateConfirmation();
    void installCoreUpdate(targetVersion)
      .then(acceptOperation)
      .catch(async () => {
        try {
          const current = await getCoreOperation();
          if (current) {
            acceptOperation(current);
            return;
          }
        } catch {
          // Safe recovery copy is rendered from the existing check state.
        }
        if (mounted.current) {
          setUpdateCheck(undefined);
          setUpdateCheckFailed(false);
          setUpdateStartFailed(true);
        }
      })
      .finally(() => {
        updateRequested.current = false;
      });
  }, [
    acceptOperation,
    closeUpdateConfirmation,
    updateCheck,
  ]);

  const updateConfirmationDialog =
    updateInteractionReady &&
    updateConfirmationOpen &&
    updateCheck?.code === "update_available" &&
    updateCheck.targetVersion !== undefined ? (
      <div className="dialog-backdrop">
        <div
          className="confirmation-dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="update-confirmation-heading"
          aria-describedby="update-confirmation-description"
          onKeyDown={handleDialogKeyDown}
        >
          <p className="section-label">{t("operation.update.title")}</p>
          <h2 id="update-confirmation-heading">
            {t("operation.update.confirmTitle")}
          </h2>
          <p id="update-confirmation-description">
            {t("operation.update.available", {
              currentVersion: updateCheck.currentVersion,
              targetVersion: updateCheck.targetVersion,
            })} {t("operation.update.confirmBody")}
          </p>
          <dl className="core-operation__versions">
            <div>
              <dt>{t("operation.version.current")}</dt>
              <dd>
                <code dir="ltr">{updateCheck.currentVersion}</code>
              </dd>
            </div>
            <div>
              <dt>{t("operation.version.target")}</dt>
              <dd>
                <code dir="ltr">{updateCheck.targetVersion}</code>
              </dd>
            </div>
          </dl>
          <div className="dialog-actions">
            <button
              className="button button--secondary"
              type="button"
              onClick={closeUpdateConfirmation}
            >
              {t("operation.update.cancel")}
            </button>
            <button
              ref={confirmUpdateButton}
              className="button button--primary"
              type="button"
              onClick={confirmUpdate}
            >
              {t("operation.update.confirmAction")}
            </button>
          </div>
        </div>
      </div>
    ) : null;

  if (
    status.data?.runtime_channel === "production" &&
    setupFailure === "bridge"
  ) {
    return (
      <>
        <section
          className="health-panel"
          aria-labelledby="core-bridge-recovery-heading"
        >
          <p className="section-label">{t("operation.monitoring.title")}</p>
          <div className="status-line status-line--error">
            <span className="status-mark" aria-hidden="true">
              !
            </span>
            <h1 id="core-bridge-recovery-heading" tabIndex={-1}>
              {t("operation.monitoring.unavailableTitle")}
            </h1>
          </div>
          <p className="health-summary">
            {t("operation.monitoring.unavailableSummary")}
          </p>
          <div className="recovery">
            <button
              className="button button--primary"
              type="button"
              onClick={retrySetup}
            >
              {t("operation.monitoring.reconnect")}
            </button>
            <p className="action-note">
              {t("operation.monitoring.reconnectNote")}
            </p>
          </div>
        </section>
        <ManagementPanel />
      </>
    );
  }

  if (waitsForAnotherProcess) {
    return (
      <section
        className="health-panel core-operation-panel"
        aria-labelledby="external-core-operation-heading"
      >
        <p className="section-label">{t("operation.install.title")}</p>
        <h1 id="external-core-operation-heading" tabIndex={-1}>
          {t("operation.install.otherProcess")}
        </h1>
        <p className="health-summary">
          {t("operation.install.otherProcessSummary")}
        </p>
        <div
          className="core-progress"
          role="progressbar"
          aria-label={t("operation.install.waitAria")}
        >
          <span className="core-progress__bar core-progress__bar--indeterminate" />
        </div>
      </section>
    );
  }

  if (
    status.data?.runtime_channel === "production" &&
    operation
  ) {
    return (
      <>
        <CoreOperationPanel
          operation={operation}
          onRetry={
            operation.operation === "update"
              ? retryUpdate
              : retryInstall
          }
          diagnosticsAvailable={diagnosticsAvailable}
          onOpenDiagnostics={
            diagnosticsAvailable
              ? () => {
                  setRequestedManagementArea("diagnostics");
                  setRequestedManagementAreaRequestId(
                    (requestId) => requestId + 1,
                  );
                }
              : undefined
          }
        />
        {operation.operation === "update" &&
          operation.state === "failed" && (
            <ManagementPanel
              requestedArea={requestedManagementArea}
              requestedAreaRequestId={requestedManagementAreaRequestId}
            />
          )}
        {updateConfirmationDialog}
      </>
    );
  }

  if (
    setupFailure === "status" &&
    statusRecoveryOperation !== undefined
  ) {
    const updateStatusUnavailable =
      statusRecoveryOperation === "update";
    return (
      <>
        <section
          className="health-panel"
          aria-labelledby="core-status-recovery-heading"
        >
          <p className="section-label">
            {updateStatusUnavailable
              ? t("operation.update.title")
              : t("operation.install.title")}
          </p>
          <div className="status-line status-line--error">
            <span className="status-mark" aria-hidden="true">
              !
            </span>
            <h1 id="core-status-recovery-heading" tabIndex={-1}>
              {updateStatusUnavailable
                ? t("operation.statusRecovery.updateTitle")
                : t("operation.statusRecovery.setupTitle")}
            </h1>
          </div>
          <p className="health-summary">
            {updateStatusUnavailable
              ? t("operation.statusRecovery.updateSummary")
              : t("operation.statusRecovery.setupSummary")}
          </p>
          <div className="recovery">
            <button
              className="button button--primary"
              type="button"
              onClick={retrySetup}
            >
              {t("operation.statusRecovery.checkAgain")}
            </button>
            <p className="action-note">
              {t("operation.recovery.actionNote")}
            </p>
          </div>
        </section>
        {updateStatusUnavailable && <ManagementPanel />}
      </>
    );
  }

  if (
    status.data?.runtime_channel === "production" &&
    updateStartFailed
  ) {
    return (
      <>
        <section
          className="health-panel"
          aria-labelledby="core-update-start-failure-heading"
        >
          <p className="section-label">{t("operation.update.title")}</p>
          <div className="status-line status-line--error">
            <span className="status-mark" aria-hidden="true">
              !
            </span>
            <h1 id="core-update-start-failure-heading" tabIndex={-1}>
              {t("operation.update.startFailedTitle")}
            </h1>
          </div>
          <p className="health-summary">
            {t("operation.update.startFailedSummary")}
          </p>
          <div className="recovery">
            <button
              className="button button--primary"
              type="button"
              disabled={updateCheckPending}
              onClick={(event) => retryUpdate(event.currentTarget)}
            >
              {updateCheckPending
                ? t("operation.update.checkingLong")
                : t("operation.update.retrySafely")}
            </button>
            <p className="action-note">
              {t("operation.update.retryRequirement")}
            </p>
          </div>
        </section>
        <ManagementPanel />
        {updateConfirmationDialog}
      </>
    );
  }

  if (
    status.data?.runtime_channel === "production" &&
    status.data.state === "missing" &&
    setupFailure
  ) {
    return (
      <section
        className="health-panel"
        aria-labelledby="core-setup-failure-heading"
      >
        <p className="section-label">{t("operation.install.title")}</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="core-setup-failure-heading" tabIndex={-1}>
            {t("operation.install.unavailableTitle")}
          </h1>
        </div>
        <p className="health-summary">
          {t("operation.install.unavailableSummary")}
        </p>
        <div className="recovery">
          <button
            className="button button--primary"
            type="button"
            onClick={retrySetup}
          >
            {t("operation.install.retry")}
          </button>
          <p className="action-note">
            {t("operation.recovery.actionNote")}
          </p>
        </div>
      </section>
    );
  }

  if (
    status.data?.runtime_channel === "production" &&
    status.data.state === "missing"
  ) {
    return (
      <section
        className="health-panel core-operation-panel"
        aria-labelledby="core-setup-preflight-heading"
      >
        <p className="section-label">{t("operation.install.title")}</p>
        <h1 id="core-setup-preflight-heading" tabIndex={-1}>
          {t("operation.install.preflightTitle")}
        </h1>
        <p className="health-summary">
          {t("operation.install.preflightSummary")}
        </p>
        <div
          className="core-progress"
          role="progressbar"
          aria-label={t("operation.install.preflightAria")}
        >
          <span className="core-progress__bar core-progress__bar--indeterminate" />
        </div>
      </section>
    );
  }

  return (
    <>
      <CoreHealth
        updatesEnabled={updateInteractionReady}
        updateCheck={updateCheck}
        updateCheckFailed={updateCheckFailed}
        updateCheckPending={updateCheckPending}
        onCheckForUpdates={checkForUpdates}
        onUpgrade={requestUpdate}
      />
      <ManagementPanel />
      {updateConfirmationDialog}
    </>
  );
}
