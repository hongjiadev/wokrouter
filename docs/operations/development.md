# Development

WokRouter's foundation is pinned to Rust 1.97.1 and pnpm 11.17.0. Use the
checked-in lockfiles and toolchain file; do not substitute floating tool
versions in CI.

## Windows prerequisites

Install these prerequisites before building:

- Rust 1.97.1 through `rustup`, using an MSVC host toolchain. The repository's
  `rust-toolchain.toml` also requires the `clippy` and `rustfmt` components.
- Node.js 24.x and pnpm 11.17.0. Node 24 is the version used by CI and is
  compatible with the pinned pnpm release.
- cargo-deny 0.20.2, matching the version bundled by the pinned CI action.
- PowerShell 7 (`pwsh`) for the repository quality scripts.
- Microsoft Edge WebView2 Runtime. It is normally present on current Windows
  releases, but it remains a required Tauri runtime dependency.
- Visual Studio 2022 Build Tools with the **Desktop development with C++**
  workload, including the MSVC v143 toolset and a Windows 10 or Windows 11 SDK.
  Windows ARM64 cross-builds also require the
  `Microsoft.VisualStudio.Component.VC.Tools.ARM64` component.

Confirm the pinned tools before running the project:

```powershell
rustc --version
node --version
pnpm --version
cargo deny --version
```

The reported Rust, pnpm, and cargo-deny versions must be `1.97.1`, `11.17.0`,
and `0.20.2`, respectively. Install the CI-matching cargo-deny release when it
is absent:

```powershell
cargo install --locked cargo-deny --version 0.20.2
cargo deny --version
```

If `cargo deny --version` is unavailable locally, then the local dependency
policy gate has not run; do not report it as passing. CI uses
`EmbarkStudios/cargo-deny-action@v2.1.1`, which bundles the same cargo-deny
0.20.2 release.

Install the locked frontend dependency graph with:

```powershell
pnpm --dir apps/desktop install --frozen-lockfile
```

## Development WokCore runtime acceptance

The Cursor workspace compound `wok: debug` starts `wokcore: debug` with
`serve` and starts `wokrouter: dev` with
`WOKROUTER_DEV_WOKCORE_EXECUTABLE` pointing at the WokCore debug executable.
Run these six paths before accepting changes to development runtime selection:

1. **Development match.** Stop any system WokCore, start `wok: debug`, and
   wait for both configurations to reach their running state. Confirm WokRouter
   reports `runtime_channel: "development"` and a debugger breakpoint is hit
   in the IDE-started WokCore.
2. **Delayed development match.** Delay the WokCore debug launch by less than
   five seconds while starting `wok: debug`. Confirm WokRouter waits and then
   connects on the development channel, with no production download during
   the wait.
3. **System production fallback.** Disable `wokcore: debug`, keep a system
   WokCore available, and start `wokrouter: dev` with its configured
   development variable. Confirm WokRouter does not mistake the system process
   for the configured debug executable and selects it on the production
   channel after five seconds.
4. **Signed-install production fallback.** Remove both the development and
   system WokCore, start `wokrouter: dev`, and wait for the five-second
   development deadline. Confirm the desktop enters the production signed
   automatic-install flow, reports real downloaded and total byte counts, and
   reaches running without an install click.
5. **Release ignores the variable.** Set
   `WOKROUTER_DEV_WOKCORE_EXECUTABLE` and start a release build. Confirm it
   selects only through production discovery and never reports the development
   channel. The variable name and its parsing must not be present in release
   metadata.
6. **Development runtime remains IDE-managed.** With the development channel
   selected, close WokRouter and confirm the IDE-started WokCore keeps running.
   An explicit stop, update check, or update-install backend request must
   return `development_runtime_managed_by_ide`; no WokCore upgrade prompt or
   update child may appear.

Runtime status exposed through JSON or the Tauri bridge may include
`runtime_channel`, but must never include a field named `pid`, `path`, or
`executable`.

## WokCore lifecycle acceptance evidence

The repository does not currently provide a command that drives a live signed
loopback WokCore through the packaged desktop GUI. In particular, there is no
manual signed-loopback CLI for the update, rollback, close/reopen, or
child-process observations described below. Do not invent one and do not use a
production Minisign private key for acceptance. The reproducible evidence
available today is the fixed-host Rust suite, the frontend unit suite, and the
foundation source-contract suite listed in the quality gate below.

Run frontend lifecycle evidence with:

```powershell
pnpm.cmd --dir apps/desktop exec vitest run src/components/CoreLifecycle.test.tsx
```

