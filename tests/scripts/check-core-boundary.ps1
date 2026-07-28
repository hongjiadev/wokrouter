[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
}
else {
    $RepositoryRoot = (Resolve-Path -LiteralPath $RepositoryRoot).Path
}

$findings = [System.Collections.Generic.List[string]]::new()
$rootPrefix = $RepositoryRoot.TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + [System.IO.Path]::DirectorySeparatorChar

function Get-RelativePath {
    param(
        [Parameter(Mandatory)]
        [string] $Path
    )

    if (
        -not $Path.StartsWith(
            $rootPrefix,
            [System.StringComparison]::OrdinalIgnoreCase
        )
    ) {
        throw "Path is outside the repository root: $Path"
    }
    return $Path.Substring($rootPrefix.Length).Replace("\", "/")
}

$legacyCrates = @(
    "wokrouter-core",
    "wokrouter-protocols",
    "wokrouter-control",
    "wokrouter-daemon"
)

foreach ($crate in $legacyCrates) {
    $path = Join-Path $RepositoryRoot "crates/$crate"
    if (
        (Test-Path -LiteralPath $path -PathType Container) -and
        $null -ne (
            Get-ChildItem -LiteralPath $path -Recurse -File |
                Select-Object -First 1
        )
    ) {
        $findings.Add("legacy crate remains: crates/$crate")
    }
}

foreach (
    $manifest in Get-ChildItem `
        -LiteralPath $RepositoryRoot `
        -Recurse `
        -File `
        -Filter "Cargo.toml"
) {
    $content = Get-Content -LiteralPath $manifest.FullName -Raw -Encoding UTF8
    if ($content -match '(?i)wokrouter[-_](core|protocols|control|daemon)') {
        $relative = Get-RelativePath -Path $manifest.FullName
        $findings.Add("legacy workspace member or dependency: $relative")
    }
    if ($content -match '(?i)\b(rusqlite|libsqlite3-sys)\b') {
        $relative = Get-RelativePath -Path $manifest.FullName
        $findings.Add("embedded WokCore state dependency: $relative")
    }
}

$sourceFiles = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
foreach ($sourceRootName in @("apps", "crates")) {
    $sourceRoot = Join-Path $RepositoryRoot $sourceRootName
    if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
        continue
    }
    foreach (
        $file in Get-ChildItem -LiteralPath $sourceRoot -Recurse -File |
            Where-Object {
                $_.FullName -match '[\\/]src[\\/]' -and
                $_.Extension -in @(
                    ".rs", ".toml", ".json", ".ts", ".tsx", ".js", ".mjs", ".sql"
                )
            }
    ) {
        $sourceFiles.Add($file)
    }
}

foreach ($file in $sourceFiles) {
    $relative = Get-RelativePath -Path $file.FullName
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8

    if ($content -match '(?i)wokrouter[-_](core|protocols|control|daemon)') {
        $findings.Add("legacy source import: $relative")
    }
    if (
        $content -match '\b(CanonicalRequest|CanonicalEvent|ProtocolRegistry|AdapterKind)\b'
    ) {
        $findings.Add("Provider adapter or canonical proxy implementation: $relative")
    }
    if ($content -match '(?i)\broute\s*\(\s*["'']\/v1\/') {
        $findings.Add("proxy /v1 server route: $relative")
    }
    if ($content -match '\b(TcpListener|UnixListener|LocalSocketListener)\b') {
        $findings.Add("production listener implementation: $relative")
    }
    if ($content -match '(?i)\b(rusqlite|libsqlite3)\b|state\.sqlite3') {
        $findings.Add("embedded WokCore SQLite state: $relative")
    }
    if (
        $relative -like "apps/desktop/src/*" -and
        $relative -notmatch '\.test\.(ts|tsx)$' -and
        $content -match (
            '\b(managementToken|management_token|proxyToken|proxy_token)\b|' +
            '["'']token["'']\s*:|\btoken\s*:'
        )
    ) {
        $findings.Add("management token crosses into the frontend: $relative")
    }
}

if ($findings.Count -gt 0) {
    foreach ($finding in $findings | Sort-Object -Unique) {
        Write-Host "CORE BOUNDARY ERROR: $finding"
    }
    exit 1
}

Write-Output "core boundary check passed"
