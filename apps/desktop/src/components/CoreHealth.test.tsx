import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps, ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  getCoreStatus,
  startCore,
  stopCore,
  type CoreStatus,
} from "../control";
import { initializeI18n } from "../i18n";
import { CoreHealth } from "./CoreHealth";

vi.mock("../control", () => ({
  coreStatusQueryKey: ["core-status"],
  getCoreStatus: vi.fn(),
  startCore: vi.fn(),
  stopCore: vi.fn(),
}));

function renderHealth(props: ComponentProps<typeof CoreHealth> = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  return render(<CoreHealth {...props} />, { wrapper: Wrapper });
}

function status(
  state: CoreStatus["state"],
  fields: Partial<CoreStatus> = {},
): CoreStatus {
  return {
    state,
    runtime_channel: "production",
    capabilities: [],
    ...fields,
  };
}

function deferred<T>() {
  let reject!: (reason?: unknown) => void;
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    reject = rejectPromise;
    resolve = resolvePromise;
  });
  return { promise, reject, resolve };
}

describe("CoreHealth", () => {
  beforeEach(async () => {
    await initializeI18n("en");
    vi.mocked(getCoreStatus).mockReset();
    vi.mocked(startCore).mockReset();
    vi.mocked(stopCore).mockReset();
  });

  it.each([
    ["missing", "WokCore not installed"],
    ["starting", "WokCore starting"],
    ["draining", "WokCore draining"],
    ["authorization_required", "WokRouter authorization required"],
    ["incompatible", "WokCore update required"],
    ["invalid_runtime", "WokCore runtime invalid"],
  ] as const)("renders the %s state honestly", async (stateName, title) => {
    vi.mocked(getCoreStatus).mockResolvedValue(status(stateName));

    renderHealth();

    expect(await screen.findByText(title)).toBeInTheDocument();
  });

  it("renders missing runtime metadata in Simplified Chinese", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getCoreStatus).mockResolvedValue(status("missing"));

    renderHealth();

    expect(await screen.findByText("WokCore 未安装")).toBeInTheDocument();
    expect(screen.getByText("运行状态")).toBeInTheDocument();
    expect(screen.getByText("版本")).toBeInTheDocument();
    expect(screen.getByText("未连接")).toBeInTheDocument();
  });

  it.each([
    ["missing", "WokCore 未安装"],
    ["stopped", "WokCore 已停止"],
    ["starting", "WokCore 正在启动"],
    ["running", "WokCore 正在运行"],
    ["draining", "WokCore 正在排空请求"],
    ["authorization_required", "需要授权 WokRouter"],
    ["incompatible", "需要更新 WokCore"],
    ["invalid_runtime", "WokCore 运行时无效"],
  ] as const)("translates the %s state title", async (stateName, title) => {
    await initializeI18n("zh-CN");
    vi.mocked(getCoreStatus).mockResolvedValue(status(stateName));

    renderHealth();

    expect(await screen.findByRole("heading", { name: title })).toBeInTheDocument();
  });

  it("translates runtime channel, phase, active requests, and actions", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getCoreStatus).mockResolvedValue(
      status("stopped", {
        version: "0.1.23",
        phase: "awaiting_cancellation",
        active_requests: 3,
      }),
    );

    renderHealth();

    expect(await screen.findByText("生产环境")).toBeInTheDocument();
    expect(screen.getByText("正在等待取消")).toBeInTheDocument();
    expect(screen.getByText("活动请求")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 WokCore" })).toBeEnabled();
    expect(screen.getByText("关闭此窗口不会停止 WokCore。")).toBeInTheDocument();
    expect(screen.getByText("0.1.23").closest("dd")).toHaveAttribute(
      "dir",
      "ltr",
    );
  });

  it.each([
    ["en", "Active requests"],
    ["zh-CN", "活动请求"],
  ] as const)(
    "formats active-request counts with the selected %s locale",
    async (locale, label) => {
      await initializeI18n(locale);
      vi.mocked(getCoreStatus).mockResolvedValue(
        status("running", { active_requests: 1_000_000 }),
      );

      renderHealth();

      const field = await screen.findByText(label);
      expect(field.nextElementSibling).toHaveTextContent(
        new Intl.NumberFormat(locale).format(1_000_000),
      );
    },
  );

  it("translates unavailable status and its live announcement without bridge details", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getCoreStatus).mockRejectedValue(
      new Error("failed at C:\\private\\status.json"),
    );

    renderHealth();

    expect(
      await screen.findByRole("heading", { name: "WokCore 状态不可用" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "WokCore 状态不可用。请重新检查。" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新检查" })).toBeEnabled();
    expect(screen.queryByText(/private|status\.json/i)).not.toBeInTheDocument();
  });

  it("translates update availability and failed-check recovery", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getCoreStatus).mockResolvedValue(
      status("running", { version: "0.1.22" }),
    );

    const view = renderHealth({
      updateCheck: {
        code: "update_available",
        currentVersion: "0.1.22",
        targetVersion: "0.1.23",
      },
      onUpgrade: vi.fn(),
    });

    expect(
      await screen.findByRole("button", { name: "升级 WokCore" }),
    ).toBeEnabled();

    view.rerender(
      <CoreHealth
        updateCheckFailed
        onCheckForUpdates={vi.fn()}
      />,
    );
    expect(await screen.findByText("无法检查 WokCore 更新")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
  });

  it("updates static state maps when the locale changes without remounting", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue(status("stopped"));

    renderHealth();

    const heading = await screen.findByRole("heading", {
      name: "WokCore stopped",
    });
    const liveRegion = screen.getByRole("status");
    await act(async () => {
      await initializeI18n("zh-CN");
    });

    expect(screen.getByRole("heading", { name: "WokCore 已停止" })).toBe(heading);
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveAccessibleName("WokCore 已停止。");
  });

  it.each([
    ["development", "Development"],
    ["production", "Production"],
  ] as const)(
    "renders the selected %s channel from backend status",
    async (runtimeChannel, label) => {
      vi.mocked(getCoreStatus).mockResolvedValue(
        status("stopped", { runtime_channel: runtimeChannel }),
      );

      renderHealth();

      expect(await screen.findByText("Runtime channel")).toBeInTheDocument();
      expect(screen.getByText(label)).toBeInTheDocument();
    },
  );

  it.each([
    ["missing", "WokCore not installed"],
    ["stopped", "WokCore stopped"],
    ["running", "WokCore running"],
    ["authorization_required", "WokRouter authorization required"],
  ] as const)(
    "keeps the development %s runtime read-only",
    async (stateName, title) => {
      vi.mocked(getCoreStatus).mockResolvedValue(
        status(stateName, { runtime_channel: "development" }),
      );

      renderHealth();

      expect(await screen.findByText(title)).toBeInTheDocument();
      expect(screen.getByText("Development")).toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: /start wokcore|authorize wokrouter|stop wokcore|check again/i,
        }),
      ).not.toBeInTheDocument();
      expect(startCore).not.toHaveBeenCalled();
      expect(stopCore).not.toHaveBeenCalled();
    },
  );

  it("keeps status refetch available for an incompatible development runtime", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue(
      status("incompatible", { runtime_channel: "development" }),
    );

    renderHealth();

    expect(
      await screen.findByRole("button", { name: "Check again" }),
    ).toBeEnabled();
    expect(startCore).not.toHaveBeenCalled();
    expect(stopCore).not.toHaveBeenCalled();
  });

  it("offers the verified production update candidate through its owner", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue(
      status("running", { version: "0.1.22" }),
    );
    const onUpgrade = vi.fn();
    const user = userEvent.setup();

    renderHealth({
      updateCheck: {
        code: "update_available",
        currentVersion: "0.1.22",
        targetVersion: "0.1.23",
      },
      onUpgrade,
    });

    const trigger = await screen.findByRole("button", {
      name: "Upgrade WokCore",
    });
    await user.click(trigger);
    expect(onUpgrade).toHaveBeenCalledWith(trigger);
  });

  it("suppresses even a supplied update candidate for development", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue(
      status("running", {
        runtime_channel: "development",
        version: "0.1.22",
      }),
    );

    renderHealth({
      updateCheck: {
        code: "update_available",
        currentVersion: "0.1.22",
        targetVersion: "0.1.23",
      },
      updateCheckFailed: true,
      onCheckForUpdates: vi.fn(),
      onUpgrade: vi.fn(),
    });

    expect(await screen.findByText("Development")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: /upgrade wokcore|check for updates/i,
      }),
    ).not.toBeInTheDocument();
  });

  it("starts a stopped WokCore and refreshes its status", async () => {
    vi.mocked(getCoreStatus)
      .mockResolvedValueOnce(status("stopped", { version: "0.1.0" }))
      .mockResolvedValueOnce(
        status("running", {
          version: "0.1.1",
          phase: "running",
          active_requests: 0,
        }),
      );
    vi.mocked(startCore).mockResolvedValue(undefined);
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Start WokCore" }),
    );
    expect(await screen.findByText("WokCore running")).toBeInTheDocument();
    expect(screen.getByText("Loopback HTTP")).toBeInTheDocument();
    expect(screen.getByText("0.1.1")).toBeInTheDocument();
    expect(getCoreStatus).toHaveBeenCalledTimes(2);
  });

  it("stops a running WokCore only through the explicit action", async () => {
    vi.mocked(getCoreStatus)
      .mockResolvedValueOnce(status("running", { version: "0.1.1" }))
      .mockResolvedValueOnce(status("stopped", { version: "0.1.1" }));
    vi.mocked(stopCore).mockResolvedValue(undefined);
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Stop WokCore" }),
    );
    expect(await screen.findByText("WokCore stopped")).toBeInTheDocument();
    expect(stopCore).toHaveBeenCalledOnce();
  });

  it("uses an accessible skeleton while checking status", () => {
    vi.mocked(getCoreStatus).mockReturnValue(new Promise(() => {}));

    renderHealth();

    expect(
      screen.getByRole("status", { name: "Checking WokCore status" }),
    ).toBeInTheDocument();
  });

  it("offers a safe retry without exposing bridge details", async () => {
    vi.mocked(getCoreStatus)
      .mockRejectedValueOnce(new Error("C:\\Users\\someone\\token.json"))
      .mockResolvedValueOnce(status("running", { version: "0.1.0" }));
    const user = userEvent.setup();

    renderHealth();

    expect(
      await screen.findByText("WokCore status unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/someone|token\.json/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  });

  it("keeps recovery available after start fails", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue(status("stopped"));
    vi.mocked(startCore).mockRejectedValue(
      new Error("process failed at C:\\private\\wokcore.exe"),
    );
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Start WokCore" }),
    );
    expect(
      await screen.findByText("WokCore could not start"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/private|wokcore\.exe/i)).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Try starting again" }),
    ).toBeEnabled();
  });

  it("keeps one live region mounted through lifecycle changes", async () => {
    const initialStatus = deferred<CoreStatus>();
    const startResult = deferred<void>();
    vi.mocked(getCoreStatus)
      .mockReturnValueOnce(initialStatus.promise)
      .mockResolvedValueOnce(status("running", { version: "0.1.1" }));
    vi.mocked(startCore).mockReturnValue(startResult.promise);
    const user = userEvent.setup();

    renderHealth();

    const liveRegion = screen.getByRole("status");
    initialStatus.resolve(status("stopped", { version: "0.1.0" }));
    expect(await screen.findByText("WokCore stopped")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);

    await user.click(screen.getByRole("button", { name: "Start WokCore" }));
    expect(liveRegion).toHaveTextContent("Starting WokCore");
    startResult.resolve();

    expect(await screen.findByText("WokCore running")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });
});
