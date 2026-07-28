import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const identifierSchema = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[a-z0-9._-]*[a-z0-9]$/);
const secretRefSchema = z.string().min(1).max(96).startsWith("secret:");
const utcTimestampSchema = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
const opaqueKeySchema = z.string().regex(/^[a-f0-9]{64}$/);
const providerCapabilitiesSchema = z.object({
  text: z.boolean(),
  streaming: z.boolean(),
  tools: z.boolean(),
  vision: z.boolean(),
  images: z.boolean(),
  reasoning: z.boolean(),
});
const providerDefinitionSchema = z.object({
  id: identifierSchema,
  label: z.string().min(1).max(256),
  adapter: z.enum([
    "open_ai_responses",
    "open_ai_chat",
    "anthropic",
    "google",
    "azure_open_ai",
    "cursor",
    "kiro",
    "mimo_free",
  ]),
  base_url: z.string().min(1).max(2048),
  auth_kind: z.enum(["forward", "oauth", "key", "local"]),
  endpoint_policy: z.enum([
    "public_https",
    "https_template",
    "loopback_http",
  ]),
  model_source: z.enum(["none", "static", "live", "hybrid"]),
  aliases: z.array(identifierSchema).max(32),
  models: z.array(z.string().min(1).max(256)).max(512),
  default_model: z.string().max(256).nullable(),
  allow_endpoint_override: z.boolean(),
  key_optional: z.boolean(),
  allow_key_auth_override: z.boolean(),
  reasoning_efforts: z.array(z.string().max(32)).max(16),
  reasoning_effort_map: z.record(z.string(), z.string().max(32)),
  capabilities: providerCapabilitiesSchema,
});
const publicModelSchema = z.object({
  id: z.string().min(1).max(256),
  owned_by: identifierSchema,
  capabilities: providerCapabilitiesSchema,
});
const providerAccountAuthSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("forward"), credential: secretRefSchema }),
  z.object({
    kind: z.literal("oauth"),
    access: secretRefSchema,
    refresh: secretRefSchema.nullable().optional().default(null),
  }),
  z.object({ kind: z.literal("api_key"), secret: secretRefSchema }),
  z.object({ kind: z.literal("local") }),
]);
const providerConfigSchema = z.object({
  instances: z
    .array(
      z.object({
        id: identifierSchema,
        catalog_id: identifierSchema,
        enabled: z.boolean(),
        endpoint: z.string().max(2048).nullable(),
        allow_private_network: z.boolean(),
      }),
    )
    .max(64),
  accounts: z
    .array(
      z.object({
        id: identifierSchema,
        provider: identifierSchema,
        enabled: z.boolean(),
        auth: providerAccountAuthSchema,
      }),
    )
    .max(256),
});
const routeTargetSchema = z.object({
  provider: identifierSchema,
  model: z.string().min(1).max(256),
});
const routingConfigSchema = z.object({
  aliases: z
    .array(
      z.object({
        alias: z.string().min(1).max(256),
        target: routeTargetSchema,
      }),
    )
    .max(1024),
  rules: z
    .array(
      z.object({
        client_id: identifierSchema.nullable().optional().default(null),
        model: z.string().max(256).nullable().optional().default(null),
        target: routeTargetSchema,
      }),
    )
    .max(1024),
  default: routeTargetSchema.nullable().optional().default(null),
});
const providerCandidateSchema = z.object({
  providers: providerConfigSchema,
  routing: routingConfigSchema,
});
const providerCatalogSchema = z.object({
  schema_version: z.literal(1),
  catalog_schema_version: z.literal(1),
  baseline_commit: z.string().min(1).max(64),
  providers: z.array(providerDefinitionSchema).min(1).max(256),
});
const providerRuntimeSchema = z.object({
  schema_version: z.literal(1),
  revision: z.number().int().nonnegative(),
  snapshot_revision: z.number().int().nonnegative(),
  reload_status: z.enum(["ready", "failed"]),
  provider_count: z.number().int().nonnegative().max(64),
  models: z.array(publicModelSchema),
  providers: providerConfigSchema,
  routing: routingConfigSchema,
});
const providerModelsSchema = z.object({
  schema_version: z.literal(1),
  models: z.array(publicModelSchema),
});
const providerValidationSchema = z.object({
  schema_version: z.literal(1),
  valid: z.literal(true),
  provider_count: z.number().int().nonnegative().max(64),
  models: z.array(publicModelSchema),
});
const providerCommitSchema = z.object({
  schema_version: z.literal(1),
  revision: z.number().int().nonnegative(),
  snapshot_revision: z.number().int().nonnegative(),
  provider_count: z.number().int().nonnegative().max(64),
  models: z.array(publicModelSchema),
});
const providerSecretResponseSchema = z.object({
  schema_version: z.literal(1),
  operation: z.enum(["created", "replaced", "deleted"]),
  secret_ref: secretRefSchema,
});

