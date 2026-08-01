import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { getCoreStatus } from "./control";
import { App } from "./App";
import { initializeI18n } from "./i18n";

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

beforeEach(async () => {
  await initializeI18n("en");
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

it("renders the English shell copy from the default catalog", () => {
  render(<App />);

  expect(screen.getByText("Local desktop control")).toBeInTheDocument();
  expect(
    screen.getByText(
      "Desktop controls communicate with WokCore over loopback HTTP.",
    ),
  ).toBeInTheDocument();
});

it("renders the Simplified Chinese shell copy", async () => {
  await initializeI18n("zh-CN");

  render(<App />);

  expect(screen.getByText("本地桌面控制")).toBeInTheDocument();
  expect(
    screen.getByText("桌面控制通过环回 HTTP 与 WokCore 通信。"),
  ).toBeInTheDocument();
});
