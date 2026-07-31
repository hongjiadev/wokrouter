import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import {
  getCoreStatus,
  startCore,
  stopCore,
  type CoreStatus,
} from "../control";
import {
  getCoreOperation,
  installAndStartCore,
  listenForCoreOperation,
  type CoreOperation,
} from "../coreOperation";
import { CoreLifecycle } from "./CoreLifecycle";

vi.mock("./ManagementPanel", () => ({
  ManagementPanel: () => <section>WokCore workspace</section>,
}));
vi.mock("../control", () => ({
  coreStatusQueryKey: ["core-status"],
  getCoreStatus: vi.fn(),
  startCore: vi.fn(),
  stopCore: vi.fn(),
}));
vi.mock("../coreOperation", () => ({
  getCoreOperation: vi.fn(),
  installAndStartCore: vi.fn(),
  listenForCoreOperation: vi.fn(),
}));

const missingStatus: CoreStatus = {
  state: "missing",
  runtime_channel: "production",
  capabilities: [],
};
const runningStatus: CoreStatus = {
  state: "running",
  runtime_channel: "production",
  version: "0.2.0",
  management_api_major: 1,
  capabilities: ["provider.catalog.v1"],
  phase: "running",
};
const checkingOperation: CoreOperation = {
  schemaVersion: 1,
  operationId: "11111111-1111-4111-8111-111111111111",
  sequence: 0,
  operation: "install",
  state: "running",
  phase: "checking_release",
};
const completedOperation: CoreOperation = {
  ...checkingOperation,
  sequence: 1,
  state: "succeeded",
  phase: "completed",
  targetVersion: "0.2.0",
};
const failedOperation: CoreOperation = {
  ...checkingOperation,
  sequence: 1,
  state: "failed",
  phase: "checking_release",
  errorCode: "download_failed",
};
const retryOperation: CoreOperation = {
  ...checkingOperation,
  operationId: "22222222-2222-4222-8222-222222222222",
};
const downloadingOperation: CoreOperation = {
  ...checkingOperation,
  sequence: 4,
  phase: "downloading",
  bytesCompleted: 25,
  bytesTotal: 100,
  targetVersion: "0.2.0",
};

let operationListener: ((operation: CoreOperation) => void) | undefined;
const unlisten = vi.fn();

function deferred<T>() {
  let reject!: (reason?: unknown) => void;
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    reject = rejectPromise;
    resolve = resolvePromise;
  });
  return { promise, reject, resolve };
}

function renderLifecycle(
  queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  }),
  strict = true,
) {
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return {
    queryClient,
    ...render(
      strict ? (
        <StrictMode>
          <CoreLifecycle />
        </StrictMode>
      ) : (
        <CoreLifecycle />
      ),
      { wrapper: Wrapper },
    ),
  };
}

function queryClientWithCoreRefreshFailure(
  failure: "reject" | "throw",
) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const originalInvalidate =
    queryClient.invalidateQueries.bind(queryClient);
  const invalidateQueries = vi
    .spyOn(queryClient, "invalidateQueries")
    .mockImplementation((filters, options) => {
      if (filters?.queryKey?.[0] === "core-status") {
        if (failure === "throw") {
          throw new Error(
            "core refresh threw at C:\\private\\status.json",
          );
        }
        return Promise.reject(
          new Error(
            "core refresh rejected at C:\\private\\status.json",
          ),
        );
      }
      return originalInvalidate(filters, options);
    });
  return { invalidateQueries, queryClient };
}

beforeEach(() => {
  operationListener = undefined;
  unlisten.mockReset();
  vi.mocked(getCoreStatus).mockReset();
  vi.mocked(getCoreOperation).mockReset();
  vi.mocked(installAndStartCore).mockReset();
  vi.mocked(listenForCoreOperation).mockReset();
  vi.mocked(startCore).mockReset();
  vi.mocked(stopCore).mockReset();
  vi.mocked(listenForCoreOperation).mockImplementation(async (listener) => {
    operationListener = listener;
    return unlisten;
  });
});

afterEach(() => {
  vi.useRealTimers();
});

