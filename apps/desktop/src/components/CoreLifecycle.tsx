import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

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
import { RecentSuccesses } from "../recentSuccesses";
import { CoreHealth } from "./CoreHealth";
import { CoreOperationPanel } from "./CoreOperationPanel";
import { ManagementPanel } from "./ManagementPanel";

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
const updateEligibleStates = new Set<CoreStatus["state"]>([
  "running",
  "stopped",
  "authorization_required",
  "incompatible",
]);

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

export function CoreLifecycle() {
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
  const installRequested = useRef(false);
  const startupCheckRequested = useRef(false);
  const startupCheckRevision = useRef(0);
  const updateCheckRequested = useRef(false);
  const updateRequested = useRef(false);
  const updateTrigger = useRef<HTMLButtonElement | null>(null);
  const confirmUpdateButton = useRef<HTMLButtonElement>(null);
  const mounted = useRef(false);
  const observedRuntimeChannel =
    useRef<CoreStatus["runtime_channel"] | undefined>(undefined);
  const processedSuccesses = useRef(new RecentSuccesses());
  const retryPending = useRef(false);
  if (status.data?.runtime_channel !== undefined) {
    observedRuntimeChannel.current = status.data.runtime_channel;
  }
  const waitsForAnotherProcess =
    status.data?.runtime_channel === "production" &&
    operation?.state === "failed" &&
    operation.errorCode === "install_in_progress";

  const acceptOperation = useCallback(
    (incoming: CoreOperation) => {
      if (!mounted.current) {
        return;
      }
      if (
        incoming.operation === "update" &&
        incoming.state === "succeeded" &&
        incoming.phase === "completed"
      ) {
        rememberCoreUpdateCompletion(incoming);
        startupCheckRevision.current += 1;
      }
      const terminalKey = `${incoming.operationId}:${incoming.sequence}`;
      if (
        incoming.state === "succeeded" &&
        processedSuccesses.current.has(terminalKey)
      ) {
        return;
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
      startupCheckRequested.current ||
      operation !== undefined ||
      status.data?.runtime_channel !== "production" ||
      !updateEligibleStates.has(status.data.state)
    ) {
      return;
    }
    startupCheckRequested.current = true;
    const revision = startupCheckRevision.current;
    void checkCoreUpdateOnce()
      .then((result) => {
        if (
          !mounted.current ||
          revision !== startupCheckRevision.current
        ) {
          return;
        }
        setUpdateCheck(result);
        setUpdateCheckFailed(false);
      })
      .catch(() => {
        if (
          mounted.current &&
          revision === startupCheckRevision.current
        ) {
          setUpdateCheck(undefined);
          setUpdateCheckFailed(true);
        }
      });
  }, [operation, status.data]);

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
      void status.refetch().then((result) => {
        if (
          mounted.current &&
          result.data &&
          result.data.state !== "missing"
        ) {
          setOperation(undefined);
        }
      });
    }, 1_000);
    return () => window.clearInterval(poll);
  }, [status.data, status.refetch, waitsForAnotherProcess]);

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
      setSetupFailure(undefined);
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
        updateCheckRequested.current ||
        status.data?.runtime_channel !== "production" ||
        !updateEligibleStates.has(status.data.state)
      ) {
        return;
      }
      updateCheckRequested.current = true;
      setUpdateCheckPending(true);
      void retryCoreUpdateCheck()
        .then((result) => {
          if (!mounted.current) {
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
          if (!mounted.current) {
            return;
          }
          setUpdateCheck(undefined);
          setUpdateCheckFailed(true);
        })
        .finally(() => {
          updateCheckRequested.current = false;
          if (mounted.current) {
            setUpdateCheckPending(false);
          }
        });
    },
    [status.data],
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
        updateTrigger.current?.focus();
      }
    });
  }, []);

  const requestUpdate = useCallback(
    (trigger?: HTMLButtonElement) => {
      if (
        status.data?.runtime_channel !== "production" ||
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
    [status.data?.runtime_channel, updateCheck],
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
      status.data?.runtime_channel !== "production" ||
      updateCheck?.code !== "update_available" ||
      targetVersion === undefined
    ) {
      return;
    }
    updateRequested.current = true;
    setUpdateStartFailed(false);
    setUpdateConfirmationOpen(false);
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
    status.data?.runtime_channel,
    updateCheck,
  ]);

  const updateConfirmationDialog =
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
          <p className="section-label">WokCore update</p>
          <h2 id="update-confirmation-heading">Upgrade WokCore?</h2>
          <p id="update-confirmation-description">
            WokCore may briefly stop while the verified update is
            installed. Active requests can defer the update safely.
          </p>
          <dl className="core-operation__versions">
            <div>
              <dt>Current version</dt>
              <dd>
                <code dir="ltr">{updateCheck.currentVersion}</code>
              </dd>
            </div>
            <div>
              <dt>Target version</dt>
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
              Cancel
            </button>
            <button
              ref={confirmUpdateButton}
              className="button button--primary"
              type="button"
              onClick={confirmUpdate}
            >
              Confirm upgrade
            </button>
          </div>
        </div>
      </div>
    ) : null;

  if (waitsForAnotherProcess) {
    return (
      <section
        className="health-panel core-operation-panel"
        aria-labelledby="external-core-operation-heading"
      >
        <p className="section-label">WokCore setup</p>
        <h1 id="external-core-operation-heading">
          WokCore operation continues in another process
        </h1>
        <p className="health-summary">
          The trusted installation is still in progress. WokRouter will
          reconnect when it finishes.
        </p>
        <div
          className="core-progress"
          role="progressbar"
          aria-label="Waiting for WokCore installation"
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
        />
        {operation.operation === "update" &&
          operation.state === "failed" && <ManagementPanel />}
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
              ? "WokCore update"
              : "WokCore setup"}
          </p>
          <div className="status-line status-line--error">
            <span className="status-mark" aria-hidden="true">
              !
            </span>
            <h1 id="core-status-recovery-heading">
              WokCore{" "}
              {updateStatusUnavailable ? "update" : "setup"} completed,
              but status is unavailable
            </h1>
          </div>
          <p className="health-summary">
            WokRouter could not confirm the new WokCore status. Check the
            trusted status again without repeating{" "}
            {updateStatusUnavailable ? "the update" : "installation"}.
          </p>
          <div className="recovery">
            <button
              className="button button--primary"
              type="button"
              onClick={retrySetup}
            >
              Check status again
            </button>
            <p className="action-note">
              Closing this window never cancels a WokCore operation.
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
          <p className="section-label">WokCore update</p>
          <div className="status-line status-line--error">
            <span className="status-mark" aria-hidden="true">
              !
            </span>
            <h1 id="core-update-start-failure-heading">
              WokCore update could not start
            </h1>
          </div>
          <p className="health-summary">
            WokRouter could not begin or recover a verified update
            operation. The current WokCore was not assumed to have changed.
          </p>
          <div className="recovery">
            <button
              className="button button--primary"
              type="button"
              disabled={updateCheckPending}
              onClick={(event) => retryUpdate(event.currentTarget)}
            >
              {updateCheckPending
                ? "Checking for updates…"
                : "Retry update safely"}
            </button>
            <p className="action-note">
              A fresh signed check and confirmation are required before
              retrying.
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
        <p className="section-label">WokCore setup</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="core-setup-failure-heading">
            WokCore setup unavailable
          </h1>
        </div>
        <p className="health-summary">
          WokRouter could not begin or recover the verified setup
          operation. Your local configuration was not assumed to have
          changed.
        </p>
        <div className="recovery">
          <button
            className="button button--primary"
            type="button"
            onClick={retrySetup}
          >
            Try again
          </button>
          <p className="action-note">
            Closing this window never cancels a WokCore operation.
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
        <p className="section-label">WokCore setup</p>
        <h1 id="core-setup-preflight-heading">
          Checking existing WokCore setup
        </h1>
        <p className="health-summary">
          WokRouter is checking for an operation it can safely resume
          before starting installation.
        </p>
        <div
          className="core-progress"
          role="progressbar"
          aria-label="Check WokCore setup progress"
        >
          <span className="core-progress__bar core-progress__bar--indeterminate" />
        </div>
      </section>
    );
  }

  return (
    <>
      <CoreHealth
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
