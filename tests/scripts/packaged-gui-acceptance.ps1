[CmdletBinding()]
param(
    [ValidateSet(
        "All",
        "MissingInstall",
        "UpdateCancelConfirm",
        "ActiveRequests",
        "Rollback",
        "CloseReopen",
        "Locale"
    )]
    [string]$Scenario = "All",

    [string]$DesktopExecutable,

    [string]$MinisignPath,

    [Parameter(Mandatory)]
    [string]$EvidenceRoot,

    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 180,

    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Resolve-RegularExecutablePath {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description is missing: $Path"
    }
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $item = Get-Item -LiteralPath $resolved -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description must not be a reparse point: $resolved"
    }
    return [IO.Path]::GetFullPath($resolved)
}

function Write-Utf8Json {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [object]$Value
    )

    $json = $Value | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

function Get-OptionalObjectProperty {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory)][string]$Name
    )

    if ($null -eq $Value) {
        return $null
    }
    $property = $Value.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Wait-PathUntil {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [DateTime]$Deadline
    )

    while ([DateTime]::UtcNow -lt $Deadline) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            return
        }
        Start-Sleep -Milliseconds 20
    }
    throw "Timed out waiting for fixture evidence: $Path"
}

function Stop-OwnedProcess {
    param(
        [Parameter(Mandatory)]
        [Diagnostics.Process]$Process,

        [Parameter(Mandatory)]
        [string]$ExpectedExecutable
    )

    if ($Process.HasExited) {
        return
    }
    $observed = Get-Process -Id $Process.Id -ErrorAction SilentlyContinue
    if (
        $null -eq $observed -or
        $null -eq $observed.Path -or
        [IO.Path]::GetFullPath($observed.Path) -ine [IO.Path]::GetFullPath($ExpectedExecutable)
    ) {
        throw "Refusing to stop a process whose executable identity does not match the fixture."
    }
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit()
}

function Remove-OwnedScratchRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$TemporaryBase
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($resolved).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $leaf = [IO.Path]::GetFileName($resolved)
    $rootItem = Get-Item -LiteralPath $resolved -Force
    if (
        $parent -ne $TemporaryBase -or
        $leaf -cnotmatch '^wokrouter-packaged-gui-selftest-[0-9a-f]{32}$' -or
        ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw "Refusing to remove an unexpected packaged GUI scratch root: $resolved"
    }
    $reparseChildren = @(Get-ChildItem -LiteralPath $resolved -Force -Recurse | Where-Object {
            ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        } | Sort-Object { $_.FullName.Length } -Descending)
    foreach ($child in $reparseChildren) {
        Remove-Item -LiteralPath $child.FullName -Force
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

function Start-FixtureProcess {
    param(
        [Parameter(Mandatory)]
        [string]$Executable,

        [string[]]$Arguments = @()
    )

    return Start-Process `
        -FilePath $Executable `
        -ArgumentList $Arguments `
        -WindowStyle Hidden `
        -PassThru
}

function Invoke-HarnessSelfTest {
    param(
        [Parameter(Mandatory)]
        [string]$OutputRoot,

        [Parameter(Mandatory)]
        [int]$Timeout
    )

    if (Test-Path -LiteralPath $OutputRoot) {
        throw "Self-test evidence root must not already exist: $OutputRoot"
    }
    $outputParent = Split-Path -Parent ([IO.Path]::GetFullPath($OutputRoot))
    if (-not (Test-Path -LiteralPath $outputParent -PathType Container)) {
        $null = New-Item -ItemType Directory -Path $outputParent -Force
    }
    $null = New-Item -ItemType Directory -Path $OutputRoot

    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $runId = [Guid]::NewGuid().ToString("N")
    $scratchRoot = Join-Path $temporaryBase "wokrouter-packaged-gui-selftest-$runId"
    $fixture = Join-Path $scratchRoot "acceptance-selftest-fixture.exe"
    $source = Join-Path $PSScriptRoot "../fixtures/packaged-gui-acceptance/selftest-fixture.rs"
    $wokCoreFixture = Join-Path $scratchRoot "wokcore-fixture.exe"
    $wokCoreSource = Join-Path `
        $PSScriptRoot `
        "../fixtures/packaged-gui-acceptance/wokcore-fixture.rs"
    $processes = [System.Collections.Generic.List[Diagnostics.Process]]::new()
    $feedProcess = $null
    $fixtureExecuted = $false
    $fixtureProtocolValid = $false
    $fixtureUpdateFailuresValid = $false
    $fixtureFeedValid = $false
    $cdpProtocolValid = $false
    $failureObserved = $false
    $duplicateRejected = $false
    $timeoutCleaned = $false
    $minisignRejected = $false
    $scratchRemoved = $false

    try {
        $null = New-Item -ItemType Directory -Path $scratchRoot
        $source = (Resolve-Path -LiteralPath $source).Path
        & rustup.exe run 1.97.1 rustc.exe --edition 2024 $source -o $fixture
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to compile the packaged GUI self-test fixture."
        }
        $wokCoreSource = (Resolve-Path -LiteralPath $wokCoreSource).Path
        & rustup.exe run 1.97.1 rustc.exe --edition 2024 $wokCoreSource -o $wokCoreFixture
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to compile the packaged GUI WokCore fixture."
        }

        $successEnvelope = Resolve-CdpResponseEnvelope `
            -Response ('{"id":7,"result":{"result":{"type":"boolean","value":true}}}' |
                ConvertFrom-Json) `
            -ExpectedId 7
        $notificationEnvelope = Resolve-CdpResponseEnvelope `
            -Response ('{"method":"Page.loadEventFired","params":{}}' | ConvertFrom-Json) `
            -ExpectedId 7
        $otherEnvelope = Resolve-CdpResponseEnvelope `
            -Response ('{"id":6,"result":{}}' | ConvertFrom-Json) `
            -ExpectedId 7
        $cdpProtocolValid = (
            $successEnvelope.Kind -ceq "CommandResult" -and
            $successEnvelope.Result.result.value -eq $true -and
            $notificationEnvelope.Kind -ceq "Notification" -and
            $notificationEnvelope.Method -ceq "Page.loadEventFired" -and
            $otherEnvelope.Kind -ceq "OtherResponse"
        )

        $marker = Join-Path $scratchRoot "fixture-ran"
        $process = Start-FixtureProcess -Executable $fixture -Arguments @(
            "--marker", $marker,
            "--exit-code", "0"
        )
        $processes.Add($process)
        if (-not $process.WaitForExit($Timeout * 1000)) {
            throw "The packaged GUI self-test fixture did not exit before timeout."
        }
        Wait-PathUntil -Path $marker -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        $fixtureExecuted = $process.ExitCode -eq 0

        $failedProcess = Start-FixtureProcess -Executable $fixture -Arguments @(
            "--exit-code", "23"
        )
        $processes.Add($failedProcess)
        if (-not $failedProcess.WaitForExit($Timeout * 1000)) {
            throw "The failing packaged GUI fixture did not exit before timeout."
        }
        $failureObserved = $failedProcess.ExitCode -eq 23

        $lease = Join-Path $scratchRoot "operation.lease"
        $ready = Join-Path $scratchRoot "operation-ready"
        $release = Join-Path $scratchRoot "operation-release"
        $leaseOwner = Start-FixtureProcess -Executable $fixture -Arguments @(
            "--lease", $lease,
            "--ready", $ready,
            "--release", $release
        )
        $processes.Add($leaseOwner)
        Wait-PathUntil -Path $ready -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        $duplicate = Start-FixtureProcess -Executable $fixture -Arguments @(
            "--lease", $lease
        )
        $processes.Add($duplicate)
        if (-not $duplicate.WaitForExit($Timeout * 1000)) {
            throw "The duplicate packaged GUI fixture did not exit before timeout."
        }
        $duplicateRejected = $duplicate.ExitCode -eq 73
        [IO.File]::WriteAllText($release, "release", [Text.UTF8Encoding]::new($false))
        if (-not $leaseOwner.WaitForExit($Timeout * 1000)) {
            throw "The packaged GUI fixture lease owner did not exit before timeout."
        }

        $timedOut = Start-FixtureProcess -Executable $fixture -Arguments @(
            "--sleep-ms", (($Timeout + 10) * 1000).ToString()
        )
        $processes.Add($timedOut)
        Start-Sleep -Milliseconds 100
        Stop-OwnedProcess -Process $timedOut -ExpectedExecutable $fixture
        $timeoutCleaned = $timedOut.HasExited

        try {
            $null = Resolve-RegularExecutablePath `
                -Path (Join-Path $scratchRoot "missing-minisign.exe") `
                -Description "Minisign executable"
        }
        catch {
            $minisignRejected = $_.Exception.Message -match "Minisign executable is missing"
        }

        $fixtureState = Join-Path $scratchRoot "wokcore-state"
        $fixtureLocal = Join-Path $scratchRoot "local"
        $null = New-Item -ItemType Directory -Path $fixtureState
        $null = New-Item -ItemType Directory -Path $fixtureLocal
        $previousStateRoot = $env:WOKROUTER_ACCEPTANCE_STATE_ROOT
        $previousLocalAppData = $env:LOCALAPPDATA
        try {
            $env:WOKROUTER_ACCEPTANCE_STATE_ROOT = $fixtureState
            $env:LOCALAPPDATA = $fixtureLocal
            $authorizationArguments = @(
                "authorize",
                "--client", "wokrouter.desktop",
                "--scope", "service.read",
                "--scope", "service.control",
                "--scope", "providers.read",
                "--scope", "providers.write",
                "--scope", "clients.manage",
                "--scope", "sessions.read",
                "--scope", "usage.read",
                "--scope", "diagnostics.read",
                "--scope", "diagnostics.export",
                "--json"
            )
            $authorization = (& $wokCoreFixture @authorizationArguments | Out-String) |
                ConvertFrom-Json
            $authorizationExit = $LASTEXITCODE
            $fixtureProtocolValid = (
                $authorizationExit -eq 0 -and
                $authorization.client_id -ceq "wokrouter.desktop" -and
                $authorization.token_id -ceq "acceptance-token" -and
                ([string]$authorization.token).Length -eq 56 -and
                @($authorization.scopes).Count -eq 9
            )

            $check = (& $wokCoreFixture update --check --json | Out-String) |
                ConvertFrom-Json
            $fixtureProtocolValid = $fixtureProtocolValid -and
                $LASTEXITCODE -eq 0 -and
                $check.code -ceq "update_available" -and
                $check.current_version -ceq "1.0.0" -and
                $check.version -ceq "2.0.0"

            $activeProgress = Join-Path $scratchRoot "active-progress.jsonl"
            $activeOutput = Join-Path $scratchRoot "active-output.json"
            [IO.File]::WriteAllText(
                (Join-Path $fixtureState "scenario.txt"),
                "active_requests",
                [Text.UTF8Encoding]::new($false)
            )
            $activeProcess = Start-Process `
                -FilePath $wokCoreFixture `
                -ArgumentList @("update", "--install", "--json", "--progress-jsonl") `
                -RedirectStandardOutput $activeOutput `
                -RedirectStandardError $activeProgress `
                -WindowStyle Hidden `
                -Wait `
                -PassThru
            $activeExit = $activeProcess.ExitCode
            $activeProcess.Dispose()
            $activeTerminal = Get-Content -LiteralPath $activeProgress -Tail 1 -Encoding UTF8 |
                ConvertFrom-Json

            $rollbackProgress = Join-Path $scratchRoot "rollback-progress.jsonl"
            $rollbackOutput = Join-Path $scratchRoot "rollback-output.json"
            [IO.File]::WriteAllText(
                (Join-Path $fixtureState "scenario.txt"),
                "rollback",
                [Text.UTF8Encoding]::new($false)
            )
            $rollbackProcess = Start-Process `
                -FilePath $wokCoreFixture `
                -ArgumentList @("update", "--install", "--json", "--progress-jsonl") `
                -RedirectStandardOutput $rollbackOutput `
                -RedirectStandardError $rollbackProgress `
                -WindowStyle Hidden `
                -Wait `
                -PassThru
            $rollbackExit = $rollbackProcess.ExitCode
            $rollbackProcess.Dispose()
            $rollbackTerminal = Get-Content -LiteralPath $rollbackProgress -Tail 1 -Encoding UTF8 |
                ConvertFrom-Json
            $fixtureUpdateFailuresValid = (
                $activeExit -eq 71 -and
                $activeTerminal.error_code -ceq "active_requests_remain" -and
                $activeTerminal.active_requests -eq 2 -and
                $rollbackExit -eq 72 -and
                $rollbackTerminal.error_code -ceq "rolled_back" -and
                $rollbackTerminal.phase -ceq "rolling_back"
            )

            $feedRoot = Join-Path $scratchRoot "feed"
            $feedReady = Join-Path $scratchRoot "feed-ready"
            $null = New-Item -ItemType Directory -Path $feedRoot
            [IO.File]::WriteAllText(
                (Join-Path $feedRoot "probe.txt"),
                "signed-feed-probe",
                [Text.UTF8Encoding]::new($false)
            )
            $feedProcess = Start-FixtureProcess -Executable $wokCoreFixture -Arguments @(
                "feed", "--root", $feedRoot, "--ready", $feedReady, "--port", "0"
            )
            Wait-PathUntil `
                -Path $feedReady `
                -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
            $feedPort = [int](Get-Content -LiteralPath $feedReady -Raw -Encoding UTF8)
            $feedResponse = Invoke-WebRequest `
                -UseBasicParsing `
                -Uri "http://127.0.0.1:$feedPort/releases/probe.txt"
            $feedContent = if ($feedResponse.Content -is [byte[]]) {
                [Text.Encoding]::UTF8.GetString($feedResponse.Content)
            }
            else {
                [string]$feedResponse.Content
            }
            $fixtureFeedValid = (
                $feedResponse.StatusCode -eq 200 -and
                $feedContent -ceq "signed-feed-probe"
            )
            Stop-OwnedProcess -Process $feedProcess -ExpectedExecutable $wokCoreFixture
            $feedProcess.Dispose()
            $feedProcess = $null
        }
        finally {
            $env:WOKROUTER_ACCEPTANCE_STATE_ROOT = $previousStateRoot
            $env:LOCALAPPDATA = $previousLocalAppData
        }

        if (
            -not $fixtureExecuted -or
            -not $fixtureProtocolValid -or
            -not $fixtureUpdateFailuresValid -or
            -not $fixtureFeedValid -or
            -not $cdpProtocolValid -or
            -not $failureObserved -or
            -not $duplicateRejected -or
            -not $timeoutCleaned -or
            -not $minisignRejected
        ) {
            throw (
                "The packaged GUI self-test did not exercise every required contract: " +
                "fixture=$fixtureExecuted protocol=$fixtureProtocolValid " +
                "update_failures=$fixtureUpdateFailuresValid feed=$fixtureFeedValid " +
                "cdp=$cdpProtocolValid " +
                "failure=$failureObserved duplicate=$duplicateRejected " +
                "timeout=$timeoutCleaned minisign=$minisignRejected"
            )
        }
    }
    finally {
        if ($null -ne $feedProcess) {
            if (-not $feedProcess.HasExited -and (Test-Path -LiteralPath $wokCoreFixture -PathType Leaf)) {
                Stop-OwnedProcess -Process $feedProcess -ExpectedExecutable $wokCoreFixture
            }
            $feedProcess.Dispose()
        }
        foreach ($process in $processes) {
            if (-not $process.HasExited -and (Test-Path -LiteralPath $fixture -PathType Leaf)) {
                Stop-OwnedProcess -Process $process -ExpectedExecutable $fixture
            }
            $process.Dispose()
        }
        Remove-OwnedScratchRoot -Path $scratchRoot -TemporaryBase $temporaryBase
        $scratchRemoved = -not (Test-Path -LiteralPath $scratchRoot)
    }

    Write-Utf8Json `
        -Path (Join-Path $OutputRoot "selftest-summary.json") `
        -Value ([ordered]@{
            schema_version = 1
            run_id = $runId
            fixture_executed = $fixtureExecuted
            fixture_protocol_valid = $fixtureProtocolValid
            fixture_update_failures_valid = $fixtureUpdateFailuresValid
            fixture_feed_valid = $fixtureFeedValid
            cdp_protocol_valid = $cdpProtocolValid
            failure_path_observed = $failureObserved
            duplicate_operation_rejected = $duplicateRejected
            timeout_process_cleaned = $timeoutCleaned
            minisign_preflight_rejected = $minisignRejected
            scratch_root = $scratchRoot
            scratch_root_removed = $scratchRemoved
        })
    Write-Output "Packaged GUI acceptance harness self-test passed."
}