const sessionSourceSchema = z.enum(["codex", "claude", "gemini"]);
const sessionListSchema = z.object({
  schema_version: z.literal(1),
  items: z
    .array(
      z.object({
        session_key: opaqueKeySchema,
        source: sessionSourceSchema,
        created_at: utcTimestampSchema,
        last_active_at: utcTimestampSchema,
        availability: z.enum(["available", "unavailable"]),
        message_count: z.number().int().nonnegative(),
        usage_event_count: z.number().int().nonnegative(),
        title: z.string().optional(),
      }),
    )
    .max(200),
  next_cursor: z.string().nullable(),
  index_status: z.object({
    phase: z.enum(["starting", "scanning", "idle"]),
    sources: z
      .array(
        z.object({
          source: sessionSourceSchema,
          status: z.enum([
            "undiscovered",
            "available",
            "stale",
            "unavailable",
            "resource_limited",
          ]),
          last_transition_at: utcTimestampSchema.optional(),
          error_code: z.string().max(128).optional(),
        }),
      )
      .max(3),
  }),
});
const sessionMessagesSchema = z.object({
  schema_version: z.literal(1),
  items: z
    .array(
      z.object({
        message_key: opaqueKeySchema,
        role: z.enum(["user", "assistant", "system", "tool"]),
        timestamp: utcTimestampSchema,
        content: z.string(),
        fragment_offset_bytes: z.number().int().nonnegative(),
        fragment_final: z.boolean(),
      }),
    )
    .max(500),
  next_cursor: z.string().nullable(),
  source_generation: z.number().int().positive(),
});
const usageTotalsSchema = z.object({
  input_tokens: z.number().int().nonnegative(),
  output_tokens: z.number().int().nonnegative(),
  cache_read_tokens: z.number().int().nonnegative(),
  cache_write_tokens: z.number().int().nonnegative(),
  reasoning_tokens: z.number().int().nonnegative(),
  session_count: z.number().int().nonnegative(),
});
const usageSchema = z.object({
  schema_version: z.literal(1),
  group_by: z.enum(["day", "source", "model"]),
  totals: usageTotalsSchema,
  buckets: z
    .array(
      usageTotalsSchema.extend({
        key: z.string().min(1),
        start: utcTimestampSchema.optional(),
        end: utcTimestampSchema.optional(),
      }),
    )
    .max(500),
  next_cursor: z.string().nullable(),
});
const diagnosticLogsSchema = z.object({
  schema_version: z.literal(1),
  items: z.array(z.record(z.string(), z.unknown())).max(1000),
  next_cursor: z.string().nullable(),
  dropped_events: z.number().int().nonnegative(),
});
const diagnosticExportReceiptSchema = z.object({
  file_name: z
    .string()
    .regex(/^wokcore-diagnostics-[a-z0-9-]+\.zip$/)
    .max(96),
  bytes: z.number().int().nonnegative().max(67_108_864),
});

export type ProviderCapabilities = z.infer<
  typeof providerCapabilitiesSchema
>;
export type ProviderDefinition = z.infer<typeof providerDefinitionSchema>;
export type ProviderConfig = z.infer<typeof providerConfigSchema>;
export type RoutingConfig = z.infer<typeof routingConfigSchema>;
export type ProviderCandidate = z.infer<typeof providerCandidateSchema>;
export type ProviderCatalog = z.infer<typeof providerCatalogSchema>;
export type ProviderRuntime = z.infer<typeof providerRuntimeSchema>;
export type ProviderModels = z.infer<typeof providerModelsSchema>;
export type ProviderValidation = z.infer<typeof providerValidationSchema>;
export type ProviderCommit = z.infer<typeof providerCommitSchema>;
export type ProviderSecretResponse = z.infer<
  typeof providerSecretResponseSchema
>;
export type SessionList = z.infer<typeof sessionListSchema>;
export type SessionMessages = z.infer<typeof sessionMessagesSchema>;
export type UsageResponse = z.infer<typeof usageSchema>;
export type DiagnosticLogs = z.infer<typeof diagnosticLogsSchema>;
export type DiagnosticExportReceipt = z.infer<
  typeof diagnosticExportReceiptSchema
>;

export type ProviderCommitRequest = ProviderCandidate & {
  expected_revision: number;
};
export type ProviderSecretPurpose =
  | "api_key"
  | "oauth_access"
  | "oauth_refresh"
  | "lan_token"
  | "auxiliary";
