import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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
  checkCoreUpdateOnce,
  getCoreOperation,
  installAndStartCore,
  installCoreUpdate,
  listenForCoreOperation,
  rememberCoreUpdateCompletion,
  retryCoreUpdateCheck,
  type CoreOperation,
} from "../coreOperation";
import { initializeI18n } from "../i18n";
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
  checkCoreUpdateOnce: vi.fn(),
  getCoreOperation: vi.fn(),
  installAndStartCore: vi.fn(),
  installCoreUpdate: vi.fn(),
  listenForCoreOperation: vi.fn(),
  rememberCoreUpdateCompletion: vi.fn(),
  retryCoreUpdateCheck: vi.fn(),
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
const ineligibleUpdateStatuses: readonly [
  string,
  CoreStatus,
][] = [
  [
    "development",
    {
      ...runningStatus,
      runtime_channel: "development",
    },
  ],
  [
    "starting",
    {
      ...runningStatus,
      state: "starting",
      phase: "starting",
    },
  ],
  [
    "draining",
    {
      ...runningStatus,
      state: "draining",
      phase: "draining",
    },
  ],
  ["missing", missingStatus],
  [
    "invalid runtime",
    {
      ...runningStatus,
      state: "invalid_runtime",
      phase: undefined,
    },
  ],
];
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
const checkingUpdateOperation: CoreOperation = {
  schemaVersion: 1,
  operationId: "33333333-3333-4333-8333-333333333333",
  sequence: 0,
  operation: "update",
  state: "running",
  phase: "checking_release",
  currentVersion: "0.2.0",
  targetVersion: "0.2.1",
};
const completedUpdateOperation: CoreOperation = {
  ...checkingUpdateOperation,
  sequence: 4,
  state: "succeeded",
  phase: "completed",
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

beforeEach(async () => {
  await initializeI18n("en");
  operationListener = undefined;
  unlisten.mockReset();
  vi.mocked(getCoreStatus).mockReset();
  vi.mocked(checkCoreUpdateOnce).mockReset();
  vi.mocked(getCoreOperation).mockReset();
  vi.mocked(installAndStartCore).mockReset();
  vi.mocked(installCoreUpdate).mockReset();
  vi.mocked(listenForCoreOperation).mockReset();
  vi.mocked(rememberCoreUpdateCompletion).mockReset();
  vi.mocked(retryCoreUpdateCheck).mockReset();
  vi.mocked(startCore).mockReset();
  vi.mocked(stopCore).mockReset();
  vi.mocked(listenForCoreOperation).mockImplementation(async (listener) => {
    operationListener = listener;
    return unlisten;
  });
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
    code: "current",
    currentVersion: "0.2.0",
  });
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "current",
    currentVersion: "0.2.0",
  });
  vi.mocked(installAndStartCore).mockResolvedValue(checkingOperation);
  vi.mocked(installCoreUpdate).mockResolvedValue(checkingUpdateOperation);
});

afterEach(() => {
  vi.useRealTimers();
});

it("translates the verified update confirmation without changing ownership", async () => {
  await initializeI18n("zh-CN");
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", { name: "升级 WokCore" }),
  );
  const dialog = screen.getByRole("dialog", { name: "升级 WokCore？" });
  expect(dialog).toHaveTextContent("将 WokCore 从 0.2.0 升级到 0.2.1");
  expect(dialog).toHaveTextContent(
    "安装经过验证的更新时，WokCore 可能会短暂停止。活动请求可以安全地延后更新。",
  );
  expect(screen.getByText("当前版本")).toBeInTheDocument();
  expect(screen.getByText("目标版本")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "确认升级" })).toHaveFocus();
  expect(screen.getByRole("button", { name: "取消" })).toBeEnabled();
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it("translates operation-monitoring recovery without exposing bridge details", async () => {
  await initializeI18n("zh-CN");
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(listenForCoreOperation).mockRejectedValue(
    new Error("listener failed at C:\\private\\events.json"),
  );

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", { name: "WokCore 操作监控不可用" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "重新连接操作监控" }),
  ).toBeEnabled();
  expect(screen.getByText("操作监控")).toBeInTheDocument();
  expect(screen.queryByText(/private|events\.json/i)).not.toBeInTheDocument();
});

it("translates another-process install progress", async () => {
  await initializeI18n("zh-CN");
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockResolvedValue({
    ...failedOperation,
    errorCode: "install_in_progress",
  });

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", {
      name: "WokCore 操作正在另一个进程中继续",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("progressbar", { name: "正在等待 WokCore 安装" }),
  ).not.toHaveAttribute("aria-valuenow");
  expect(screen.queryByRole("button", { name: "重试" })).not.toBeInTheDocument();
});