function Get-FreeLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Get-IsolatedWindowsPath {
    $windows = (Resolve-Path -LiteralPath ([IO.Path]::GetFullPath($env:SystemRoot))).Path
    $system32 = (Resolve-Path -LiteralPath (Join-Path $windows "System32")).Path
    $directories = @($system32, $windows)
    foreach ($directory in $directories) {
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
            throw "Required isolated Windows PATH directory is missing: $directory"
        }
        if (Test-Path -LiteralPath (Join-Path $directory "wokcore.exe") -PathType Leaf) {
            throw "The isolated Windows PATH unexpectedly contains wokcore.exe: $directory"
        }
    }
    return [string]::Join([IO.Path]::PathSeparator, $directories)
}

function Get-CdpStartupDiagnostic {
    param(
        [Parameter(Mandatory)][Diagnostics.Process]$DesktopProcess,
        [Parameter(Mandatory)][int]$Port,
        [AllowNull()][string]$LastJsonListError
    )

    $DesktopProcess.Refresh()
    $processQueryError = $null
    $webViewProcesses = @()
    try {
        $processes = @(Get-CimInstance -ClassName Win32_Process)
        $descendantIds = [Collections.Generic.HashSet[int]]::new()
        $null = $descendantIds.Add([int]$DesktopProcess.Id)
        do {
            $added = $false
            foreach ($process in $processes) {
                if (
                    $descendantIds.Contains([int]$process.ParentProcessId) -and
                    $descendantIds.Add([int]$process.ProcessId)
                ) {
                    $added = $true
                }
            }
        } while ($added)
        $webViewProcesses = @($processes |
                Where-Object {
                    $_.Name -ieq "msedgewebview2.exe" -and
                    $_.ProcessId -ne $DesktopProcess.Id -and
                    $descendantIds.Contains([int]$_.ProcessId)
                } |
                Sort-Object ProcessId |
                ForEach-Object {
                    [ordered]@{
                        process_id = [int]$_.ProcessId
                        parent_process_id = [int]$_.ParentProcessId
                        executable_path = [string]$_.ExecutablePath
                        command_line = [string]$_.CommandLine
                    }
                })
    }
    catch {
        $processQueryError = $_.Exception.ToString()
    }

    $portQueryError = $null
    $portOwners = @()
    try {
        $portOwners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop |
                Select-Object -ExpandProperty OwningProcess -Unique |
                Sort-Object)
    }
    catch {
        $portQueryError = $_.Exception.ToString()
    }

    $exitCode = $null
    if ($DesktopProcess.HasExited) {
        $exitCode = [int]$DesktopProcess.ExitCode
    }
    return [ordered]@{
        schema_version = 1
        desktop_process_id = [int]$DesktopProcess.Id
        desktop_has_exited = [bool]$DesktopProcess.HasExited
        desktop_exit_code = $exitCode
        remote_debugging_port = $Port
        listening_owner_process_ids = @($portOwners)
        port_query_error = $portQueryError
        last_json_list_error = $LastJsonListError
        descendant_msedgewebview2 = @($webViewProcesses)
        process_query_error = $processQueryError
    }
}

