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

## WokCore lifecycle manual acceptance

Run these paths in a disposable Windows user account or VM. For a clean
app-data case, start WokRouter from a shell whose `APPDATA` and `LOCALAPPDATA`
point to newly created empty directories, with no trusted WokCore install
record or `wokcore.exe` on `PATH`. For update fault cases, use the signed
loopback release fixture and a throttled artifact response so every transition
is observable. Record the loopback request log and child-process list.

1. **Missing to running without a click.** Open the desktop against clean
   app-data. Do not press any install or retry action. Confirm the primary
   panel advances from release checking to a download whose completed bytes
   start at zero, increase monotonically, and finish at the signed total.
   Confirm verification, installation, start, authorization, and runtime
   verification remain indeterminate, then confirm the desktop reaches
   running and restores management.
2. **Signed update cancel and confirm.** Serve a valid signed newer loopback
   release to a trusted running WokCore. Confirm the desktop shows current and
   target versions. Cancel once and verify that no artifact request or update
   child occurs. Reopen, confirm, and verify a fresh signed check, real-byte
   download progress, and the verified version reported by the child.
3. **Active requests remain.** Keep one or more requests active in the
   loopback runtime, confirm the update, and verify drain ends with
   `active_requests_remain` and the bounded active-request count. Confirm the
   old runtime remains available, management returns, and retry requires a
   fresh check and confirmation.
4. **Verification failure and rollback.** First serve an invalid
   signature/hash and verify `update_verification_failed` without installing
   untrusted bytes. Then use a correctly signed artifact whose post-install
   runtime verification fails; confirm rollback runs, `rolled_back` is shown,
   and the previous trusted runtime/version is restored.
5. **Close and reopen during an operation.** Start a throttled install or
   update, close WokRouter during download, and confirm the transactional child
   remains alive. Reopen the desktop and verify operation status or the
   installer lease restores progress without a duplicate child; let it finish
   and confirm the final trusted runtime status.
6. **IDE Development performs zero update work.** Start `wok: debug` and
   confirm the desktop reports Development. Observe that startup, refetch, and
   manual UI paths issue no update check or update-install request. Invoke the
   backend check and install commands directly and confirm both return
   `development_runtime_managed_by_ide`; the loopback request log and
   child-process list must remain unchanged. Closing WokRouter must not stop
   the IDE-owned WokCore.
7. **Chinese and English UI.** Verify locale detection and every visible or
   ARIA lifecycle string through the separate Windows/i18n acceptance plan;
   do not treat the lifecycle checks above as i18n coverage.

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
