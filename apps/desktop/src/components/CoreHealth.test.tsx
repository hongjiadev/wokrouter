import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  getCoreStatus,
  startCore,
  stopCore,
  type CoreStatus,
} from "../control";
import { CoreHealth } from "./CoreHealth";

vi.mock("../control", () => ({
  coreStatusQueryKey: ["core-status"],
  getCoreStatus: vi.fn(),
  startCore: vi.fn(),
  stopCore: vi.fn(),
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

  return render(<CoreHealth />, { wrapper: Wrapper });
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
  beforeEach(() => {
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
