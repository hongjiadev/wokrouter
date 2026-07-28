[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptUnderTest = Join-Path $PSScriptRoot "check-release-contract.ps1"
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).Path
$shell = (Get-Process -Id $PID).Path
$fixtureRoots = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$scenarioCount = 0
$fixtureBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)

function New-ReleaseFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) (
        "wokrouter-release-contract-" + [guid]::NewGuid()
    )
    $null = New-Item -ItemType Directory -Path (Join-Path $root ".github/workflows") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs/operations") -Force
    foreach ($relativePath in @(
            ".github/workflows/ci.yml",
            ".github/workflows/release.yml",
            "docs/operations/development.md"
        )) {
        Copy-Item `
            -LiteralPath (Join-Path $repositoryRoot $relativePath) `
            -Destination (Join-Path $root $relativePath)
    }
    $fixtureRoots.Add($root)
    return $root
}

function Edit-FixtureFile {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$RelativePath,
        [Parameter(Mandatory)][string]$OldText,
        [Parameter(Mandatory)][AllowEmptyString()][string]$NewText
    )

    $path = Join-Path $Root $RelativePath
    $content = (Get-Content -LiteralPath $path -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $old = $OldText.Replace("`r`n", "`n")
    $new = $NewText.Replace("`r`n", "`n")
    if (-not $content.Contains($old)) {
        throw "Fixture mutation source was not found in ${RelativePath}: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($old, $new) -Encoding UTF8
}

function Invoke-Check {
    param([Parameter(Mandatory)][string]$Root)

    $arguments = @("-NoProfile")
    if ($PSVersionTable.PSEdition -eq "Desktop") {
        $arguments += @("-ExecutionPolicy", "Bypass")
    }
    $arguments += @("-File", $scriptUnderTest, "-Root", $Root)
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $shell @arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return @{ ExitCode = $exitCode; Output = ($output | Out-String) }
}

function Assert-Passes {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Scenario)

    $result = Invoke-Check -Root $Root
    if ($result.ExitCode -ne 0) {
        throw "$Scenario should pass, but exited $($result.ExitCode): $($result.Output)"
    }
}

function Assert-Rejects {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$ExpectedText,
        [Parameter(Mandatory)][string]$Scenario
    )

    $result = Invoke-Check -Root $Root
    if ($result.ExitCode -ne 1) {
        throw "$Scenario should exit 1, but exited $($result.ExitCode): $($result.Output)"
    }
    if ($result.Output -notmatch [regex]::Escape($ExpectedText)) {
        throw "$Scenario did not identify '$ExpectedText': $($result.Output)"
    }
}

function Invoke-Scenario {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][scriptblock]$Test)

    $script:scenarioCount += 1
    try {
        & $Test
        Write-Host "PASS: $Name"
    }
    catch {
        $script:failures.Add("${Name}: $($_.Exception.Message)")
        Write-Host "FAIL: $Name"
    }
}

try {
    Invoke-Scenario -Name "real release workflow satisfies the contract" -Test {
        $root = New-ReleaseFixture
        Assert-Passes -Root $root -Scenario "real release fixture"
    }

    Invoke-Scenario -Name "release matrix must retain Linux arm64" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile -Root $root -RelativePath ".github/workflows/release.yml" -OldText @"
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            extension: tar.gz
"@ -NewText ""
        Assert-Rejects -Root $root -ExpectedText "aarch64-unknown-linux-gnu" -Scenario "missing target"
    }

    Invoke-Scenario -Name "release version cannot couple to WokCore" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "      WOKROUTER_RELEASE_VERSION: `${{ needs.release-version.outputs.version }}" `
            -NewText "      WOKCORE_RELEASE_VERSION: 1.2.3"
        Assert-Rejects -Root $root -ExpectedText "WokCore version" -Scenario "WokCore version coupling"
    }

    Invoke-Scenario -Name "manual release verification must remain available" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "  workflow_dispatch:" `
            -NewText "  disabled_dispatch:"
        Assert-Rejects -Root $root -ExpectedText "manual verification" -Scenario "missing dispatch"
    }

    Invoke-Scenario -Name "manual verification must checkout the requested tag commit" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '          ref: ${{ needs.release-version.outputs.source_sha }}' `
            -NewText '          ref: ${{ github.sha }}'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "requested WokRouter tag" `
            -Scenario "release jobs checking out the dispatch branch"
    }

    Invoke-Scenario -Name "online artifact boundary cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "contains a WokCore or legacy daemon payload." `
            -NewText "contains a forbidden payload."
        Assert-Rejects `
            -Root $root `
            -ExpectedText "missing required boundary text" `
            -Scenario "missing online boundary"
    }

    Invoke-Scenario -Name "compatibility matrix must retain older same-major coverage" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "legacy_same_major_runtime_without_installation_id_remains_running" `
            -NewText "redirects_are_not_followed"
        Assert-Rejects -Root $root -ExpectedText "legacy_same_major" -Scenario "missing compatibility case"
    }

    Invoke-Scenario -Name "provider credentials must be empty in release" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '  OPENAI_API_KEY: ""' `
            -NewText "  OPENAI_API_KEY: inherited"
        Assert-Rejects -Root $root -ExpectedText "OPENAI_API_KEY" -Scenario "provider credential inheritance"
    }

    Invoke-Scenario -Name "write permission must remain publish-only" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "permissions:`n  contents: read" `
            -NewText "permissions:`n  contents: write"
        Assert-Rejects -Root $root -ExpectedText "contents: read" -Scenario "broad write permission"
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "RELEASE CONTRACT SELF-TEST ERROR: $failure"
        }
        Write-Host "Release contract self-tests failed: $($failures.Count) of $scenarioCount scenario(s)."
        exit 1
    }

    Write-Host "Release contract self-tests passed: $scenarioCount scenario(s)."
}
finally {
    foreach ($root in $fixtureRoots) {
        if (-not (Test-Path -LiteralPath $root)) {
            continue
        }
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
                "wokrouter-release-contract-",
                [System.StringComparison]::Ordinal
            )
        ) {
            throw "Refusing to remove unexpected release fixture path '$resolvedRoot'."
        }
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
