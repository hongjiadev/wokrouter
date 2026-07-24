[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$scriptUnderTest = Join-Path $PSScriptRoot "check-no-body-persistence.ps1"
$shell = (Get-Process -Id $PID).Path
$forbiddenFields = @(
    "request_body",
    "response_body",
    "prompt",
    "tool_arguments",
    "authorization"
)
$fixtureRoots = [System.Collections.Generic.List[string]]::new()
$fixtureBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)

function New-PrivacyFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("wokrouter-privacy-" + [guid]::NewGuid())
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/src/config") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/src/state") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/migrations") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-control/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs") -Force
    $fixtureRoots.Add($root)
    return $root
}

function Set-FixtureFile {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$RelativePath,

        [Parameter(Mandatory)]
        [string]$Content
    )

    $path = Join-Path $Root $RelativePath
    $parent = Split-Path -Parent $path
    $null = New-Item -ItemType Directory -Path $parent -Force
    Set-Content -LiteralPath $path -Value $Content -Encoding UTF8
}

function Invoke-PrivacyCheck {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $shellArguments = @("-NoProfile")
    if ($PSVersionTable.PSEdition -eq "Desktop") {
        $shellArguments += @("-ExecutionPolicy", "Bypass")
    }
    $shellArguments += @("-File", $scriptUnderTest, "-Root", $Root)

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $shell @shellArguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    return @{
        ExitCode = $exitCode
        Output = ($output | Out-String)
    }
}

function Assert-CheckPasses {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-PrivacyCheck -Root $Root
    if ($result.ExitCode -ne 0) {
        throw "$Scenario should pass, but exited $($result.ExitCode): $($result.Output)"
    }
}

function Assert-CheckRejects {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Field,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-PrivacyCheck -Root $Root
    if ($result.ExitCode -eq 0) {
        throw "$Scenario should reject persisted field '$Field', but passed."
    }
    if ($result.Output -notmatch [regex]::Escape($Field)) {
        throw "$Scenario rejected the fixture without identifying '$Field': $($result.Output)"
    }
}

try {
    $cleanRoot = New-PrivacyFixture
    Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/src/config/model.rs" -Content @'
pub struct AppConfig {
    pub port: u16,
}
'@
    Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/src/state/store.rs" -Content @'
pub struct RequestMetric {
    pub latency_ms: i64,
}
'@
    Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/migrations/0001_initial.sql" -Content @'
CREATE TABLE request_metrics(latency_ms INTEGER NOT NULL);
'@
    Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-control/src/protocol.rs" -Content @'
pub struct RequestDto {
    pub request_body: String,
    pub response_body: String,
    pub prompt: String,
    pub tool_arguments: String,
    pub authorization: String,
}
'@
    Set-FixtureFile -Root $cleanRoot -RelativePath "docs/privacy.md" -Content @'
DTO documentation may discuss request_body, response_body, prompt, tool_arguments, and authorization.
'@
    Assert-CheckPasses -Root $cleanRoot -Scenario "ordinary DTO and documentation fields"

    foreach ($field in $forbiddenFields) {
        $rustRoot = New-PrivacyFixture
        Set-FixtureFile -Root $rustRoot -RelativePath "crates/wokrouter-storage/src/config/model.rs" -Content @"
pub struct PersistedConfig {
    pub ${field}: String,
}
"@
        Assert-CheckRejects -Root $rustRoot -Field $field -Scenario "persistent Rust struct"

        $sqlRoot = New-PrivacyFixture
        Set-FixtureFile -Root $sqlRoot -RelativePath "crates/wokrouter-storage/migrations/0001_initial.sql" -Content @"
CREATE TABLE persisted_requests(id TEXT PRIMARY KEY, ${field} TEXT);
"@
        Assert-CheckRejects -Root $sqlRoot -Field $field -Scenario "SQL migration"
    }

    Write-Host "Privacy checker self-tests passed."
}
finally {
    foreach ($root in $fixtureRoots) {
        if (Test-Path -LiteralPath $root) {
            $resolvedRoot = (Resolve-Path -LiteralPath $root).Path
            $resolvedParent = [System.IO.Path]::GetFullPath(
                (Split-Path -Parent $resolvedRoot)
            ).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar,
                [System.IO.Path]::AltDirectorySeparatorChar
            )
            $leaf = Split-Path -Leaf $resolvedRoot
            if (
                -not $resolvedParent.Equals(
                    $fixtureBase,
                    [System.StringComparison]::OrdinalIgnoreCase
                ) -or
                -not $leaf.StartsWith(
                    "wokrouter-privacy-",
                    [System.StringComparison]::Ordinal
                )
            ) {
                throw "Refusing to remove unexpected privacy fixture path '$resolvedRoot'."
            }

            Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
        }
    }
}
