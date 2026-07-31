import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  coreStatusQueryKey,
  getCoreStatus,
  type CoreStatus,
} from "../control";
import {
  getCoreOperation,
  installAndStartCore,
  listenForCoreOperation,
  type CoreOperation,
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
  const installRequested = useRef(false);
  const mounted = useRef(false);
  const processedSuccesses = useRef(new RecentSuccesses());
  const retryPending = useRef(false);
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
      const cachedStatus =
        queryClient.getQueryData<CoreStatus>(coreStatusQueryKey);
      const coreQueryState =
        queryClient.getQueryState<CoreStatus>(coreStatusQueryKey);
      const coreNeedsRecovery =
        refreshes[0]?.status === "rejected" ||
        cachedStatus === undefined ||
        cachedStatus.state === "missing" ||
        coreQueryState?.status === "error";
      if (coreNeedsRecovery) {
        let recovered = false;
        try {
          const result = await status.refetch();
          recovered =
            result.data !== undefined &&
            result.data.state !== "missing";
        } catch {
          recovered = false;
        }
        if (!mounted.current) {
          return;
        }
        setSetupFailure(recovered ? undefined : "status");
      }
      setOperation((current) =>
        current?.operationId === operation.operationId &&
        current.sequence === operation.sequence
          ? undefined
          : current,
      );
    })();
  }, [operation, queryClient, status.refetch]);

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
        setSetupFailure(undefined);
        setBridgeReady(true);
      } catch {
        unlisten?.();
        unlisten = undefined;
        if (active) {
          setBridgeReady(false);
          setSetupFailure("bridge");
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
      <CoreOperationPanel operation={operation} onRetry={retryInstall} />
    );
  }

  if (
    status.data?.runtime_channel === "production" &&
    status.data.state === "missing" &&
    setupFailure === "status"
  ) {
    return (
      <section
        className="health-panel"
        aria-labelledby="core-status-recovery-heading"
      >
        <p className="section-label">WokCore setup</p>
        <div className="status-line status-line--error">
          <span className="status-mark" aria-hidden="true">
            !
          </span>
          <h1 id="core-status-recovery-heading">
            WokCore setup completed, but status is unavailable
          </h1>
        </div>
        <p className="health-summary">
          WokRouter could not confirm the new WokCore status. Check the
          trusted status again without repeating installation.
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
      <CoreHealth />
      <ManagementPanel />
    </>
  );
}
