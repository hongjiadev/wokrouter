import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { getDaemonStatus } from "./control";
import { App } from "./App";

vi.mock("./control", () => ({
  getDaemonStatus: vi.fn(),
  startDaemon: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(getDaemonStatus).mockResolvedValue({
    state: "running",
    version: "0.1.0",
  });
});

it("keeps daemon health as the desktop shell’s primary surface", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  render(
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>,
  );

  expect(screen.getByText("WokRouter")).toBeInTheDocument();
  expect(await screen.findByText("Daemon running")).toBeInTheDocument();
  expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
});
