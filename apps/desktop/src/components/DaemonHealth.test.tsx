import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getDaemonStatus, startDaemon } from "../control";
import { DaemonHealth } from "./DaemonHealth";

vi.mock("../control", () => ({
  getDaemonStatus: vi.fn(),
  startDaemon: vi.fn(),
}));

function renderHealth() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  return render(<DaemonHealth />, { wrapper: Wrapper });
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

describe("DaemonHealth", () => {
  beforeEach(() => {
    vi.mocked(getDaemonStatus).mockReset();
    vi.mocked(startDaemon).mockReset();
  });

  it("shows daemon version and stopped recovery action", async () => {
    vi.mocked(getDaemonStatus).mockResolvedValue({
      state: "stopped",
      version: "0.1.0",
    });

    renderHealth();

    expect(await screen.findByText("Daemon stopped")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Start WokRouter" }),
    ).toBeEnabled();
  });

  it("shows the running daemon and its version", async () => {
    vi.mocked(getDaemonStatus).mockResolvedValue({
      state: "running",
      version: "0.1.7",
    });

    renderHealth();

    expect(await screen.findByText("Daemon running")).toBeInTheDocument();
    expect(screen.getByText("0.1.7")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Start WokRouter" }),
    ).not.toBeInTheDocument();
  });

  it("uses an accessible skeleton while checking status", () => {
    vi.mocked(getDaemonStatus).mockReturnValue(new Promise(() => {}));

    renderHealth();

    expect(
      screen.getByRole("status", { name: "Checking daemon status" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Daemon stopped")).not.toBeInTheDocument();
  });

  it("offers a safe retry when the status check fails", async () => {
    vi.mocked(getDaemonStatus)
      .mockRejectedValueOnce(new Error("C:\\Users\\someone\\control.sock"))
      .mockResolvedValueOnce({ state: "running", version: "0.1.0" });
    const user = userEvent.setup();

    renderHealth();

    expect(
      await screen.findByText("Daemon status unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/someone|control\.sock/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("Daemon running")).toBeInTheDocument();
  });

  it("disables the recovery action while start is pending", async () => {
    vi.mocked(getDaemonStatus).mockResolvedValue({
      state: "stopped",
      version: "0.1.0",
    });
    vi.mocked(startDaemon).mockReturnValue(new Promise(() => {}));
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Start WokRouter" }),
    );
    expect(
      screen.getByRole("button", { name: "Starting WokRouter…" }),
    ).toBeDisabled();
  });

  it("keeps recovery available after start fails", async () => {
    vi.mocked(getDaemonStatus).mockResolvedValue({
      state: "stopped",
      version: "0.1.0",
    });
    vi.mocked(startDaemon).mockRejectedValue(
      new Error("process failed at C:\\private\\wokrouter.exe"),
    );
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Start WokRouter" }),
    );
    expect(
      await screen.findByText("WokRouter couldn’t start"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/private|wokrouter\.exe/i)).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Try starting again" }),
    ).toBeEnabled();
  });

  it("refetches status after a successful start", async () => {
    vi.mocked(getDaemonStatus)
      .mockResolvedValueOnce({ state: "stopped", version: "0.1.0" })
      .mockResolvedValueOnce({ state: "running", version: "0.1.1" });
    vi.mocked(startDaemon).mockResolvedValue(undefined);
    const user = userEvent.setup();

    renderHealth();

    await user.click(
      await screen.findByRole("button", { name: "Start WokRouter" }),
    );
    expect(await screen.findByText("Daemon running")).toBeInTheDocument();
    expect(screen.getByText("0.1.1")).toBeInTheDocument();
    expect(getDaemonStatus).toHaveBeenCalledTimes(2);
  });

  it("keeps one live region mounted from loading through start success", async () => {
    const initialStatus = deferred<{
      state: "stopped";
      version: string;
    }>();
    const startResult = deferred<void>();
    vi.mocked(getDaemonStatus)
      .mockReturnValueOnce(initialStatus.promise)
      .mockResolvedValueOnce({ state: "running", version: "0.1.1" });
    vi.mocked(startDaemon).mockReturnValue(startResult.promise);
    const user = userEvent.setup();

    renderHealth();

    const liveRegion = screen.getByRole("status");
    expect(liveRegion).toHaveTextContent("Checking daemon status");
    initialStatus.resolve({ state: "stopped", version: "0.1.0" });
    expect(await screen.findByText("Daemon stopped")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveTextContent("Daemon stopped. Version 0.1.0");

    await user.click(screen.getByRole("button", { name: "Start WokRouter" }));
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveTextContent("Starting WokRouter");
    startResult.resolve();

    expect(await screen.findByText("Daemon running")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveTextContent("Daemon running. Version 0.1.1");
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });

  it("keeps the same live region through status error recovery", async () => {
    const initialStatus = deferred<never>();
    vi.mocked(getDaemonStatus)
      .mockReturnValueOnce(initialStatus.promise)
      .mockResolvedValueOnce({ state: "running", version: "0.1.0" });
    const user = userEvent.setup();

    renderHealth();

    const liveRegion = screen.getByRole("status");
    initialStatus.reject(new Error("private IPC detail"));
    expect(
      await screen.findByText("Daemon status unavailable"),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveTextContent("Daemon status unavailable");

    await user.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("Daemon running")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBe(liveRegion);
  });
});
