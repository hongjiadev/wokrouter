[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$DesktopExecutable,

    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"

if (-not $IsWindows -and $PSVersionTable.PSEdition -eq "Core") {
    throw "The packaged event bridge smoke test requires Windows."
}

$desktop = (Resolve-Path -LiteralPath $DesktopExecutable).Path
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "wokrouter-event-smoke-" + [guid]::NewGuid().ToString("N")
)
$applicationRoot = Join-Path $temporaryRoot "app"
$roamingRoot = Join-Path $temporaryRoot "roaming"
$localRoot = Join-Path $temporaryRoot "local"
$webViewDataRoot = Join-Path $temporaryRoot "webview-user-data"
$fakeSource = Join-Path $temporaryRoot "fake-wokrouter.rs"
$fakeSidecar = Join-Path $applicationRoot "wokrouter.exe"
$desktopCopy = Join-Path $applicationRoot "wokrouter-desktop.exe"
$sidecarMarker = Join-Path $temporaryRoot "sidecar-started"
$process = $null
$socket = $null
$webViewProcesses = @()
$port = 0
$previousAppData = $env:APPDATA
$previousLocalAppData = $env:LOCALAPPDATA
$previousWebViewArguments = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$previousWebViewDataRoot = $env:WEBVIEW2_USER_DATA_FOLDER
$previousSmokeMarker = $env:WOKROUTER_EVENT_SMOKE_MARKER