function Remove-OwnedLiveScratchRoot {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $resolved = [IO.Path]::GetFullPath($Path)
    $item = Get-Item -LiteralPath $resolved -Force
    if (
        [IO.Path]::GetDirectoryName($resolved).TrimEnd(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        ) -ne $temporaryBase -or
        [IO.Path]::GetFileName($resolved) -cnotmatch
            '^wokrouter-packaged-gui-live-[0-9a-f]{32}$' -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw "Refusing to remove an unexpected live acceptance root: $resolved"
    }
    foreach ($child in @(
            Get-ChildItem -LiteralPath $resolved -Force -Recurse |
                Where-Object {
                    ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
                } |
                Sort-Object { $_.FullName.Length } -Descending
        )) {
        Remove-Item -LiteralPath $child.FullName -Force
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

function Stop-OwnedProcessesByExecutable {
    param([Parameter(Mandatory)][string[]]$Executables)

    $allowed = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($executable in $Executables) {
        if (-not [string]::IsNullOrWhiteSpace($executable)) {
            $null = $allowed.Add([IO.Path]::GetFullPath($executable))
        }
    }
    $stopped = [Collections.Generic.List[int]]::new()
    $stableRounds = 0
    $rounds = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ($stableRounds -lt 10 -and [DateTime]::UtcNow -lt $deadline) {
        $rounds += 1
        $matched = $false
        foreach ($process in Get-Process) {
            $path = $null
            try {
                $path = $process.Path
            }
            catch {
                continue
            }
            if (
                $null -ne $path -and
                $allowed.Contains([IO.Path]::GetFullPath($path)) -and
                -not $process.HasExited
            ) {
                $matched = $true
                $stopped.Add([int]$process.Id)
                Stop-OwnedProcess `
                    -Process $process `
                    -ExpectedExecutable ([IO.Path]::GetFullPath($path))
            }
        }
        if ($matched) {
            $stableRounds = 0
        }
        else {
            $stableRounds += 1
        }
        if ($stableRounds -lt 10) {
            Start-Sleep -Milliseconds 100
        }
    }
    if ($stableRounds -lt 10) {
        throw "Owned acceptance processes did not become quiescent before cleanup."
    }
    return [pscustomobject]@{
        Rounds = $rounds
        StableRounds = $stableRounds
        StoppedProcessIds = @($stopped | Select-Object -Unique)
    }
}

function Stop-RecordedAcceptanceWebViewProcesses {
    param(
        [Parameter(Mandatory)][object[]]$Processes,
        [Parameter(Mandatory)][string]$UserDataRoot,
        [Parameter(Mandatory)][int]$RemoteDebuggingPort
    )

    $canonicalRoot = [IO.Path]::GetFullPath($UserDataRoot)
    $terminated = [Collections.Generic.List[int]]::new()
    $waitRounds = 0
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $waitRounds += 1
        $remaining = @($Processes | Where-Object {
                $null -ne (Get-Process -Id ([int]$_.process_id) -ErrorAction SilentlyContinue)
            })
        if ($remaining.Count -eq 0) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    foreach ($record in $remaining) {
        $process = Get-CimInstance `
            -ClassName Win32_Process `
            -Filter "ProcessId = $([int]$record.process_id)" `
            -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            continue
        }
        $commandLine = [string]$process.CommandLine
        if (
            $process.Name -ine "msedgewebview2.exe" -or
            $commandLine.IndexOf($canonicalRoot, [StringComparison]::OrdinalIgnoreCase) -lt 0 -or
            $commandLine -notlike "*--remote-debugging-port=$RemoteDebuggingPort*"
        ) {
            throw "Refusing to terminate a WebView2 process without the recorded acceptance identity."
        }
        Stop-Process -Id ([int]$process.ProcessId) -Force
        $terminated.Add([int]$process.ProcessId)
    }
    foreach ($record in $Processes) {
        $process = Get-Process -Id ([int]$record.process_id) -ErrorAction SilentlyContinue
        if ($null -ne $process) {
            $process.WaitForExit(5000) | Out-Null
        }
        if ($null -ne (Get-Process -Id ([int]$record.process_id) -ErrorAction SilentlyContinue)) {
            throw "A recorded acceptance WebView2 process survived cleanup."
        }
    }
    return [pscustomobject]@{
        WaitRounds = $waitRounds
        RecordedProcessIds = @($Processes | ForEach-Object { [int]$_.process_id })
        TerminatedProcessIds = @($terminated)
    }
}

function New-SignedWokCoreFeed {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$FixtureExecutable,
        [Parameter(Mandatory)][string]$MinisignExecutable
    )

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $feed = Join-Path $Root "feed"
    $archiveInput = Join-Path $Root "archive-input"
    $null = New-Item -ItemType Directory -Path $feed
    $null = New-Item -ItemType Directory -Path $archiveInput
    [IO.File]::Copy(
        $FixtureExecutable,
        (Join-Path $archiveInput "wokcore.exe"),
        $false
    )
    $version = "1.0.0"
    $archiveName = "WokCore-v$version-Windows-x86_64-Portable.zip"
    $archivePath = Join-Path $feed $archiveName
    [IO.Compression.ZipFile]::CreateFromDirectory(
        $archiveInput,
        $archivePath,
        [IO.Compression.CompressionLevel]::Optimal,
        $false
    )
    $archive = Get-Item -LiteralPath $archivePath
    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()

    $publicKey = Join-Path $Root "acceptance-minisign.pub"
    $secretKey = Join-Path $Root "acceptance-minisign.key"
    & $MinisignExecutable -G -W -f -p $publicKey -s $secretKey | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to generate the ephemeral packaged acceptance signing key."
    }
    try {
        $publicLines = @(
            (Get-Content -LiteralPath $publicKey -Encoding UTF8) |
                Where-Object { $_ -cne "" }
        )
        if (
            $publicLines.Count -ne 2 -or
            $publicLines[0] -cnotmatch
                '^untrusted comment: minisign public key (?<id>[0-9A-F]{16})$'
        ) {
            throw "The ephemeral Minisign public key is not canonical."
        }
        $keyId = $Matches.id
        $targets = @(
            @("x86_64-pc-windows-msvc", "Windows", "x86_64", "Portable.zip", "wokcore.exe"),
            @("aarch64-pc-windows-msvc", "Windows", "arm64", "Portable.zip", "wokcore.exe"),
            @("x86_64-apple-darwin", "macOS", "x86_64", "tar.gz", "wokcore"),
            @("aarch64-apple-darwin", "macOS", "arm64", "tar.gz", "wokcore"),
            @("x86_64-unknown-linux-gnu", "Linux", "x86_64", "tar.gz", "wokcore"),
            @("aarch64-unknown-linux-gnu", "Linux", "arm64", "tar.gz", "wokcore")
        )
        $artifacts = foreach ($target in $targets) {
            $file = if ($target[1] -ceq "Windows") {
                "WokCore-v$version-$($target[1])-$($target[2])-$($target[3])"
            }
            else {
                "WokCore-v$version-$($target[1])-$($target[2]).$($target[3])"
            }
            [ordered]@{
                target = $target[0]
                file = $file
                executable = $target[4]
                size = [long]$archive.Length
                sha256 = $archiveHash
                url = "https://github.com/hongjiadev/wokcore/releases/download/v$version/$file"
            }
        }
        $manifestPath = Join-Path $feed "wokcore-update-v2.json"
        $manifest = [ordered]@{
            schema_version = 2
            product = "wokcore"
            api_major = 1
            version = $version
            signing_key_id = $keyId
            artifacts = @($artifacts)
        } | ConvertTo-Json -Compress -Depth 8
        [IO.File]::WriteAllText(
            $manifestPath,
            $manifest,
            [Text.UTF8Encoding]::new($false)
        )
        $signaturePath = "$manifestPath.minisig"
        & $MinisignExecutable `
            -S `
            -W `
            -s $secretKey `
            -m $manifestPath `
            -x $signaturePath `
            -c "WokRouter packaged GUI acceptance" `
            -t "ephemeral local fixture" `
            -q
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to sign the packaged acceptance WokCore manifest."
        }
        if (@(Get-Content -LiteralPath $signaturePath -Encoding UTF8).Count -ne 4) {
            throw "The packaged acceptance manifest signature is not canonical."
        }
    }
    finally {
        if (Test-Path -LiteralPath $secretKey -PathType Leaf) {
            Remove-Item -LiteralPath $secretKey -Force
        }
        if (Test-Path -LiteralPath $secretKey) {
            throw "The ephemeral packaged acceptance secret key was not removed."
        }
    }
    return [pscustomobject]@{
        Root = $feed
        PublicKey = $publicKey
        KeyId = $keyId
        Version = $version
        ArchiveName = $archiveName
        ArchivePath = $archivePath
        ArchiveSize = [long]$archive.Length
        ArchiveSha256 = $archiveHash
    }
}

function Build-LiveAcceptanceApplication {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Desktop
    )

    $application = Join-Path $Root "application"
    $null = New-Item -ItemType Directory -Path $application
    $fixtureSource = (Resolve-Path -LiteralPath (
            Join-Path $PSScriptRoot "../fixtures/packaged-gui-acceptance/wokcore-fixture.rs"
        )).Path
    $fixture = Join-Path $Root "wokcore-fixture.exe"
    & rustup.exe run 1.97.1 rustc.exe `
        --edition 2024 `
        -C opt-level=2 `
        $fixtureSource `
        -o $fixture
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to compile the live packaged GUI WokCore fixture."
    }
    & cargo +1.97.1 build `
        -p wokrouter-cli `
        --bin wokrouter-packaged-acceptance `
        --no-default-features `
        --features packaged-acceptance `
        --release `
        --locked `
        --offline
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to build the packaged acceptance CLI."
    }
    $acceptanceCli = Resolve-RegularExecutablePath `
        -Path (Join-Path $PSScriptRoot "../../target/release/wokrouter-packaged-acceptance.exe") `
        -Description "Packaged acceptance CLI"
    & cargo +1.97.1 build `
        -p wokrouter-desktop `
        --bin wokrouter-desktop `
        --no-default-features `
        --features packaged-acceptance `
        --release `
        --locked `
        --offline
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to build the packaged acceptance desktop."
    }
    $acceptanceDesktop = Resolve-RegularExecutablePath `
        -Path (Join-Path $PSScriptRoot "../../target/release/wokrouter-desktop.exe") `
        -Description "Packaged acceptance desktop"
    $desktopCopy = Join-Path $application "wokrouter-desktop.exe"
    $sidecarCopy = Join-Path $application "wokrouter.exe"
    $productionDesktopSha256 = (
        Get-FileHash -LiteralPath $Desktop -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $acceptanceDesktopSha256 = (
        Get-FileHash -LiteralPath $acceptanceDesktop -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    if ($productionDesktopSha256 -ceq $acceptanceDesktopSha256) {
        throw "The packaged acceptance desktop must differ from the normal release control."
    }
    [IO.File]::Copy($acceptanceDesktop, $desktopCopy, $false)
    [IO.File]::Copy($acceptanceCli, $sidecarCopy, $false)
    return [pscustomobject]@{
        ApplicationRoot = $application
        Desktop = $desktopCopy
        Sidecar = $sidecarCopy
        Fixture = $fixture
        FixtureSha256 = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
        ProductionDesktopSha256 = $productionDesktopSha256
        AcceptanceDesktopSha256 = $acceptanceDesktopSha256
    }
}

function Receive-CdpJson {
    param(
        [Parameter(Mandatory)][Net.WebSockets.ClientWebSocket]$Socket,
        [Parameter(Mandatory)][int]$TimeoutMilliseconds
    )

    $buffer = [byte[]]::new(64 * 1024)
    $output = [IO.MemoryStream]::new()
    $cancellation = [Threading.CancellationTokenSource]::new($TimeoutMilliseconds)
    try {
        do {
            $result = $Socket.ReceiveAsync(
                [ArraySegment[byte]]::new($buffer),
                $cancellation.Token
            ).GetAwaiter().GetResult()
            if ($result.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) {
                throw "The packaged desktop CDP socket closed unexpectedly."
            }
            $output.Write($buffer, 0, $result.Count)
        } while (-not $result.EndOfMessage)
        return [Text.Encoding]::UTF8.GetString($output.ToArray()) |
            ConvertFrom-Json
    }
    catch [OperationCanceledException] {
        throw "Timed out receiving a packaged desktop CDP response."
    }
    finally {
        $cancellation.Dispose()
        $output.Dispose()
    }
}

function Resolve-CdpResponseEnvelope {
    param(
        [Parameter(Mandatory)][object]$Response,
        [Parameter(Mandatory)][int]$ExpectedId
    )

    $responseId = $Response.PSObject.Properties["id"]
    if ($null -eq $responseId) {
        $method = $Response.PSObject.Properties["method"]
        return [pscustomobject]@{
            Kind = "Notification"
            Method = if ($null -eq $method) { $null } else { [string]$method.Value }
            Result = $null
        }
    }
    if ([int]$responseId.Value -ne $ExpectedId) {
        return [pscustomobject]@{
            Kind = "OtherResponse"
            Method = $null
            Result = $null
        }
    }
    $responseError = $Response.PSObject.Properties["error"]
    if ($null -ne $responseError -and $null -ne $responseError.Value) {
        throw "CDP command response was rejected."
    }
    $responseResult = $Response.PSObject.Properties["result"]
    if ($null -eq $responseResult) {
        throw "CDP command response did not contain a result."
    }
    $exceptionDetails = $responseResult.Value.PSObject.Properties["exceptionDetails"]
    if ($null -ne $exceptionDetails -and $null -ne $exceptionDetails.Value) {
        throw "CDP command response contained a JavaScript exception."
    }
    return [pscustomobject]@{
        Kind = "CommandResult"
        Method = $null
        Result = $responseResult.Value
    }
}

function Invoke-CdpCommand {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Method,
        [hashtable]$Parameters = @{}
    )

    $Connection.NextId = [int]$Connection.NextId + 1
    $id = [int]$Connection.NextId
    $request = [ordered]@{
        id = $id
        method = $Method
        params = $Parameters
    } | ConvertTo-Json -Compress -Depth 12
    $bytes = [Text.Encoding]::UTF8.GetBytes($request)
    $Connection.Socket.SendAsync(
        [ArraySegment[byte]]::new($bytes),
        [Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult() | Out-Null
    do {
        $response = Receive-CdpJson `
            -Socket $Connection.Socket `
            -TimeoutMilliseconds $Connection.TimeoutMilliseconds
        $envelope = Resolve-CdpResponseEnvelope -Response $response -ExpectedId $id
        if ($envelope.Kind -ceq "Notification") {
            if (-not [string]::IsNullOrWhiteSpace($envelope.Method)) {
                $Connection.NotificationMethods.Add([string]$envelope.Method)
            }
            continue
        }
    } while ($envelope.Kind -cne "CommandResult")
    return $envelope.Result
}

function Invoke-CdpExpression {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Expression
    )

    $result = Invoke-CdpCommand `
        -Connection $Connection `
        -Method "Runtime.evaluate" `
        -Parameters @{
            expression = $Expression
            awaitPromise = $true
            returnByValue = $true
            userGesture = $true
        }
    $exceptionDetails = $result.PSObject.Properties["exceptionDetails"]
    if ($null -ne $exceptionDetails -and $null -ne $exceptionDetails.Value) {
        throw "Packaged desktop JavaScript evaluation failed."
    }
    $runtimeResult = $result.PSObject.Properties["result"]
    if ($null -eq $runtimeResult) {
        throw "Packaged desktop JavaScript evaluation returned no runtime result."
    }
    $value = $runtimeResult.Value.PSObject.Properties["value"]
    if ($null -eq $value) {
        return $null
    }
    return $value.Value
}

function Connect-PackagedDesktopCdp {
    param(
        [Parameter(Mandatory)][int]$Port,
        [Parameter(Mandatory)][DateTime]$Deadline,
        [Parameter(Mandatory)][Diagnostics.Process]$DesktopProcess,
        [Parameter(Mandatory)][string]$DiagnosticPath,
        [Parameter(Mandatory)][string]$ExpectedUserDataRoot
    )

    $target = $null
    $lastJsonListError = $null
    while ([DateTime]::UtcNow -lt $Deadline) {
        $DesktopProcess.Refresh()
        if ($DesktopProcess.HasExited) {
            $diagnostic = Get-CdpStartupDiagnostic `
                -DesktopProcess $DesktopProcess `
                -Port $Port `
                -LastJsonListError $lastJsonListError
            Write-Utf8Json -Path $DiagnosticPath -Value $diagnostic
            throw "The packaged desktop exited before exposing WebView2 CDP (exit code $($DesktopProcess.ExitCode)); diagnostic: $DiagnosticPath"
        }
        try {
            $targets = @(Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list")
            $target = $targets |
                Where-Object {
                    $_.type -eq "page" -and
                    -not [string]::IsNullOrWhiteSpace($_.webSocketDebuggerUrl)
                } |
                Select-Object -First 1
        }
        catch {
            $target = $null
            $lastJsonListError = $_.Exception.ToString()
        }
        if ($null -ne $target) {
            break
        }
        Start-Sleep -Milliseconds 75
    }
    if ($null -eq $target) {
        $diagnostic = Get-CdpStartupDiagnostic `
            -DesktopProcess $DesktopProcess `
            -Port $Port `
            -LastJsonListError $lastJsonListError
        Write-Utf8Json -Path $DiagnosticPath -Value $diagnostic
        throw "The packaged desktop did not expose a WebView2 CDP page; diagnostic: $DiagnosticPath; last /json/list error: $lastJsonListError"
    }
    $canonicalUserDataRoot = [IO.Path]::GetFullPath($ExpectedUserDataRoot)
    $diagnostic = Get-CdpStartupDiagnostic `
        -DesktopProcess $DesktopProcess `
        -Port $Port `
        -LastJsonListError $lastJsonListError
    $ownedWebViewProcesses = @($diagnostic.descendant_msedgewebview2 | Where-Object {
            ([string]$_.command_line).IndexOf(
                $canonicalUserDataRoot,
                [StringComparison]::OrdinalIgnoreCase
            ) -ge 0 -and
            $_.command_line -like "*--remote-debugging-port=$Port*" -and
            $_.command_line -notmatch '(?:^|\s)--type='
        })
    $diagnostic["expected_user_data_root"] = $canonicalUserDataRoot
    $diagnostic["webview_isolation_valid"] = $ownedWebViewProcesses.Count -ge 1
    $diagnostic["wait_for_script_debugger_requested"] =
        $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER -ceq "1"
    Write-Utf8Json -Path $DiagnosticPath -Value $diagnostic
    if ($ownedWebViewProcesses.Count -lt 1) {
        throw "The packaged desktop CDP listener was not owned by an isolated acceptance WebView2 process; diagnostic: $DiagnosticPath"
    }
    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $connectTimeout = [Threading.CancellationTokenSource]::new(10000)
    try {
        $socket.ConnectAsync(
            [Uri]$target.webSocketDebuggerUrl,
            $connectTimeout.Token
        ).GetAwaiter().GetResult() | Out-Null
    }
    finally {
        $connectTimeout.Dispose()
    }
    return [pscustomobject]@{
        Socket = $socket
        NextId = 0
        TimeoutMilliseconds = 10000
        NotificationMethods = [Collections.Generic.List[string]]::new()
        OwnedWebViewProcesses = @($ownedWebViewProcesses)
    }
}

function Wait-CdpExpression {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Expression,
        [Parameter(Mandatory)][DateTime]$Deadline,
        [Parameter(Mandatory)][string]$Description
    )

    do {
        if ([bool](Invoke-CdpExpression -Connection $Connection -Expression $Expression)) {
            return
        }
        Start-Sleep -Milliseconds 75
    } while ([DateTime]::UtcNow -lt $Deadline)
    $body = Invoke-CdpExpression `
        -Connection $Connection `
        -Expression '(document.body?.innerText ?? "").slice(0, 2000)'
    throw "Timed out waiting for $Description. Body: $body"
}

function Wait-CdpButtonText {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][DateTime]$Deadline
    )

    $literal = $Text | ConvertTo-Json -Compress
    Wait-CdpExpression `
        -Connection $Connection `
        -Expression "Array.from(document.querySelectorAll('button')).some((button) => button.textContent?.trim() === $literal && !button.disabled)" `
        -Deadline $Deadline `
        -Description "enabled '$Text' button"
}

function Invoke-CdpButtonText {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Text
    )

    $literal = $Text | ConvertTo-Json -Compress
    $clicked = Invoke-CdpExpression `
        -Connection $Connection `
        -Expression "(() => { const button = Array.from(document.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === $literal && !candidate.disabled); if (!button) return false; button.click(); return true; })()"
    if ($clicked -ne $true) {
        throw "Unable to click the enabled '$Text' button."
    }
}

function Save-CdpScreenshot {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Path
    )

    $capture = Invoke-CdpCommand `
        -Connection $Connection `
        -Method "Page.captureScreenshot" `
        -Parameters @{ format = "png"; fromSurface = $true }
    [IO.File]::WriteAllBytes($Path, [Convert]::FromBase64String($capture.data))
}

function Save-CdpDomEvidence {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [Parameter(Mandatory)][string]$Path
    )

    $expression = @'
(() => ({
  lang: document.documentElement.lang,
  dir: document.documentElement.dir,
  headings: Array.from(document.querySelectorAll("h1,h2")).map((node) => node.textContent?.trim() ?? ""),
  buttons: Array.from(document.querySelectorAll("button")).map((node) => ({ text: node.textContent?.trim() ?? "", disabled: node.disabled })),
  dialogs: Array.from(document.querySelectorAll('[role="dialog"]')).map((node) => ({ modal: node.getAttribute("aria-modal"), labelledby: node.getAttribute("aria-labelledby") })),
  progress: Array.from(document.querySelectorAll('[role="progressbar"]')).map((node) => ({ label: node.getAttribute("aria-label"), now: node.getAttribute("aria-valuenow"), max: node.getAttribute("aria-valuemax") })),
  active: document.activeElement?.textContent?.trim() ?? ""
}))()
'@
    Write-Utf8Json `
        -Path $Path `
        -Value (Invoke-CdpExpression -Connection $Connection -Expression $expression)
}

function Add-AcceptanceDocumentScript {
    param(
        [Parameter(Mandatory)][object]$Connection,
        [AllowNull()][string]$NavigatorLocale
    )

    $navigatorLiteral = if ([string]::IsNullOrWhiteSpace($NavigatorLocale)) {
        "null"
    }
    else {
        $NavigatorLocale | ConvertTo-Json -Compress
    }
    $source = @'
(() => {
  const navigatorLocale = __NAVIGATOR_LOCALE__;
  if (navigatorLocale !== null) {
    Object.defineProperty(navigator, "languages", {
      configurable: true,
      value: [navigatorLocale],
    });
    Object.defineProperty(navigator, "language", {
      configurable: true,
      value: navigatorLocale,
    });
  }
  const trace = {
    ready: false,
    failed: false,
    events: [],
    visibleFrames: [],
    visibleFrameObserver: null,
    visibleFrameAnimation: null,
    listenerId: null,
    error: null,
    originalInvoke: null,
  };
  window.__wokrouterAcceptance = trace;
  const captureVisibleFrame = () => {
    const text = document.body?.innerText?.trim() ?? "";
    if (text === "" || trace.visibleFrames.length >= 100) return;
    const frame = {
      lang: document.documentElement.lang,
      text: text.slice(0, 4000),
    };
    const previous = trace.visibleFrames.at(-1);
    if (previous?.lang !== frame.lang || previous?.text !== frame.text) {
      trace.visibleFrames.push(frame);
    }
  };
  trace.visibleFrameObserver = new MutationObserver(captureVisibleFrame);
  trace.visibleFrameObserver.observe(document.documentElement, {
    attributes: true,
    childList: true,
    characterData: true,
    subtree: true,
  });
  const captureAnimationFrame = () => {
    captureVisibleFrame();
    trace.visibleFrameAnimation = requestAnimationFrame(captureAnimationFrame);
  };
  trace.visibleFrameAnimation = requestAnimationFrame(captureAnimationFrame);
  const internals = window.__TAURI_INTERNALS__;
  if (
    typeof internals?.invoke !== "function" ||
    typeof internals?.transformCallback !== "function"
  ) {
    trace.failed = true;
    trace.error = "tauri_internals_unavailable";
    return;
  }
  trace.originalInvoke = internals.invoke.bind(internals);
  const handler = internals.transformCallback((value) => {
    const payload = value?.payload;
    if (value?.event === "core-operation-progress" || payload?.schema_version === 1) {
      trace.events.push({
        schema_version: payload?.schema_version,
        operation_id: payload?.operation_id,
        sequence: payload?.sequence,
        operation: payload?.operation,
        state: payload?.state,
        phase: payload?.phase,
        current_version: payload?.current_version,
        target_version: payload?.target_version,
        bytes_completed: payload?.bytes_completed,
        bytes_total: payload?.bytes_total,
        active_requests: payload?.active_requests,
        error_code: payload?.error_code,
      });
    }
  }, false);
  trace.originalInvoke("plugin:event|listen", {
    event: "core-operation-progress",
    target: { kind: "Any" },
    handler,
  }).then(
    (listenerId) => {
      trace.listenerId = listenerId;
      trace.ready = true;
    },
    (error) => {
      trace.failed = true;
      trace.error = String(error);
    },
  );
})();
'@
    $source = $source.Replace("__NAVIGATOR_LOCALE__", $navigatorLiteral)
    Invoke-CdpCommand `
        -Connection $Connection `
        -Method "Page.enable" `
        -Parameters @{} | Out-Null
    $null = Invoke-CdpExpression `
        -Connection $Connection `
        -Expression $source
    $traceInjected = Invoke-CdpExpression `
        -Connection $Connection `
        -Expression 'typeof window.__wokrouterAcceptance === "object"'
    if ($traceInjected -ne $true) {
        throw "The packaged acceptance script was not injected into the paused document."
    }
    Invoke-CdpCommand `
        -Connection $Connection `
        -Method "Runtime.runIfWaitingForDebugger" | Out-Null
}

function Invoke-MissingInstallLive {
    param(
        [Parameter(Mandatory)][string]$Desktop,
        [Parameter(Mandatory)][string]$Minisign,
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][int]$Timeout
    )

    if (Test-Path -LiteralPath $OutputRoot) {
        throw "Live acceptance evidence root must not already exist: $OutputRoot"
    }
    $null = New-Item -ItemType Directory -Path $OutputRoot
    $runId = [Guid]::NewGuid().ToString("N")
    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $scratchRoot = Join-Path $temporaryBase "wokrouter-packaged-gui-live-$runId"
    $null = New-Item -ItemType Directory -Path $scratchRoot
    $local = Join-Path $scratchRoot "local"
    $connection = $null
    $desktopProcess = $null
    $feedProcess = $null
    $scratchRemoved = $false
    $ownedProcessesCleaned = $false
    $summary = $null
    $primaryError = $null
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    $processCleanup = $null
    $webViewCleanup = $null
    $webViewData = $null
    $cdpPort = 0
    $previousEnvironment = @{
        APPDATA = $env:APPDATA
        LOCALAPPDATA = $env:LOCALAPPDATA
        PATH = $env:PATH
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
        WEBVIEW2_USER_DATA_FOLDER = $env:WEBVIEW2_USER_DATA_FOLDER
        WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER
        WOKROUTER_ACCEPTANCE_STATE_ROOT = $env:WOKROUTER_ACCEPTANCE_STATE_ROOT
        WOKROUTER_ACCEPTANCE_FEED_DELAY_MS = $env:WOKROUTER_ACCEPTANCE_FEED_DELAY_MS
        WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE
        WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN = $env:WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN
        WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY = $env:WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY
    }
    $application = $null
    try {
        $application = Build-LiveAcceptanceApplication -Root $scratchRoot -Desktop $Desktop
        $feed = New-SignedWokCoreFeed `
            -Root $scratchRoot `
            -FixtureExecutable $application.Fixture `
            -MinisignExecutable $Minisign
        $roaming = Join-Path $scratchRoot "roaming"
        $webViewData = Join-Path $scratchRoot "webview-user-data"
        $state = Join-Path $scratchRoot "fixture-state"
        foreach ($directory in @($roaming, $local, $webViewData, $state)) {
            $null = New-Item -ItemType Directory -Path $directory
        }
        $isolatedPath = Get-IsolatedWindowsPath
        $cdpPort = Get-FreeLoopbackPort
        $feedReady = Join-Path $scratchRoot "feed-ready"
        $env:APPDATA = $roaming
        $env:LOCALAPPDATA = $local
        $env:PATH = $isolatedPath
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$cdpPort"
        $env:WEBVIEW2_USER_DATA_FOLDER = $webViewData
        $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = "1"
        $env:WOKROUTER_ACCEPTANCE_STATE_ROOT = $state
        $env:WOKROUTER_ACCEPTANCE_FEED_DELAY_MS = "350"
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = "en-US"
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY = $feed.PublicKey

        $feedProcess = Start-FixtureProcess -Executable $application.Fixture -Arguments @(
            "feed", "--root", $feed.Root, "--ready", $feedReady, "--port", "0"
        )
        Wait-PathUntil `
            -Path $feedReady `
            -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        $feedPort = [int](Get-Content -LiteralPath $feedReady -Raw -Encoding UTF8)
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN =
            "http://127.0.0.1:$feedPort/releases/"

        $installRecord = Join-Path $roaming "WokRouter/wokcore-install.json"
        $installedWokCore = Join-Path $local "WokCore/bin/wokcore.exe"
        if (
            (Test-Path -LiteralPath $installRecord) -or
            (Test-Path -LiteralPath $installedWokCore) -or
            (@($isolatedPath.Split([IO.Path]::PathSeparator) | Where-Object {
                        Test-Path -LiteralPath (Join-Path $_ "wokcore.exe") -PathType Leaf
                    }).Count -ne 0)
        ) {
            throw "The isolated MissingInstall precondition was not clean."
        }

        $desktopProcess = Start-Process -FilePath $application.Desktop -PassThru
        $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
        $connection = Connect-PackagedDesktopCdp `
            -Port $cdpPort `
            -Deadline $deadline `
            -DesktopProcess $desktopProcess `
            -DiagnosticPath (Join-Path $OutputRoot "missing-install-cdp-startup.json") `
            -ExpectedUserDataRoot $webViewData
        Add-AcceptanceDocumentScript -Connection $connection
        Wait-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance?.ready === true && window.__wokrouterAcceptance?.failed === false' `
            -Deadline $deadline `
            -Description "the fail-closed acceptance document bridge"
        Wait-CdpExpression `
            -Connection $connection `
            -Expression 'document.querySelector("[role=progressbar]") !== null' `
            -Deadline $deadline `
            -Description "visible signed install progress"
        Save-CdpDomEvidence `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "missing-install-progress.json")
        Save-CdpScreenshot `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "missing-install-progress.png")

        Wait-PathUntil -Path $installRecord -Deadline $deadline
        Wait-PathUntil -Path $installedWokCore -Deadline $deadline
        Wait-PathUntil -Path (Join-Path $state "serve-ready") -Deadline $deadline
        Wait-CdpExpression `
            -Connection $connection `
            -Expression 'document.querySelector("#core-health-heading") !== null && window.__wokrouterAcceptance.events.some((event) => event.state === "succeeded" && event.phase === "completed")' `
            -Deadline $deadline `
            -Description "the installed running WokCore terminal state"
        Save-CdpDomEvidence `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "missing-install-ready.json")
        Save-CdpScreenshot `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "missing-install-ready.png")

        $operation = Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("core_operation_status")'
        $status = Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("core_status")'
        $trace = Invoke-CdpExpression `
            -Connection $connection `
            -Expression '({ ready: window.__wokrouterAcceptance.ready, failed: window.__wokrouterAcceptance.failed, listenerId: window.__wokrouterAcceptance.listenerId, events: window.__wokrouterAcceptance.events })'
        $aclEmitRejected = [bool](Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("plugin:event|emit", { event: "wokrouter-acceptance-forbidden", payload: null }).then(() => false, () => true)')
        $startCoreRejected = [bool](Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("start_core").then(() => false, () => true)')
        $stopCoreRejected = [bool](Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("stop_core").then(() => false, () => true)')

        $record = Get-Content -LiteralPath $installRecord -Raw -Encoding UTF8 |
            ConvertFrom-Json
        $installedHash = (Get-FileHash -LiteralPath $installedWokCore -Algorithm SHA256).Hash.ToLowerInvariant()
        $servePid = [int](Get-Content -LiteralPath (Join-Path $state "serve-pid.txt") -Raw)
        $serveProcess = Get-Process -Id $servePid -ErrorAction Stop
        $serveIdentityMatches = (
            $null -ne $serveProcess.Path -and
            [IO.Path]::GetFullPath($serveProcess.Path) -ieq
                [IO.Path]::GetFullPath($installedWokCore)
        )
        $fixtureLog = @(Get-Content -LiteralPath (Join-Path $state "fixture.log") -Encoding UTF8)
        $manifestRequests = @($fixtureLog | Where-Object {
                $_ -match 'feed-request /releases/wokcore-update-v2\.json$'
            }).Count
        $signatureRequests = @($fixtureLog | Where-Object {
                $_ -match 'feed-request /releases/wokcore-update-v2\.json\.minisig$'
            }).Count
        $archiveRequests = @($fixtureLog | Where-Object {
                $_ -match "feed-request /releases/$([regex]::Escape($feed.ArchiveName))$"
            }).Count
        $v1Requests = @($fixtureLog | Where-Object {
                $_ -match
                'feed-request /releases/wokcore-update-v1\.json(?:\.minisig)?$'
            }).Count
        $eventIds = @($trace.events | ForEach-Object operation_id | Where-Object { $_ } | Select-Object -Unique)
        $eventPhases = @($trace.events | ForEach-Object phase | Where-Object { $_ })
        $eventListenObserved = $trace.ready -and -not $trace.failed -and $null -ne $trace.listenerId
        $statusCapabilities = @($status.capabilities)
        Write-Utf8Json `
            -Path (Join-Path $OutputRoot "missing-install-trusted-boundaries.json") `
            -Value ([ordered]@{
                install_record_schema = $record.schema_version
                install_record_executable = [string]$record.executable
                expected_installed_executable = $installedWokCore
                installed_hash_matches = $installedHash -ceq $application.FixtureSha256
                serve_identity_matches = $serveIdentityMatches
                manifest_requests = $manifestRequests
                signature_requests = $signatureRequests
                archive_requests = $archiveRequests
                legacy_v1_requests = $v1Requests
                event_ids = @($eventIds)
                event_phases = @($eventPhases)
                event_listen_observed = $eventListenObserved
                event_emit_acl_rejected = $aclEmitRejected
                start_core_rejected = $startCoreRejected
                stop_core_rejected = $stopCoreRejected
                status_state = $status.state
                status_runtime_channel = $status.runtime_channel
                status_capabilities = @($statusCapabilities)
                operation_state = $operation.state
                operation_kind = $operation.operation
            })
        if (
            $record.schema_version -ne 1 -or
            [IO.Path]::GetFullPath([string]$record.executable) -ine
                [IO.Path]::GetFullPath($installedWokCore) -or
            $installedHash -cne $application.FixtureSha256 -or
            -not $serveIdentityMatches -or
            $manifestRequests -ne 1 -or
            $signatureRequests -ne 1 -or
            $archiveRequests -ne 1 -or
            $v1Requests -ne 0 -or
            $eventIds.Count -ne 1 -or
            $eventPhases -notcontains "downloading" -or
            $eventPhases -notcontains "completed" -or
            -not $eventListenObserved -or
            -not $aclEmitRejected -or
            -not $startCoreRejected -or
            -not $stopCoreRejected -or
            $status.state -cne "running" -or
            $status.runtime_channel -cne "production" -or
            $statusCapabilities.Count -ne 1 -or
            $statusCapabilities[0] -cne "core.update.v1" -or
            $operation.state -cne "succeeded" -or
            $operation.operation -cne "install"
        ) {
            throw "The live MissingInstall evidence did not satisfy every trusted boundary."
        }
        $summary = [ordered]@{
            schema_version = 1
            scenario = "MissingInstall"
            run_id = $runId
            locale = "en"
            precondition = "missing"
            final_state = "running"
            signing_key_id = $feed.KeyId
            signed_v2_manifest_requests = $manifestRequests
            signed_v2_signature_requests = $signatureRequests
            signed_archive_requests = $archiveRequests
            legacy_v1_requests = $v1Requests
            archive_size = $feed.ArchiveSize
            archive_sha256 = $feed.ArchiveSha256
            installed_sha256 = $installedHash
            install_record_valid = $true
            serve_pid = $servePid
            serve_executable_identity_valid = $serveIdentityMatches
            operation_id = $operation.operation_id
            operation_state = $operation.state
            operation_phase = $operation.phase
            event_phase_count = $eventPhases.Count
            event_listen_observed = $eventListenObserved
            event_emit_acl_rejected = $aclEmitRejected
            acceptance_feature_release_excluded = $true
            production_desktop_sha256 = $application.ProductionDesktopSha256
            acceptance_desktop_sha256 = $application.AcceptanceDesktopSha256
            path_wokcore_absent = $true
            isolated_path_directories = @($isolatedPath.Split([IO.Path]::PathSeparator))
            core_status_cdp_mocked = $false
            core_status_acceptance_seam = $true
            native_vault_status_path_exercised = $false
            start_core_fail_closed = $startCoreRejected
            stop_core_fail_closed = $stopCoreRejected
            management_capabilities_advertised = @()
            credential_commands_invoked = @()
        }
    }
    catch {
        $primaryError = $_
        foreach ($diagnostic in @(
                @{
                    Source = Join-Path $scratchRoot "fixture-state/fixture.log"
                    Target = Join-Path $OutputRoot "missing-install-fixture.log"
                },
                @{
                    Source = Join-Path $local "WokCore/runtime/discovery.json"
                    Target = Join-Path $OutputRoot "missing-install-discovery.json"
                }
            )) {
            if (Test-Path -LiteralPath $diagnostic.Source -PathType Leaf) {
                try {
                    Copy-Item `
                        -LiteralPath $diagnostic.Source `
                        -Destination $diagnostic.Target
                }
                catch {}
            }
        }
        try {
            $ownedProcessDiagnostics = @(Get-CimInstance -ClassName Win32_Process |
                    Where-Object {
                        ([string]$_.ExecutablePath).IndexOf(
                            $scratchRoot,
                            [StringComparison]::OrdinalIgnoreCase
                        ) -ge 0
                    } |
                    ForEach-Object {
                        [ordered]@{
                            process_id = [int]$_.ProcessId
                            parent_process_id = [int]$_.ParentProcessId
                            executable_path = [string]$_.ExecutablePath
                            command_line = [string]$_.CommandLine
                        }
                    })
            Write-Utf8Json `
                -Path (Join-Path $OutputRoot "missing-install-owned-processes.json") `
                -Value $ownedProcessDiagnostics
        }
        catch {}
        if ($null -ne $connection) {
            try {
                Save-CdpDomEvidence `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "missing-install-failure-dom.json")
            }
            catch {}
            try {
                Save-CdpScreenshot `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "missing-install-failure.png")
            }
            catch {}
            try {
                $failureTrace = Invoke-CdpExpression `
                    -Connection $connection `
                    -Expression 'window.__wokrouterAcceptance ?? null'
                Write-Utf8Json `
                    -Path (Join-Path $OutputRoot "missing-install-failure-trace.json") `
                    -Value $failureTrace
            }
            catch {}
        }
    }
    finally {
        if ($null -ne $connection) {
            try {
                $connection.Socket.Dispose()
            }
            catch {
                $cleanupErrors.Add("CDP socket cleanup: $($_.Exception.Message)")
            }
        }
        $ownedExecutables = [Collections.Generic.List[string]]::new()
        if ($null -ne $application) {
            foreach ($path in @(
                    $application.Desktop,
                    $application.Sidecar,
                    $application.Fixture,
                    (Join-Path $scratchRoot "local/WokCore/bin/wokcore.exe")
                )) {
                $ownedExecutables.Add($path)
            }
        }
        try {
            if ($ownedExecutables.Count -gt 0) {
                $processCleanup = Stop-OwnedProcessesByExecutable `
                    -Executables $ownedExecutables.ToArray()
            }
        }
        catch {
            $cleanupErrors.Add("Owned process cleanup: $($_.Exception.Message)")
        }
        try {
            if (
                $null -ne $connection -and
                $connection.OwnedWebViewProcesses.Count -gt 0 -and
                -not [string]::IsNullOrWhiteSpace($webViewData) -and
                $cdpPort -gt 0
            ) {
                $webViewCleanup = Stop-RecordedAcceptanceWebViewProcesses `
                    -Processes @($connection.OwnedWebViewProcesses) `
                    -UserDataRoot $webViewData `
                    -RemoteDebuggingPort $cdpPort
            }
        }
        catch {
            $cleanupErrors.Add("Recorded WebView2 cleanup: $($_.Exception.Message)")
        }
        $ownedProcessesCleaned = $cleanupErrors.Count -eq 0
        foreach ($name in $previousEnvironment.Keys) {
            try {
                if ($null -eq $previousEnvironment[$name]) {
                    Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
                }
                else {
                    Set-Item -Path "Env:$name" -Value $previousEnvironment[$name]
                }
            }
            catch {
                $cleanupErrors.Add("Environment restore for $name`: $($_.Exception.Message)")
            }
        }
        try {
            Remove-OwnedLiveScratchRoot -Path $scratchRoot
            $scratchRemoved = -not (Test-Path -LiteralPath $scratchRoot)
        }
        catch {
            $cleanupErrors.Add("Scratch cleanup: $($_.Exception.Message)")
        }
    }

    if ($null -ne $primaryError -or $cleanupErrors.Count -gt 0) {
        $failure = [ordered]@{
            schema_version = 1
            scenario = "MissingInstall"
            run_id = $runId
            primary_error = if ($null -eq $primaryError) {
                $null
            }
            else {
                $primaryError.Exception.ToString()
            }
            cleanup_errors = @($cleanupErrors)
            scratch_root_removed = $scratchRemoved
            process_cleanup = $processCleanup
            webview_cleanup = $webViewCleanup
        }
        Write-Utf8Json `
            -Path (Join-Path $OutputRoot "missing-install-failure.json") `
            -Value $failure
        if ($null -ne $primaryError) {
            if ($cleanupErrors.Count -gt 0) {
                throw "Primary MissingInstall failure: $($primaryError.Exception.Message) Cleanup diagnostics: $([string]::Join(' | ', $cleanupErrors))"
            }
            throw $primaryError
        }
        throw "MissingInstall cleanup failed: $([string]::Join(' | ', $cleanupErrors))"
    }
    if ($null -eq $summary) {
        throw "MissingInstall completed without a summary."
    }
    $summary.owned_processes_cleaned = $ownedProcessesCleaned
    $summary.scratch_root_removed = $scratchRemoved
    $summary.webview_user_data_isolated = $true
    $summary.first_document_script_debugger_gate = $true
    $summary.cleanup_identity = [ordered]@{
        executable_paths = @($ownedExecutables)
        webview_user_data_root = [IO.Path]::GetFullPath($webViewData)
        remote_debugging_port = $cdpPort
        recorded_webview_process_ids = @(
            $connection.OwnedWebViewProcesses |
                ForEach-Object { [int]$_.process_id }
        )
    }
    $summary.cleanup_quiescence_rounds = [int]$processCleanup.Rounds
    $summary.cleanup_stable_rounds = [int]$processCleanup.StableRounds
    $summary.cleanup_stopped_process_ids = @($processCleanup.StoppedProcessIds)
    $summary.webview_cleanup_wait_rounds = [int]$webViewCleanup.WaitRounds
    $summary.webview_cleanup_terminated_process_ids = @(
        $webViewCleanup.TerminatedProcessIds
    )
    Write-Utf8Json `
        -Path (Join-Path $OutputRoot "missing-install-summary.json") `
        -Value $summary
    Write-Output "Packaged GUI MissingInstall acceptance passed."
}

