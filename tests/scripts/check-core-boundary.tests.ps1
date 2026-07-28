[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$checker = Join-Path $PSScriptRoot "check-core-boundary.ps1"
if (-not (Test-Path -LiteralPath $checker -PathType Leaf)) {
    throw "Missing core boundary checker: $checker"
}

function Set-FixtureFile {
    param(
        [Parameter(Mandatory)]
        [string] $Root,

        [Parameter(Mandatory)]
        [string] $RelativePath,

        [Parameter(Mandatory)]
        [string] $Content
    )

    $path = Join-Path $Root $RelativePath
    $parent = Split-Path -Parent $path
    $null = New-Item -ItemType Directory -Path $parent -Force
    Set-Content -LiteralPath $path -Value $Content -Encoding UTF8
}

function New-CleanFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) (
        "wokrouter-core-boundary-{0}" -f [Guid]::NewGuid().ToString("N")
    )
    $null = New-Item -ItemType Directory -Path $root
    Set-FixtureFile -Root $root -RelativePath "Cargo.toml" -Content @'
[workspace]
members = [
  "crates/wokrouter-storage",
  "crates/wokrouter-platform",
  "crates/wokrouter-wokcore-client",
  "apps/cli",
  "apps/desktop/src-tauri",
]

[workspace.dependencies]
reqwest = "0.13"
'@
    Set-FixtureFile `
        -Root $root `
        -RelativePath "crates/wokrouter-wokcore-client/src/lib.rs" `
        -Content 'pub struct WokCoreClient;'
    Set-FixtureFile `
        -Root $root `
        -RelativePath "apps/desktop/src/management.ts" `
        -Content 'export const capability = "providers.read";'
    return $root
}

function Invoke-Checker {
    param(
        [Parameter(Mandatory)]
        [string] $Root
    )

    & powershell.exe `
        -NoProfile `
        -ExecutionPolicy Bypass `
        -File $checker `
        -RepositoryRoot $Root *> $null
    return $LASTEXITCODE
}

$cleanRoot = New-CleanFixture
try {
    if ((Invoke-Checker -Root $cleanRoot) -ne 0) {
        throw "The clean fixture must pass."
    }
}
finally {
    Remove-Item -LiteralPath $cleanRoot -Recurse -Force
}

$cases = @(
    @{
        Name = "legacy crate directory"
        Path = "crates/wokrouter-core/src/lib.rs"
        Content = "pub struct Legacy;"
    },
    @{
        Name = "legacy workspace member"
        Path = "Cargo.toml"
        Content = @'
[workspace]
members = ["crates/wokrouter-daemon"]
'@
    },
    @{
        Name = "legacy dependency"
        Path = "apps/cli/Cargo.toml"
        Content = @'
[dependencies]
wokrouter-control = { path = "../../crates/wokrouter-control" }
'@
    },
    @{
        Name = "legacy source import"
        Path = "apps/cli/src/lib.rs"
        Content = "use wokrouter_protocols::canonical::CanonicalRequest;"
    },
    @{
        Name = "provider adapter implementation"
        Path = "apps/cli/src/provider.rs"
        Content = "struct CanonicalRequest; struct ProtocolRegistry;"
    },
    @{
        Name = "proxy route"
        Path = "apps/cli/src/server.rs"
        Content = 'router.route("/v1/responses", post(handler));'
    },
    @{
        Name = "production listener"
        Path = "apps/cli/src/server.rs"
        Content = 'use tokio::net::TcpListener;'
    },
    @{
        Name = "embedded state database"
        Path = "crates/wokrouter-storage/src/state.rs"
        Content = 'const STATE: &str = "state.sqlite3";'
    },
    @{
        Name = "frontend management token"
        Path = "apps/desktop/src/management.ts"
        Content = 'const managementToken = response.token;'
    }
)

foreach ($case in $cases) {
    $root = New-CleanFixture
    try {
        Set-FixtureFile `
            -Root $root `
            -RelativePath $case.Path `
            -Content $case.Content
        if ((Invoke-Checker -Root $root) -eq 0) {
            throw "Expected boundary violation was not reported: $($case.Name)"
        }
    }
    finally {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}

Write-Output "core boundary checker tests passed"
