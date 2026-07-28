[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$checker = Join-Path $PSScriptRoot "check-no-body-persistence.ps1"

function New-Fixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) (
        "wokrouter-persistence-{0}" -f [Guid]::NewGuid().ToString("N")
    )
    $modelDirectory = Join-Path $root "crates/wokrouter-storage/src/config"
    $null = New-Item -ItemType Directory -Path $modelDirectory -Force
    Set-Content `
        -LiteralPath (Join-Path $modelDirectory "model.rs") `
        -Encoding UTF8 `
        -Value @'
pub struct AppConfig {
    pub ui: UiConfig,
}

pub struct UiConfig {
    pub locale_override: Option<String>,
}
'@
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
        -Root $Root *> $null
    return $LASTEXITCODE
}

$clean = New-Fixture
try {
    if ((Invoke-Checker -Root $clean) -ne 0) {
        throw "The clean persistence fixture must pass."
    }
}
finally {
    Remove-Item -LiteralPath $clean -Recurse -Force
}

foreach ($field in @(
    "request_body",
    "response_body",
    "prompt",
    "tool_arguments",
    "authorization"
)) {
    $root = New-Fixture
    try {
        Add-Content `
            -LiteralPath (
                Join-Path $root "crates/wokrouter-storage/src/config/model.rs"
            ) `
            -Encoding UTF8 `
            -Value "struct Forbidden {`n    ${field}: String,`n}"
        if ((Invoke-Checker -Root $root) -eq 0) {
            throw "Expected forbidden field was not reported: $field"
        }
    }
    finally {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}

$sqlRoot = New-Fixture
try {
    $migrationDirectory = Join-Path $sqlRoot "crates/wokrouter-storage/migrations"
    $null = New-Item -ItemType Directory -Path $migrationDirectory -Force
    Set-Content `
        -LiteralPath (Join-Path $migrationDirectory "0001.sql") `
        -Encoding UTF8 `
        -Value "CREATE TABLE journal (prompt TEXT);"
    if ((Invoke-Checker -Root $sqlRoot) -eq 0) {
        throw "Expected forbidden SQL column was not reported."
    }
}
finally {
    Remove-Item -LiteralPath $sqlRoot -Recurse -Force
}

Write-Output "persistence privacy checker tests passed"