it("starts one production install in StrictMode and restores normal content after success", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

  renderLifecycle(queryClient);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
  expect(
    screen.queryByRole("button", { name: "Check again" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();

  operationListener?.(completedOperation);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  await waitFor(() => {
    expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  });
  for (const queryKey of [
    ["core-status"],
    ["provider-catalog"],
    ["provider-runtime"],
    ["provider-models"],
    ["sessions"],
    ["usage"],
    ["diagnostic-logs"],
  ]) {
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey });
  }
});

it("rechecks a failed operation is terminal before starting a safe retry", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation)
    .mockResolvedValueOnce(failedOperation)
    .mockResolvedValueOnce(failedOperation);
  vi.mocked(installAndStartCore).mockResolvedValue(retryOperation);
  const user = userEvent.setup();

  renderLifecycle();

  expect(
    await screen.findByText(
      "WokCore could not be downloaded. Check the network and try again.",
    ),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
});

it("reconciles a running retry status without starting a second child", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation)
    .mockResolvedValueOnce(failedOperation)
    .mockResolvedValueOnce(downloadingOperation);
  const user = userEvent.setup();

  renderLifecycle();

  await user.click(
    await screen.findByRole("button", { name: "Try again" }),
  );

  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).not.toHaveBeenCalled();
  expect(
    await screen.findByRole("progressbar", {
      name: "Download WokCore progress",
    }),
  ).toHaveAttribute("aria-valuenow", "25");
});

it("reconciles a succeeded retry status without starting another install", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation)
    .mockResolvedValueOnce(failedOperation)
    .mockResolvedValueOnce({ ...completedOperation, sequence: 2 });
  vi.mocked(installAndStartCore).mockResolvedValue(retryOperation);
  const user = userEvent.setup();

  renderLifecycle();

  await user.click(
    await screen.findByRole("button", { name: "Try again" }),
  );

  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).not.toHaveBeenCalled();
  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
});

it("treats install_in_progress as another process and polls trusted status without retrying", async () => {
  vi.useFakeTimers();
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue({
    ...failedOperation,
    errorCode: "install_in_progress",
  });

  renderLifecycle();
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });

  expect(
    screen.getByText(/operation continues in another process/i),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Try again" }),
  ).not.toBeInTheDocument();
  expect(installAndStartCore).not.toHaveBeenCalled();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000);
    await Promise.resolve();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  expect(
    vi.mocked(getCoreStatus).mock.calls.length,
  ).toBeGreaterThanOrEqual(2);
  expect(screen.getByText("WokCore running")).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(installAndStartCore).not.toHaveBeenCalled();
});

it("keeps a missing development runtime IDE-managed without using a production operation", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue({
    ...missingStatus,
    runtime_channel: "development",
  });
  vi.mocked(getCoreOperation).mockResolvedValue(failedOperation);

  renderLifecycle();

  expect(await screen.findByText("WokCore not installed")).toBeInTheDocument();
  expect(screen.getByText("Development")).toBeInTheDocument();
  expect(
    screen.getByText("This development WokCore is managed by the IDE."),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(
      "WokCore could not be downloaded. Check the network and try again.",
    ),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Check again" }),
  ).not.toBeInTheDocument();
  expect(installAndStartCore).not.toHaveBeenCalled();
});

it.each([
  ["missing", "WokCore not installed"],
  ["stopped", "WokCore stopped"],
  ["running", "WokCore running"],
  ["authorization_required", "WokRouter authorization required"],
] as const)(
  "never starts production installation for a development %s status",
  async (stateName, title) => {
    vi.mocked(getCoreStatus).mockResolvedValue({
      state: stateName,
      runtime_channel: "development",
      capabilities: [],
    });
    vi.mocked(getCoreOperation).mockResolvedValue(null);

    renderLifecycle();

    expect(await screen.findByText(title)).toBeInTheDocument();
    expect(installAndStartCore).not.toHaveBeenCalled();
  },
);