function Get-FreeLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new(
        [Net.IPAddress]::Loopback,
        0
    )
    $listener.Start()
    try {
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Receive-WebSocketJson {
    param(
        [Parameter(Mandatory)]
        [Net.WebSockets.ClientWebSocket]$Socket
    )

    $buffer = [byte[]]::new(16 * 1024)
    $output = [IO.MemoryStream]::new()
    try {
        do {
            $segment = [ArraySegment[byte]]::new($buffer)
            $result = $Socket.ReceiveAsync(
                $segment,
                [Threading.CancellationToken]::None
            ).GetAwaiter().GetResult()
            if ($result.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) {
                throw "The WebView2 DevTools socket closed before the smoke assertion."
            }
            $output.Write($buffer, 0, $result.Count)
        } while (-not $result.EndOfMessage)
        return [Text.Encoding]::UTF8.GetString($output.ToArray()) |
            ConvertFrom-Json
    }
    finally {
        $output.Dispose()
    }
}

function Invoke-DevToolsExpression {
    param(
        [Parameter(Mandatory)]
        [Net.WebSockets.ClientWebSocket]$Socket,

        [Parameter(Mandatory)]
        [int]$Id,

        [Parameter(Mandatory)]
        [string]$Expression
    )

    $request = @{
        id = $Id
        method = "Runtime.evaluate"
        params = @{
            expression = $Expression
            returnByValue = $true
        }
    } | ConvertTo-Json -Compress -Depth 4
    $bytes = [Text.Encoding]::UTF8.GetBytes($request)
    $null = $Socket.SendAsync(
        [ArraySegment[byte]]::new($bytes),
        [Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult()
    do {
        $response = Receive-WebSocketJson -Socket $Socket
    } while ($response.id -ne $Id)
    return $response.result.result.value
}

function Get-OwnedWebViewProcesses {
    param(
        [Parameter(Mandatory)][int]$DesktopProcessId,
        [Parameter(Mandatory)][string]$UserDataRoot,
        [Parameter(Mandatory)][int]$RemoteDebuggingPort
    )

    $processes = @(Get-CimInstance -ClassName Win32_Process)
    $descendantIds = [Collections.Generic.HashSet[int]]::new()
    $null = $descendantIds.Add($DesktopProcessId)
    do {
        $added = $false
        foreach ($candidate in $processes) {
            if (
                $descendantIds.Contains([int]$candidate.ParentProcessId) -and
                $descendantIds.Add([int]$candidate.ProcessId)
            ) {
                $added = $true
            }
        }
    } while ($added)
    $canonicalRoot = [IO.Path]::GetFullPath($UserDataRoot)
    return @($processes | Where-Object {
            $_.Name -ieq "msedgewebview2.exe" -and
            $descendantIds.Contains([int]$_.ProcessId) -and
            ([string]$_.CommandLine).IndexOf(
                $canonicalRoot,
                [StringComparison]::OrdinalIgnoreCase
            ) -ge 0 -and
            ([string]$_.CommandLine) -like
                "*--remote-debugging-port=$RemoteDebuggingPort*"
        } | ForEach-Object {
            [pscustomobject]@{
                ProcessId = [int]$_.ProcessId
                ExecutablePath = [string]$_.ExecutablePath
                CommandLine = [string]$_.CommandLine
            }
        })
}

function Stop-OwnedWebViewProcesses {
    param(
        [Parameter(Mandatory)][object[]]$Processes,
        [Parameter(Mandatory)][string]$UserDataRoot,
        [Parameter(Mandatory)][int]$RemoteDebuggingPort
    )

    $canonicalRoot = [IO.Path]::GetFullPath($UserDataRoot)
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $remaining = @($Processes | Where-Object {
                $null -ne (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue)
            })
        if ($remaining.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    foreach ($record in $remaining) {
        $current = Get-CimInstance `
            -ClassName Win32_Process `
            -Filter "ProcessId = $($record.ProcessId)" `
            -ErrorAction SilentlyContinue
        if ($null -eq $current) {
            continue
        }
        $commandLine = [string]$current.CommandLine
        if (
            $current.Name -ine "msedgewebview2.exe" -or
            [string]$current.ExecutablePath -ine $record.ExecutablePath -or
            $commandLine -cne $record.CommandLine -or
            $commandLine.IndexOf(
                $canonicalRoot,
                [StringComparison]::OrdinalIgnoreCase
            ) -lt 0 -or
            $commandLine -notlike "*--remote-debugging-port=$RemoteDebuggingPort*"
        ) {
            throw "Refusing to stop a WebView2 process without its exact smoke identity."
        }
        Stop-Process -Id $record.ProcessId -Force
        $ownedProcess = Get-Process -Id $record.ProcessId -ErrorAction SilentlyContinue
        if ($null -ne $ownedProcess) {
            if (-not $ownedProcess.WaitForExit(5000)) {
                throw "An exact smoke WebView2 process did not exit after it was stopped."
            }
            $ownedProcess.Dispose()
        }
    }
}

function Remove-SmokeRoot {
    param([Parameter(Mandatory)][string]$Path)

    $canonical = [IO.Path]::GetFullPath($Path)
    $temporary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    if (
        [IO.Path]::GetDirectoryName($canonical).TrimEnd(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        ) -cne $temporary -or
        [IO.Path]::GetFileName($canonical) -cnotmatch
            '^wokrouter-event-smoke-[0-9a-f]{32}$'
    ) {
        throw "Refusing to remove an unexpected packaged event smoke root."
    }
    foreach ($attempt in 1..50) {
        try {
            Remove-Item -LiteralPath $canonical -Recurse -Force -ErrorAction Stop
            return
        }
        catch [IO.IOException], [UnauthorizedAccessException] {
            if ($attempt -eq 50) {
                throw
            }
            Start-Sleep -Milliseconds 100
        }
    }
}

try {
    $null = New-Item -ItemType Directory -Path $applicationRoot -Force
    $null = New-Item -ItemType Directory -Path $roamingRoot -Force
    $null = New-Item -ItemType Directory -Path $localRoot -Force
    $null = New-Item -ItemType Directory -Path $webViewDataRoot -Force
    [IO.File]::Copy($desktop, $desktopCopy)

    $fakeProgram = @'
use std::{io::Write, thread, time::Duration};

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments != ["start", "--json", "--progress-jsonl"] {
        std::process::exit(64);
    }
    std::fs::write(
        std::env::var_os("WOKROUTER_EVENT_SMOKE_MARKER").unwrap(),
        std::process::id().to_string(),
    ).unwrap();
    eprintln!("{}", r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"checking_release"}"#);
    std::io::stderr().flush().unwrap();
    thread::sleep(Duration::from_secs(20));
    eprintln!("{}", r#"{"schema_version":1,"sequence":1,"operation":"install","state":"succeeded","phase":"completed"}"#);
    println!("{}", r#"{"code":"running"}"#);
}
'@
    [IO.File]::WriteAllText(
        $fakeSource,
        $fakeProgram,
        [Text.UTF8Encoding]::new($false)
    )
    & rustup.exe run 1.97.1 rustc.exe --edition 2024 $fakeSource -o $fakeSidecar
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to compile the temporary WokRouter smoke sidecar."
    }

    $port = Get-FreeLoopbackPort
    $env:APPDATA = $roamingRoot
    $env:LOCALAPPDATA = $localRoot
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$port"
    $env:WEBVIEW2_USER_DATA_FOLDER = $webViewDataRoot
    $env:WOKROUTER_EVENT_SMOKE_MARKER = $sidecarMarker
    $process = Start-Process -FilePath $desktopCopy -PassThru

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $target = $null
    do {
        try {
            $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$port/json/list"
            $target = @($targets | Where-Object {
                    $_.type -eq "page" -and
                    -not [string]::IsNullOrWhiteSpace($_.webSocketDebuggerUrl)
                } | Select-Object -First 1)
        }
        catch {
            $target = $null
        }
        if ($null -eq $target -or $target.Count -eq 0) {
            Start-Sleep -Milliseconds 100
        }
    } while (($null -eq $target -or $target.Count -eq 0) -and [DateTime]::UtcNow -lt $deadline)
    if ($null -eq $target -or $target.Count -eq 0) {
        throw "The packaged desktop did not expose a WebView2 page before the smoke timeout."
    }

    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $null = $socket.ConnectAsync(
        [Uri]$target[0].webSocketDebuggerUrl,
        [Threading.CancellationToken]::None
    ).GetAwaiter().GetResult()
    $webViewProcesses = @(Get-OwnedWebViewProcesses `
            -DesktopProcessId $process.Id `
            -UserDataRoot $webViewDataRoot `
            -RemoteDebuggingPort $port)
    if ($webViewProcesses.Count -eq 0) {
        throw "The packaged desktop smoke did not own an isolated WebView2 process."
    }

    $requestId = 0
    $progressVisible = $false
    do {
        $requestId += 1
        $progressVisible = [bool](Invoke-DevToolsExpression `
            -Socket $socket `
            -Id $requestId `
            -Expression 'document.querySelector(''[role="progressbar"]'') !== null')
        if (-not $progressVisible) {
            Start-Sleep -Milliseconds 100
        }
    } while (
        (
            -not $progressVisible -or
            -not (Test-Path -LiteralPath $sidecarMarker -PathType Leaf)
        ) -and
        [DateTime]::UtcNow -lt $deadline
    )
    if (-not $progressVisible -or -not (Test-Path -LiteralPath $sidecarMarker -PathType Leaf)) {
        $requestId += 1
        $body = Invoke-DevToolsExpression `
            -Socket $socket `
            -Id $requestId `
            -Expression 'document.body?.innerText ?? ""'
        throw "The packaged desktop never started the operation sidecar with observable progress. Body: $body"
    }

    Write-Output "Packaged desktop event bridge smoke passed."
}
finally {
    if ($null -ne $socket) {
        $socket.Dispose()
    }
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
        $process.WaitForExit()
        $process.Dispose()
    }
    if ($webViewProcesses.Count -gt 0 -and $port -gt 0) {
        Stop-OwnedWebViewProcesses `
            -Processes @($webViewProcesses) `
            -UserDataRoot $webViewDataRoot `
            -RemoteDebuggingPort $port
    }
    if (Test-Path -LiteralPath $sidecarMarker -PathType Leaf) {
        $sidecarProcessId = 0
        if ([int]::TryParse(
                (Get-Content -LiteralPath $sidecarMarker -Raw),
                [ref]$sidecarProcessId
            )) {
            $sidecarProcess = Get-Process -Id $sidecarProcessId -ErrorAction SilentlyContinue
            if (
                $null -ne $sidecarProcess -and
                $null -ne $sidecarProcess.Path -and
                [IO.Path]::GetFullPath($sidecarProcess.Path) -ieq
                [IO.Path]::GetFullPath($fakeSidecar)
            ) {
                Stop-Process -Id $sidecarProcessId -Force
                $sidecarProcess.WaitForExit()
            }
        }
    }
    $env:APPDATA = $previousAppData
    $env:LOCALAPPDATA = $previousLocalAppData
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousWebViewArguments
    $env:WEBVIEW2_USER_DATA_FOLDER = $previousWebViewDataRoot
    $env:WOKROUTER_EVENT_SMOKE_MARKER = $previousSmokeMarker
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-SmokeRoot -Path $temporaryRoot
    }
}
