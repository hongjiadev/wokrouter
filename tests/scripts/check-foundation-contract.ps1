[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$workflowPath = Join-Path $rootPath ".github/workflows/ci.yml"
$denyPath = Join-Path $rootPath "deny.toml"
$developmentPath = Join-Path $rootPath "docs/operations/development.md"
$failures = [System.Collections.Generic.List[string]]::new()

function Assert-TextMatch {
    param(
        [Parameter(Mandatory)]
        [string]$Text,

        [Parameter(Mandatory)]
        [string]$Pattern,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if ($Text -notmatch $Pattern) {
        $failures.Add($Message)
    }
}

function Get-YamlBlock {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$Lines,

        [Parameter(Mandatory)]
        [string]$StartPattern,

        [Parameter(Mandatory)]
        [string]$EndPattern
    )

    $start = -1
    for ($index = 0; $index -lt $Lines.Count; $index += 1) {
        if ($Lines[$index] -match $StartPattern) {
            $start = $index
            break
        }
    }
    if ($start -lt 0) {
        return @()
    }

    $end = $Lines.Count
    for ($index = $start + 1; $index -lt $Lines.Count; $index += 1) {
        if ($Lines[$index] -match $EndPattern) {
            $end = $index
            break
        }
    }

    return @($Lines[$start..($end - 1)])
}

function Get-NamedStep {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$JobLines,

        [Parameter(Mandatory)]
        [string]$Name
    )

    $escapedName = [regex]::Escape($Name)
    return Get-YamlBlock `
        -Lines $JobLines `
        -StartPattern "^      - name:\s*$escapedName\s*$" `
        -EndPattern "^      - "
}

$workflowLines = @(Get-Content -LiteralPath $workflowPath -Encoding UTF8)
$workflow = $workflowLines -join "`n"
$deny = Get-Content -LiteralPath $denyPath -Raw -Encoding UTF8
$development = Get-Content -LiteralPath $developmentPath -Raw -Encoding UTF8

foreach ($job in @("rust", "frontend", "platform-check")) {
    Assert-TextMatch `
        -Text $workflow `
        -Pattern "(?m)^  $([regex]::Escape($job)):\s*$" `
        -Message "CI is missing required job id '$job'."
}

foreach ($runner in @("windows-latest", "macos-15", "ubuntu-24.04")) {
    Assert-TextMatch `
        -Text $workflow `
        -Pattern "(?m)^\s+- $([regex]::Escape($runner))\s*$" `
        -Message "CI platform matrix is missing '$runner'."
}

Assert-TextMatch -Text $workflow -Pattern "(?m)^\s+toolchain:\s*1\.97\.1\s*$" `
    -Message "CI must pin Rust 1.97.1."
Assert-TextMatch -Text $workflow -Pattern "(?m)^\s+version:\s*11\.17\.0\s*$" `
    -Message "CI must pin pnpm 11.17.0."
Assert-TextMatch -Text $workflow -Pattern "EmbarkStudios/cargo-deny-action@v2\.1\.1" `
    -Message "CI must pin cargo-deny-action v2.1.1 (cargo-deny 0.20.2)."

$platformJob = Get-YamlBlock `
    -Lines $workflowLines `
    -StartPattern "^  platform-check-matrix:\s*$" `
    -EndPattern "^  [A-Za-z0-9_-]+:\s*$"
$selfTestStep = Get-NamedStep `
    -JobLines $platformJob `
    -Name "Self-test persistence privacy checker"
$repositoryCheckStep = Get-NamedStep `
    -JobLines $platformJob `
    -Name "Check persistence privacy boundary"

if ($selfTestStep.Count -eq 0) {
    $failures.Add("Privacy checker self-test must be a dedicated CI step.")
}
elseif (
    ($selfTestStep -join "`n") -notmatch
    "(?m)^        run:\s*pwsh tests/scripts/check-no-body-persistence\.tests\.ps1\s*$"
) {
    $failures.Add("Privacy checker self-test step must run only its test script.")
}

if ($repositoryCheckStep.Count -eq 0) {
    $failures.Add("Repository privacy scan must be a dedicated CI step.")
}
elseif (
    ($repositoryCheckStep -join "`n") -notmatch
    "(?m)^        run:\s*pwsh tests/scripts/check-no-body-persistence\.ps1\s*$"
) {
    $failures.Add("Repository privacy step must run only the real scan.")
}

if (
    ($selfTestStep -join "`n") -match "continue-on-error" -or
    ($repositoryCheckStep -join "`n") -match "continue-on-error"
) {
    $failures.Add("Privacy steps must propagate their own non-zero exit status.")
}

Assert-TextMatch -Text $deny -Pattern "(?m)^yanked\s*=\s*`"deny`"\s*$" `
    -Message "deny.toml must deny yanked dependencies."
Assert-TextMatch -Text $deny -Pattern '(?m)^\s*"aarch64-apple-darwin",\s*$' `
    -Message "deny.toml must include the Apple Silicon target."

Assert-TextMatch -Text $development -Pattern "cargo-deny 0\.20\.2" `
    -Message "Development docs must pin cargo-deny 0.20.2."
Assert-TextMatch `
    -Text $development `
    -Pattern "cargo install --locked cargo-deny --version 0\.20\.2" `
    -Message "Development docs must give the CI-matching cargo-deny install command."
Assert-TextMatch -Text $development -Pattern "cargo deny --version" `
    -Message "Development docs must require cargo-deny version verification."

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "CONTRACT ERROR: $failure"
    }
    exit 1
}

Write-Host "Foundation CI/configuration contract passed."