export type ProviderSecretCreateRequest = {
  provider_id: string;
  account_id: string | null;
  purpose: ProviderSecretPurpose;
  secret: string;
};
export type SessionQuery = {
  source?: "codex" | "claude" | "gemini";
  availability?: "available" | "unavailable";
  before?: string;
  limit?: number;
};
export type SessionMessageQuery = {
  after?: string;
  limit?: number;
  max_bytes?: number;
};
export type UsageQuery = {
  source?: "codex" | "claude" | "gemini";
  session_key?: string;
  since?: string;
  until?: string;
  group_by?: "day" | "source" | "model";
  after?: string;
  limit?: number;
};
export type DiagnosticLogQuery = {
  request_id?: string;
  trace_id?: string;
  session_key?: string;
  level_min?: "debug" | "info" | "warn" | "error";
  component?: string;
  since?: string;
  until?: string;
  order?: "asc" | "desc";
  after?: string;
  limit?: number;
};
export type DiagnosticExportQuery = {
  request_id?: string;
  trace_id?: string;
  session_key?: string;
  since?: string;
  until?: string;
  include_snapshots?: boolean;
  max_bytes?: number;
};

async function invokeParsed<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  schema: z.ZodType<T>,
  label: string,
): Promise<T> {
  const value =
    args === undefined
      ? await invoke<unknown>(command)
      : await invoke<unknown>(command, args);
  const parsed = schema.safeParse(value);
  if (!parsed.success) {
    throw new Error(`Invalid WokCore ${label} returned by desktop bridge.`, {
      cause: parsed.error,
    });
  }
  return parsed.data;
}

export function getProviderCatalog(): Promise<ProviderCatalog> {
  return invokeParsed(
    "provider_catalog",
    undefined,
    providerCatalogSchema,
    "provider catalog",
  );
}

export function getProviderRuntime(): Promise<ProviderRuntime> {
  return invokeParsed(
    "provider_runtime",
    undefined,
    providerRuntimeSchema,
    "provider runtime",
  );
}

export function getProviderModels(): Promise<ProviderModels> {
  return invokeParsed(
    "provider_models",
    undefined,
    providerModelsSchema,
    "provider models",
  );
}

export function validateProviderConfig(
  candidate: ProviderCandidate,
): Promise<ProviderValidation> {
  return invokeParsed(
    "validate_provider_config",
    { candidate },
    providerValidationSchema,
    "provider validation",
  );
}

export function commitProviderConfig(
  request: ProviderCommitRequest,
): Promise<ProviderCommit> {
  return invokeParsed(
    "commit_provider_config",
    { request },
    providerCommitSchema,
    "provider commit",
  );
}

export function reloadProviders(): Promise<ProviderCommit> {
  return invokeParsed(
    "reload_providers",
    undefined,
    providerCommitSchema,
    "provider reload",
  );
}

export function createProviderSecret(
  request: ProviderSecretCreateRequest,
): Promise<ProviderSecretResponse> {
  return invokeParsed(
    "create_provider_secret",
    { request },
    providerSecretResponseSchema,
    "provider secret response",
  );
}

export function replaceProviderSecret(
  secretRef: string,
  secret: string,
): Promise<ProviderSecretResponse> {
  return invokeParsed(
    "replace_provider_secret",
    { request: { secret_ref: secretRef, secret } },
    providerSecretResponseSchema,
    "provider secret response",
  );
}

export function deleteProviderSecret(
  secretRef: string,
): Promise<ProviderSecretResponse> {
  return invokeParsed(
    "delete_provider_secret",
    { secretRef },
    providerSecretResponseSchema,
    "provider secret response",
  );
}

export function getSessions(query: SessionQuery): Promise<SessionList> {
  return invokeParsed(
    "list_sessions",
    { query },
    sessionListSchema,
    "session list",
  );
}

export function getSessionMessages(
  sessionKey: string,
  query: SessionMessageQuery,
): Promise<SessionMessages> {
  return invokeParsed(
    "session_messages",
    { sessionKey, query },
    sessionMessagesSchema,
    "session messages",
  );
}

export function getUsage(query: UsageQuery): Promise<UsageResponse> {
  return invokeParsed("usage", { query }, usageSchema, "usage response");
}

export function getDiagnosticLogs(
  query: DiagnosticLogQuery,
): Promise<DiagnosticLogs> {
  return invokeParsed(
    "diagnostic_logs",
    { query },
    diagnosticLogsSchema,
    "diagnostic logs",
  );
}

export function exportDiagnostics(
  query: DiagnosticExportQuery,
): Promise<DiagnosticExportReceipt> {
  return invokeParsed(
    "export_diagnostics",
    { query },
    diagnosticExportReceiptSchema,
    "diagnostic export receipt",
  );
}
