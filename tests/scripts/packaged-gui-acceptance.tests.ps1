[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$harness = Join-Path $PSScriptRoot "packaged-gui-acceptance.ps1"
if (-not (Test-Path -LiteralPath $harness -PathType Leaf)) {
    throw "Missing packaged GUI acceptance harness: $harness"
}

$temporaryBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)
$testRoot = Join-Path $temporaryBase (
    "wokrouter-packaged-gui-tests-" + [Guid]::NewGuid().ToString("N")
)

try {
    $null = New-Item -ItemType Directory -Path $testRoot
    $runIds = [System.Collections.Generic.List[string]]::new()

    foreach ($iteration in 1..2) {
        $evidenceRoot = Join-Path $testRoot "evidence-$iteration"
        & $harness -SelfTest -EvidenceRoot $evidenceRoot -TimeoutSeconds 10
        if ($LASTEXITCODE -ne 0) {
            throw "Packaged GUI acceptance self-test iteration $iteration failed."
        }

        $summaryPath = Join-Path $evidenceRoot "selftest-summary.json"
        if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf)) {
            throw "Self-test iteration $iteration did not write its summary."
        }
        $summary = Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8 |
            ConvertFrom-Json
        if (
            $summary.schema_version -ne 1 -or
            -not $summary.fixture_executed -or
            -not $summary.fixture_protocol_valid -or
            -not $summary.fixture_update_failures_valid -or
            -not $summary.fixture_feed_valid -or
            -not $summary.cdp_protocol_valid -or
            -not $summary.failure_path_observed -or
            -not $summary.duplicate_operation_rejected -or
            -not $summary.timeout_process_cleaned -or
            -not $summary.minisign_preflight_rejected -or
            -not $summary.scratch_root_removed
        ) {
            throw "Self-test iteration $iteration did not prove every harness contract."
        }
        if (Test-Path -LiteralPath ([string]$summary.scratch_root)) {
            throw "Self-test iteration $iteration left its scratch root behind."
        }
        $runIds.Add([string]$summary.run_id)
    }

    if ($runIds[0] -eq $runIds[1]) {
        throw "Repeated self-tests reused a run identity."
    }

    $preflightEvidence = Join-Path $testRoot "invalid-minisign-evidence"
    $missingMinisign = Join-Path $testRoot "missing-minisign.exe"
    $preflightRejected = $false
    try {
        & $harness `
            -Scenario MissingInstall `
            -DesktopExecutable (Get-Process -Id $PID).Path `
            -MinisignPath $missingMinisign `
            -EvidenceRoot $preflightEvidence `
            -TimeoutSeconds 10
    }
    catch {
        $preflightRejected = $true
        if ($_.Exception.Message -notmatch "Minisign executable") {
            throw "Invalid Minisign preflight returned the wrong error: $($_.Exception.Message)"
        }
    }
    if (-not $preflightRejected) {
        throw "A missing Minisign executable passed acceptance preflight."
    }
    if (Test-Path -LiteralPath $preflightEvidence) {
        throw "Failed preflight created an evidence directory."
    }

    $privateHeader = "minisign (encrypted )?secret key"
    $evidenceText = Get-ChildItem -LiteralPath $testRoot -Recurse -File |
        ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8 } |
        Out-String
    if ($evidenceText -match $privateHeader) {
        throw "Self-test evidence retained a Minisign private-key header."
    }

    Write-Output "packaged GUI acceptance harness tests passed"
}
finally {
    if (Test-Path -LiteralPath $testRoot) {
        $resolvedTestRoot = [System.IO.Path]::GetFullPath($testRoot)
        $leaf = [System.IO.Path]::GetFileName($resolvedTestRoot)
        $parent = [System.IO.Path]::GetDirectoryName($resolvedTestRoot).TrimEnd(
            [System.IO.Path]::DirectorySeparatorChar,
            [System.IO.Path]::AltDirectorySeparatorChar
        )
        if (
            $parent -ne $temporaryBase -or
            $leaf -cnotmatch '^wokrouter-packaged-gui-tests-[0-9a-f]{32}$'
        ) {
            throw "Refusing to remove an unexpected packaged GUI test root: $resolvedTestRoot"
        }
        Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
    }
}