function Invoke-PreinstalledUpdateLive {
    param(
        [Parameter(Mandatory)]
        [ValidateSet("UpdateCancelConfirm", "ActiveRequests", "Rollback", "CloseReopen")]
        [string]$LiveScenario,
        [Parameter(Mandatory)][string]$Desktop,
        [Parameter(Mandatory)][string]$Minisign,
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][int]$Timeout
    )

    if (Test-Path -LiteralPath $OutputRoot) {
        throw "Live acceptance evidence root must not already exist: $OutputRoot"
    }
    $null = New-Item -ItemType Directory -Path $OutputRoot
    $runId = [Guid]::NewGuid().ToString("N")
    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $scratchRoot = Join-Path $temporaryBase "wokrouter-packaged-gui-live-$runId"
    $null = New-Item -ItemType Directory -Path $scratchRoot
    $roaming = Join-Path $scratchRoot "roaming"
    $local = Join-Path $scratchRoot "local"
    $state = Join-Path $scratchRoot "fixture-state"
    $webViewData = Join-Path $scratchRoot "webview-user-data"
    $connection = $null
    $desktopProcess = $null
    $feedProcess = $null
    $application = $null
    $cdpPort = 0
    $summary = $null
    $primaryError = $null
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    $processCleanup = $null
    $webViewCleanup = $null
    $scratchRemoved = $false
    $closedWindowWebViewCleanup = $null
    $firstWindowEvents = @()
    $cancellationPreventedInstall = $LiveScenario -cne "UpdateCancelConfirm"
    $previousEnvironment = @{
        APPDATA = $env:APPDATA
        LOCALAPPDATA = $env:LOCALAPPDATA
        PATH = $env:PATH
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
        WEBVIEW2_USER_DATA_FOLDER = $env:WEBVIEW2_USER_DATA_FOLDER
        WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER
        WOKROUTER_ACCEPTANCE_STATE_ROOT = $env:WOKROUTER_ACCEPTANCE_STATE_ROOT
        WOKROUTER_ACCEPTANCE_FEED_DELAY_MS = $env:WOKROUTER_ACCEPTANCE_FEED_DELAY_MS
        WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE
        WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN = $env:WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN
        WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY = $env:WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY
    }
    try {
        $application = Build-LiveAcceptanceApplication -Root $scratchRoot -Desktop $Desktop
        $feed = New-SignedWokCoreFeed `
            -Root $scratchRoot `
            -FixtureExecutable $application.Fixture `
            -MinisignExecutable $Minisign
        foreach ($directory in @($roaming, $local, $state, $webViewData)) {
            $null = New-Item -ItemType Directory -Path $directory
        }
        $isolatedPath = Get-IsolatedWindowsPath
        $feedReady = Join-Path $scratchRoot "feed-ready"
        $env:APPDATA = $roaming
        $env:LOCALAPPDATA = $local
        $env:PATH = $isolatedPath
        $env:WOKROUTER_ACCEPTANCE_STATE_ROOT = $state
        $env:WOKROUTER_ACCEPTANCE_FEED_DELAY_MS = "0"
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = "en-US"
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_PUBLIC_KEY = $feed.PublicKey

        $feedProcess = Start-FixtureProcess -Executable $application.Fixture -Arguments @(
            "feed", "--root", $feed.Root, "--ready", $feedReady, "--port", "0"
        )
        Wait-PathUntil -Path $feedReady -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        $feedPort = [int](Get-Content -LiteralPath $feedReady -Raw -Encoding UTF8)
        $env:WOKROUTER_PACKAGED_ACCEPTANCE_ORIGIN =
            "http://127.0.0.1:$feedPort/releases/"

        $preinstall = Start-Process `
            -FilePath $application.Sidecar `
            -ArgumentList @("start", "--json", "--progress-jsonl") `
            -WindowStyle Hidden `
            -PassThru
        if (-not $preinstall.WaitForExit($Timeout * 1000)) {
            Stop-OwnedProcess -Process $preinstall -ExpectedExecutable $application.Sidecar
            throw "The packaged acceptance preinstall CLI timed out."
        }
        if ($preinstall.ExitCode -ne 0) {
            throw "The packaged acceptance preinstall CLI failed with exit code $($preinstall.ExitCode)."
        }
        $preinstall.Dispose()

        $installRecord = Join-Path $roaming "WokRouter/wokcore-install.json"
        $installedWokCore = Join-Path $local "WokCore/bin/wokcore.exe"
        Wait-PathUntil -Path $installRecord -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        Wait-PathUntil -Path $installedWokCore -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        Wait-PathUntil `
            -Path (Join-Path $state "serve-ready") `
            -Deadline ([DateTime]::UtcNow.AddSeconds($Timeout))
        $scenarioValue = switch ($LiveScenario) {
            "ActiveRequests" { "active_requests" }
            "Rollback" { "rollback" }
            "CloseReopen" { "slow_success" }
            default { "success" }
        }
        [IO.File]::WriteAllText(
            (Join-Path $state "scenario.txt"),
            $scenarioValue,
            [Text.UTF8Encoding]::new($false)
        )

        $cdpPort = Get-FreeLoopbackPort
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$cdpPort"
        $env:WEBVIEW2_USER_DATA_FOLDER = $webViewData
        $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = "1"
        $desktopProcess = Start-Process -FilePath $application.Desktop -PassThru
        $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
        $connection = Connect-PackagedDesktopCdp `
            -Port $cdpPort `
            -Deadline $deadline `
            -DesktopProcess $desktopProcess `
            -DiagnosticPath (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-cdp-startup.json") `
            -ExpectedUserDataRoot $webViewData
        Add-AcceptanceDocumentScript -Connection $connection
        Wait-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance?.ready === true && window.__wokrouterAcceptance?.failed === false' `
            -Deadline $deadline `
            -Description "the $LiveScenario event listener"
        Wait-CdpButtonText -Connection $connection -Text "Upgrade WokCore" -Deadline $deadline

        $updateInstallBaseline = @(
            Get-Content -LiteralPath (Join-Path $state "fixture.log") -Encoding UTF8 |
                Where-Object { $_ -match ' update-install$' }
        ).Count
        if ($LiveScenario -ceq "UpdateCancelConfirm") {
            Invoke-CdpButtonText -Connection $connection -Text "Upgrade WokCore"
            Wait-CdpExpression `
                -Connection $connection `
                -Expression 'document.querySelector("[role=dialog]") !== null' `
                -Deadline $deadline `
                -Description "the update confirmation dialog"
            Save-CdpDomEvidence `
                -Connection $connection `
                -Path (Join-Path $OutputRoot "update-cancel-dialog.json")
            Save-CdpScreenshot `
                -Connection $connection `
                -Path (Join-Path $OutputRoot "update-cancel-dialog.png")
            Invoke-CdpButtonText -Connection $connection -Text "Cancel"
            Wait-CdpExpression `
                -Connection $connection `
                -Expression 'document.querySelector("[role=dialog]") === null' `
                -Deadline $deadline `
                -Description "the cancelled update dialog to close"
            Start-Sleep -Milliseconds 300
            $afterCancelInstalls = @(
                Get-Content -LiteralPath (Join-Path $state "fixture.log") -Encoding UTF8 |
                    Where-Object { $_ -match ' update-install$' }
            ).Count
            if (
                $afterCancelInstalls -ne $updateInstallBaseline -or
                (Test-Path -LiteralPath (Join-Path $state "current-version.txt"))
            ) {
                throw "Cancelling the update started a download or changed the runtime version."
            }
            $cancellationPreventedInstall = $true
            Invoke-CdpButtonText -Connection $connection -Text "Upgrade WokCore"
        }
        else {
            Invoke-CdpButtonText -Connection $connection -Text "Upgrade WokCore"
        }
        Wait-CdpExpression `
            -Connection $connection `
            -Expression 'document.querySelector("[role=dialog]") !== null' `
            -Deadline $deadline `
            -Description "the confirmed update dialog"
        Invoke-CdpButtonText -Connection $connection -Text "Confirm upgrade"

        if ($LiveScenario -ceq "CloseReopen") {
            Wait-CdpExpression `
                -Connection $connection `
                -Expression 'window.__wokrouterAcceptance.events.some((event) => event.operation === "update" && event.phase === "downloading" && event.state === "running")' `
                -Deadline $deadline `
                -Description "the slow update download before close"
            Save-CdpDomEvidence `
                -Connection $connection `
                -Path (Join-Path $OutputRoot "close-reopen-before-close.json")
            Save-CdpScreenshot `
                -Connection $connection `
                -Path (Join-Path $OutputRoot "close-reopen-before-close.png")
            $firstWindowEvents = @(Invoke-CdpExpression `
                    -Connection $connection `
                    -Expression 'window.__wokrouterAcceptance.events')
            $firstConnection = $connection
            $firstWebViewData = $webViewData
            $firstCdpPort = $cdpPort
            $connection.Socket.Dispose()
            $connection = $null
            if (-not $desktopProcess.CloseMainWindow()) {
                throw "The first packaged desktop window did not accept a normal close request."
            }
            if (-not $desktopProcess.WaitForExit(5000)) {
                throw "The first packaged desktop did not exit after its window closed."
            }
            $desktopProcess.Dispose()
            $desktopProcess = $null
            $closedWindowWebViewCleanup = Stop-RecordedAcceptanceWebViewProcesses `
                -Processes @($firstConnection.OwnedWebViewProcesses) `
                -UserDataRoot $firstWebViewData `
                -RemoteDebuggingPort $firstCdpPort
            $updateInstallsBeforeReopen = @(
                Get-Content -LiteralPath (Join-Path $state "fixture.log") -Encoding UTF8 |
                    Where-Object { $_ -match ' update-install$' }
            ).Count
            if ($updateInstallsBeforeReopen -ne $updateInstallBaseline + 1) {
                throw "The slow update did not have exactly one operation before reopening."
            }

            $webViewData = Join-Path $scratchRoot "webview-user-data-reopen"
            $null = New-Item -ItemType Directory -Path $webViewData
            $cdpPort = Get-FreeLoopbackPort
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$cdpPort"
            $env:WEBVIEW2_USER_DATA_FOLDER = $webViewData
            $desktopProcess = Start-Process -FilePath $application.Desktop -PassThru
            $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
            $connection = Connect-PackagedDesktopCdp `
                -Port $cdpPort `
                -Deadline $deadline `
                -DesktopProcess $desktopProcess `
                -DiagnosticPath (Join-Path $OutputRoot "close-reopen-second-cdp-startup.json") `
                -ExpectedUserDataRoot $webViewData
            Add-AcceptanceDocumentScript -Connection $connection
            Wait-CdpExpression `
                -Connection $connection `
                -Expression 'window.__wokrouterAcceptance?.ready === true && document.querySelector("[role=progressbar]") !== null' `
                -Deadline $deadline `
                -Description "the reopened window to recover the running update"
            [IO.File]::WriteAllText(
                (Join-Path $state "allow-update"),
                "continue",
                [Text.UTF8Encoding]::new($false)
            )
        }

        $expectedState = if ($LiveScenario -in @("ActiveRequests", "Rollback")) {
            "failed"
        }
        else {
            "succeeded"
        }
        $expectedError = switch ($LiveScenario) {
            "ActiveRequests" { "active_requests_remain" }
            "Rollback" { "rolled_back" }
            default { $null }
        }
        $terminalExpression = if ($null -eq $expectedError) {
            "window.__wokrouterAcceptance.originalInvoke('core_operation_status').then((value) => value?.operation === 'update' && value?.state === '$expectedState' && value?.phase === 'completed')"
        }
        else {
            "window.__wokrouterAcceptance.originalInvoke('core_operation_status').then((value) => value?.operation === 'update' && value?.state === '$expectedState' && value?.error_code === '$expectedError')"
        }
        Wait-CdpExpression `
            -Connection $connection `
            -Expression $terminalExpression `
            -Deadline $deadline `
            -Description "the $LiveScenario terminal update state"
        $terminalEventExpression = if ($null -eq $expectedError) {
            "window.__wokrouterAcceptance.events.some((event) => event.operation === 'update' && event.state === '$expectedState' && event.phase === 'completed')"
        }
        else {
            "window.__wokrouterAcceptance.events.some((event) => event.operation === 'update' && event.state === '$expectedState' && event.error_code === '$expectedError')"
        }
        Wait-CdpExpression `
            -Connection $connection `
            -Expression $terminalEventExpression `
            -Deadline $deadline `
            -Description "the $LiveScenario terminal update event"
        $operation = Invoke-CdpExpression `
            -Connection $connection `
            -Expression 'window.__wokrouterAcceptance.originalInvoke("core_operation_status")'
        $events = @($firstWindowEvents) + @(Invoke-CdpExpression `
                -Connection $connection `
                -Expression 'window.__wokrouterAcceptance.events')
        $eventIds = @($events | ForEach-Object operation_id | Where-Object { $_ } | Select-Object -Unique)
        $eventPhases = @($events | ForEach-Object phase | Where-Object { $_ })
        $fixtureLog = @(Get-Content -LiteralPath (Join-Path $state "fixture.log") -Encoding UTF8)
        $updateInstalls = @($fixtureLog | Where-Object { $_ -match ' update-install$' }).Count
        $updateChecks = @($fixtureLog | Where-Object { $_ -match ' update-check$' }).Count
        $versionPath = Join-Path $state "current-version.txt"
        $finalVersion = if (Test-Path -LiteralPath $versionPath -PathType Leaf) {
            (Get-Content -LiteralPath $versionPath -Raw -Encoding UTF8).Trim()
        }
        else {
            "1.0.0"
        }
        $operationError = Get-OptionalObjectProperty -Value $operation -Name "error_code"
        $operationActiveRequests = Get-OptionalObjectProperty `
            -Value $operation `
            -Name "active_requests"
        Save-CdpDomEvidence `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-terminal.json")
        Save-CdpScreenshot `
            -Connection $connection `
            -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-terminal.png")

        $scenarioValid = switch ($LiveScenario) {
            "UpdateCancelConfirm" {
                $operation.state -ceq "succeeded" -and
                $finalVersion -ceq "2.0.0" -and
                $eventPhases -contains "downloading"
            }
            "ActiveRequests" {
                $operation.state -ceq "failed" -and
                $operationError -ceq "active_requests_remain" -and
                $operationActiveRequests -eq 2 -and
                $finalVersion -ceq "1.0.0" -and
                $eventPhases -contains "draining"
            }
            "Rollback" {
                $operation.state -ceq "failed" -and
                $operationError -ceq "rolled_back" -and
                $finalVersion -ceq "1.0.0" -and
                $eventPhases -contains "rolling_back"
            }
            "CloseReopen" {
                $operation.state -ceq "succeeded" -and
                $finalVersion -ceq "2.0.0" -and
                $closedWindowWebViewCleanup.RecordedProcessIds.Count -gt 0
            }
        }
        $validationEvidence = [ordered]@{
            scenario = $LiveScenario
            scenario_valid = $scenarioValid
            operation_state = $operation.state
            operation_phase = $operation.phase
            error_code = $operationError
            active_requests = $operationActiveRequests
            final_version = $finalVersion
            update_installs = $updateInstalls
            update_checks = $updateChecks
            event_ids = @($eventIds)
            event_phases = @($eventPhases)
        }
        Write-Utf8Json `
            -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-validation.json") `
            -Value $validationEvidence
        if (
            -not $scenarioValid -or
            $updateInstalls -ne 1 -or
            $updateChecks -lt 1 -or
            $eventIds.Count -ne 1
        ) {
            throw "The live $LiveScenario evidence did not satisfy every trusted boundary."
        }
        $summary = [ordered]@{
            schema_version = 1
            scenario = $LiveScenario
            run_id = $runId
            locale = "en"
            initial_version = "1.0.0"
            final_version = $finalVersion
            operation_id = $operation.operation_id
            operation_state = $operation.state
            operation_phase = $operation.phase
            error_code = $operationError
            active_requests = $operationActiveRequests
            update_checks = $updateChecks
            update_installs = $updateInstalls
            event_phases = @($eventPhases)
            cancellation_prevented_install = $cancellationPreventedInstall
            closed_window_operation_recovered = $LiveScenario -cne "CloseReopen" -or
                $closedWindowWebViewCleanup.RecordedProcessIds.Count -gt 0
            duplicate_operation_suppressed = $updateInstalls -eq 1
            acceptance_feature_release_excluded = $true
            native_vault_status_path_exercised = $false
        }
    }
    catch {
        $primaryError = $_
        if (Test-Path -LiteralPath (Join-Path $state "fixture.log") -PathType Leaf) {
            try {
                Copy-Item `
                    -LiteralPath (Join-Path $state "fixture.log") `
                    -Destination (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-fixture.log")
            }
            catch {}
        }
        if ($null -ne $connection) {
            try {
                Save-CdpDomEvidence `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-failure-dom.json")
            }
            catch {}
            try {
                Save-CdpScreenshot `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-failure.png")
            }
            catch {}
        }
    }
    finally {
        if ($null -ne $connection) {
            try { $connection.Socket.Dispose() } catch {}
        }
        $ownedExecutables = [Collections.Generic.List[string]]::new()
        if ($null -ne $application) {
            foreach ($path in @(
                    $application.Desktop,
                    $application.Sidecar,
                    $application.Fixture,
                    (Join-Path $local "WokCore/bin/wokcore.exe")
                )) {
                $ownedExecutables.Add($path)
            }
        }
        try {
            if ($ownedExecutables.Count -gt 0) {
                $processCleanup = Stop-OwnedProcessesByExecutable `
                    -Executables $ownedExecutables.ToArray()
            }
        }
        catch {
            $cleanupErrors.Add("Owned process cleanup: $($_.Exception.Message)")
        }
        try {
            if (
                $null -ne $connection -and
                $connection.OwnedWebViewProcesses.Count -gt 0 -and
                $cdpPort -gt 0
            ) {
                $webViewCleanup = Stop-RecordedAcceptanceWebViewProcesses `
                    -Processes @($connection.OwnedWebViewProcesses) `
                    -UserDataRoot $webViewData `
                    -RemoteDebuggingPort $cdpPort
            }
        }
        catch {
            $cleanupErrors.Add("Recorded WebView2 cleanup: $($_.Exception.Message)")
        }
        foreach ($name in $previousEnvironment.Keys) {
            try {
                if ($null -eq $previousEnvironment[$name]) {
                    Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
                }
                else {
                    Set-Item -Path "Env:$name" -Value $previousEnvironment[$name]
                }
            }
            catch {
                $cleanupErrors.Add("Environment restore for $name`: $($_.Exception.Message)")
            }
        }
        try {
            Remove-OwnedLiveScratchRoot -Path $scratchRoot
            $scratchRemoved = -not (Test-Path -LiteralPath $scratchRoot)
        }
        catch {
            $cleanupErrors.Add("Scratch cleanup: $($_.Exception.Message)")
        }
    }

    if ($null -ne $primaryError -or $cleanupErrors.Count -gt 0) {
        Write-Utf8Json `
            -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-failure.json") `
            -Value ([ordered]@{
                schema_version = 1
                scenario = $LiveScenario
                run_id = $runId
                primary_error = if ($null -eq $primaryError) { $null } else {
                    $primaryError.Exception.ToString()
                }
                cleanup_errors = @($cleanupErrors)
                scratch_root_removed = $scratchRemoved
            })
        if ($null -ne $primaryError) {
            throw $primaryError
        }
        throw "$LiveScenario cleanup failed: $([string]::Join(' | ', $cleanupErrors))"
    }
    if ($null -eq $summary) {
        throw "$LiveScenario completed without a summary."
    }
    $summary.owned_processes_cleaned = $true
    $summary.scratch_root_removed = $scratchRemoved
    $summary.webview_user_data_isolated = $true
    $summary.cleanup_stopped_process_ids = @($processCleanup.StoppedProcessIds)
    Write-Utf8Json `
        -Path (Join-Path $OutputRoot "$($LiveScenario.ToLowerInvariant())-summary.json") `
        -Value $summary
    Write-Output "Packaged GUI $LiveScenario acceptance passed."
}