it("translates missing-runtime preflight progress", async () => {
  await initializeI18n("zh-CN");
  const operationStatus = deferred<CoreOperation | null>();
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  vi.mocked(getCoreOperation).mockReturnValue(operationStatus.promise);

  renderLifecycle(undefined, false);

  expect(
    await screen.findByRole("heading", {
      name: "正在检查现有 WokCore 设置",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("progressbar", { name: "检查 WokCore 设置进度" }),
  ).not.toHaveAttribute("aria-valuenow");
  expect(installAndStartCore).not.toHaveBeenCalled();
});

it("updates an open confirmation in place and preserves its target across locale changes", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  const dialog = screen.getByRole("dialog", { name: "Upgrade WokCore?" });
  await act(async () => {
    await initializeI18n("zh-CN");
  });

  expect(screen.getByRole("dialog", { name: "升级 WokCore？" })).toBe(dialog);
  await user.click(screen.getByRole("button", { name: "确认升级" }));
  expect(installCoreUpdate).toHaveBeenCalledOnce();
  expect(installCoreUpdate).toHaveBeenCalledWith("0.2.1");
});

it.each([
  ["running", true],
  ["stopped", true],
  ["authorization_required", true],
  ["incompatible", true],
  ["missing", false],
  ["invalid_runtime", false],
] as const)(
  "checks production updates for %s: %s",
  async (state, shouldCheck) => {
    vi.mocked(getCoreStatus).mockResolvedValue({
      state,
      runtime_channel: "production",
      version: state === "missing" ? undefined : "0.2.0",
      capabilities: [],
    });

    renderLifecycle(undefined, false);

    await screen.findByText(
      state === "running"
        ? "WokCore running"
        : state === "stopped"
          ? "WokCore stopped"
          : state === "authorization_required"
            ? "WokRouter authorization required"
            : state === "incompatible"
              ? "WokCore update required"
              : state === "invalid_runtime"
                ? "WokCore runtime invalid"
                : "Checking existing WokCore setup",
    );
    await waitFor(() => {
      expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(
        shouldCheck ? 1 : 0,
      );
    });
  },
);

it("waits for listener registration and initial operation arbitration before checking updates", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const listenerReady = deferred<() => void>();
  const initialOperation = deferred<CoreOperation | null>();
  vi.mocked(listenForCoreOperation).mockImplementation((listener) => {
    operationListener = listener;
    return listenerReady.promise;
  });
  vi.mocked(getCoreOperation).mockReturnValue(initialOperation.promise);

  renderLifecycle(undefined, false);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(checkCoreUpdateOnce).not.toHaveBeenCalled();
  expect(
    screen.queryByRole("button", { name: "Upgrade WokCore" }),
  ).not.toBeInTheDocument();

  await act(async () => {
    listenerReady.resolve(unlisten);
    await listenerReady.promise;
  });
  expect(getCoreOperation).toHaveBeenCalledOnce();
  expect(checkCoreUpdateOnce).not.toHaveBeenCalled();

  await act(async () => {
    initialOperation.resolve(null);
    await initialOperation.promise;
  });

  expect(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  ).toBeInTheDocument();
  expect(checkCoreUpdateOnce).toHaveBeenCalledOnce();
});

it.each(["listen", "initial operation"] as const)(
  "keeps an installed production runtime update-free when %s bridge arbitration fails",
  async (failurePoint) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    });
    if (failurePoint === "listen") {
      vi.mocked(listenForCoreOperation).mockRejectedValue(
        new Error("listener failed at C:\\private\\events.json"),
      );
    } else {
      vi.mocked(getCoreOperation).mockRejectedValue(
        new Error("snapshot failed at C:\\private\\operation.json"),
      );
    }

    renderLifecycle(undefined, false);

    expect(
      await screen.findByRole("heading", {
        name: "WokCore operation monitoring unavailable",
      }),
    ).toBeInTheDocument();
    expect(checkCoreUpdateOnce).not.toHaveBeenCalled();
    expect(retryCoreUpdateCheck).not.toHaveBeenCalled();
    expect(installCoreUpdate).not.toHaveBeenCalled();
    expect(
      screen.queryByText(/private|events\.json|operation\.json/i),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "Reconnect operation monitoring",
      }),
    ).toBeEnabled();
  },
);

