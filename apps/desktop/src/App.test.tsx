import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { getCoreStatus } from "./control";
import { App } from "./App";

vi.mock("./components/ManagementPanel", () => ({
  ManagementPanel: () => <section>WokCore workspace</section>,
}));
vi.mock("./components/CoreLifecycle", () => ({
  CoreLifecycle: () => <section>WokCore lifecycle owner</section>,
}));
vi.mock("./control", () => ({
  coreStatusQueryKey: ["core-status"],
  getCoreStatus: vi.fn(),
  startCore: vi.fn(),
  stopCore: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(getCoreStatus).mockResolvedValue({
    state: "running",
    runtime_channel: "production",
    version: "0.1.0",
    management_api_major: 1,
    capabilities: [],
    phase: "running",
  });
});

it("mounts one lifecycle owner for health, setup, and management", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  render(
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>,
  );

  expect(screen.getByText("WokRouter")).toBeInTheDocument();
  expect(screen.getByText("WokCore lifecycle owner")).toBeInTheDocument();
  expect(screen.queryByText("WokCore workspace")).not.toBeInTheDocument();
  expect(getCoreStatus).not.toHaveBeenCalled();
});
