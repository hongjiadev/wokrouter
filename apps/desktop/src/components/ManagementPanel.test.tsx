import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getCoreStatus } from "../control";
import {
  commitProviderConfig,
  exportDiagnostics,
  getDiagnosticLogs,
  getProviderCatalog,
  getProviderModels,
  getProviderRuntime,
  getSessionMessages,
  getSessions,
  getUsage,
  reloadProviders,
  validateProviderConfig,
} from "../management";
import { ManagementPanel } from "./ManagementPanel";

vi.mock("../control", () => ({
  coreStatusQueryKey: ["core-status"],
  getCoreStatus: vi.fn(),
}));
vi.mock("../management", () => ({
  commitProviderConfig: vi.fn(),
  createProviderSecret: vi.fn(),
  deleteProviderSecret: vi.fn(),
  exportDiagnostics: vi.fn(),
  getDiagnosticLogs: vi.fn(),
  getProviderCatalog: vi.fn(),
  getProviderModels: vi.fn(),
  getProviderRuntime: vi.fn(),
  getSessionMessages: vi.fn(),
  getSessions: vi.fn(),
  getUsage: vi.fn(),
  reloadProviders: vi.fn(),
  replaceProviderSecret: vi.fn(),
  validateProviderConfig: vi.fn(),
}));

const capabilities = [
  "diagnostics.events.v1",
  "diagnostics.export.v1",
  "provider.catalog.v1",
  "provider.config.v1",
  "provider.models.v1",
  "provider.secrets.v1",
  "sessions.index.v1",
  "sessions.messages.v1",
  "usage.session.v1",
];

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return render(<ManagementPanel />, { wrapper: Wrapper });
}

function mockProviderData() {
  vi.mocked(getProviderCatalog).mockResolvedValue({
    schema_version: 1,
    catalog_schema_version: 1,
    baseline_commit: "synthetic",
    providers: [
      {
        id: "openai",
        label: "OpenAI",
        adapter: "open_ai_responses",
        base_url: "https://example.invalid",
        auth_kind: "key",
        endpoint_policy: "public_https",
        model_source: "live",
        aliases: [],
        models: ["gpt-synthetic"],
        default_model: "gpt-synthetic",
        allow_endpoint_override: false,
        key_optional: false,
        allow_key_auth_override: false,
        reasoning_efforts: [],
        reasoning_effort_map: {},
        capabilities: {
          text: true,
          streaming: true,
          tools: true,
          vision: false,
          images: false,
          reasoning: true,
        },
      },
    ],
  });
  vi.mocked(getProviderRuntime).mockResolvedValue({
    schema_version: 1,
    revision: 4,
    snapshot_revision: 4,
    reload_status: "ready",
    provider_count: 1,
    models: [],
    providers: {
      instances: [
        {
          id: "primary",
          catalog_id: "openai",
          enabled: true,
          endpoint: null,
          allow_private_network: false,
        },
      ],
      accounts: [],
    },
    routing: { aliases: [], rules: [], default: null },
  });
  vi.mocked(getProviderModels).mockResolvedValue({
    schema_version: 1,
    models: [],
  });
}

describe("ManagementPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getCoreStatus).mockResolvedValue({
      state: "running",
      version: "0.1.0",
      management_api_major: 1,
      capabilities,
      phase: "running",
    });
    mockProviderData();
    vi.mocked(getSessions).mockResolvedValue({
      schema_version: 1,
      items: [
        {
          session_key: "a".repeat(64),
          source: "codex",
          created_at: "2026-07-27T08:00:00Z",
          last_active_at: "2026-07-27T08:01:00Z",
          availability: "available",
          message_count: 1,
          usage_event_count: 1,
          title: "Synthetic session",
        },
      ],
      next_cursor: null,
      index_status: { phase: "idle", sources: [] },
    });
    vi.mocked(getSessionMessages).mockResolvedValue({
      schema_version: 1,
      items: [
        {
          message_key: "b".repeat(64),
          role: "assistant",
          timestamp: "2026-07-27T08:01:00Z",
          content: "Synthetic response",
          fragment_offset_bytes: 0,
          fragment_final: true,
        },
      ],
      next_cursor: null,
      source_generation: 1,
    });
    vi.mocked(getUsage).mockResolvedValue({
      schema_version: 1,
      group_by: "day",
      totals: {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 2,
        cache_write_tokens: 1,
        reasoning_tokens: 3,
        session_count: 1,
      },
      buckets: [],
      next_cursor: null,
    });
    vi.mocked(getDiagnosticLogs).mockResolvedValue({
      schema_version: 1,
      items: [{ level: "info", message: "synthetic log" }],
      next_cursor: null,
      dropped_events: 0,
    });
    vi.mocked(exportDiagnostics).mockResolvedValue({
      file_name: "wokcore-diagnostics-synthetic.zip",
      bytes: 1024,
    });
  });

  it("shows only capability-backed management areas", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue({
      state: "running",
      capabilities: ["sessions.index.v1"],
    });

    renderPanel();

    expect(
      await screen.findByRole("tab", { name: "Sessions" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "Providers" }),
    ).not.toBeInTheDocument();
    expect(getProviderCatalog).not.toHaveBeenCalled();
    expect(getSessions).toHaveBeenCalledOnce();
  });

  it("loads provider metadata only after WokCore is running", async () => {
    renderPanel();

    expect(
      await screen.findByRole("heading", { name: "OpenAI" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Revision 4")).toBeInTheDocument();
    expect(getProviderCatalog).toHaveBeenCalledOnce();
    expect(getProviderRuntime).toHaveBeenCalledOnce();
  });

  it("loads session bodies only after the user selects a session", async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "Sessions" }));
    expect(await screen.findByText("Synthetic session")).toBeInTheDocument();
    expect(getSessionMessages).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Synthetic session/ }));
    expect(await screen.findByText("Synthetic response")).toBeInTheDocument();
    expect(getSessionMessages).toHaveBeenCalledOnce();
  });

  it("loads usage and diagnostic logs only when their tabs are opened", async () => {
    const user = userEvent.setup();
    renderPanel();

    expect(getUsage).not.toHaveBeenCalled();
    expect(getDiagnosticLogs).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("tab", { name: "Usage" }));
    expect(await screen.findByText("10")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText(/synthetic log/)).toBeInTheDocument();
  });

  it("validates before committing provider changes", async () => {
    vi.mocked(validateProviderConfig).mockResolvedValue({
      schema_version: 1,
      valid: true,
      provider_count: 1,
      models: [],
    });
    vi.mocked(commitProviderConfig).mockResolvedValue({
      schema_version: 1,
      revision: 5,
      snapshot_revision: 5,
      provider_count: 1,
      models: [],
    });
    const user = userEvent.setup();
    renderPanel();

    await user.click(
      await screen.findByRole("checkbox", { name: "Enable primary" }),
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(validateProviderConfig).toHaveBeenCalledOnce();
    expect(commitProviderConfig).toHaveBeenCalledOnce();
    expect(
      vi.mocked(validateProviderConfig).mock.invocationCallOrder[0],
    ).toBeLessThan(
      vi.mocked(commitProviderConfig).mock.invocationCallOrder[0] ?? 0,
    );
    expect(reloadProviders).not.toHaveBeenCalled();
  });
});