it("recovers an active snapshot before considering a startup update check", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const initialOperation = deferred<CoreOperation | null>();
  vi.mocked(getCoreOperation).mockReturnValue(initialOperation.promise);

  renderLifecycle(undefined, false);

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(checkCoreUpdateOnce).not.toHaveBeenCalled();

  await act(async () => {
    initialOperation.resolve(checkingUpdateOperation);
    await initialOperation.promise;
  });

  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  expect(checkCoreUpdateOnce).not.toHaveBeenCalled();
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it("prioritizes read-only bridge recovery over an event received before failed arbitration", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  const initialOperation = deferred<CoreOperation | null>();
  vi.mocked(getCoreOperation).mockReturnValue(initialOperation.promise);

  renderLifecycle(undefined, false);
  await waitFor(() => {
    expect(operationListener).toBeDefined();
  });
  operationListener?.(checkingUpdateOperation);
  initialOperation.reject(
    new Error("snapshot failed at C:\\private\\operation.json"),
  );

  expect(
    await screen.findByRole("heading", {
      name: "WokCore operation monitoring unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).not.toBeInTheDocument();
  expect(checkCoreUpdateOnce).not.toHaveBeenCalled();
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it.each(ineligibleUpdateStatuses)(
  "invalidates an open production update dialog when status becomes %s",
  async (_scenario, nextStatus) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    });
    vi.mocked(installAndStartCore).mockReturnValue(
      new Promise<CoreOperation>(() => undefined),
    );
    const user = userEvent.setup();
    const view = renderLifecycle(undefined, false);

    const trigger = await screen.findByRole("button", {
      name: "Upgrade WokCore",
    });
    await user.click(trigger);
    const confirm = screen.getByRole("button", {
      name: "Confirm upgrade",
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    act(() => {
      view.queryClient.setQueryData(
        ["core-status"],
        nextStatus,
      );
    });

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(
      screen.queryByRole("button", { name: "Upgrade WokCore" }),
    ).not.toBeInTheDocument();
    expect(document.activeElement?.tagName).toBe("H1");

    fireEvent.click(confirm);
    fireEvent.click(trigger);
    expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(1);
    expect(retryCoreUpdateCheck).not.toHaveBeenCalled();
    expect(installCoreUpdate).not.toHaveBeenCalled();
  },
);

it.each(ineligibleUpdateStatuses)(
  "discards a startup update result that settles after status becomes %s",
  async (_scenario, nextStatus) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    const updateResult = deferred<
      Awaited<ReturnType<typeof checkCoreUpdateOnce>>
    >();
    vi.mocked(checkCoreUpdateOnce).mockReturnValue(
      updateResult.promise,
    );
    vi.mocked(installAndStartCore).mockReturnValue(
      new Promise<CoreOperation>(() => undefined),
    );
    const view = renderLifecycle(undefined, false);

    await waitFor(() => {
      expect(checkCoreUpdateOnce).toHaveBeenCalledOnce();
    });
    act(() => {
      view.queryClient.setQueryData(
        ["core-status"],
        nextStatus,
      );
    });
    await act(async () => {
      updateResult.resolve({
        code: "update_available",
        currentVersion: "0.2.0",
        targetVersion: "0.2.1",
      });
      await updateResult.promise;
    });

    expect(
      screen.queryByRole("button", { name: "Upgrade WokCore" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(1);
    expect(retryCoreUpdateCheck).not.toHaveBeenCalled();
    expect(installCoreUpdate).not.toHaveBeenCalled();
  },
);

it.each([
  ["candidate", "before"],
  ["error", "before"],
  ["candidate", "while"],
  ["error", "while"],
] as const)(
  "restores a cached automatic %s result settled %s a transient ineligible state",
  async (outcome, settleTiming) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    const automaticResult = deferred<
      Awaited<ReturnType<typeof checkCoreUpdateOnce>>
    >();
    const underlyingCheck = vi.fn(() => automaticResult.promise);
    const cachedAutomaticResult = underlyingCheck();
    vi.mocked(checkCoreUpdateOnce).mockImplementation(
      () => cachedAutomaticResult,
    );
    const view = renderLifecycle(undefined, false);

    await waitFor(() => {
      expect(checkCoreUpdateOnce).toHaveBeenCalledOnce();
    });
    if (settleTiming === "before") {
      await act(async () => {
        if (outcome === "candidate") {
          automaticResult.resolve({
            code: "update_available",
            currentVersion: "0.2.0",
            targetVersion: "0.2.1",
          });
          await automaticResult.promise;
        } else {
          automaticResult.reject(new Error("automatic check failed"));
          await automaticResult.promise.catch(() => undefined);
        }
      });
      if (outcome === "candidate") {
        expect(
          await screen.findByRole("button", {
            name: "Upgrade WokCore",
          }),
        ).toBeEnabled();
      } else {
        expect(
          await screen.findByText("WokCore update check unavailable"),
        ).toBeInTheDocument();
      }
    }

    act(() => {
      view.queryClient.setQueryData(["core-status"], {
        ...runningStatus,
        state: "starting",
        phase: "starting",
      });
    });
    expect(await screen.findByText("WokCore starting")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Upgrade WokCore" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("WokCore update check unavailable"),
    ).not.toBeInTheDocument();

    if (settleTiming === "while") {
      await act(async () => {
        if (outcome === "candidate") {
          automaticResult.resolve({
            code: "update_available",
            currentVersion: "0.2.0",
            targetVersion: "0.2.1",
          });
          await automaticResult.promise;
        } else {
          automaticResult.reject(new Error("automatic check failed"));
          await automaticResult.promise.catch(() => undefined);
        }
      });
    }

    act(() => {
      view.queryClient.setQueryData(["core-status"], runningStatus);
    });
    if (outcome === "candidate") {
      expect(
        await screen.findByRole("button", {
          name: "Upgrade WokCore",
        }),
      ).toBeEnabled();
    } else {
      expect(
        await screen.findByText("WokCore update check unavailable"),
      ).toBeInTheDocument();
    }
    expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(2);
    expect(underlyingCheck).toHaveBeenCalledOnce();
  },
);

it("discards a manual update result when production becomes development", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockRejectedValue(
    new Error("update check unavailable"),
  );
  const updateResult = deferred<
    Awaited<ReturnType<typeof retryCoreUpdateCheck>>
  >();
  vi.mocked(retryCoreUpdateCheck).mockReturnValue(
    updateResult.promise,
  );
  const user = userEvent.setup();
  const view = renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", {
      name: "Check for updates",
    }),
  );
  act(() => {
    view.queryClient.setQueryData(["core-status"], {
      ...runningStatus,
      runtime_channel: "development",
    });
  });
  await act(async () => {
    updateResult.resolve({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    });
    await updateResult.promise;
  });

  expect(
    screen.queryByRole("button", { name: "Upgrade WokCore" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("WokCore update check unavailable"),
  ).not.toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(retryCoreUpdateCheck).toHaveBeenCalledOnce();
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it("invalidates an open confirmation when operation monitoring reports active update work", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  const trigger = await screen.findByRole("button", {
    name: "Upgrade WokCore",
  });
  await user.click(trigger);
  const confirm = screen.getByRole("button", {
    name: "Confirm upgrade",
  });

  act(() => {
    operationListener?.(checkingUpdateOperation);
  });

  await waitFor(() => {
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  expect(
    screen.getByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toHaveFocus();
  fireEvent.click(confirm);
  fireEvent.click(trigger);
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it("discards a fresh retry result when operation monitoring reports active update work", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "current",
    currentVersion: "0.2.0",
  });
  vi.mocked(getCoreOperation).mockResolvedValue({
    ...checkingUpdateOperation,
    sequence: 8,
    state: "failed",
    phase: "completed",
    errorCode: "recovery_required",
  });
  const retryResult = deferred<
    Awaited<ReturnType<typeof retryCoreUpdateCheck>>
  >();
  vi.mocked(retryCoreUpdateCheck).mockReturnValue(retryResult.promise);
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", { name: "Try update again" }),
  );
  expect(retryCoreUpdateCheck).toHaveBeenCalledOnce();

  act(() => {
    operationListener?.({
      ...checkingUpdateOperation,
      operationId: "55555555-5555-4555-8555-555555555555",
    });
  });
  await act(async () => {
    retryResult.resolve({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.2",
    });
    await retryResult.promise;
  });

  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Upgrade WokCore" }),
  ).not.toBeInTheDocument();
  expect(installCoreUpdate).not.toHaveBeenCalled();
});

it.each(["resolve", "reject"] as const)(
  "releases invalidated manual ownership without letting stale %s override a newer retry",
  async (staleOutcome) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    vi.mocked(getCoreOperation).mockResolvedValue({
      ...checkingUpdateOperation,
      sequence: 8,
      state: "failed",
      phase: "completed",
      errorCode: "recovery_required",
    });
    const firstRetry = deferred<
      Awaited<ReturnType<typeof retryCoreUpdateCheck>>
    >();
    const secondRetry = deferred<
      Awaited<ReturnType<typeof retryCoreUpdateCheck>>
    >();
    const unexpectedRetry = deferred<
      Awaited<ReturnType<typeof retryCoreUpdateCheck>>
    >();
    vi.mocked(retryCoreUpdateCheck)
      .mockReturnValueOnce(firstRetry.promise)
      .mockReturnValueOnce(secondRetry.promise)
      .mockReturnValue(unexpectedRetry.promise);
    const user = userEvent.setup();

    renderLifecycle(undefined, false);

    await user.click(
      await screen.findByRole("button", {
        name: "Try update again",
      }),
    );
    const concurrentOperation: CoreOperation = {
      ...checkingUpdateOperation,
      operationId: "55555555-5555-4555-8555-555555555555",
    };
    act(() => {
      operationListener?.(concurrentOperation);
    });
    expect(
      await screen.findByRole("heading", {
        name: "Checking for a WokCore release",
      }),
    ).toBeInTheDocument();
    act(() => {
      operationListener?.({
        ...concurrentOperation,
        sequence: 1,
        state: "failed",
        phase: "completed",
        errorCode: "recovery_required",
      });
    });

    await user.click(
      await screen.findByRole("button", {
        name: "Try update again",
      }),
    );
    expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(2);

    await act(async () => {
      if (staleOutcome === "resolve") {
        firstRetry.resolve({
          code: "update_available",
          currentVersion: "0.2.0",
          targetVersion: "0.2.9",
        });
        await firstRetry.promise;
      } else {
        firstRetry.reject(new Error("stale retry failed"));
        await firstRetry.promise.catch(() => undefined);
      }
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "Try update again",
      }),
    );
    expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(2);

    await act(async () => {
      secondRetry.resolve({
        code: "update_available",
        currentVersion: "0.2.0",
        targetVersion: "0.2.2",
      });
      await secondRetry.promise;
    });

    expect(
      await screen.findByRole("dialog", {
        name: "Upgrade WokCore?",
      }),
    ).toHaveTextContent("0.2.2");
    expect(screen.queryByText("0.2.9")).not.toBeInTheDocument();
  },
);

it.each([
  "running",
  "stopped",
  "authorization_required",
  "incompatible",
] as const)(
  "never checks or installs updates for a development %s runtime",
  async (state) => {
    vi.mocked(getCoreStatus).mockResolvedValue({
      state,
      runtime_channel: "development",
      version: "0.2.0",
      capabilities: [],
    });
    const user = userEvent.setup();

    const view = renderLifecycle(undefined, false);

    expect(await screen.findByText("Development")).toBeInTheDocument();
    view.rerender(
      <StrictMode>
        <CoreLifecycle />
      </StrictMode>,
    );
    await act(async () => {
      await view.queryClient.invalidateQueries({
        queryKey: ["core-status"],
      });
    });
    const checkAgain = screen.queryByRole("button", {
      name: "Check again",
    });
    if (checkAgain) {
      await user.click(checkAgain);
    }

    expect(checkCoreUpdateOnce).not.toHaveBeenCalled();
    expect(retryCoreUpdateCheck).not.toHaveBeenCalled();
    expect(installCoreUpdate).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("button", { name: "Upgrade WokCore" }),
    ).not.toBeInTheDocument();
  },
);

it("keeps the automatic production update check once-only across status changes", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  const { queryClient } = renderLifecycle(undefined, false);

  await waitFor(() => {
    expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(1);
  });

  queryClient.setQueryData<CoreStatus>(["core-status"], {
    ...runningStatus,
    state: "stopped",
    phase: undefined,
  });

  await waitFor(() => {
    expect(screen.getByText("WokCore stopped")).toBeInTheDocument();
  });
  await queryClient.invalidateQueries({ queryKey: ["core-status"] });
  expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(1);
});

