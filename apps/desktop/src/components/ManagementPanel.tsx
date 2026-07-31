import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { coreStatusQueryKey, getCoreStatus } from "../control";
import type { SupportedLocale } from "../i18n";
import { formatBytes, formatLocalTime, formatNumber } from "../i18n/format";
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
  type ProviderCandidate,
  type ProviderConfig,
  type ProviderDefinition,
  type ProviderSecretPurpose,
  type SessionList,
  type UsageQuery,
} from "../management";

export type ManagementArea =
  | "providers"
  | "sessions"
  | "usage"
  | "diagnostics";

type ManagementAreaTranslationKey = `management.${ManagementArea}.tab`;

const areaDefinitions: {
  id: ManagementArea;
  labelKey: ManagementAreaTranslationKey;
  capability: string;
}[] = [
  {
    id: "providers",
    labelKey: "management.providers.tab",
    capability: "provider.catalog.v1",
  },
  {
    id: "sessions",
    labelKey: "management.sessions.tab",
    capability: "sessions.index.v1",
  },
  {
    id: "usage",
    labelKey: "management.usage.tab",
    capability: "usage.session.v1",
  },
  {
    id: "diagnostics",
    labelKey: "management.diagnostics.tab",
    capability: "diagnostics.events.v1",
  },
];

export function ManagementPanel({
  requestedArea,
  requestedAreaRequestId,
}: {
  requestedArea?: ManagementArea;
  requestedAreaRequestId?: number;
}) {
  const { t } = useTranslation();
  const status = useQuery({
    queryKey: coreStatusQueryKey,
    queryFn: getCoreStatus,
  });
  const areas = useMemo(() => {
    if (status.data?.state !== "running") {
      return [];
    }
    const capabilities = new Set(status.data.capabilities);
    return areaDefinitions.filter((area) =>
      capabilities.has(area.capability),
    );
  }, [status.data]);
  const [activeArea, setActiveArea] =
    useState<ManagementArea>("providers");
  const tabRefs = useRef<
    Partial<Record<ManagementArea, HTMLButtonElement>>
  >({});

  useEffect(() => {
    if (
      areas.length > 0 &&
      !areas.some((area) => area.id === activeArea)
    ) {
      setActiveArea(areas[0]!.id);
    }
  }, [activeArea, areas]);

  useEffect(() => {
    if (
      requestedArea !== undefined &&
      areas.some((area) => area.id === requestedArea)
    ) {
      setActiveArea(requestedArea);
      tabRefs.current[requestedArea]?.focus({ preventScroll: true });
    }
  }, [areas, requestedArea, requestedAreaRequestId]);

  if (status.data?.state !== "running" || areas.length === 0) {
    return null;
  }

  const capabilities = new Set(status.data.capabilities);
  const active = areas.some((area) => area.id === activeArea)
    ? activeArea
    : areas[0]!.id;

  return (
    <section className="management-panel" aria-labelledby="management-heading">
      <header className="management-header">
        <div>
          <p className="section-label">
            {t("management.providers.panelLabel")}
          </p>
          <h2 id="management-heading">
            {t("management.providers.panelHeading")}
          </h2>
        </div>
        <span className="management-connection">
          {t("management.providers.connection")}
        </span>
      </header>
      <div
        className="management-tabs"
        role="tablist"
        aria-label={t("management.providers.areasLabel")}
      >
        {areas.map((area) => (
          <button
            className="management-tab"
            type="button"
            role="tab"
            aria-selected={active === area.id}
            aria-controls={`management-${area.id}`}
            id={`management-tab-${area.id}`}
            key={area.id}
            ref={(element) => {
              tabRefs.current[area.id] = element ?? undefined;
            }}
            onClick={() => setActiveArea(area.id)}
          >
            {t(area.labelKey)}
          </button>
        ))}
      </div>
      <div
        className="management-content"
        role="tabpanel"
        id={`management-${active}`}
        aria-labelledby={`management-tab-${active}`}
      >
        {active === "providers" && (
          <ProviderPanel
            canWrite={capabilities.has("provider.config.v1")}
            canManageSecrets={capabilities.has("provider.secrets.v1")}
          />
        )}
        {active === "sessions" && (
          <SessionsPanel
            canReadMessages={capabilities.has("sessions.messages.v1")}
          />
        )}
        {active === "usage" && <UsagePanel />}
        {active === "diagnostics" && (
          <DiagnosticsPanel
            canExport={capabilities.has("diagnostics.export.v1")}
          />
        )}
      </div>
    </section>
  );
}

