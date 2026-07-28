import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

import { getCoreStatus } from "./control";
import { App } from "./App";

vi.mock("./control", () => ({
  getCoreStatus: vi.fn(),
  startCore: vi.fn(),
  stopCore: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(getCoreStatus).mockResolvedValue({
    state: "running",
    version: "0.1.0",
    management_api_major: 1,
    capabilities: [],
    phase: "running",
  });
});

it("keeps WokCore health as the desktop shell's primary surface", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  render(
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>,
  );

  expect(screen.getByText("WokRouter")).toBeInTheDocument();
  expect(await screen.findByText("WokCore running")).toBeInTheDocument();
  expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
});
