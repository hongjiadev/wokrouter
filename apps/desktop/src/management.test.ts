import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  commitProviderConfig,
  exportDiagnostics,
  getDiagnosticLogs,
  getProviderCatalog,
  getProviderRuntime,
  getSessionMessages,
  getSessions,
  getUsage,
  reloadProviders,
  validateProviderConfig,
  type ProviderCandidate,
} from "./management";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const emptyCandidate: ProviderCandidate = {
  providers: { instances: [], accounts: [] },
  routing: { aliases: [], rules: [], default: null },
};

describe("WokCore management bridge", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("accepts same-major provider fields without exposing unvalidated data", async () => {
    vi.mocked(invoke).mockResolvedValue({
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
          future_provider_field: true,
        },
      ],
      future_catalog_field: { enabled: true },
    });

    const catalog = await getProviderCatalog();

    expect(catalog.providers[0]?.label).toBe("OpenAI");
    expect(catalog).not.toHaveProperty("future_catalog_field");
    expect(invoke).toHaveBeenCalledWith("provider_catalog");
  });

  it("rejects malformed management responses", async () => {
    vi.mocked(invoke).mockResolvedValue({
      schema_version: 1,
      revision: -1,
      providers: "not-a-config",
    });

    await expect(getProviderRuntime()).rejects.toThrow(
      "Invalid WokCore provider runtime",
    );
  });

  it("uses narrow commands for revisioned provider updates", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({
        schema_version: 1,
        valid: true,
        provider_count: 0,
        models: [],
      })
      .mockResolvedValueOnce({
        schema_version: 1,
        revision: 2,
        snapshot_revision: 2,
        provider_count: 0,
        models: [],
      })
      .mockResolvedValueOnce({
        schema_version: 1,
        revision: 2,
        snapshot_revision: 2,
        provider_count: 0,
        models: [],
      });

    await validateProviderConfig(emptyCandidate);
    await commitProviderConfig({
      expected_revision: 1,
      ...emptyCandidate,
    });
    await reloadProviders();

    expect(invoke).toHaveBeenNthCalledWith(1, "validate_provider_config", {
      candidate: emptyCandidate,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "commit_provider_config", {
      request: { expected_revision: 1, ...emptyCandidate },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "reload_providers");
  });

  it("pages sessions, messages, usage, and logs on demand", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({
        schema_version: 1,
        items: [],
        next_cursor: null,
        index_status: { phase: "idle", sources: [] },
      })
      .mockResolvedValueOnce({
        schema_version: 1,
        items: [],
        next_cursor: null,
        source_generation: 1,
      })
      .mockResolvedValueOnce({
        schema_version: 1,
        group_by: "day",
        totals: {
          input_tokens: 0,
          output_tokens: 0,
          cache_read_tokens: 0,
          cache_write_tokens: 0,
          reasoning_tokens: 0,
          session_count: 0,
        },
        buckets: [],
        next_cursor: null,
      })
      .mockResolvedValueOnce({
        schema_version: 1,
        items: [],
        next_cursor: null,
        dropped_events: 0,
      });

    await getSessions({ limit: 25 });
    await getSessionMessages("a".repeat(64), { limit: 50 });
    await getUsage({ group_by: "day", limit: 20 });
    await getDiagnosticLogs({ order: "desc", limit: 10 });

    expect(invoke).toHaveBeenNthCalledWith(1, "list_sessions", {
      query: { limit: 25 },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "session_messages", {
      sessionKey: "a".repeat(64),
      query: { limit: 50 },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "usage", {
      query: { group_by: "day", limit: 20 },
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "diagnostic_logs", {
      query: { order: "desc", limit: 10 },
    });
  });

  it("returns only a diagnostic export receipt", async () => {
    vi.mocked(invoke).mockResolvedValue({
      file_name: "wokcore-diagnostics-synthetic.zip",
      bytes: 1024,
    });

    await expect(
      exportDiagnostics({ include_snapshots: false, max_bytes: 65_536 }),
    ).resolves.toEqual({
      file_name: "wokcore-diagnostics-synthetic.zip",
      bytes: 1024,
    });
    expect(invoke).toHaveBeenCalledWith("export_diagnostics", {
      query: { include_snapshots: false, max_bytes: 65_536 },
    });
  });
});
