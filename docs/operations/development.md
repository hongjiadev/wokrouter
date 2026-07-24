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
```

The reported Rust and pnpm versions must be `1.97.1` and `11.17.0`
respectively.

Install the locked frontend dependency graph with:

```powershell
pnpm --dir apps/desktop install --frozen-lockfile
```

## Foundation quality gate

Run the same gates used by CI from the repository root:

```powershell
pwsh tests/scripts/check-no-body-persistence.tests.ps1
pwsh tests/scripts/check-no-body-persistence.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop build
cargo deny check
```

The privacy check is deliberately scoped to persisted Rust models under
`crates/wokrouter-storage/src/config/model.rs` and
`crates/wokrouter-storage/src/state`, plus SQL files under crate `migrations`
directories. Request/control DTOs and documentation are not persistence and are
not scanned. Persisted fields named `request_body`, `response_body`, `prompt`,
`tool_arguments`, or `authorization` fail the gate.

The stable branch-protection checks are `rust`, `frontend`, and
`platform-check`. The last check aggregates the Windows, macOS, and Linux
platform matrix and succeeds only when every matrix entry succeeds.

## macOS and Linux boundaries

CI runs platform checks on `macos-15` and `ubuntu-24.04` in addition to
`windows-latest`. Every runner executes the privacy checker and compiles both
the platform crate and the Tauri desktop crate for its native host. Ubuntu CI
installs the Tauri WebKitGTK 4.1, AppIndicator, SSL, SVG, XDO, and native build
dependencies before compiling. A local Linux build needs the corresponding
packages for its distribution; a local macOS build needs Xcode Command Line
Tools.

These platform checks validate source compilation and Tauri build
configuration. They do not install or start an operating-system service, build
release installers, test code signing/notarization, or prove compatibility with
Linux distributions other than the Ubuntu runner. Those operations belong to
the later integration and release gates.
