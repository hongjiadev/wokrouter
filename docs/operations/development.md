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

On Windows, never execute Cargo's hashed test programs directly. Run the
self-test and the full workspace through the stable
`wokrouter-test-host.exe` name:

```powershell
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File tests/scripts/run-fixed-test-host.tests.ps1

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
coverage, and target checks for Windows x64, macOS x64/arm64, and Linux
x64/arm64. CI clears the OpenAI, Anthropic, Gemini, and Google provider
environment variables before running any gate.

## macOS and Linux boundaries

CI uses native GitHub-hosted runners for these Rust targets:

- `x86_64-pc-windows-msvc`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Ubuntu runners install the Tauri WebKitGTK 4.1, AppIndicator, SSL, SVG, XDO,
and native build dependencies before compiling. A local Linux build needs the
corresponding packages for its distribution; a local macOS build needs Xcode
Command Line Tools.

## Independent releases

The Release workflow builds the five targets above from a WokRouter tag. Its
version is derived only from that tag and remains independent of the installed
WokCore version. A manual dispatch accepts the WokRouter tag and performs the
same build and verification without publishing; a tag push publishes only
after all five archives and the compatibility matrix pass.

Every ordinary online WokRouter artifact contains the WokRouter desktop
installers and lifecycle binary, but no WokCore binary, legacy daemon, provider
simulator, or load generator. WokCore installation and upgrades use the
separate signed WokCore release channel. Updating either product therefore
does not overwrite the other product's binary or version.

The compatibility matrix covers current and newer same-API WokCore responses,
an older same-API WokCore with capability degradation, non-overlapping API
majors, and both independent-upgrade directions. The release gate does not
perform production code signing or notarization without the corresponding
repository credentials.
