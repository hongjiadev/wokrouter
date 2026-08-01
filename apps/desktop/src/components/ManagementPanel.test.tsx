import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getCoreStatus } from "../control";
import type { CoreOperation } from "../coreOperation";
import { initializeI18n } from "../i18n";
import {
  commitProviderConfig,
  createProviderSecret,
  deleteProviderSecret,
  exportDiagnostics,
  getDiagnosticLogs,
  getProviderCatalog,
  getProviderModels,
  getProviderRuntime,
  getSessionMessages,
  getSessions,
  getUsage,
  reloadProviders,
  replaceProviderSecret,
  validateProviderConfig,
} from "../management";
import { ManagementPanel } from "./ManagementPanel";
import { CoreOperationPanel } from "./CoreOperationPanel";

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
  return { ...render(<ManagementPanel />, { wrapper: Wrapper }), queryClient };
}

const recoveryRequiredOperation: CoreOperation = {
  schemaVersion: 1,
  operationId: "44444444-4444-4444-8444-444444444444",
  sequence: 4,
  operation: "update",
  state: "failed",
  phase: "completed",
  currentVersion: "0.2.0",
  targetVersion: "0.2.1",
  errorCode: "recovery_required",
};

function RecoveryDiagnosticsHarness({
  diagnosticsAvailable,
}: {
  diagnosticsAvailable: boolean;
}) {
  const [requestedArea, setRequestedArea] =
    useState<"diagnostics" | undefined>();
  const [requestedAreaRequestId, setRequestedAreaRequestId] = useState(0);
  return (
    <>
      <CoreOperationPanel
        operation={recoveryRequiredOperation}
        onRetry={() => undefined}
        diagnosticsAvailable={diagnosticsAvailable}
        onOpenDiagnostics={() => {
          setRequestedArea("diagnostics");
          setRequestedAreaRequestId((requestId) => requestId + 1);
        }}
      />
      <ManagementPanel
        requestedArea={requestedArea}
        requestedAreaRequestId={requestedAreaRequestId}
      />
    </>
  );
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

function mockProviderRuntimeWithAccount() {
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
      accounts: [
        {
          id: "work",
          provider: "primary",
          enabled: true,
          auth: { kind: "api_key", secret: "secret-ref" },
        },
      ],
    },
    routing: { aliases: [], rules: [], default: null },
  });
}