On Windows, run all referenced Rust tests through the
`tests/scripts/run-fixed-test-host.ps1` command in the next section; never run
Cargo's hashed test binaries directly. Each numbered path maps to these real
test names and fixtures:

1. **Missing to running without a click.** Frontend fixture `starts one
   production install in StrictMode and restores normal content after success`;
   Rust fixtures
   `missing_production_runtime_installs_starts_authorizes_and_reports_structured_progress`
   and
   `signed_release_reports_monotonic_download_and_authoritative_install_phases`.
2. **Signed update cancel and confirm.** Frontend fixture `requires an
   accessible confirmation and invokes the expected version once` covers
   cancel, confirmation, and exactly-once invocation. Rust fixture
   `system_runner_uses_only_the_three_fixed_child_commands` fixes the real
   update-install argv. A live signed update artifact request through the GUI
   remains unautomated because the repository has no such harness.
3. **Active requests remain.** Frontend fixture `returns management after
   active requests defer the update and reconfirms retry` covers recovery and
   fresh confirmation. Parser fixtures
   `versions_bytes_and_active_requests_are_strictly_validated` and
   `update_active_requests_are_valid_during_rolling_back` cover the bounded
   count. A real draining WokCore process remains outside this repository's
   executable acceptance surface.
4. **Verification failure and rollback.** Signed-release fixtures
   `artifact_hash_mismatch_leaves_no_install_or_record` and
   `invalid_manifest_signature_is_rejected_before_artifact_download` prove
   untrusted install bytes are rejected. Frontend error fixtures cover
   `update_verification_failed` and `rolled_back`; a process-level rollback to
   a previous runtime remains unautomated here.
5. **Close and reopen during an operation.** Coordinator fixture
   `duplicate_installs_coalesce_conflicts_fail_and_terminal_allows_retry` and
   frontend fixtures `subscribes before recovering a running snapshot and
   unmounts only the listener` and `treats install_in_progress as another
   process and polls trusted status without retrying` cover reconciliation and
   duplicate suppression. There is no packaged-GUI process harness that can
   close the window and inspect the surviving child.
6. **IDE Development performs zero update work.** Rust fixture
   `development_suppresses_every_install_and_update_path_before_authority_or_runner`,
   frontend fixtures `never checks or installs updates for a development
   runtime` and `never starts production installation for a development
   status`, and runtime fixture
   `a_selected_development_session_never_switches_to_production` cover the
   backend, frontend, and session-lifetime gates.
7. **Chinese and English UI.** Run
   `pnpm.cmd --dir apps/desktop exec vitest run src/locale.test.ts` for operating-system
   locale detection, including `zh-CN`. Full translated visible and ARIA
   lifecycle strings belong to the separate Windows/i18n acceptance plan and
   are not claimed by the lifecycle fixtures above.

## Foundation quality gate

Run the same gates used by CI from the repository root:

```powershell
pwsh tests/scripts/check-foundation-contract.tests.ps1
pwsh tests/scripts/check-foundation-contract.ps1
pwsh tests/scripts/check-release-contract.tests.ps1
pwsh tests/scripts/check-release-contract.ps1
pwsh tests/scripts/check-public-repo-hygiene.tests.ps1
pwsh tests/scripts/check-public-repo-hygiene.ps1
pwsh tests/scripts/check-core-boundary.tests.ps1
pwsh tests/scripts/check-core-boundary.ps1
pwsh tests/scripts/check-no-body-persistence.tests.ps1
pwsh tests/scripts/check-no-body-persistence.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop build
cargo deny --all-features check
```

On Windows, never execute Cargo's hashed test programs directly. Compile Rust
tests with Cargo, then execute every test binary only through
`tests/scripts/run-fixed-test-host.ps1`, which copies each artifact to the
stable `wokrouter-test-host.exe` filename. Run the wrapper self-test and the
full workspace as follows:

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File tests/scripts/run-fixed-test-host.tests.ps1

$env:OPENAI_API_KEY = ""
$env:ANTHROPIC_API_KEY = ""
$env:GEMINI_API_KEY = ""
$env:GOOGLE_API_KEY = ""
$repositoryRoot = (Get-Location).Path
$targetDirectory = Join-Path $repositoryRoot 'target'
$command = @"
& './tests/scripts/run-fixed-test-host.ps1' `
  -TargetDirectory '$targetDirectory' `
  -RepositoryRoot '$repositoryRoot' `
  -Offline `
  -HarnessArguments @('--nocapture')
