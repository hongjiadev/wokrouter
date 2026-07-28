[CmdletBinding()]
param(
    [string] $Root
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}
$rootPath = (Resolve-Path -LiteralPath $Root).Path
$storageRoot = Join-Path $rootPath "crates/wokrouter-storage"
$configModel = Join-Path $storageRoot "src/config/model.rs"

if (-not (Test-Path -LiteralPath $configModel -PathType Leaf)) {
    Write-Host "PERSISTENCE INVENTORY ERROR: missing $configModel"
    exit 2
}

$forbiddenFields = @(
    "request_body",
    "response_body",
    "prompt",
    "tool_arguments",
    "authorization"
)
$findings = [System.Collections.Generic.List[string]]::new()

foreach (
    $file in Get-ChildItem `
        -LiteralPath (Join-Path $storageRoot "src") `
        -Recurse `
        -File `
        -Filter "*.rs"
) {
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    foreach ($field in $forbiddenFields) {
        if (
            $content -match (
                "(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?{0}\s*:" -f
                [regex]::Escape($field)
            )
        ) {
            $findings.Add("forbidden persisted field '$field': $($file.FullName)")
        }
    }
}

foreach (
    $file in Get-ChildItem `
        -LiteralPath $storageRoot `
        -Recurse `
        -File `
        -Filter "*.sql"
) {
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    foreach ($field in $forbiddenFields) {
        if ($content -match "(?i)\b$([regex]::Escape($field))\b") {
            $findings.Add("forbidden persisted SQL column '$field': $($file.FullName)")
        }
    }
}

if ($findings.Count -gt 0) {
    foreach ($finding in $findings | Sort-Object -Unique) {
        Write-Host $finding
    }
    exit 1
}

Write-Output "persistence privacy check passed"