it("keeps a failed startup update check non-blocking and retries it manually", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockRejectedValue(
    new Error("failed at C:\\private\\manifest.json with token"),
  );
  vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  expect(
    await screen.findByText("WokCore update check unavailable"),
  ).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(
    screen.queryByText(/private|manifest\.json|token/i),
  ).not.toBeInTheDocument();

  await user.click(
    screen.getByRole("button", { name: "Check for updates" }),
  );

  expect(retryCoreUpdateCheck).toHaveBeenCalledOnce();
  expect(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  ).toBeEnabled();
});

it("does not automatically recheck after a rejected startup check remount", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  const automaticBridgeCheck = vi
    .fn()
    .mockRejectedValue(new Error("offline"));
  const automaticAttempt = automaticBridgeCheck();
  vi.mocked(checkCoreUpdateOnce).mockImplementation(
    () => automaticAttempt,
  );
  vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  const first = renderLifecycle(undefined, false);
  expect(
    await screen.findByText("WokCore update check unavailable"),
  ).toBeInTheDocument();
  first.unmount();

  renderLifecycle(undefined, false);
  expect(
    await screen.findByText("WokCore update check unavailable"),
  ).toBeInTheDocument();
  expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(2);
  expect(automaticBridgeCheck).toHaveBeenCalledTimes(1);
  expect(retryCoreUpdateCheck).not.toHaveBeenCalled();

  await user.click(
    screen.getByRole("button", { name: "Check for updates" }),
  );
  expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(1);
});