function ProviderPanel({
  canWrite,
  canManageSecrets,
}: {
  canWrite: boolean;
  canManageSecrets: boolean;
}) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage as SupportedLocale;
  const queryClient = useQueryClient();
  const catalog = useQuery({
    queryKey: ["provider-catalog"],
    queryFn: getProviderCatalog,
  });
  const runtime = useQuery({
    queryKey: ["provider-runtime"],
    queryFn: getProviderRuntime,
  });
  const models = useQuery({
    queryKey: ["provider-models"],
    queryFn: getProviderModels,
  });
  const [draft, setDraft] = useState<ProviderCandidate | null>(null);
  const [pendingSecretDeletes, setPendingSecretDeletes] = useState<string[]>(
    [],
  );

  useEffect(() => {
    if (runtime.data) {
      setDraft(cloneCandidate(runtime.data.providers, runtime.data.routing));
      setPendingSecretDeletes([]);
    }
  }, [runtime.data]);

  const save = useMutation({
    mutationFn: async () => {
      if (!draft || !runtime.data) {
        throw new Error("Provider configuration is not ready.");
      }
      await validateProviderConfig(draft);
      const result = await commitProviderConfig({
        expected_revision: runtime.data.revision,
        ...draft,
      });
      await Promise.allSettled(
        pendingSecretDeletes.map((secretRef) =>
          deleteProviderSecret(secretRef),
        ),
      );
      return result;
    },
    onSuccess: async () => {
      setPendingSecretDeletes([]);
      await queryClient.invalidateQueries({ queryKey: ["provider-runtime"] });
      await queryClient.invalidateQueries({ queryKey: ["provider-models"] });
    },
  });
  const reload = useMutation({
    mutationFn: reloadProviders,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["provider-runtime"] });
      await queryClient.invalidateQueries({ queryKey: ["provider-models"] });
    },
  });

  if (catalog.isPending || runtime.isPending || models.isPending || !draft) {
    return <PanelLoading label={t("management.providers.loading")} />;
  }
  if (catalog.isError || runtime.isError || models.isError) {
    return (
      <PanelError
        title={t("management.providers.unavailable")}
        action={() => {
          void catalog.refetch();
          void runtime.refetch();
          void models.refetch();
        }}
      />
    );
  }

  return (
    <div className="management-stack">
      <div className="management-summary">
        <div>
          <span>{t("management.providers.configured")}</span>
          <strong>{formatNumber(runtime.data.provider_count, locale)}</strong>
        </div>
        <div>
          <span>{t("management.providers.models")}</span>
          <strong>{formatNumber(models.data.models.length, locale)}</strong>
        </div>
        <div>
          <span>{t("management.providers.revision")}</span>
          <strong>{runtime.data.revision}</strong>
        </div>
        <p className="revision-label">
          {t("management.providers.revisionLabel", {
            revision: runtime.data.revision,
          })}
        </p>
      </div>

      <div className="provider-grid">
        {catalog.data.providers.map((provider) => {
          const instances = draft.providers.instances.filter(
            (instance) => instance.catalog_id === provider.id,
          );
          return (
            <article className="provider-card" key={provider.id}>
              <div className="provider-card-heading">
                <div>
                  <h3>{provider.label}</h3>
                  <p>{provider.adapter.replaceAll("_", " ")}</p>
                </div>
                <span>{provider.auth_kind}</span>
              </div>
              <p className="provider-capabilities">
                {capabilityLabels(provider).join(" · ") ||
                  t("management.providers.textOnly")}
              </p>
              {instances.length === 0 ? (
                <p className="empty-inline">
                  {t("management.providers.notConfigured")}
                </p>
              ) : (
                <ul className="provider-instances">
                  {instances.map((instance) => (
                    <li key={instance.id}>
                      <label>
                        <input
                          type="checkbox"
                          aria-label={t("management.providers.enableInstance", {
                            id: instance.id,
                          })}
                          checked={instance.enabled}
                          disabled={!canWrite}
                          onChange={(event) => {
                            const enabled = event.currentTarget.checked;
                            setDraft((current) =>
                              current
                                ? {
                                    ...current,
                                    providers: {
                                      ...current.providers,
                                      instances:
                                        current.providers.instances.map(
                                          (candidate) =>
                                            candidate.id === instance.id
                                              ? {
                                                  ...candidate,
                                                  enabled,
                                                }
                                              : candidate,
                                        ),
                                    },
                                  }
                                : current,
                            );
                          }}
                        />
                        <span>{instance.id}</span>
                      </label>
                      <span>
                        {instance.enabled
                          ? t("management.providers.enabled")
                          : t("management.providers.disabled")}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </article>
          );
        })}
      </div>

      {canWrite && (
        <AddProviderForm
          providers={catalog.data.providers}
          draft={draft}
          onChange={setDraft}
        />
      )}

      {canWrite && canManageSecrets && (
        <AccountsEditor
          catalog={catalog.data.providers}
          draft={draft}
          onChange={setDraft}
          onRemoveSecret={(secretRef) =>
            setPendingSecretDeletes((current) => [
              ...new Set([...current, secretRef]),
            ])
          }
        />
      )}

      <div className="management-actions">
        {canWrite && (
          <button
            className="button button--primary"
            type="button"
            disabled={save.isPending}
            onClick={() => save.mutate()}
          >
            {save.isPending
              ? t("management.providers.saving")
              : t("management.providers.saveChanges")}
          </button>
        )}
        {canWrite && (
          <button
            className="button button--secondary"
            type="button"
            disabled={reload.isPending}
            onClick={() => reload.mutate()}
          >
            {reload.isPending
              ? t("management.providers.reloading")
              : t("management.providers.reload")}
          </button>
        )}
        {(save.isError || reload.isError) && (
          <p className="inline-error">
            {t("management.providers.changesNotApplied")}
          </p>
        )}
      </div>
    </div>
  );
}

function AddProviderForm({
  providers,
  draft,
  onChange,
}: {
  providers: ProviderDefinition[];
  draft: ProviderCandidate;
  onChange: (draft: ProviderCandidate) => void;
}) {
  const { t } = useTranslation();
  const [catalogId, setCatalogId] = useState(providers[0]?.id ?? "");
  const [instanceId, setInstanceId] = useState("");

  function submit(event: FormEvent) {
    event.preventDefault();
    const id = instanceId.trim().toLowerCase();
    if (
      !/^[a-z0-9._-]*[a-z0-9]$/.test(id) ||
      draft.providers.instances.some((instance) => instance.id === id)
    ) {
      return;
    }
    onChange({
      ...draft,
      providers: {
        ...draft.providers,
        instances: [
          ...draft.providers.instances,
          {
            id,
            catalog_id: catalogId,
            enabled: true,
            endpoint: null,
            allow_private_network: false,
          },
        ],
      },
    });
    setInstanceId("");
  }

  return (
    <form className="management-form" onSubmit={submit}>
      <div>
        <h3>{t("management.providers.addTitle")}</h3>
        <p>{t("management.providers.addDescription")}</p>
      </div>
      <label>
        {t("management.providers.provider")}
        <select
          value={catalogId}
          onChange={(event) => setCatalogId(event.currentTarget.value)}
        >
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        {t("management.providers.instanceId")}
        <input
          value={instanceId}
          maxLength={128}
          pattern="[a-z0-9._-]*[a-z0-9]"
          placeholder={t("management.providers.instanceIdPlaceholder")}
          onChange={(event) => setInstanceId(event.currentTarget.value)}
        />
      </label>
      <button
        className="button button--secondary"
        type="submit"
        disabled={!catalogId || instanceId.trim() === ""}
      >
        {t("management.providers.addInstance")}
      </button>
    </form>
  );
}

function AccountsEditor({
  catalog,
  draft,
  onChange,
  onRemoveSecret,
}: {
  catalog: ProviderDefinition[];
  draft: ProviderCandidate;
  onChange: (draft: ProviderCandidate) => void;
  onRemoveSecret: (secretRef: string) => void;
}) {
  const { t } = useTranslation();
  const instances = draft.providers.instances;
  const [providerId, setProviderId] = useState(instances[0]?.id ?? "");
  const [accountId, setAccountId] = useState("");
  const [secret, setSecret] = useState("");
  const [replacement, setReplacement] = useState<Record<string, string>>({});
  const create = useMutation({
    mutationFn: async () => {
      const instance = instances.find(
        (candidate) => candidate.id === providerId,
      );
      const definition = catalog.find(
        (candidate) => candidate.id === instance?.catalog_id,
      );
      if (!instance || !definition || definition.auth_kind === "local") {
        throw new Error("Selected provider does not accept a secret.");
      }
      const purpose = secretPurpose(definition.auth_kind);
      const response = await createProviderSecret({
        provider_id: instance.id,
        account_id: accountId,
        purpose,
        secret,
      });
      return {
        id: accountId,
        provider: instance.id,
        enabled: true,
        auth: accountAuth(definition.auth_kind, response.secret_ref),
      } satisfies ProviderConfig["accounts"][number];
    },
    onSuccess: (account) => {
      onChange({
        ...draft,
        providers: {
          ...draft.providers,
          accounts: [...draft.providers.accounts, account],
        },
      });
      setAccountId("");
      setSecret("");
    },
  });
  const replace = useMutation({
    mutationFn: async (account: ProviderConfig["accounts"][number]) => {
      const secretRef = accountSecretRef(account);
      const next = replacement[account.id] ?? "";
      if (!secretRef || next === "") {
        throw new Error("Secret replacement is incomplete.");
      }
      await replaceProviderSecret(secretRef, next);
      return account.id;
    },
    onSuccess: (id) =>
      setReplacement((current) => ({ ...current, [id]: "" })),
  });

  if (instances.length === 0) {
    return null;
  }

  return (
    <div className="accounts-editor">
      <div>
        <h3>{t("management.providers.accountsTitle")}</h3>
        <p>{t("management.providers.accountsDescription")}</p>
      </div>
      {draft.providers.accounts.length > 0 && (
        <ul className="account-list">
          {draft.providers.accounts.map((account) => {
            const secretRef = accountSecretRef(account);
            return (
              <li key={account.id}>
                <div>
                  <strong>{account.id}</strong>
                  <span>{account.provider}</span>
                </div>
                {secretRef && (
                  <label>
                    <span className="sr-only">
                      {t("management.providers.replacementSecretAria", {
                        id: account.id,
                      })}
                    </span>
                    <input
                      type="password"
                      autoComplete="new-password"
                      value={replacement[account.id] ?? ""}
                      placeholder={t(
                        "management.providers.replacementSecretPlaceholder",
                      )}
                      onChange={(event) =>
                        setReplacement((current) => ({
                          ...current,
                          [account.id]: event.currentTarget.value,
                        }))
                      }
                    />
                  </label>
                )}
                {secretRef && (
                  <button
                    className="button button--secondary button--compact"
                    type="button"
                    disabled={
                      replace.isPending ||
                      (replacement[account.id] ?? "") === ""
                    }
                    onClick={() => replace.mutate(account)}
                  >
                    {t("management.providers.replace")}
                  </button>
                )}
                <button
                  className="button button--quiet button--compact"
                  type="button"
                  onClick={() => {
                    onChange({
                      ...draft,
                      providers: {
                        ...draft.providers,
                        accounts: draft.providers.accounts.filter(
                          (candidate) => candidate.id !== account.id,
                        ),
                      },
                    });
                    if (secretRef) {
                      onRemoveSecret(secretRef);
                    }
                  }}
                  >
                    {t("management.providers.remove")}
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <form
        className="credential-form"
        onSubmit={(event) => {
          event.preventDefault();
          create.mutate();
        }}
      >
        <label>
          {t("management.providers.instance")}
          <select
            value={providerId}
            onChange={(event) => setProviderId(event.currentTarget.value)}
          >
            {instances.map((instance) => (
              <option key={instance.id} value={instance.id}>
                {instance.id}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("management.providers.accountId")}
          <input
            value={accountId}
            maxLength={128}
            pattern="[a-z0-9._-]*[a-z0-9]"
            placeholder={t("management.providers.accountIdPlaceholder")}
            onChange={(event) => setAccountId(event.currentTarget.value)}
          />
        </label>
        <label>
          {t("management.providers.secretValue")}
          <input
            type="password"
            autoComplete="new-password"
            value={secret}
            placeholder={t("management.providers.secretPlaceholder")}
            onChange={(event) => setSecret(event.currentTarget.value)}
          />
        </label>
        <button
          className="button button--secondary"
          type="submit"
          disabled={
            create.isPending || accountId.trim() === "" || secret === ""
          }
        >
          {create.isPending
            ? t("management.providers.storing")
            : t("management.providers.storeAccount")}
        </button>
      </form>
      {(create.isError || replace.isError) && (
        <p className="inline-error">
          {t("management.providers.secretFailure")}
        </p>
      )}
    </div>
  );
}

function SessionsPanel({ canReadMessages }: { canReadMessages: boolean }) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage as SupportedLocale;
  const [cursor, setCursor] = useState<string | undefined>();
  const [selected, setSelected] = useState<
    SessionList["items"][number] | null
  >(null);
  const [messageCursor, setMessageCursor] = useState<string | undefined>();
  const sessions = useQuery({
    queryKey: ["sessions", cursor],
    queryFn: () => getSessions({ before: cursor, limit: 50 }),
  });
  const messages = useQuery({
    queryKey: ["session-messages", selected?.session_key, messageCursor],
    queryFn: () =>
      getSessionMessages(selected!.session_key, {
        after: messageCursor,
        limit: 100,
        max_bytes: 262_144,
      }),
    enabled: canReadMessages && selected !== null,
  });

  if (sessions.isPending) {
    return <PanelLoading label={t("management.sessions.loading")} />;
  }
  if (sessions.isError) {
    return (
      <PanelError
        title={t("management.sessions.unavailable")}
        action={() => void sessions.refetch()}
      />
    );
  }

  return (
    <div className="session-layout">
      <div className="session-list">
        <div className="subsection-heading">
          <div>
            <h3>{t("management.sessions.indexed")}</h3>
            <p>{sessions.data.index_status.phase}</p>
          </div>
          <span>
            {t("management.sessions.onThisPage", {
              count: sessions.data.items.length,
              formattedCount: formatNumber(sessions.data.items.length, locale),
            })}
          </span>
        </div>
        {sessions.data.items.length === 0 ? (
          <EmptyState
            title={t("management.sessions.emptyTitle")}
            detail={t("management.sessions.emptyDetail")}
          />
        ) : (
          <ul>
            {sessions.data.items.map((session) => (
              <li key={session.session_key}>
                <button
                  type="button"
                  className="session-row"
                  aria-pressed={selected?.session_key === session.session_key}
                  onClick={() => {
                    setSelected(session);
                    setMessageCursor(undefined);
                  }}
                >
                  <span>
                      <strong>
                        {session.title ?? t("management.sessions.untitled")}
                      </strong>
                      <small>
                        {session.source} ·
                        {" "}
                        {t("management.sessions.messageCount", {
                          count: session.message_count,
                          formattedCount: formatNumber(
                            session.message_count,
                            locale,
                          ),
                        })}
                      </small>
                    </span>
                    <time dateTime={session.last_active_at}>
                      {formatLocalTime(session.last_active_at, locale)}
                  </time>
                </button>
              </li>
            ))}
          </ul>
        )}
        {sessions.data.next_cursor && (
          <button
            className="button button--secondary"
            type="button"
            onClick={() => {
              setCursor(sessions.data.next_cursor ?? undefined);
              setSelected(null);
            }}
          >
            {t("management.sessions.nextPage")}
          </button>
        )}
      </div>
      <div className="message-view">
        {!selected && (
          <EmptyState
            title={t("management.sessions.selectTitle")}
            detail={t("management.sessions.selectDetail")}
          />
        )}
        {selected && !canReadMessages && (
          <EmptyState
            title={t("management.sessions.messageAccessTitle")}
            detail={t("management.sessions.messageAccessDetail")}
          />
        )}
        {selected && canReadMessages && messages.isPending && (
          <PanelLoading label={t("management.sessions.loadingMessages")} />
        )}
        {selected && canReadMessages && messages.isError && (
          <PanelError
            title={t("management.sessions.messagesUnavailable")}
            action={() => void messages.refetch()}
          />
        )}
        {selected && canReadMessages && messages.data && (
          <>
            <div className="subsection-heading">
              <div>
                <h3>{selected.title ?? t("management.sessions.untitled")}</h3>
                <p>{selected.source}</p>
              </div>
            </div>
            <ol className="message-list">
              {messages.data.items.map((message) => (
                <li key={`${message.message_key}-${message.fragment_offset_bytes}`}>
                  <header>
                    <strong>{message.role}</strong>
                    <time dateTime={message.timestamp}>
                      {formatLocalTime(message.timestamp, locale)}
                    </time>
                  </header>
                  <pre>{message.content}</pre>
                </li>
              ))}
            </ol>
            {messages.data.next_cursor && (
              <button
                className="button button--secondary"
                type="button"
                onClick={() =>
                  setMessageCursor(messages.data.next_cursor ?? undefined)
                }
              >
                {t("management.sessions.loadNextMessages")}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function UsagePanel() {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage as SupportedLocale;
  const [groupBy, setGroupBy] =
    useState<NonNullable<UsageQuery["group_by"]>>("day");
  const usage = useQuery({
    queryKey: ["usage", groupBy],
    queryFn: () => getUsage({ group_by: groupBy, limit: 100 }),
  });

  if (usage.isPending) {
    return <PanelLoading label={t("management.usage.loading")} />;
  }
  if (usage.isError) {
    return (
      <PanelError
        title={t("management.usage.unavailable")}
        action={() => void usage.refetch()}
      />
    );
  }

  const totals = usage.data.totals;
  return (
    <div className="management-stack">
      <div className="subsection-heading">
        <div>
          <h3>{t("management.usage.heading")}</h3>
          <p>{t("management.usage.description")}</p>
        </div>
        <label className="inline-select">
          {t("management.usage.groupBy")}
          <select
            value={groupBy}
            onChange={(event) =>
              setGroupBy(
                event.currentTarget.value as NonNullable<
                  UsageQuery["group_by"]
                >,
              )
            }
          >
            <option value="day">{t("management.usage.groupDay")}</option>
            <option value="source">{t("management.usage.groupSource")}</option>
            <option value="model">{t("management.usage.groupModel")}</option>
          </select>
        </label>
      </div>
      <div className="usage-totals">
        <Metric
          label={t("management.usage.inputTokens")}
          value={totals.input_tokens}
          locale={locale}
        />
        <Metric
          label={t("management.usage.outputTokens")}
          value={totals.output_tokens}
          locale={locale}
        />
        <Metric
          label={t("management.usage.cacheRead")}
          value={totals.cache_read_tokens}
          locale={locale}
        />
        <Metric
          label={t("management.usage.reasoning")}
          value={totals.reasoning_tokens}
          locale={locale}
        />
        <Metric
          label={t("management.usage.sessions")}
          value={totals.session_count}
          locale={locale}
        />
      </div>
      {usage.data.buckets.length === 0 ? (
          <EmptyState
            title={t("management.usage.emptyTitle")}
            detail={t("management.usage.emptyDetail")}
        />
      ) : (
        <div className="table-scroll">
          <table className="data-table">
            <thead>
              <tr>
                <th>{t("management.usage.bucket")}</th>
                <th>{t("management.usage.input")}</th>
                <th>{t("management.usage.output")}</th>
                <th>{t("management.usage.cache")}</th>
                <th>{t("management.usage.sessions")}</th>
              </tr>
            </thead>
            <tbody>
              {usage.data.buckets.map((bucket) => (
                <tr key={bucket.key}>
                  <th>{bucket.key}</th>
                  <td>{formatNumber(bucket.input_tokens, locale)}</td>
                  <td>{formatNumber(bucket.output_tokens, locale)}</td>
                  <td>{formatNumber(bucket.cache_read_tokens, locale)}</td>
                  <td>{formatNumber(bucket.session_count, locale)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function DiagnosticsPanel({ canExport }: { canExport: boolean }) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage as SupportedLocale;
  const [cursor, setCursor] = useState<string | undefined>();
  const logs = useQuery({
    queryKey: ["diagnostic-logs", cursor],
    queryFn: () =>
      getDiagnosticLogs({ after: cursor, order: "desc", limit: 100 }),
  });
  const exportArchive = useMutation({
    mutationFn: () =>
      exportDiagnostics({
        include_snapshots: false,
        max_bytes: 16 * 1024 * 1024,
      }),
  });

  if (logs.isPending) {
    return <PanelLoading label={t("management.diagnostics.loading")} />;
  }
  if (logs.isError) {
    return (
      <PanelError
        title={t("management.diagnostics.unavailable")}
        action={() => void logs.refetch()}
      />
    );
  }

  return (
    <div className="management-stack">
      <div className="subsection-heading">
        <div>
          <h3>{t("management.diagnostics.events")}</h3>
          <p>
            {logs.data.dropped_events === 0
              ? t("management.diagnostics.noDroppedEvents")
              : t("management.diagnostics.droppedEvents", {
                  count: logs.data.dropped_events,
                  formattedCount: formatNumber(
                    logs.data.dropped_events,
                    locale,
                  ),
                })}
          </p>
        </div>
        {canExport && (
          <button
            className="button button--secondary"
            type="button"
            disabled={exportArchive.isPending}
            onClick={() => exportArchive.mutate()}
          >
            {exportArchive.isPending
              ? t("management.diagnostics.exporting")
              : t("management.diagnostics.export")}
          </button>
        )}
      </div>
      {exportArchive.data && (
        <p className="success-note" role="status">
          {t("management.diagnostics.exported", {
            fileName: exportArchive.data.file_name,
            size: formatBytes(exportArchive.data.bytes, locale),
          })}
        </p>
      )}
      {exportArchive.isError && (
        <p className="inline-error">
          {t("management.diagnostics.exportFailed")}
        </p>
      )}
      {logs.data.items.length === 0 ? (
          <EmptyState
            title={t("management.diagnostics.emptyTitle")}
            detail={t("management.diagnostics.emptyDetail")}
        />
      ) : (
        <ol className="diagnostic-list">
          {logs.data.items.map((item, index) => (
            <li key={`${cursor ?? "first"}-${index}`}>
              <pre>{JSON.stringify(item, null, 2)}</pre>
            </li>
          ))}
        </ol>
      )}
      {logs.data.next_cursor && (
        <button
          className="button button--secondary"
          type="button"
          onClick={() => setCursor(logs.data.next_cursor ?? undefined)}
        >
          {t("management.diagnostics.nextPage")}
        </button>
      )}
    </div>
  );
}

function PanelLoading({ label }: { label: string }) {
  return (
    <div className="panel-state" role="status" aria-label={label}>
      <span className="skeleton skeleton--title" aria-hidden="true" />
      <span className="skeleton skeleton--body" aria-hidden="true" />
      <span className="skeleton skeleton--meta" aria-hidden="true" />
    </div>
  );
}

function PanelError({ title, action }: { title: string; action: () => void }) {
  const { t } = useTranslation();

  return (
    <div className="panel-state">
      <h3>{title}</h3>
      <p>{t("common.localStateRecovery")}</p>
      <button className="button button--secondary" type="button" onClick={action}>
        {t("common.retry")}
      </button>
    </div>
  );
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="empty-state">
      <strong>{title}</strong>
      <p>{detail}</p>
    </div>
  );
}

function Metric({
  label,
  value,
  locale,
}: {
  label: string;
  value: number;
  locale: SupportedLocale;
}) {
  return (
    <div>
      <span>{label}</span>
      <strong>{formatNumber(value, locale)}</strong>
    </div>
  );
}

function cloneCandidate(
  providers: ProviderConfig,
  routing: ProviderCandidate["routing"],
): ProviderCandidate {
  return {
    providers: {
      instances: providers.instances.map((instance) => ({ ...instance })),
      accounts: providers.accounts.map((account) => ({
        ...account,
        auth: { ...account.auth },
      })),
    },
    routing: {
      aliases: routing.aliases.map((alias) => ({
        ...alias,
        target: { ...alias.target },
      })),
      rules: routing.rules.map((rule) => ({
        ...rule,
        target: { ...rule.target },
      })),
      default: routing.default ? { ...routing.default } : null,
    },
  };
}

function capabilityLabels(provider: ProviderDefinition): string[] {
  return Object.entries(provider.capabilities)
    .filter(([, enabled]) => enabled)
    .map(([name]) => name);
}

function secretPurpose(
  authKind: ProviderDefinition["auth_kind"],
): ProviderSecretPurpose {
  switch (authKind) {
    case "key":
      return "api_key";
    case "oauth":
      return "oauth_access";
    case "forward":
      return "auxiliary";
    case "local":
      return "auxiliary";
  }
}

function accountAuth(
  authKind: ProviderDefinition["auth_kind"],
  secretRef: string,
): ProviderConfig["accounts"][number]["auth"] {
  switch (authKind) {
    case "key":
      return { kind: "api_key", secret: secretRef };
    case "oauth":
      return { kind: "oauth", access: secretRef, refresh: null };
    case "forward":
      return { kind: "forward", credential: secretRef };
    case "local":
      return { kind: "local" };
  }
}

function accountSecretRef(
  account: ProviderConfig["accounts"][number],
): string | null {
  switch (account.auth.kind) {
    case "api_key":
      return account.auth.secret;
    case "oauth":
      return account.auth.access;
    case "forward":
      return account.auth.credential;
    case "local":
      return null;
  }
}