"@
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -Command $command
```

Linux and macOS use their native test executables:

```sh
cargo test --workspace --all-features --locked
```

The privacy check uses an explicit persisted-model list:
`AppConfig`, `VersionedConfig`, `ServerConfig`, and `UiConfig` in
`crates/wokrouter-storage/src/config/model.rs`, plus `RequestMetric` in
`crates/wokrouter-storage/src/state/store.rs`. Transient types such as
`StateHealth` and `StateStore`, request/control DTOs, and documentation are not
persistence and are not scanned. Every listed model file and struct must exist;
an incomplete model inventory fails closed with exit code 2. In crate
`migrations` directories, `CREATE TABLE` column definitions, supported
`CREATE TABLE ... AS SELECT` output names, `ALTER TABLE ... ADD [COLUMN]`
column names, and `ALTER TABLE ... RENAME COLUMN ... TO` targets are checked.
Comments, strings, table names, `FROM` table names, index names, old rename
names, constraint names, and type names are ignored. A CTAS select-list shape
whose output name cannot be determined reliably fails closed with exit code 2
and a `CTAS PARSE ERROR`. Persisted fields or columns named `request_body`,
`response_body`, `prompt`, `tool_arguments`, or `authorization` fail the gate
case-insensitively, including quoted SQL identifiers and Rust raw identifiers.

The foundation contract self-test mutates isolated workflow fixtures to prove
that job relationships, matrix runners, and required commands cannot move to a
different job without failing the gate.

The stable branch-protection checks are `rust`, `frontend`, and
`platform-check`. The last check aggregates native tests, compatibility
coverage, and target checks for Windows x64/arm64, macOS x64/arm64, and Linux
x64/arm64. The Windows ARM64 matrix entries compile tests with `--no-run` and
never execute a Rust test program. Both the fixed-host self-test and actual
Windows Rust tests run only on the Windows x64 matrix entry. Immediately before
the actual fixed-host wrapper, CI clears the OpenAI, Anthropic, Gemini, and
Google provider environment variables.

## Release targets and public names

Release builds use these six Rust target triples internally:

- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Public asset names use `Linux`, `macOS`, or `Windows` and `x86_64` or `arm64`;
they never expose Rust vendor segments such as `unknown`, `pc-windows`, or
`apple-darwin`.

Ubuntu runners install the Tauri WebKitGTK 4.1, AppIndicator, SSL, SVG, XDO,
RPM, cpio, and native build dependencies before compiling and inspecting
packages. A local Linux build needs the corresponding packages for its
distribution; a local macOS build needs Xcode Command Line Tools.

Each target produces normalized public files rather than an archive containing
another bundle tree. Linux publishes AppImage, deb, and rpm; macOS publishes
dmg, tar.gz, and zip; Windows publishes MSI and Portable.zip. Across both
architectures this is exactly 16 payloads. The platform packagers inspect
native metadata, architecture, exact inventory, and the online WokRouter
boundary before any payload can enter the signing job.

## Independent releases

The Release workflow builds the six targets above from a WokRouter tag. Its
version is derived only from that tag and remains independent of the installed
WokCore version. A manual dispatch accepts the WokRouter tag and performs the
same build, signing, and verification without creating, editing, or publishing
a GitHub Release. Only a tag push may start the serialized draft transaction,
after all target assets and the compatibility matrix pass.

Every ordinary online WokRouter artifact contains the WokRouter desktop
installers and lifecycle binary, but no WokCore binary, legacy daemon, provider
simulator, or load generator. WokCore installation and upgrades use the
separate signed WokCore release channel. Updating either product therefore
does not overwrite the other product's binary or version.

The compatibility matrix covers current and newer same-API WokCore responses,
an older same-API WokCore with capability degradation, non-overlapping API
majors, both independent-upgrade directions, signed WokCore manifest v2
preference, the missing-v2 fallback to v1, and the two present-invalid-v2
no-downgrade boundaries.

WokRouter uses its own `WOKROUTER_MINISIGN_SECRET_KEY`; it never uses a WokCore
signing key. The signing job first requires exactly 16 regular payloads, then
creates a signature for every payload plus signed `SHA256SUMS` and the public
key copy. The resulting bundle has exactly 35 files and is verified against
the committed external trust anchor at `release/minisign.pub`.

Publishing is an atomic draft transaction. An absent release is created as a
verified draft; a rerun may replace assets only while that release remains a
draft, and an existing public release is immutable to the workflow. Existing
draft assets are removed, exactly 35 files are uploaded, the complete draft is
downloaded into a separate clean directory and verified again, and only then
is `--draft=false` applied. Production Authenticode signing, Apple notarization,
and store publication remain outside this release workflow.