it("requires an accessible confirmation and invokes the expected version once", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const installResult = deferred<CoreOperation>();
  vi.mocked(installCoreUpdate).mockReturnValue(installResult.promise);
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  const trigger = await screen.findByRole("button", {
    name: "Upgrade WokCore",
  });
  await user.click(trigger);
  let dialog = screen.getByRole("dialog", {
    name: "Upgrade WokCore?",
  });
  expect(dialog).toHaveTextContent("0.2.0");
  expect(dialog).toHaveTextContent("0.2.1");
  const cancel = screen.getByRole("button", { name: "Cancel" });
  const focusedConfirm = screen.getByRole("button", {
    name: "Confirm upgrade",
  });
  expect(focusedConfirm).toHaveFocus();
  await user.tab();
  expect(cancel).toHaveFocus();
  await user.tab({ shift: true });
  expect(focusedConfirm).toHaveFocus();
  await user.click(cancel);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(installCoreUpdate).not.toHaveBeenCalled();
  expect(trigger).toHaveFocus();

  await user.click(trigger);
  dialog = screen.getByRole("dialog", { name: "Upgrade WokCore?" });
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();

  await user.click(trigger);
  const confirm = screen.getByRole("button", {
    name: "Confirm upgrade",
  });
  await user.click(confirm);
  await waitFor(() => {
    expect(trigger).toHaveFocus();
  });
  fireEvent.click(confirm);

  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
  expect(installCoreUpdate).toHaveBeenCalledWith("0.2.1");
  installResult.resolve(checkingUpdateOperation);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
});