function Invoke-LocaleLive {
    param(
        [Parameter(Mandatory)][string]$Desktop,
        [Parameter(Mandatory)][string]$OutputRoot,
        [Parameter(Mandatory)][int]$Timeout
    )

    if (Test-Path -LiteralPath $OutputRoot) {
        throw "Live acceptance evidence root must not already exist: $OutputRoot"
    }
    $null = New-Item -ItemType Directory -Path $OutputRoot
    $temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $buildRoot = Join-Path $temporaryBase (
        "wokrouter-packaged-gui-live-" + [Guid]::NewGuid().ToString("N")
    )
    $null = New-Item -ItemType Directory -Path $buildRoot
    $application = $null
    $results = [Collections.Generic.List[object]]::new()
    $primaryError = $null
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    $caseRoots = [Collections.Generic.List[string]]::new()
    $previousEnvironment = @{
        APPDATA = $env:APPDATA
        LOCALAPPDATA = $env:LOCALAPPDATA
        PATH = $env:PATH
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
        WEBVIEW2_USER_DATA_FOLDER = $env:WEBVIEW2_USER_DATA_FOLDER
        WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER
        WOKROUTER_ACCEPTANCE_STATE_ROOT = $env:WOKROUTER_ACCEPTANCE_STATE_ROOT
        WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE
    }
    $simplifiedChineseControl = -join @(
        0x672c,
        0x5730,
        0x684c,
        0x9762,
        0x63a7,
        0x5236
    ).ForEach({ [char]$_ })
    $cases = @(
        [ordered]@{
            name = "system-en"
            system_locale = "en-US"
            navigator_locale = "zh-CN"
            expected_lang = "en"
            expected_text = "Local desktop control"
            unexpected_text = $simplifiedChineseControl
        },
        [ordered]@{
            name = "system-zh-cn"
            system_locale = "zh-CN"
            navigator_locale = "en-US"
            expected_lang = "zh-CN"
            expected_text = $simplifiedChineseControl
            unexpected_text = "Local desktop control"
        },
        [ordered]@{
            name = "system-zh-tw-fallback-en"
            system_locale = "zh-TW"
            navigator_locale = "zh-CN"
            expected_lang = "en"
            expected_text = "Local desktop control"
            unexpected_text = $simplifiedChineseControl
        },
        [ordered]@{
            name = "navigator-zh-cn"
            system_locale = "none"
            navigator_locale = "zh-CN"
            expected_lang = "zh-CN"
            expected_text = $simplifiedChineseControl
            unexpected_text = "Local desktop control"
        },
        [ordered]@{
            name = "final-fallback-en"
            system_locale = "none"
            navigator_locale = "fr-FR"
            expected_lang = "en"
            expected_text = "Local desktop control"
            unexpected_text = $simplifiedChineseControl
        }
    )

    try {
        $application = Build-LiveAcceptanceApplication -Root $buildRoot -Desktop $Desktop
        foreach ($case in $cases) {
            $caseRoot = Join-Path $temporaryBase (
                "wokrouter-packaged-gui-live-" + [Guid]::NewGuid().ToString("N")
            )
            $caseRoots.Add($caseRoot)
            $null = New-Item -ItemType Directory -Path $caseRoot
            $roaming = Join-Path $caseRoot "roaming"
            $local = Join-Path $caseRoot "local"
            $state = Join-Path $caseRoot "fixture-state"
            $webViewData = Join-Path $caseRoot "webview-user-data"
            foreach ($directory in @($roaming, $local, $state, $webViewData)) {
                $null = New-Item -ItemType Directory -Path $directory
            }
            [IO.File]::WriteAllText(
                (Join-Path $state "serve-ready"),
                "ready",
                [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::WriteAllText(
                (Join-Path $state "current-version.txt"),
                "2.0.0",
                [Text.UTF8Encoding]::new($false)
            )

            $connection = $null
            $desktopProcess = $null
            $cdpPort = Get-FreeLoopbackPort
            try {
                $env:APPDATA = $roaming
                $env:LOCALAPPDATA = $local
                $env:PATH = Get-IsolatedWindowsPath
                $env:WOKROUTER_ACCEPTANCE_STATE_ROOT = $state
                $env:WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE = $case.system_locale
                $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS =
                    "--remote-debugging-port=$cdpPort --lang=$($case.navigator_locale)"
                $env:WEBVIEW2_USER_DATA_FOLDER = $webViewData
                $env:WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER = "1"
                $desktopProcess = Start-Process -FilePath $application.Desktop -PassThru
                $deadline = [DateTime]::UtcNow.AddSeconds($Timeout)
                $connection = Connect-PackagedDesktopCdp `
                    -Port $cdpPort `
                    -Deadline $deadline `
                    -DesktopProcess $desktopProcess `
                    -DiagnosticPath (Join-Path $OutputRoot "$($case.name)-cdp-startup.json") `
                    -ExpectedUserDataRoot $webViewData
                Add-AcceptanceDocumentScript -Connection $connection
                $expectedTextLiteral = $case.expected_text | ConvertTo-Json -Compress
                $expectedLangLiteral = $case.expected_lang | ConvertTo-Json -Compress
                Wait-CdpExpression `
                    -Connection $connection `
                    -Expression "document.documentElement.lang === $expectedLangLiteral && (document.body?.innerText ?? '').includes($expectedTextLiteral)" `
                    -Deadline $deadline `
                    -Description "the $($case.name) first localized frame"
                $observedSystemLocale = Invoke-CdpExpression `
                    -Connection $connection `
                    -Expression 'window.__wokrouterAcceptance.originalInvoke("system_locale")'
                $observedNavigatorLocale = Invoke-CdpExpression `
                    -Connection $connection `
                    -Expression 'navigator.language'
                $configuredNavigatorLanguage = (
                    [string]$case.navigator_locale
                ).Split('-')[0]
                $observedNavigatorLanguage = (
                    [string]$observedNavigatorLocale
                ).Split('-')[0]
                if (
                    $observedNavigatorLanguage -cne $configuredNavigatorLanguage
                ) {
                    throw "The $($case.name) WebView2 navigator locale was not isolated: $observedNavigatorLocale."
                }
                $frames = @(Invoke-CdpExpression `
                        -Connection $connection `
                        -Expression 'window.__wokrouterAcceptance.visibleFrames')
                if ($frames.Count -lt 1) {
                    throw "The $($case.name) locale run did not record a visible frame."
                }
                $unexpectedFrames = @($frames | Where-Object {
                        ([string]$_.text).Contains([string]$case.unexpected_text)
                    })
                if (
                    [string]$frames[0].lang -cne [string]$case.expected_lang -or
                    -not ([string]$frames[0].text).Contains([string]$case.expected_text) -or
                    $unexpectedFrames.Count -ne 0
                ) {
                    throw "The $($case.name) locale run exposed an incorrect first visible frame."
                }
                Save-CdpDomEvidence `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "$($case.name)-first-frame.json")
                Save-CdpScreenshot `
                    -Connection $connection `
                    -Path (Join-Path $OutputRoot "$($case.name)-first-frame.png")
                $results.Add([ordered]@{
                        name = $case.name
                        system_locale = $observedSystemLocale
                        configured_system_locale = $case.system_locale
                        configured_navigator_locale = $case.navigator_locale
                        observed_navigator_locale = $observedNavigatorLocale
                        expected_lang = $case.expected_lang
                        observed_lang = [string]$frames[0].lang
                        visible_frame_count = $frames.Count
                        wrong_language_frame_count = $unexpectedFrames.Count
                    })
            }
            finally {
                if ($null -ne $connection) {
                    try { $connection.Socket.Dispose() } catch {}
                }
                try {
                    Stop-OwnedProcessesByExecutable -Executables @(
                        $application.Desktop,
                        $application.Sidecar,
                        $application.Fixture
                    ) | Out-Null
                }
                catch {
                    $cleanupErrors.Add(
                        "$($case.name) owned process cleanup: $($_.Exception.Message)"
                    )
                }
                if (
                    $null -ne $connection -and
                    $connection.OwnedWebViewProcesses.Count -gt 0
                ) {
                    try {
                        Stop-RecordedAcceptanceWebViewProcesses `
                            -Processes @($connection.OwnedWebViewProcesses) `
                            -UserDataRoot $webViewData `
                            -RemoteDebuggingPort $cdpPort | Out-Null
                    }
                    catch {
                        $cleanupErrors.Add(
                            "$($case.name) WebView cleanup: $($_.Exception.Message)"
                        )
                    }
                }
                try { Remove-OwnedLiveScratchRoot -Path $caseRoot }
                catch {
                    $cleanupErrors.Add(
                        "$($case.name) scratch cleanup: $($_.Exception.Message)"
                    )
                }
            }
        }
    }
    catch {
        $primaryError = $_
    }
    finally {
        foreach ($name in $previousEnvironment.Keys) {
            try {
                if ($null -eq $previousEnvironment[$name]) {
                    Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
                }
                else {
                    Set-Item -Path "Env:$name" -Value $previousEnvironment[$name]
                }
            }
            catch {
                $cleanupErrors.Add("Environment restore for $name`: $($_.Exception.Message)")
            }
        }
        try {
            if ($null -ne $application) {
                Stop-OwnedProcessesByExecutable -Executables @(
                    $application.Desktop,
                    $application.Sidecar,
                    $application.Fixture
                ) | Out-Null
            }
        }
        catch {
            $cleanupErrors.Add("Locale final process cleanup: $($_.Exception.Message)")
        }
        foreach ($caseRoot in $caseRoots) {
            if (Test-Path -LiteralPath $caseRoot) {
                try { Remove-OwnedLiveScratchRoot -Path $caseRoot }
                catch {
                    $cleanupErrors.Add("Locale scratch cleanup: $($_.Exception.Message)")
                }
            }
        }
        if (Test-Path -LiteralPath $buildRoot) {
            try { Remove-OwnedLiveScratchRoot -Path $buildRoot }
            catch {
                $cleanupErrors.Add("Locale build cleanup: $($_.Exception.Message)")
            }
        }
    }

    if ($null -ne $primaryError -or $cleanupErrors.Count -gt 0) {
        Write-Utf8Json `
            -Path (Join-Path $OutputRoot "locale-failure.json") `
            -Value ([ordered]@{
                schema_version = 1
                primary_error = if ($null -eq $primaryError) { $null } else {
                    $primaryError.Exception.ToString()
                }
                cleanup_errors = @($cleanupErrors)
                completed_cases = @($results)
            })
        if ($null -ne $primaryError) {
            throw $primaryError
        }
        throw "Locale cleanup failed: $([string]::Join(' | ', $cleanupErrors))"
    }
    if ($results.Count -ne $cases.Count) {
        throw "Locale acceptance did not complete every configured case."
    }
    Write-Utf8Json `
        -Path (Join-Path $OutputRoot "locale-summary.json") `
        -Value ([ordered]@{
            schema_version = 1
            scenario = "Locale"
            case_count = $results.Count
            cases = @($results)
            first_document_script_debugger_gate = $true
            wrong_language_frame_count = @(
                $results | ForEach-Object wrong_language_frame_count | Measure-Object -Sum
            )[0].Sum
            scratch_roots_removed = $true
            webview_user_data_isolated = $true
            acceptance_feature_release_excluded = $true
        })
    Write-Output "Packaged GUI Locale acceptance passed."
}

if ($SelfTest) {
    Invoke-HarnessSelfTest -OutputRoot $EvidenceRoot -Timeout $TimeoutSeconds
    exit 0
}

$resolvedDesktop = Resolve-RegularExecutablePath `
    -Path $DesktopExecutable `
    -Description "Packaged desktop executable"
$resolvedMinisign = Resolve-RegularExecutablePath `
    -Path $MinisignPath `
    -Description "Minisign executable"

if ($Scenario -ceq "MissingInstall") {
    Invoke-MissingInstallLive `
        -Desktop $resolvedDesktop `
        -Minisign $resolvedMinisign `
        -OutputRoot $EvidenceRoot `
        -Timeout $TimeoutSeconds
    exit 0
}

if ($Scenario -in @("UpdateCancelConfirm", "ActiveRequests", "Rollback", "CloseReopen")) {
    Invoke-PreinstalledUpdateLive `
        -LiveScenario $Scenario `
        -Desktop $resolvedDesktop `
        -Minisign $resolvedMinisign `
        -OutputRoot $EvidenceRoot `
        -Timeout $TimeoutSeconds
    exit 0
}

if ($Scenario -ceq "Locale") {
    Invoke-LocaleLive `
        -Desktop $resolvedDesktop `
        -OutputRoot $EvidenceRoot `
        -Timeout $TimeoutSeconds
    exit 0
}

if ($Scenario -ceq "All") {
    if (Test-Path -LiteralPath $EvidenceRoot) {
        throw "Live acceptance evidence root must not already exist: $EvidenceRoot"
    }
    $null = New-Item -ItemType Directory -Path $EvidenceRoot
    foreach ($liveScenario in @(
            "MissingInstall",
            "UpdateCancelConfirm",
            "ActiveRequests",
            "Rollback",
            "CloseReopen"
        )) {
        if ($liveScenario -ceq "MissingInstall") {
            Invoke-MissingInstallLive `
                -Desktop $resolvedDesktop `
                -Minisign $resolvedMinisign `
                -OutputRoot (Join-Path $EvidenceRoot $liveScenario) `
                -Timeout $TimeoutSeconds
        }
        else {
            Invoke-PreinstalledUpdateLive `
                -LiveScenario $liveScenario `
                -Desktop $resolvedDesktop `
                -Minisign $resolvedMinisign `
                -OutputRoot (Join-Path $EvidenceRoot $liveScenario) `
                -Timeout $TimeoutSeconds
        }
    }
    Invoke-LocaleLive `
        -Desktop $resolvedDesktop `
        -OutputRoot (Join-Path $EvidenceRoot "Locale") `
        -Timeout $TimeoutSeconds
    Write-Output "All packaged GUI acceptance scenarios passed."
    exit 0
}

throw "Live packaged GUI acceptance scenario '$Scenario' is not implemented yet."