it("turns an initial install IPC rejection into safe manual recovery", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore)
    .mockRejectedValueOnce(
      new Error("failed at C:\\private\\wokcore.exe with token"),
    )
    .mockResolvedValueOnce(retryOperation);
  const user = userEvent.setup();

  renderLifecycle();

  expect(
    await screen.findByRole("heading", {
      name: "WokCore setup unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|wokcore\.exe|token/i),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).toHaveBeenCalledTimes(2);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
});

it("offers safe recovery when operation listener registration fails", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(listenForCoreOperation)
    .mockRejectedValueOnce(
      new Error("listener failed at C:\\private\\events.log"),
    )
    .mockImplementationOnce(async (listener) => {
      operationListener = listener;
      return unlisten;
    });
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", {
      name: "WokCore setup unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|events\.log/i),
  ).not.toBeInTheDocument();
  expect(getCoreOperation).not.toHaveBeenCalled();
  expect(installAndStartCore).not.toHaveBeenCalled();

  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(listenForCoreOperation).toHaveBeenCalledTimes(2);
  expect(getCoreOperation).toHaveBeenCalledTimes(1);
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
});

it("removes a partial listener and recovers safely when initial status reconciliation fails", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation)
    .mockRejectedValueOnce(
      new Error("status failed at C:\\private\\operation.json"),
    )
    .mockResolvedValueOnce(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", {
      name: "WokCore setup unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|operation\.json/i),
  ).not.toBeInTheDocument();
  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(installAndStartCore).not.toHaveBeenCalled();

  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(listenForCoreOperation).toHaveBeenCalledTimes(2);
  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
});

it("hides management behind indeterminate preflight for production missing", async () => {
  const operationStatus = deferred<CoreOperation | null>();
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockReturnValue(operationStatus.promise);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", {
      name: "Checking existing WokCore setup",
    }),
  ).toBeInTheDocument();
  const progress = screen.getByRole("progressbar", {
    name: "Check WokCore setup progress",
  });
  expect(progress).not.toHaveAttribute("aria-valuenow");
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
  expect(installAndStartCore).not.toHaveBeenCalled();

  operationStatus.resolve(null);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
});

it("invalidates lifecycle queries once when listener delivery precedes the matching invoke result", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockImplementation(async () => {
    operationListener?.(completedOperation);
    return completedOperation;
  });
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

  renderLifecycle(queryClient);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  await waitFor(() => {
    expect(invalidateQueries).toHaveBeenCalledTimes(7);
  });
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
});

it("settles a rejected query refresh and still restores normal content once", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const originalInvalidate =
    queryClient.invalidateQueries.bind(queryClient);
  const invalidateQueries = vi
    .spyOn(queryClient, "invalidateQueries")
    .mockImplementation((filters, options) => {
      if (filters?.queryKey?.[0] === "provider-runtime") {
        return Promise.reject(
          new Error("refresh failed at C:\\private\\provider.json"),
        );
      }
      if (filters?.queryKey?.[0] === "provider-models") {
        throw new Error(
          "refresh threw at C:\\private\\models.json",
        );
      }
      return originalInvalidate(filters, options);
    });

  renderLifecycle(queryClient);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  operationListener?.(completedOperation);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
  for (const queryKey of [
    ["core-status"],
    ["provider-catalog"],
    ["provider-runtime"],
    ["provider-models"],
    ["sessions"],
    ["usage"],
    ["diagnostic-logs"],
  ]) {
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey });
  }

  operationListener?.(completedOperation);
  await act(async () => {
    await Promise.resolve();
  });
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
});

it.each(["reject", "throw"] as const)(
  "falls back to status refetch when core-status invalidation %ss",
  async (failure) => {
    vi.mocked(getCoreStatus)
      .mockResolvedValueOnce(missingStatus)
      .mockResolvedValue(runningStatus);
    vi.mocked(getCoreOperation).mockResolvedValue(null);
    vi.mocked(installAndStartCore).mockResolvedValue(
      checkingOperation,
    );
    const { invalidateQueries, queryClient } =
      queryClientWithCoreRefreshFailure(failure);

    renderLifecycle(queryClient);

    expect(
      await screen.findByRole("heading", {
        name: "Checking for a WokCore release",
      }),
    ).toBeInTheDocument();
    operationListener?.(completedOperation);

    expect(
      await screen.findByText("WokCore running"),
    ).toBeInTheDocument();
    expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
    expect(invalidateQueries).toHaveBeenCalledTimes(7);
    expect(
      vi.mocked(getCoreStatus).mock.calls.length,
    ).toBeGreaterThanOrEqual(2);
    expect(installAndStartCore).toHaveBeenCalledTimes(1);
    const statusCalls = vi.mocked(getCoreStatus).mock.calls.length;

    operationListener?.(completedOperation);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invalidateQueries).toHaveBeenCalledTimes(7);
    expect(getCoreStatus).toHaveBeenCalledTimes(statusCalls);
  },
);

