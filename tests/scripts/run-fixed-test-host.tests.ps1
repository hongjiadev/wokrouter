[CmdletBinding()]
param(
    [switch] $SkipParentExitCodeContract
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$runner = Join-Path $PSScriptRoot "run-fixed-test-host.ps1"
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "wokrouter-fixed-test-host-{0}" -f [Guid]::NewGuid().ToString("N")
)

try {
    New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
    $fixtureSource = Join-Path $PSScriptRoot "../fixtures/run-fixed-test-host/fixed-test-artifact.rs"
    $fixtureExecutable = Join-Path $temporaryRoot "fixed-test-artifact.exe"
    $firstArtifact = Join-Path $temporaryRoot "first-artifact.exe"
    $secondArtifact = Join-Path $temporaryRoot "second-artifact.exe"
    & rustc "+1.97.1" $fixtureSource "-o" $fixtureExecutable
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to compile the fixed test host fixture with exit code $LASTEXITCODE"
    }
    Copy-Item -LiteralPath $fixtureExecutable -Destination $firstArtifact
    Copy-Item -LiteralPath $fixtureExecutable -Destination $secondArtifact

    $manifest = Join-Path $temporaryRoot "artifacts.jsonl"
    @(
        @{
            reason = "compiler-artifact"
            package_id = "fixture 0.1.0"
            target = @{ name = "first"; kind = @("test") }
            profile = @{ test = $true }
            executable = $firstArtifact
        },
        @{
            reason = "compiler-artifact"
            package_id = "fixture 0.1.0"
            target = @{ name = "second"; kind = @("test") }
            profile = @{ test = $true }
            executable = $secondArtifact
        }
    ) | ForEach-Object {
        $_ | ConvertTo-Json -Compress -Depth 5
    } | Set-Content -LiteralPath $manifest -Encoding UTF8

    $observed = Join-Path $temporaryRoot "observed-hosts.txt"
    $successfulHarnessArguments = @(
        "--observed-host",
        $observed,
        "--exit-code",
        "0"
    )
    & $runner `
        -ArtifactManifestPath $manifest `
        -TargetDirectory $temporaryRoot `
        -HarnessArguments $successfulHarnessArguments

    $hosts = @(Get-Content -LiteralPath $observed)
    if ($hosts.Count -ne 2) {
        throw "Expected two test artifacts to execute, observed $($hosts.Count)"
    }
    $fixedHost = Join-Path $temporaryRoot "wokrouter-test-host.exe"
    foreach ($hostExecutable in $hosts) {
        if ($hostExecutable -ne $fixedHost) {
            throw "Test artifact did not execute through the fixed host: $hostExecutable"
        }
    }
    if (-not (Test-Path -LiteralPath $fixedHost -PathType Leaf)) {
        throw "Fixed test host was not retained at $fixedHost"
    }

    Remove-Item -LiteralPath $observed
    $fakeCargo = Join-Path $temporaryRoot "cargo-fixture.ps1"
    $cargoInvocations = Join-Path $temporaryRoot "cargo-invocations.txt"
    @"
param([Parameter(ValueFromRemainingArguments = `$true)][string[]] `$CargoArguments)

[System.IO.File]::AppendAllText(
    '$cargoInvocations',
    ((`$CargoArguments -join ' ') + [Environment]::NewLine)
)
[Console]::Error.WriteLine('Finished fixture')
if (`$CargoArguments[1] -eq 'test') {
    Get-Content -LiteralPath '$manifest'
}
"@ | Set-Content -LiteralPath $fakeCargo -Encoding UTF8
    & $runner `
        -CargoCommand $fakeCargo `
        -RepositoryRoot $temporaryRoot `
        -TargetDirectory $temporaryRoot `
        -HarnessArguments $successfulHarnessArguments
    $compiledHosts = @(Get-Content -LiteralPath $observed)
    if ($compiledHosts.Count -ne 2) {
        throw "Expected the Cargo discovery path to execute two artifacts"
    }
    foreach ($hostExecutable in $compiledHosts) {
        if ($hostExecutable -ne $fixedHost) {
            throw "Cargo-discovered artifact bypassed the fixed host: $hostExecutable"
        }
    }
    $invocations = @(Get-Content -LiteralPath $cargoInvocations)
    if (
        $invocations.Count -ne 2 -or
        $invocations[0] -notlike "* build *-p wokrouter-cli --bin wokrouter*" -or
        $invocations[1] -notlike "* test *--no-run*"
    ) {
        throw "Expected the companion WokRouter binary to build before Cargo test discovery"
    }

    Remove-Item -LiteralPath $observed
    $singleManifest = Join-Path $temporaryRoot "single-artifact.jsonl"
    @{
        reason = "compiler-artifact"
        package_id = "fixture 0.1.0"
        target = @{ name = "single"; kind = @("test") }
        profile = @{ test = $true }
        executable = $firstArtifact
    } | ConvertTo-Json -Compress -Depth 5 |
        Set-Content -LiteralPath $singleManifest -Encoding UTF8
    & $runner `
        -ArtifactManifestPath $singleManifest `
        -TargetDirectory $temporaryRoot `
        -HarnessArguments $successfulHarnessArguments
    $singleHost = @(Get-Content -LiteralPath $observed)
    if ($singleHost.Count -ne 1 -or $singleHost[0] -ne $fixedHost) {
        throw "Expected one manifest artifact to execute through the fixed host"
    }

    Remove-Item -LiteralPath $observed
    $lockJob = Start-Job -ScriptBlock {
        param([string] $Path)

        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        try {
            Write-Output "locked"
            Start-Sleep -Milliseconds 500
        }
        finally {
            $stream.Dispose()
        }
    } -ArgumentList $fixedHost
    try {
        $locked = $false
        for ($attempt = 0; $attempt -lt 100 -and -not $locked; $attempt += 1) {
            $locked = @(Receive-Job -Job $lockJob -Keep) -contains "locked"
            if (-not $locked) {
                Start-Sleep -Milliseconds 20
            }
        }
        if (-not $locked) {
            throw "Timed out while preparing the fixed-host sharing violation fixture"
        }

        & $runner `
            -ArtifactManifestPath $singleManifest `
            -TargetDirectory $temporaryRoot `
            -HarnessArguments $successfulHarnessArguments
        $retriedHost = @(Get-Content -LiteralPath $observed)
        if ($retriedHost.Count -ne 1 -or $retriedHost[0] -ne $fixedHost) {
            throw "Expected fixed-host copy to recover from a transient sharing violation"
        }
    }
    finally {
        Wait-Job -Job $lockJob | Out-Null
        Remove-Job -Job $lockJob
    }

    $failed = $false
    try {
        & $runner `
            -ArtifactManifestPath $manifest `
            -TargetDirectory $temporaryRoot `
            -HarnessArguments @("--exit-code", "23")
    } catch {
        $failed = $_.Exception.Message -like "*exit code 23*"
    }
    if (-not $failed) {
        throw "Expected the fixed host runner to propagate test exit code 23"
    }

    if (-not $SkipParentExitCodeContract) {
        $powerShellExecutable = (Get-Process -Id $PID).Path
        $childCommand = "& '$PSCommandPath' -SkipParentExitCodeContract; exit `$LASTEXITCODE"
        & $powerShellExecutable `
            -NoProfile `
            -ExecutionPolicy Bypass `
            -Command $childCommand
        if ($LASTEXITCODE -ne 0) {
            throw "The fixed test host runner self-test must leave its parent process with exit code 0."
        }
    }

    Write-Output "fixed test host runner tests passed"
} finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}

$global:LASTEXITCODE = 0
