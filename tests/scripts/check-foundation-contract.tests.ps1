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
        [string]$NewText
    )

    $path = Join-Path $Root ".github/workflows/ci.yml"
    $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
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

    Invoke-Scenario -Name "broken platform aggregator is rejected" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "    needs: platform-check-matrix" `
            -NewText "    needs: frontend"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "needs" `
            -Scenario "broken platform aggregator"
    }

    Invoke-Scenario -Name "required commands in the wrong jobs are rejected" -Test {
        $root = New-ContractFixture
        $workflowPath = Join-Path $root ".github/workflows/ci.yml"
        $workflow = Get-Content -LiteralPath $workflowPath -Raw -Encoding UTF8
        $rustCommand = "        run: cargo fmt --all -- --check"
        $frontendCommand = "        run: pnpm --dir apps/desktop typecheck"
        if (-not $workflow.Contains($rustCommand) -or -not $workflow.Contains($frontendCommand)) {
            throw "Fixture command swap sources were not found."
        }
        $workflow = $workflow.Replace($rustCommand, "        run: __TASK9_SWAP__")
        $workflow = $workflow.Replace($frontendCommand, $rustCommand)
        $workflow = $workflow.Replace("        run: __TASK9_SWAP__", $frontendCommand)
        Set-Content -LiteralPath $workflowPath -Value $workflow -Encoding UTF8
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "cargo fmt" `
            -Scenario "misplaced required commands"
    }

    Invoke-Scenario -Name "broken platform matrix is rejected" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "          - macos-15" `
            -NewText "          - macos-14"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "macos-15" `
            -Scenario "broken platform matrix"
    }

    Invoke-Scenario -Name "jobs outside the jobs mapping are rejected" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "jobs:`n" `
            -NewText "not-jobs:`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "jobs" `
            -Scenario "invalid workflow structure"
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
