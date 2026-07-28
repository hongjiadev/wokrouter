# Client integrations

WokRouter can connect Codex, Claude Code, and GitHub Copilot App to a running
WokCore instance. Integration is enabled only when WokCore discovery,
instance identity, management API version, client-token capabilities, and the
client's required protocol all verify successfully.

WokCore remains the only data-plane and token issuer. WokRouter does not
install a routing daemon, copy a Provider or model catalog, or take ownership
of WokCore updates.

## Commands

Start WokCore before enabling an integration:

```text
wokrouter start
wokrouter integrate codex
wokrouter integrate claude
wokrouter integrate copilot
```

Codex and Claude are updated automatically. Their configured roots honor
`CODEX_HOME` and `CLAUDE_CONFIG_DIR`; unset variables fall back to
`~/.codex` and `~/.claude`. The Copilot command returns a structured JSON setup
object for its BYOK UI; WokRouter never reads or modifies Copilot's opaque
application data.

Inspect integrations without changing files or creating integration state:

```text
wokrouter doctor
wokrouter doctor --json
```

The JSON report has a stable schema and contains check identifiers, severity,
status, summary keys, and optional remediation commands. For active
integrations it also verifies WokCore discovery, installation and instance
identity, API compatibility, capabilities, protocol support, and server-side
token state. It does not contain tokens, model names, request data, or local
paths. A named repair changes only the selected client and is idempotent:

```text
wokrouter doctor --repair codex_config
wokrouter doctor --repair codex_token
```

Replace `codex` with `claude` or `copilot` for the corresponding checks.
Configuration drift is not overwritten by repair; restore or reconcile it
explicitly.

When the same WokCore installation restarts with a different instance,
version, API endpoint, or port, doctor reports the integration as runtime
drift instead of healthy. Repair refreshes Codex or Claude automatically and
returns the new manual BYOK values for Copilot.

Restore the exact pre-integration Codex or Claude configuration and revoke the
client token with:

```text
wokrouter restore codex
wokrouter restore claude
wokrouter restore copilot
```

Copilot restore revokes the WokCore token but cannot remove the BYOK provider
or API key from Copilot's system credential store. The command reports this
manual final step instead of claiming that opaque Copilot state was restored.

## Client behavior

Codex receives a `wokcore` custom model provider using the Responses wire API.
Its authentication block invokes WokRouter as a command-backed credential
helper, so no bearer token is written to `config.toml`.

Claude Code receives an `apiKeyHelper` command and an
`ANTHROPIC_BASE_URL` derived from verified WokCore discovery. No bearer token
is written to `settings.json`.

Copilot receives structured OpenAI-compatible BYOK values: the verified base
URL, provider type, API format, and an `api_key_command`. Run that command
locally and paste its output into Copilot's API key field; the command itself
is not a Copilot BYOK field. Copilot stores the pasted static key in the
system credential store. After `doctor --repair copilot_token`, run the
returned command and paste the replacement key again.

## Token and recovery boundaries

WokRouter requests a separate `proxy.use` token for each client through the
WokCore endpoint protected by `clients.manage`. Before the request it writes a
private transaction intent containing a preallocated token ID, so an
interrupted response can be found and revoked without retaining the raw
credential. Tokens are stored in per-client private files under WokRouter's
integration state. The internal
`integration-token` command writes the selected active token to stdout only
for the configured client credential helper; its output is a credential and
must not be logged.

Codex and Claude edits use the same transaction intent plus a write-ahead
mutation journal, same-directory atomic replacement, private backups, and
content hashes. Recovery handles interruption after token issue, token-file
write, journal preparation, config replacement, or registry activation.
Restore is idempotent and proceeds only when the current file still matches
the private ownership record. If the user changed the file, WokRouter leaves
it untouched, retains the ownership record, and writes a private conflict
manifest containing hashes rather than configuration contents.

The ownership record binds tokens to a stable WokCore installation identity
as well as the issuing instance, version, API major, and endpoint. A normal
restart of the same installation can refresh the endpoint after revalidation;
WokRouter never sends an old token ID to a different installation.
