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
});
