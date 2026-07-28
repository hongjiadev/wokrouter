[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$scriptUnderTest = Join-Path $PSScriptRoot "check-foundation-contract.ps1"
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).Path
$shell = (Get-Process -Id $PID).Path
$fixtureRoots = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$scenarioCount = 0
$fixtureBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)

function New-ContractFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("wokrouter-contract-" + [guid]::NewGuid())
    $null = New-Item -ItemType Directory -Path (Join-Path $root ".github/workflows") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs/operations") -Force
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot ".github/workflows/ci.yml") `
        -Destination (Join-Path $root ".github/workflows/ci.yml")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "deny.toml") `
        -Destination (Join-Path $root "deny.toml")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "docs/operations/development.md") `
        -Destination (Join-Path $root "docs/operations/development.md")
    $fixtureRoots.Add($root)
    return $root
}

function Edit-Workflow {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$OldText,

        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$NewText
    )

    $path = Join-Path $Root ".github/workflows/ci.yml"
    $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
    $content = $content.Replace("`r`n", "`n")
    $OldText = $OldText.Replace("`r`n", "`n")
    $NewText = $NewText.Replace("`r`n", "`n")
    if (-not $content.Contains($OldText)) {
        throw "Fixture mutation source was not found: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($OldText, $NewText) -Encoding UTF8
}

function Invoke-ContractCheck {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $arguments = @("-NoProfile")
    if ($PSVersionTable.PSEdition -eq "Desktop") {
        $arguments += @("-ExecutionPolicy", "Bypass")
    }
    $arguments += @("-File", $scriptUnderTest, "-Root", $Root)

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $shell @arguments 2>&1
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

function Assert-ContractPasses {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-ContractCheck -Root $Root
    if ($result.ExitCode -ne 0) {
        throw "$Scenario should pass, but exited $($result.ExitCode): $($result.Output)"
    }
}

function Assert-ContractRejects {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$ExpectedText,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-ContractCheck -Root $Root
    if ($result.ExitCode -ne 1) {
        throw "$Scenario should exit 1, but exited $($result.ExitCode): $($result.Output)"
    }
    if ($result.Output -notmatch [regex]::Escape($ExpectedText)) {
        throw "$Scenario did not identify '$ExpectedText': $($result.Output)"
    }
}

function Invoke-Scenario {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [scriptblock]$Test
    )

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
    Invoke-Scenario -Name "real workflow satisfies the structural contract" -Test {
        $root = New-ContractFixture
        Assert-ContractPasses -Root $root -Scenario "real workflow fixture"
    }

    Invoke-Scenario -Name "Windows fixed test host cannot be removed" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "          & ./tests/scripts/run-fixed-test-host.ps1 ``" `
            -NewText "          & ./tests/scripts/not-fixed-test-host.ps1 ``"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "fixed test host" `
            -Scenario "missing fixed Windows test host"
    }

    Invoke-Scenario -Name "direct Cargo tests cannot run on Windows" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "        if: runner.os != 'Windows'`n        run: cargo test --workspace --all-features --locked" `
            -NewText "        run: cargo test --workspace --all-features --locked"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "non-Windows" `
            -Scenario "unguarded direct Cargo test"
    }

    Invoke-Scenario -Name "provider credentials must remain empty" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText '  OPENAI_API_KEY: ""' `
            -NewText "  OPENAI_API_KEY: inherited"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "OPENAI_API_KEY" `
            -Scenario "non-empty provider environment"
    }

    Invoke-Scenario -Name "public hygiene gate cannot be removed" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText @"
      - name: Check public repository hygiene
        shell: pwsh
        run: pwsh tests/scripts/check-public-repo-hygiene.ps1
"@ `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "public-repo-hygiene" `
            -Scenario "missing public hygiene gate"
    }

    Invoke-Scenario -Name "WokCore boundary gate cannot be removed" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText @"
      - name: Check WokCore boundary
        shell: pwsh
        run: pwsh tests/scripts/check-core-boundary.ps1
"@ `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "core-boundary" `
            -Scenario "missing core boundary gate"
    }

    Invoke-Scenario -Name "five-target matrix cannot lose Linux arm64" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText @"
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
"@ `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "aarch64-unknown-linux-gnu" `
            -Scenario "missing Linux arm64 target"
    }

    Invoke-Scenario -Name "compatibility matrix cannot lose older same-major coverage" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "        run: cargo test -p wokrouter-wokcore-client --test handshake legacy_same_major_runtime_without_installation_id_remains_running --locked" `
            -NewText "        run: cargo test -p wokrouter-wokcore-client --test handshake redirects_are_not_followed --locked"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "legacy_same_major" `
            -Scenario "missing older same-major compatibility"
    }

    Invoke-Scenario -Name "platform aggregator cannot omit target checks" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "      - target-check-matrix`n" `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "target-check-matrix" `
            -Scenario "incomplete platform aggregator"
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "CONTRACT SELF-TEST ERROR: $failure"
        }
        Write-Host "Foundation contract self-tests failed: $($failures.Count) of $scenarioCount scenario(s)."
        exit 1
    }

    Write-Host "Foundation contract self-tests passed: $scenarioCount scenario(s)."
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
                "wokrouter-contract-",
                [System.StringComparison]::Ordinal
            )
        ) {
            throw "Refusing to remove unexpected contract fixture path '$resolvedRoot'."
        }

        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