it("falls back when fulfilled invalidation leaves core status in error with missing data", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockRejectedValueOnce(
      new Error("refetch failed at C:\\private\\status.json"),
    )
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateQueries = vi.spyOn(
    queryClient,
    "invalidateQueries",
  );

  renderLifecycle(queryClient);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  operationListener?.(completedOperation);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
  expect(
    vi.mocked(getCoreStatus).mock.calls.length,
  ).toBeGreaterThanOrEqual(3);
  expect(
    screen.queryByText(/private|status\.json/i),
  ).not.toBeInTheDocument();
});

it("does not refetch status again when invalidation already cached running", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
      mutations: { retry: false },
    },
  });

  renderLifecycle(queryClient);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  operationListener?.(completedOperation);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(getCoreStatus).toHaveBeenCalledTimes(2);
});

it("shows status-only recovery when core refresh and fallback both fail", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockRejectedValueOnce(
      new Error("status failed at C:\\private\\token.json"),
    )
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  const { invalidateQueries, queryClient } =
    queryClientWithCoreRefreshFailure("reject");
  const user = userEvent.setup();

  renderLifecycle(queryClient);

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  operationListener?.(completedOperation);

  expect(
    await screen.findByRole("heading", {
      name: "WokCore setup completed, but status is unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|token\.json/i),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("heading", {
      name: "Checking existing WokCore setup",
    }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
  expect(getCoreStatus).toHaveBeenCalledTimes(2);
  const listenerCalls = vi.mocked(listenForCoreOperation).mock.calls.length;
  const installCalls = vi.mocked(installAndStartCore).mock.calls.length;
  const operationStatusCalls =
    vi.mocked(getCoreOperation).mock.calls.length;

  await user.click(
    screen.getByRole("button", { name: "Check status again" }),
  );

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(
    vi.mocked(getCoreStatus).mock.calls.length,
  ).toBeGreaterThanOrEqual(3);
  expect(listenForCoreOperation).toHaveBeenCalledTimes(listenerCalls);
  expect(installAndStartCore).toHaveBeenCalledTimes(installCalls);
  expect(getCoreOperation).toHaveBeenCalledTimes(operationStatusCalls);
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
});

it("subscribes before recovering a running snapshot and unmounts only the listener", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(downloadingOperation);

  const view = renderLifecycle();

  expect(
    await screen.findByRole("progressbar", {
      name: "Download WokCore progress",
    }),
  ).toHaveAttribute("aria-valuenow", "25");
  expect(
    vi.mocked(listenForCoreOperation).mock.invocationCallOrder[0],
  ).toBeLessThan(vi.mocked(getCoreOperation).mock.invocationCallOrder[0]!);
  expect(installAndStartCore).not.toHaveBeenCalled();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();

  view.unmount();

  expect(unlisten).toHaveBeenCalled();
  expect(startCore).not.toHaveBeenCalled();
  expect(stopCore).not.toHaveBeenCalled();
  expect(installAndStartCore).not.toHaveBeenCalled();
});

it.each([
  ["download_failed", /could not be downloaded/i],
  ["invalid_signature", /signature could not be verified/i],
  ["install_failed", /could not be installed/i],
  ["start_failed", /installed but could not be started/i],
  ["authorization_failed", /could not be authorized/i],
] as const)(
  "shows safe %s recovery copy from a recovered terminal snapshot",
  async (errorCode, safeCopy) => {
    vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
    vi.mocked(getCoreOperation).mockResolvedValue({
      ...failedOperation,
      phase: "completed",
      errorCode,
    });

    renderLifecycle();

    expect(await screen.findByText(safeCopy)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Try again" }),
    ).toBeEnabled();
    expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
  },
);