it("recovers an operation_in_progress rejection from the active snapshot", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  vi.mocked(getCoreOperation)
    .mockResolvedValueOnce(null)
    .mockResolvedValueOnce({
      ...checkingUpdateOperation,
      sequence: 2,
      phase: "downloading",
      bytesCompleted: 40,
      bytesTotal: 100,
    });
  vi.mocked(installCoreUpdate).mockRejectedValue(
    new Error("operation_in_progress"),
  );
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );

  expect(
    await screen.findByRole("progressbar", {
      name: "Download WokCore progress",
    }),
  ).toHaveAttribute("aria-valuenow", "40");
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
  expect(getCoreOperation).toHaveBeenCalledTimes(2);
});

it.each([
  ["active_requests_remain", "Try update later"],
  ["rolled_back", "Try update again"],
  ["recovery_required", "Try update again"],
] as const)(
  "fresh-checks before retrying a recovered %s update without a cached candidate",
  async (errorCode, retryLabel) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
      code: "current",
      currentVersion: "0.2.0",
    });
    vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.2",
    });
    vi.mocked(getCoreOperation).mockResolvedValue({
      ...checkingUpdateOperation,
      sequence: 8,
      state: "failed",
      phase: "completed",
      activeRequests:
        errorCode === "active_requests_remain" ? 4 : undefined,
      errorCode,
    });
    const user = userEvent.setup();

    renderLifecycle(undefined, false);

    await user.click(
      await screen.findByRole("button", { name: retryLabel }),
    );

    expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByRole("dialog", { name: "Upgrade WokCore?" }),
    ).toHaveTextContent("0.2.2");
    expect(installCoreUpdate).not.toHaveBeenCalled();
  },
);

it("uses independent safe recovery when update invoke rejects without an active operation", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.2",
  });
  vi.mocked(getCoreOperation)
    .mockResolvedValueOnce(null)
    .mockResolvedValueOnce(null);
  vi.mocked(installCoreUpdate).mockRejectedValue(
    new Error("failed at C:\\private\\wokcore.exe with token"),
  );
  const user = userEvent.setup();

  renderLifecycle(undefined, false);
  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );

  expect(
    await screen.findByRole("heading", {
      name: "WokCore update could not start",
    }),
  ).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  expect(
    screen.queryByText(/private|wokcore\.exe|token/i),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Upgrade WokCore" }),
  ).not.toBeInTheDocument();

  await user.click(
    screen.getByRole("button", { name: "Retry update safely" }),
  );
  expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(1);
  expect(
    screen.getByRole("dialog", { name: "Upgrade WokCore?" }),
  ).toHaveTextContent("0.2.2");
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
});

it("returns management after active requests defer the update and reconfirms retry", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  vi.mocked(retryCoreUpdateCheck).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.2",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );
  operationListener?.({
    ...checkingUpdateOperation,
    sequence: 5,
    state: "failed",
    phase: "completed",
    activeRequests: 3,
    errorCode: "active_requests_remain",
  });

  expect(
    await screen.findByText(/3 active requests remain/i),
  ).toBeInTheDocument();
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();
  const retry = screen.getByRole("button", { name: "Try update later" });
  await user.click(retry);
  expect(retryCoreUpdateCheck).toHaveBeenCalledTimes(1);
  expect(
    screen.getByRole("dialog", { name: "Upgrade WokCore?" }),
  ).toBeInTheDocument();
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
});

it("clears a stale current result without claiming an installation", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce)
    .mockResolvedValueOnce({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    })
    .mockResolvedValue({
      code: "current",
      currentVersion: "0.2.0",
    });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);
  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );
  operationListener?.({
    ...completedUpdateOperation,
    targetVersion: undefined,
    currentVersion: "0.2.0",
  });

  expect(
    await screen.findByText("WokCore workspace"),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Upgrade WokCore" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText(/installed successfully/i)).not.toBeInTheDocument();
});