describe("ManagementPanel", () => {
  beforeEach(async () => {
    await initializeI18n("en");
    vi.clearAllMocks();
    vi.mocked(getCoreStatus).mockResolvedValue({
      state: "running",
      runtime_channel: "production",
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
      runtime_channel: "production",
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
    expect(getSessions).toHaveBeenCalledWith({ before: undefined, limit: 50 });
    expect(getSessionMessages).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Synthetic session/ }));
    expect(await screen.findByText("Synthetic response")).toBeInTheDocument();
    expect(getSessionMessages).toHaveBeenCalledWith("a".repeat(64), {
      after: undefined,
      limit: 100,
      max_bytes: 262_144,
    });
  });

  it("loads usage and diagnostic logs only when their tabs are opened", async () => {
    const user = userEvent.setup();
    renderPanel();

    expect(getUsage).not.toHaveBeenCalled();
    expect(getDiagnosticLogs).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("tab", { name: "Usage" }));
    expect(await screen.findByText("10")).toBeInTheDocument();
    expect(getUsage).toHaveBeenCalledWith({ group_by: "day", limit: 100 });
    await user.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(await screen.findByText(/synthetic log/)).toBeInTheDocument();
    expect(getDiagnosticLogs).toHaveBeenCalledWith({
      after: undefined,
      order: "desc",
      limit: 100,
    });
  });

  it("opens, selects, and focuses real Diagnostics content from update recovery", async () => {
    const user = userEvent.setup();
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    );

    render(
      <RecoveryDiagnosticsHarness diagnosticsAvailable />,
      { wrapper: Wrapper },
    );

    await user.click(
      await screen.findByRole("button", {
        name: "Open diagnostics",
      }),
    );

    const diagnosticsTab = screen.getByRole("tab", {
      name: "Diagnostics",
    });
    expect(diagnosticsTab).toHaveAttribute("aria-selected", "true");
    expect(diagnosticsTab).toHaveFocus();
    expect(
      await screen.findByRole("tabpanel", {
        name: "Diagnostics",
      }),
    ).toHaveTextContent("synthetic log");

    await user.click(screen.getByRole("tab", { name: "Providers" }));
    expect(diagnosticsTab).toHaveAttribute("aria-selected", "false");

    await user.click(
      screen.getByRole("button", { name: "Open diagnostics" }),
    );
    expect(diagnosticsTab).toHaveAttribute("aria-selected", "true");
    expect(diagnosticsTab).toHaveFocus();
  });

  it("renders an accurate recovery alternative when Diagnostics capability is absent", async () => {
    vi.mocked(getCoreStatus).mockResolvedValue({
      state: "running",
      runtime_channel: "production",
      version: "0.2.0",
      management_api_major: 1,
      capabilities: ["provider.catalog.v1"],
      phase: "running",
    });

    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    );
    render(
      <RecoveryDiagnosticsHarness diagnosticsAvailable={false} />,
      { wrapper: Wrapper },
    );

    expect(
      await screen.findByText(
        "Diagnostics are unavailable because this WokCore runtime does not provide diagnostic events.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Open diagnostics" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "Open diagnostics" }),
    ).not.toBeInTheDocument();
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
    const { queryClient } = renderPanel();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

    await user.click(
      await screen.findByRole("checkbox", { name: "Enable primary" }),
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(validateProviderConfig).toHaveBeenCalledOnce();
    expect(commitProviderConfig).toHaveBeenCalledOnce();
    expect(validateProviderConfig).toHaveBeenCalledWith({
      providers: {
        instances: [
          {
            id: "primary",
            catalog_id: "openai",
            enabled: false,
            endpoint: null,
            allow_private_network: false,
          },
        ],
        accounts: [],
      },
      routing: { aliases: [], rules: [], default: null },
    });
    expect(commitProviderConfig).toHaveBeenCalledWith({
      expected_revision: 4,
      providers: {
        instances: [
          {
            id: "primary",
            catalog_id: "openai",
            enabled: false,
            endpoint: null,
            allow_private_network: false,
          },
        ],
        accounts: [],
      },
      routing: { aliases: [], rules: [], default: null },
    });
    expect(
      vi.mocked(validateProviderConfig).mock.invocationCallOrder[0],
    ).toBeLessThan(
      vi.mocked(commitProviderConfig).mock.invocationCallOrder[0] ?? 0,
    );
    expect(reloadProviders).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["provider-runtime"],
      });
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["provider-models"],
      });
    });
  });

  it("sends the exact secret create payload from the provider account form", async () => {
    mockProviderRuntimeWithAccount();
    vi.mocked(createProviderSecret).mockResolvedValue({
      schema_version: 1,
      operation: "created",
      secret_ref: "new-secret-ref",
    });
    const user = userEvent.setup();

    renderPanel();

    await user.type(
      await screen.findByRole("textbox", { name: "Account ID" }),
      "personal",
    );
    await user.type(screen.getByLabelText("Secret value"), "new-secret");
    await user.click(screen.getByRole("button", { name: "Store account" }));

    await waitFor(() => {
      expect(createProviderSecret).toHaveBeenCalledWith({
        provider_id: "primary",
        account_id: "personal",
        purpose: "api_key",
        secret: "new-secret",
      });
    });
  });

  it("replaces a provider secret with the existing secret reference", async () => {
    mockProviderRuntimeWithAccount();
    vi.mocked(replaceProviderSecret).mockResolvedValue({
      schema_version: 1,
      operation: "replaced",
      secret_ref: "secret-ref",
    });
    const user = userEvent.setup();

    renderPanel();

    const replacement = await screen.findByPlaceholderText(
      "Enter a replacement secret",
    );
    await user.type(replacement, "next-secret");
    await user.click(screen.getByRole("button", { name: "Replace" }));

    await waitFor(() => {
      expect(replaceProviderSecret).toHaveBeenCalledWith(
        "secret-ref",
        "next-secret",
      );
    });
    expect(replacement).toHaveValue("");
  });

  it("deletes removed provider secrets only after the config commit", async () => {
    mockProviderRuntimeWithAccount();
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
    vi.mocked(deleteProviderSecret).mockResolvedValue({
      schema_version: 1,
      operation: "deleted",
      secret_ref: "secret-ref",
    });
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("button", { name: "Remove" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => {
      expect(commitProviderConfig).toHaveBeenCalledWith({
        expected_revision: 4,
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
      expect(deleteProviderSecret).toHaveBeenCalledWith("secret-ref");
    });
    expect(
      vi.mocked(commitProviderConfig).mock.invocationCallOrder[0],
    ).toBeLessThan(
      vi.mocked(deleteProviderSecret).mock.invocationCallOrder[0] ?? 0,
    );
  });

  it("preserves the English management workspace and area labels", async () => {
    renderPanel();

    expect(
      await screen.findByRole("heading", { name: "WokCore workspace" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Management")).toBeInTheDocument();
    expect(screen.getByText("Live · loopback only")).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Providers" }),
    ).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Sessions" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Usage" })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Diagnostics" }),
    ).toBeInTheDocument();
  });

  it("renders the Chinese sessions workspace and empty state", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getSessions).mockResolvedValue({
      schema_version: 1,
      items: [],
      next_cursor: null,
      index_status: { phase: "idle", sources: [] },
    });
    const user = userEvent.setup();

    renderPanel();

    expect(
      await screen.findByRole("heading", { name: "WokCore 工作区" }),
    ).toBeInTheDocument();
    expect(screen.getByText("管理")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "会话" }));
    expect(screen.getByText("未找到会话")).toBeInTheDocument();
    expect(
      screen.getByText(
        "当索引可用时，WokCore 会显示本地 Codex、Claude 和 Gemini 会话。",
      ),
    ).toBeInTheDocument();
  });

  it.each([
    ["en", "starting", "Starting session index"],
    ["en", "scanning", "Scanning session indexes"],
    ["en", "idle", "Session indexes are up to date"],
    ["zh-CN", "starting", "正在启动会话索引"],
    ["zh-CN", "scanning", "正在扫描会话索引"],
    ["zh-CN", "idle", "会话索引已就绪"],
  ] as const)(
    "translates the %s session index phase as %s",
    async (locale, phase, expected) => {
      await initializeI18n(locale);
      vi.mocked(getSessions).mockResolvedValue({
        schema_version: 1,
        items: [],
        next_cursor: null,
        index_status: { phase, sources: [] },
      });
      const user = userEvent.setup();

      renderPanel();

      await user.click(
        await screen.findByRole("tab", {
          name: locale === "en" ? "Sessions" : "会话",
        }),
      );
      expect(await screen.findByText(expected)).toBeInTheDocument();
      expect(screen.queryByText(phase)).not.toBeInTheDocument();
    },
  );

  it("formats day buckets with the selected catalog locale", async () => {
    await initializeI18n("zh-CN");
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
      buckets: [
        {
          key: "2026-07-27",
          start: "2026-07-27T00:00:00Z",
          end: "2026-07-28T00:00:00Z",
          input_tokens: 10,
          output_tokens: 5,
          cache_read_tokens: 2,
          cache_write_tokens: 1,
          reasoning_tokens: 3,
          session_count: 1,
        },
      ],
      next_cursor: null,
    });
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "用量" }));
    expect(await screen.findByText("2026年7月27日")).toBeInTheDocument();
    expect(screen.queryByText("2026-07-27")).not.toBeInTheDocument();
  });

  it("keeps source bucket keys literal when the grouping changes", async () => {
    vi.mocked(getUsage).mockImplementation(async ({ group_by }) => ({
      schema_version: 1,
      group_by: group_by ?? "day",
      totals: {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 2,
        cache_write_tokens: 1,
        reasoning_tokens: 3,
        session_count: 1,
      },
      buckets: [
        {
          key: "codex",
          start: "2026-07-27T00:00:00Z",
          end: "2026-07-28T00:00:00Z",
          input_tokens: 10,
          output_tokens: 5,
          cache_read_tokens: 2,
          cache_write_tokens: 1,
          reasoning_tokens: 3,
          session_count: 1,
        },
      ],
      next_cursor: null,
    }));
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "Usage" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Group by" }), "source");
    expect(await screen.findByText("codex")).toBeInTheDocument();
    expect(getUsage).toHaveBeenLastCalledWith({ group_by: "source", limit: 100 });
  });

  it("shows a safe localized provider failure without leaking a bridge error", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getProviderCatalog).mockRejectedValue(
      new Error("bridge failure: C:\\private\\secret.txt"),
    );

    renderPanel();

    expect(await screen.findByText("供应商数据不可用")).toBeInTheDocument();
    expect(
      screen.getByText(
        "WokRouter 未假定任何本地状态发生变化。请检查 WokCore 后重试。",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/bridge failure/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/secret\.txt/i)).not.toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => {
      expect(getProviderCatalog).toHaveBeenCalledTimes(2);
      expect(getProviderRuntime).toHaveBeenCalledTimes(2);
      expect(getProviderModels).toHaveBeenCalledTimes(2);
    });
  });

  it("shows localized secret field labels and placeholders for provider account changes", async () => {
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
        accounts: [
          {
            id: "work",
            provider: "primary",
            enabled: true,
            auth: { kind: "api_key", secret: "secret-ref" },
          },
        ],
      },
      routing: { aliases: [], rules: [], default: null },
    });

    renderPanel();

    expect(
      await screen.findByPlaceholderText("Enter a secret"),
    ).toHaveAccessibleName("Secret value");
    expect(
      screen.getByPlaceholderText("Enter a replacement secret"),
    ).toHaveAccessibleName("Replacement secret for work");
  });

  it("shows a localized usage error state", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getUsage).mockRejectedValue(new Error("usage bridge failure"));
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "用量" }));
    expect(await screen.findByText("用量不可用")).toBeInTheDocument();
    expect(screen.queryByText(/usage bridge failure/i)).not.toBeInTheDocument();
  });

  it("shows a localized empty diagnostics state", async () => {
    await initializeI18n("zh-CN");
    vi.mocked(getDiagnosticLogs).mockResolvedValue({
      schema_version: 1,
      items: [],
      next_cursor: null,
      dropped_events: 0,
    });
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "诊断" }));
    expect(await screen.findByText("没有诊断事件")).toBeInTheDocument();
    expect(
      screen.getByText("可用时，最近的有界诊断事件会显示在这里。"),
    ).toBeInTheDocument();
  });

  it("exports diagnostics with localized live status while preserving archive data", async () => {
    await initializeI18n("zh-CN");
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "诊断" }));
    await user.click(screen.getByRole("button", { name: "导出诊断信息" }));
    expect(exportDiagnostics).toHaveBeenCalledWith({
      include_snapshots: false,
      max_bytes: 16 * 1024 * 1024,
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "已保存 wokcore-diagnostics-synthetic.zip（1.0 KiB）。",
    );
  });

  it("updates mounted provider labels and live status without clearing the provider form", async () => {
    const user = userEvent.setup();

    renderPanel();

    const instanceId = await screen.findByRole("textbox", {
      name: "Instance ID",
    });
    await user.type(instanceId, "secondary");
    await initializeI18n("zh-CN");

    expect(
      await screen.findByRole("tab", { name: "供应商", selected: true }),
    ).toBeInTheDocument();
    expect(screen.getByText("实时 · 仅限环回")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "实例 ID" })).toHaveValue(
      "secondary",
    );
  });

  it("updates mounted session labels without clearing the selected session", async () => {
    const user = userEvent.setup();

    renderPanel();

    await user.click(await screen.findByRole("tab", { name: "Sessions" }));
    await user.click(screen.getByRole("button", { name: /Synthetic session/ }));
    expect(await screen.findByText("Synthetic response")).toBeInTheDocument();

    await initializeI18n("zh-CN");

    expect(
      await screen.findByRole("tab", { name: "会话", selected: true }),
    ).toBeInTheDocument();
    expect(screen.getByText("Synthetic response")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Synthetic session" }))
      .toBeInTheDocument();
  });
});