it("refreshes every lifecycle query and reauthorizes only after update status requires it", async () => {
  const authorizationStatus: CoreStatus = {
    ...runningStatus,
    state: "authorization_required",
    phase: undefined,
  };
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(runningStatus)
    .mockResolvedValueOnce(authorizationStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  vi.mocked(startCore).mockResolvedValue(undefined);
  const user = userEvent.setup();
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

  renderLifecycle(queryClient, false);
  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );
  operationListener?.(completedUpdateOperation);

  await waitFor(() => {
    expect(startCore).toHaveBeenCalledTimes(1);
  });
  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
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

it("does not authorize when update refresh selects a development runtime", async () => {
  const developmentAuthorizationStatus: CoreStatus = {
    ...runningStatus,
    state: "authorization_required",
    runtime_channel: "development",
    phase: undefined,
  };
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(runningStatus)
    .mockResolvedValue(developmentAuthorizationStatus);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const user = userEvent.setup();

  renderLifecycle(undefined, false);
  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );
  operationListener?.(completedUpdateOperation);

  expect(await screen.findByText("Development")).toBeInTheDocument();
  expect(startCore).not.toHaveBeenCalled();
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
  expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(1);
  expect(retryCoreUpdateCheck).not.toHaveBeenCalled();
});

it("uses status-only recovery when an update refresh and fallback both fail", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(runningStatus)
    .mockRejectedValue(
      new Error("status failed at C:\\private\\status.json"),
    );
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const { queryClient } =
    queryClientWithCoreRefreshFailure("reject");
  const user = userEvent.setup();

  renderLifecycle(queryClient, false);
  await user.click(
    await screen.findByRole("button", { name: "Upgrade WokCore" }),
  );
  await user.click(
    screen.getByRole("button", { name: "Confirm upgrade" }),
  );
  operationListener?.(completedUpdateOperation);

  expect(
    await screen.findByRole("heading", {
      name: "WokCore update completed, but status is unavailable",
    }),
  ).toBeInTheDocument();
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
  expect(screen.getByText("WokCore workspace")).toBeInTheDocument();

  vi.mocked(getCoreStatus).mockResolvedValue({
    ...runningStatus,
    version: "0.2.1",
  });
  await user.click(
    screen.getByRole("button", { name: "Check status again" }),
  );

  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(installCoreUpdate).toHaveBeenCalledTimes(1);
});

it.each([
  ["running", "WokCore running"],
  ["stopped", "WokCore stopped"],
] as const)(
  "preserves the resulting %s service state after update",
  async (state, title) => {
    const finalStatus: CoreStatus = {
      state,
      runtime_channel: "production",
      version: "0.2.1",
      capabilities: [],
      ...(state === "running" ? { phase: "running" as const } : {}),
    };
    vi.mocked(getCoreStatus)
      .mockResolvedValueOnce(runningStatus)
      .mockResolvedValue(finalStatus);
    vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
      code: "update_available",
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    });
    const user = userEvent.setup();

    renderLifecycle(undefined, false);
    await user.click(
      await screen.findByRole("button", { name: "Upgrade WokCore" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Confirm upgrade" }),
    );
    operationListener?.(completedUpdateOperation);

    expect(await screen.findByText(title)).toBeInTheDocument();
    expect(startCore).not.toHaveBeenCalled();
    expect(stopCore).not.toHaveBeenCalled();
  },
);

it.each([
  [
    "the installed target",
    {
      ...completedUpdateOperation,
      targetVersion: "0.2.2",
    },
    "0.2.2",
  ],
  [
    "the verified current version",
    {
      ...completedUpdateOperation,
      targetVersion: undefined,
      currentVersion: "0.2.0",
    },
    "0.2.0",
  ],
] as const)(
  "replaces the process update cache with %s for the next eligible epoch and a remount",
  async (_scenario, terminalOperation, currentVersion) => {
    vi.mocked(getCoreStatus).mockResolvedValue(runningStatus);
    const underlyingCheck = vi.fn().mockResolvedValue({
      code: "update_available" as const,
      currentVersion: "0.2.0",
      targetVersion: "0.2.1",
    });
    let processCheck = underlyingCheck();
    vi.mocked(checkCoreUpdateOnce).mockImplementation(
      () => processCheck,
    );
    vi.mocked(rememberCoreUpdateCompletion).mockImplementation(
      (operation) => {
        processCheck = Promise.resolve({
          code: "current",
          currentVersion:
            operation.targetVersion ?? operation.currentVersion!,
        });
      },
    );
    const user = userEvent.setup();

    const first = renderLifecycle(undefined, false);
    await user.click(
      await screen.findByRole("button", {
        name: "Upgrade WokCore",
      }),
    );
    await user.click(
      screen.getByRole("button", { name: "Confirm upgrade" }),
    );
    operationListener?.(terminalOperation);

    await waitFor(() => {
      expect(rememberCoreUpdateCompletion).toHaveBeenCalledWith(
        terminalOperation,
      );
    });
    await waitFor(() => {
      expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(2);
    });
    first.unmount();

    renderLifecycle(undefined, false);
    await screen.findByText("WokCore running");
    await waitFor(() => {
      expect(checkCoreUpdateOnce).toHaveBeenCalledTimes(3);
    });
    expect(underlyingCheck).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByRole("button", { name: "Upgrade WokCore" }),
    ).not.toBeInTheDocument();
    await expect(processCheck).resolves.toEqual({
      code: "current",
      currentVersion,
    });
  },
);

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
      name: "WokCore operation monitoring unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|events\.log/i),
  ).not.toBeInTheDocument();
  expect(getCoreOperation).not.toHaveBeenCalled();
  expect(installAndStartCore).not.toHaveBeenCalled();

  await user.click(
    screen.getByRole("button", {
      name: "Reconnect operation monitoring",
    }),
  );

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
      name: "WokCore operation monitoring unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/private|operation\.json/i),
  ).not.toBeInTheDocument();
  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(installAndStartCore).not.toHaveBeenCalled();

  await user.click(
    screen.getByRole("button", {
      name: "Reconnect operation monitoring",
    }),
  );

  expect(listenForCoreOperation).toHaveBeenCalledTimes(2);
  expect(getCoreOperation).toHaveBeenCalledTimes(2);
  expect(installAndStartCore).toHaveBeenCalledTimes(1);
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();
});

it("recovers missing setup from repeated bridge arbitration without retaining stale progress", async () => {
  vi.mocked(getCoreStatus).mockResolvedValue(missingStatus);
  const initialOperation = deferred<CoreOperation | null>();
  vi.mocked(getCoreOperation)
    .mockReturnValueOnce(initialOperation.promise)
    .mockRejectedValueOnce(
      new Error("retry failed at C:\\private\\operation.json"),
    )
    .mockResolvedValueOnce(downloadingOperation);
  const user = userEvent.setup();

  renderLifecycle(undefined, false);

  await waitFor(() => {
    expect(operationListener).toBeDefined();
    expect(getCoreOperation).toHaveBeenCalledOnce();
  });
  act(() => {
    operationListener?.(checkingOperation);
  });
  expect(
    await screen.findByRole("heading", {
      name: "Checking for a WokCore release",
    }),
  ).toBeInTheDocument();

  await act(async () => {
    initialOperation.reject(
      new Error("snapshot failed at C:\\private\\operation.json"),
    );
    await initialOperation.promise.catch(() => undefined);
  });

  expect(
    await screen.findByRole("heading", {
      name: "WokCore operation monitoring unavailable",
    }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("progressbar", {
      name: "WokCore release check progress",
    }),
  ).not.toBeInTheDocument();
  expect(installAndStartCore).not.toHaveBeenCalled();

  await user.click(
    screen.getByRole("button", {
      name: "Reconnect operation monitoring",
    }),
  );
  await waitFor(() => {
    expect(getCoreOperation).toHaveBeenCalledTimes(2);
  });
  expect(
    screen.getByRole("heading", {
      name: "WokCore operation monitoring unavailable",
    }),
  ).toBeInTheDocument();

  await user.click(
    screen.getByRole("button", {
      name: "Reconnect operation monitoring",
    }),
  );

  expect(
    await screen.findByRole("progressbar", {
      name: "Download WokCore progress",
    }),
  ).toHaveAttribute("aria-valuenow", "25");
  expect(listenForCoreOperation).toHaveBeenCalledTimes(3);
  expect(getCoreOperation).toHaveBeenCalledTimes(3);
  expect(installAndStartCore).not.toHaveBeenCalled();
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

it("ignores a delayed terminal invoke receipt after success cleanup", async () => {
  vi.mocked(getCoreStatus)
    .mockResolvedValueOnce(missingStatus)
    .mockResolvedValue(runningStatus);
  vi.mocked(getCoreOperation).mockResolvedValue(null);
  vi.mocked(checkCoreUpdateOnce).mockResolvedValue({
    code: "update_available",
    currentVersion: "0.2.0",
    targetVersion: "0.2.1",
  });
  const installResult = deferred<CoreOperation>();
  vi.mocked(installAndStartCore).mockReturnValue(installResult.promise);
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");
  const user = userEvent.setup();

  renderLifecycle(queryClient, false);

  await waitFor(() => {
    expect(installAndStartCore).toHaveBeenCalledOnce();
  });
  act(() => {
    operationListener?.(completedOperation);
  });

  const upgrade = await screen.findByRole("button", {
    name: "Upgrade WokCore",
  });
  expect(invalidateQueries).toHaveBeenCalledTimes(7);

  await act(async () => {
    installResult.resolve(completedOperation);
    await installResult.promise;
  });
  await user.click(upgrade);

  expect(
    screen.getByRole("dialog", { name: "Upgrade WokCore?" }),
  ).toBeInTheDocument();
  expect(invalidateQueries).toHaveBeenCalledTimes(7);
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
