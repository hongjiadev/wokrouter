[CmdletBinding()]
param(
    [string]$ScenarioPattern = ""
)

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
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/cli/src/commands") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src/components") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src/i18n/locales") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src-tauri/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src-tauri/capabilities") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src-tauri/src/core_operation") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/cli/src/commands/start") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/src/wokcore_install") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/src/system") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/tests") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-wokcore-client/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs/operations") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "tests/release") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "tests/scripts") -Force
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot ".github/workflows/ci.yml") `
        -Destination (Join-Path $root ".github/workflows/ci.yml")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "deny.toml") `
        -Destination (Join-Path $root "deny.toml")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "docs/operations/development.md") `
        -Destination (Join-Path $root "docs/operations/development.md")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/cli/src/commands/mod.rs") `
        -Destination (Join-Path $root "apps/cli/src/commands/mod.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/control.ts") `
        -Destination (Join-Path $root "apps/desktop/src/control.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/coreUpdateEligibility.ts") `
        -Destination (Join-Path $root "apps/desktop/src/coreUpdateEligibility.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/components/CoreLifecycle.tsx") `
        -Destination (Join-Path $root "apps/desktop/src/components/CoreLifecycle.tsx")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/components/CoreLifecycle.test.tsx") `
        -Destination (Join-Path $root "apps/desktop/src/components/CoreLifecycle.test.tsx")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/components/ManagementPanel.tsx") `
        -Destination (Join-Path $root "apps/desktop/src/components/ManagementPanel.tsx")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/locale.test.ts") `
        -Destination (Join-Path $root "apps/desktop/src/locale.test.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/locale.ts") `
        -Destination (Join-Path $root "apps/desktop/src/locale.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/package.json") `
        -Destination (Join-Path $root "apps/desktop/package.json")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/main.tsx") `
        -Destination (Join-Path $root "apps/desktop/src/main.tsx")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/i18n/index.ts") `
        -Destination (Join-Path $root "apps/desktop/src/i18n/index.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/i18n/locales/en.json") `
        -Destination (Join-Path $root "apps/desktop/src/i18n/locales/en.json")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/i18n/locales/zh-CN.json") `
        -Destination (Join-Path $root "apps/desktop/src/i18n/locales/zh-CN.json")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/control.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/control.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/core_operation.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/core_operation.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/core_operation/helper.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/core_operation/helper.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/core_operation/journal.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/core_operation/journal.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/core_operation/parser.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/core_operation/parser.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/lib.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/lib.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/capabilities/main.json") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/capabilities/main.json")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/main.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/main.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "tests/release/package-windows-assets.ps1") `
        -Destination (Join-Path $root "tests/release/package-windows-assets.ps1")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/src/wokcore_runtime.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/src/wokcore_runtime.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-wokcore-client/src/lib.rs") `
        -Destination (Join-Path $root "crates/wokrouter-wokcore-client/src/lib.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/src/system/locale.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/src/system/locale.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "tests/scripts/smoke-packaged-event-bridge.ps1") `
        -Destination (Join-Path $root "tests/scripts/smoke-packaged-event-bridge.ps1")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "tests/scripts/packaged-gui-acceptance.ps1") `
        -Destination (Join-Path $root "tests/scripts/packaged-gui-acceptance.ps1")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "tests/scripts/packaged-gui-acceptance.tests.ps1") `
        -Destination (Join-Path $root "tests/scripts/packaged-gui-acceptance.tests.ps1")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/tests/wokcore_runtime.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/tests/wokcore_runtime.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/tests/wokcore_install.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/tests/wokcore_install.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/cli/src/commands/start/tests.rs") `
        -Destination (Join-Path $root "apps/cli/src/commands/start/tests.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/cli/src/commands/start.rs") `
        -Destination (Join-Path $root "apps/cli/src/commands/start.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub") `
        -Destination (Join-Path $root "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub")
    $fixtureRoots.Add($root)
    return $root
}
function Edit-FixtureFile {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$RelativePath,

        [Parameter(Mandatory)]
        [string]$OldText,

        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$NewText
    )

    $path = Join-Path $Root $RelativePath
    $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
    $content = $content.Replace("`r`n", "`n")
    $OldText = $OldText.Replace("`r`n", "`n")
    $NewText = $NewText.Replace("`r`n", "`n")
    $occurrences = [regex]::Matches(
        $content,
        [regex]::Escape($OldText)
    ).Count
    if ($occurrences -ne 1) {
        throw "Fixture mutation source must occur exactly once in ${RelativePath}; found ${occurrences}: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($OldText, $NewText) -Encoding UTF8
}

function Add-FixtureTextFile {
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
    if (Test-Path -LiteralPath $path) {
        throw "Fixture text file must not already exist: $RelativePath"
    }
    Set-Content -LiteralPath $path -Value $Content -Encoding UTF8
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
    $occurrences = [regex]::Matches(
        $content,
        [regex]::Escape($OldText)
    ).Count
    if ($occurrences -ne 1) {
        throw "Fixture mutation source must occur exactly once; found ${occurrences}: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($OldText, $NewText) -Encoding UTF8
}

function Edit-WorkflowJob {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$JobName,

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
    $jobPattern = "(?ms)^  $([regex]::Escape($JobName)):`n.*?(?=^  [A-Za-z0-9_-]+:`n|^`S|\z)"
    $jobs = [regex]::Matches($content, $jobPattern)
    if ($jobs.Count -ne 1) {
        throw "Workflow job '$JobName' must occur exactly once; found $($jobs.Count)."
    }
    $job = $jobs[0]
    $occurrences = [regex]::Matches(
        $job.Value,
        [regex]::Escape($OldText)
    ).Count
    if ($occurrences -ne 1) {
        throw "Fixture mutation source must occur exactly once in workflow job '$JobName'; found ${occurrences}: $OldText"
    }
    $changedJob = $job.Value.Replace($OldText, $NewText)
    $changed = $content.Remove($job.Index, $job.Length).Insert($job.Index, $changedJob)
    Set-Content -LiteralPath $path -Value $changed -Encoding UTF8
}

function Set-FixedHostCondition {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Condition
    )

    Edit-Workflow `
        -Root $Root `
        -OldText @"
      - name: Test workspace through fixed Windows host
        if: runner.os == 'Windows' && matrix.target == 'x86_64-pc-windows-msvc'
"@ `
        -NewText @"
      - name: Test workspace through fixed Windows host
        if: $Condition
"@
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

    if (
        $ScenarioPattern.Length -gt 0 -and
        $Name -notmatch $ScenarioPattern
    ) {
        return
    }
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

    Invoke-Scenario -Name "foundation checker accepts CRLF fixture sources" -Test {
        $root = New-ContractFixture
        foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File) {
            $source = [IO.File]::ReadAllText($file.FullName).Replace(
                "`r`n",
                "`n"
            ).Replace("`r", "`n").Replace("`n", "`r`n")
            [IO.File]::WriteAllText(
                $file.FullName,
                $source,
                [Text.UTF8Encoding]::new($false)
            )
        }
        Assert-ContractPasses -Root $root -Scenario "CRLF workflow fixture"
    }

    Invoke-Scenario -Name "an unrelated timer does not obscure the external install poll" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/CoreLifecycle.tsx" `
            -OldText '  const waitsForAnotherProcess =' `
            -NewText @'
  useEffect(() => {
    const poll = window.setInterval(() => {
      void Promise.resolve();
    }, 1_000);
    return () => window.clearInterval(poll);
  }, []);
  const waitsForAnotherProcess =
'@
        Assert-ContractPasses `
            -Root $root `
            -Scenario "core lifecycle with an unrelated timer"
    }

    Invoke-Scenario -Name "desktop event capability cannot omit unlisten" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/capabilities/main.json" `
            -OldText '    "core:event:allow-unlisten"' `
            -NewText '    "core:event:allow-emit"'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "only core event listen and unlisten" `
            -Scenario "desktop event capability without unlisten"
    }

    Invoke-Scenario -Name "desktop event capability cannot grant event emission" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/capabilities/main.json" `
            -OldText '    "core:event:allow-unlisten"' `
            -NewText "    `"core:event:allow-unlisten`",`n    `"core:event:allow-emit`""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "only core event listen and unlisten" `
            -Scenario "desktop event capability with emit permission"
    }

    Invoke-Scenario -Name "desktop event capability cannot target every window" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/capabilities/main.json" `
            -OldText '  "windows": ["main"],' `
            -NewText '  "windows": ["*"],'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "only core event listen and unlisten" `
            -Scenario "desktop event capability targeting every window"
    }

    Invoke-Scenario -Name "system locale cannot turn an invalid OS candidate into English" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/system/locale.rs" `
            -OldText "    candidate.and_then(normalize_locale)`n" `
            -NewText "    candidate.and_then(normalize_locale).or_else(|| Some(FALLBACK_LOCALE.into()))`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Option::None" `
            -Scenario "system locale invalid candidate coerced to English"
    }

    Invoke-Scenario -Name "Tauri locale command cannot erase candidate absence" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/src/lib.rs" `
            -OldText 'wokrouter_platform::detect_system_locale()' `
            -NewText 'Some("en".into())'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "preserve the optional OS locale" `
            -Scenario "Tauri locale command erasing candidate absence"
    }

    Invoke-Scenario -Name "frontend locale resolver must accept null" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/locale.ts" `
            -OldText "  systemLocale: string | null | undefined,`n" `
            -NewText "  systemLocale: string | undefined,`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "accept a null OS candidate" `
            -Scenario "frontend locale resolver rejecting null"
    }

    Invoke-Scenario -Name "desktop bootstrap invoke must preserve null" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText 'invoke<string | null>("system_locale")' `
            -NewText 'invoke<string>("system_locale")'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "desktop bootstrap invoke erasing null"
    }

    Invoke-Scenario -Name "packaged event smoke cannot skip the sidecar marker" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/smoke-packaged-event-bridge.ps1" `
            -OldText @'
            -not $progressVisible -or
            -not (Test-Path -LiteralPath $sidecarMarker -PathType Leaf)
'@ `
            -NewText @'
            -not $progressVisible
'@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/smoke-packaged-event-bridge.ps1" `
            -OldText 'if (-not $progressVisible -or -not (Test-Path -LiteralPath $sidecarMarker -PathType Leaf)) {' `
            -NewText 'if (-not $progressVisible) {'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "observe a started sidecar plus WebView progress" `
            -Scenario "packaged event smoke without sidecar marker assertion"
    }

    Invoke-Scenario -Name "packaged event smoke must isolate WebView2 user data" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/smoke-packaged-event-bridge.ps1" `
            -OldText '$env:WEBVIEW2_USER_DATA_FOLDER = $webViewDataRoot' `
            -NewText '$null = $webViewDataRoot'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "isolate exact WebView ownership" `
            -Scenario "packaged event smoke sharing WebView user data"
    }

    Invoke-Scenario -Name "packaged GUI acceptance self-test must execute the harness" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/packaged-gui-acceptance.tests.ps1" `
            -OldText '& $harness -SelfTest -EvidenceRoot $evidenceRoot -TimeoutSeconds 10' `
            -NewText '$null = $harness'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "packaged GUI acceptance self-test" `
            -Scenario "packaged GUI acceptance self-test bypass"
    }

    Invoke-Scenario -Name "packaged GUI live acceptance must preserve cold-start locale input" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/packaged-gui-acceptance.ps1" `
            -OldText '"--remote-debugging-port=$cdpPort --lang=$($case.navigator_locale)"' `
            -NewText '"--remote-debugging-port=$cdpPort"'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "all six isolated scenarios" `
            -Scenario "packaged GUI live acceptance without cold-start navigator locale"
    }

    Invoke-Scenario -Name "packaged GUI All dispatcher must invoke locale as active code" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/packaged-gui-acceptance.ps1" `
            -OldText @'
    Invoke-LocaleLive `
        -Desktop $resolvedDesktop `
        -OutputRoot (Join-Path $EvidenceRoot "Locale") `
        -Timeout $TimeoutSeconds
'@ `
            -NewText @'
    # Invoke-LocaleLive `
    #     -Desktop $resolvedDesktop `
    #     -OutputRoot (Join-Path $EvidenceRoot "Locale") `
    #     -Timeout $TimeoutSeconds
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "packaged GUI live harness" `
            -Scenario "packaged GUI All dispatcher with a commented locale call"
    }

    Invoke-Scenario -Name "packaged GUI All dispatcher cannot exit before scenarios" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/packaged-gui-acceptance.ps1" `
            -OldText 'if ($Scenario -ceq "All") {' `
            -NewText "if (`$Scenario -ceq `"All`") {`n    exit 0"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "packaged GUI live harness" `
            -Scenario "packaged GUI All dispatcher with an early exit"
    }

    Invoke-Scenario -Name "packaged GUI live functions cannot hide a nested early return" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/scripts/packaged-gui-acceptance.ps1" `
            -OldText 'function Invoke-LocaleLive {' `
            -NewText "function Invoke-LocaleLive {`n    if (`$true) { return }"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "packaged GUI live harness" `
            -Scenario "packaged GUI locale function with a nested early return"
    }

    Invoke-Scenario -Name "fixture mutations require one exact source occurrence" -Test {
        $root = New-ContractFixture
        $rejected = $false
        try {
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
                -OldText "CoreConnection" `
                -NewText "ChangedConnection"
        }
        catch {
            $rejected = $true
            if ($_.Exception.Message -notmatch "exactly once") {
                throw "duplicate mutation reported the wrong failure: $($_.Exception.Message)"
            }
        }
        if (-not $rejected) {
            throw "duplicate mutation source should be rejected"
        }
    }

    Invoke-Scenario -Name "desktop bootstrap cannot render before i18n initialization" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  await initializeI18n(locale);`n" `
            -NewText ""
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText @"
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
"@ `
            -NewText @"
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
  await initializeI18n(locale);
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "before desktop rendering" `
            -Scenario "desktop bootstrap with reversed i18n initialization order"
    }

    Invoke-Scenario -Name "desktop bootstrap requirements cannot survive in a template literal" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText @'
export async function bootstrap(): Promise<void> {
  const systemLocale = await invoke<string | null>("system_locale").catch(
    () => undefined,
  );
  const locale = resolveSupportedLocale(
    systemLocale,
    browserLocaleCandidates(window.navigator),
  );
  await initializeI18n(locale);
  initializeDocumentLocale(document.documentElement, locale);

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("WokRouter desktop root is missing.");
  }

  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}
'@ `
            -NewText @'
export async function bootstrap(): Promise<void> {
  const decoy = `
invoke<string | null>("system_locale")
await initializeI18n(locale)
createRoot(root).render()
`;
  void decoy;
}
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap contract text retained only in a template literal"
    }

    Invoke-Scenario -Name "desktop bootstrap requirements cannot survive in a nested template interpolation" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText @'
export async function bootstrap(): Promise<void> {
  const systemLocale = await invoke<string | null>("system_locale").catch(
    () => undefined,
  );
  const locale = resolveSupportedLocale(
    systemLocale,
    browserLocaleCandidates(window.navigator),
  );
  await initializeI18n(locale);
  initializeDocumentLocale(document.documentElement, locale);

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("WokRouter desktop root is missing.");
  }

  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}
'@ `
            -NewText @'
export async function bootstrap(): Promise<void> {
  const decoy = `outer \` ${/* block comment */
// line comment
`
;
  const systemLocale = await invoke<string | null>("system_locale").catch(
    () => undefined,
  );
  const locale = resolveSupportedLocale(
    systemLocale,
    browserLocaleCandidates(window.navigator),
  );
  await initializeI18n(locale);
  initializeDocumentLocale(document.documentElement, locale);

  const root = document.getElementById("root");
  if (!root) {
    throw new Error("WokRouter desktop root is missing.");
  }

  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
`}`;
  void decoy;
}
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap contract text retained in a nested interpolation template"
    }

    Invoke-Scenario -Name "desktop bootstrap requirements cannot live under if false" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n" `
            -NewText "  if (false) {`n    const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  );`n}`n`nvoid bootstrap();" `
            -NewText "  );`n  }`n}`n`nvoid bootstrap();"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap contract statements retained only under if false"
    }

    Invoke-Scenario -Name "desktop bootstrap requirements cannot live in an uncalled nested function" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText @'
export async function bootstrap(): Promise<void> {
  const systemLocale = await invoke<string | null>("system_locale").catch(
'@ `
            -NewText @'
export async function bootstrap(): Promise<void> {
  async function decoy(): Promise<void> {
    const systemLocale = await invoke<string | null>("system_locale").catch(
'@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  );`n}`n`nvoid bootstrap();" `
            -NewText "  );`n  }`n  void decoy;`n}`n`nvoid bootstrap();"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap contract statements retained only in an uncalled nested function"
    }

    Invoke-Scenario -Name "desktop bootstrap cannot return before locale initialization" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n" `
            -NewText "  return;`n  const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap returning before locale initialization"
    }

    Invoke-Scenario -Name "desktop bootstrap cannot continue after literal true return" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "  const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n" `
            -NewText "  if (true) { return; }`n  const systemLocale = await invoke<string | null>(`"system_locale`").catch(`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable direct bootstrap statements" `
            -Scenario "bootstrap continuing after a literal-true terminal branch"
    }

    Invoke-Scenario -Name "desktop module must invoke bootstrap" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/main.tsx" `
            -OldText "`nvoid bootstrap();`n" `
            -NewText "`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "invoke bootstrap at module scope" `
            -Scenario "desktop module without its bootstrap invocation"
    }

    Invoke-Scenario -Name "desktop supported languages cannot append another locale" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/i18n/index.ts" `
            -OldText '    supportedLngs: ["en", "zh-CN"],' `
            -NewText '    supportedLngs: ["en", "zh-CN"].concat(["fr"]),'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "awaited i18n.init options" `
            -Scenario "desktop supported language list with an appended locale"
    }

    Invoke-Scenario -Name "desktop supported languages cannot survive only in a dead object" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/i18n/index.ts" `
            -OldText "  await i18n.use(initReactI18next).init({`n" `
            -NewText "  const decoy = { supportedLngs: [`"en`", `"zh-CN`"] };`n  void decoy;`n  await i18n.use(initReactI18next).init({`n"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/i18n/index.ts" `
            -OldText "    supportedLngs: [`"en`", `"zh-CN`"],`n" `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "awaited i18n.init options" `
            -Scenario "supported languages retained only in an unrelated object"
    }

    Invoke-Scenario -Name "desktop supported languages cannot be overridden by spread" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/i18n/index.ts" `
            -OldText "  await i18n.use(initReactI18next).init({`n" `
            -NewText "  const override = { supportedLngs: [`"fr`"] };`n  await i18n.use(initReactI18next).init({`n"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/i18n/index.ts" `
            -OldText "    supportedLngs: [`"en`", `"zh-CN`"],`n" `
            -NewText "    supportedLngs: [`"en`", `"zh-CN`"],`n    ...override,`n"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "awaited i18n.init options" `
            -Scenario "desktop supported language list overridden by a later spread"
    }

    Invoke-Scenario -Name "Simplified Chinese catalog cannot be removed" -Test {
        $root = New-ContractFixture
        Remove-Item `
            -LiteralPath (Join-Path $root "apps/desktop/src/i18n/locales/zh-CN.json") `
            -Force
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Simplified Chinese catalog" `
            -Scenario "desktop without the Simplified Chinese catalog"
    }

    Invoke-Scenario -Name "visible management copy cannot bypass i18n through comment or key decoys" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/ManagementPanel.tsx" `
            -OldText '{t("management.providers.panelLabel")}' `
            -NewText @'
{"Management"}
            {/* t("management.providers.panelLabel") keeps the catalog token */}
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Management panel label must be rendered through its translation key." `
            -Scenario "management panel with visible English and inert translation decoys"
    }

    Invoke-Scenario -Name "dead management translation decoy cannot replace the live label key" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/ManagementPanel.tsx" `
            -OldText '{t("management.providers.panelLabel")}' `
            -NewText '{t("common.retry")}'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/ManagementPanel.tsx" `
            -OldText 'export function ManagementPanel({' `
            -NewText @'
function unusedManagementLabel() {
  return (
    <p className="section-label">
      {t("management.providers.panelLabel")}
    </p>
  );
}

export function ManagementPanel({
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Management panel label must be rendered through its translation key." `
            -Scenario "management panel using a wrong live key and an unused exact-key decoy"
    }

    Invoke-Scenario -Name "hidden management translation decoy cannot replace an expression-class live label" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/ManagementPanel.tsx" `
            -OldText @'
<p className="section-label">
            {t("management.providers.panelLabel")}
          </p>
'@ `
            -NewText @'
<div hidden>
            <p className="section-label">
              {t("management.providers.panelLabel")}
            </p>
          </div>
          <p className={"section-label"}>{t("common.retry")}</p>
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Management panel label must be rendered through its translation key." `
            -Scenario "management panel using a hidden exact-key decoy and an expression-class wrong live key"
    }

    Invoke-Scenario -Name "desktop PE check cannot survive only in dead code" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/release/package-windows-assets.ps1" `
            -OldText @"
if ((& `$peSubsystemCommand -Path `$desktop) -ne 2) {
    throw "Windows desktop executable must use the GUI subsystem."
}
"@ `
            -NewText @"
if (`$false) {
    if ((& `$peSubsystemCommand -Path `$desktop) -ne 2) {
        throw "Windows desktop executable must use the GUI subsystem."
    }
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "active script-scope source desktop GUI subsystem check" `
            -Scenario "desktop PE subsystem check retained only in dead code"
    }

    Invoke-Scenario -Name "desktop PE check cannot follow a top-level return" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/release/package-windows-assets.ps1" `
            -OldText @"
if ((& `$peSubsystemCommand -Path `$desktop) -ne 2) {
    throw "Windows desktop executable must use the GUI subsystem."
}
"@ `
            -NewText @"
return
if ((& `$peSubsystemCommand -Path `$desktop) -ne 2) {
    throw "Windows desktop executable must use the GUI subsystem."
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "reachable script-scope source desktop GUI subsystem check" `
            -Scenario "desktop PE subsystem check after a top-level return"
    }

    Invoke-Scenario -Name "frontend CI cannot omit the catalog check" -Test {
        $root = New-ContractFixture
        Edit-WorkflowJob `
            -Root $root `
            -JobName "frontend" `
            -OldText @"
      - name: Check desktop translation catalogs
        run: pnpm --dir apps/desktop i18n:check
"@ `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "pnpm --dir apps/desktop i18n:check" `
            -Scenario "frontend CI without the desktop catalog check"
    }

    $lifecycleMutations = @(
        @{
            Name = "production WokCore key id cannot change"
            Path = "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub"
            Old = "untrusted comment: minisign public key 7EF262CD8E9FE136"
            New = "untrusted comment: minisign public key 0000000000000000"
            Expected = "production Minisign public key"
        },
        @{
            Name = "Minisign private key header is rejected"
            Path = "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub"
            Old = "untrusted comment: minisign public key 7EF262CD8E9FE136"
            New = (
                "untrusted comment: minisign " +
                "secret key 7EF262CD8E9FE136"
            )
            Expected = "Minisign private or encrypted secret key header"
        },
        @{
            Name = "Minisign encrypted private key header is rejected"
            Path = "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub"
            Old = "untrusted comment: minisign public key 7EF262CD8E9FE136"
            New = (
                "untrusted comment: minisign encrypted " +
                "secret key 7EF262CD8E9FE136"
            )
            Expected = "Minisign private or encrypted secret key header"
        },
        @{
            Name = "structured WokRouter start arguments cannot change"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = 'Self::raw(program, ["start", "--json", "--progress-jsonl"])'
            New = 'Self::raw(program, ["start", "--json"])'
            Expected = "structured WokRouter start arguments"
        },
        @{
            Name = "structured WokCore update arguments cannot change"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '["update", "--install", "--json", "--progress-jsonl"],'
            New = '["update", "--install", "--json"],'
            Expected = "structured WokCore update-install arguments"
        },
        @{
            Name = "system runner install request must use the install command spec"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
                OperationRequest::Install => (
                    CoreOperationKind::Install,
                    CommandSpec::install(bundled_wokrouter_executable()?),
                ),
"@
            New = @"
                OperationRequest::Install => (
                    CoreOperationKind::Install,
                    CommandSpec::update_check(bundled_wokrouter_executable()?),
                ),
"@
            Expected = "System operation runner install wiring"
        },
        @{
            Name = "system runner update request must use the update-install command spec"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
                OperationRequest::Update { executable } => (
                    CoreOperationKind::Update,
                    CommandSpec::update_install(executable),
                ),
"@
            New = @"
                OperationRequest::Update { executable } => (
                    CoreOperationKind::Update,
                    CommandSpec::update_check(executable),
                ),
"@
            Expected = "System operation runner update wiring"
        },
        @{
            Name = "long child Windows no-window policy cannot change"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = "creation_flags: 0x0800_0000,"
            New = "creation_flags: 0,"
            Expected = "CREATE_NO_WINDOW policy"
        },
        @{
            Name = "long child Windows no-window policy cannot survive only in a comment"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = "creation_flags: 0x0800_0000,"
            New = @'
creation_flags: 0,
/*
ChildProcessPolicy {
    kill_on_drop: false,
    #[cfg(windows)]
    creation_flags: 0x0800_0000,
}
*/
'@
            Expected = "CREATE_NO_WINDOW policy"
        },
        @{
            Name = "long child must apply the Windows no-window policy"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = "    command.creation_flags(policy.creation_flags);"
            New = "    let _ = policy.creation_flags;"
            Expected = "apply CREATE_NO_WINDOW"
        },
        @{
            Name = "long child Windows no-window application cannot survive only in a comment"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = "    command.creation_flags(policy.creation_flags);"
            New = @'
    let _ = policy.creation_flags;
    /*
    #[cfg(windows)]
    command.creation_flags(policy.creation_flags);
    */
'@
            Expected = "apply CREATE_NO_WINDOW"
        },
        @{
            Name = "long child Windows no-window application must be a direct statement"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
    #[cfg(windows)]
    command.creation_flags(policy.creation_flags);
"@
            New = @"
    if false {
        #[cfg(windows)]
        command.creation_flags(policy.creation_flags);
    }
"@
            Expected = "directly apply CREATE_NO_WINDOW"
        },
        @{
            Name = "core operation progress event name cannot change"
            Path = "apps/desktop/src-tauri/src/lib.rs"
            Old = 'self.app.emit("core-operation-progress", snapshot)'
            New = 'self.app.emit("core-progress", snapshot)'
            Expected = "core-operation-progress event"
        },
        @{
            Name = "core operation progress event cannot survive only in a comment"
            Path = "apps/desktop/src-tauri/src/lib.rs"
            Old = 'self.app.emit("core-operation-progress", snapshot)'
            New = @'
self.app.emit("core-progress", snapshot) /*
self.app.emit("core-operation-progress", snapshot)
*/
'@
            Expected = "core-operation-progress event"
        },
        @{
            Name = "install command must wire the Tauri operation event sink"
            Path = "apps/desktop/src-tauri/src/lib.rs"
            Old = '.install_and_start(Arc::new(TauriOperationEventSink { app }))'
            New = '.install_and_start(Arc::new(NoopOperationEventSink))'
            Expected = "install command Tauri operation event sink wiring"
        },
        @{
            Name = "update command must wire the Tauri operation event sink"
            Path = "apps/desktop/src-tauri/src/lib.rs"
            Old = '.install_update(&expected_version, Arc::new(TauriOperationEventSink { app }))'
            New = '.install_update(&expected_version, Arc::new(NoopOperationEventSink))'
            Expected = "update command Tauri operation event sink wiring"
        },
        @{
            Name = "operation conflict stable code cannot change"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '#[error("operation_in_progress")]'
            New = '#[error("busy")]'
            Expected = "operation_in_progress"
        },
        @{
            Name = "operation conflict stable code cannot survive only in a comment"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '#[error("operation_in_progress")]'
            New = @'
#[error("busy")]
/*
#[error("operation_in_progress")]
OperationInProgress,
*/
'@
            Expected = "operation_in_progress"
        },
        @{
            Name = "update check conflict must return operation_in_progress"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        if let Some(check) = self.state.lock().await.update_check.clone() {
            return Ok(check);
        }
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self.trusted_production_executable().await?;
"@
            New = @"
        if let Some(check) = self.state.lock().await.update_check.clone() {
            return Ok(check);
        }
        if self.operation_is_active().await? {
            return Err(CoreOperationError::InvalidProgress);
        }
        let executable = self.trusted_production_executable().await?;
"@
            Expected = "check_update operation_in_progress return"
        },
        @{
            Name = "update install conflict must return operation_in_progress"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        Version::parse(expected_version).map_err(|_| CoreOperationError::InvalidProgress)?;
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self.trusted_production_executable().await?;
"@
            New = @"
        Version::parse(expected_version).map_err(|_| CoreOperationError::InvalidProgress)?;
        if self.operation_is_active().await? {
            return Err(CoreOperationError::InvalidProgress);
        }
        let executable = self.trusted_production_executable().await?;
"@
            Expected = "install_update operation_in_progress return"
        },
        @{
            Name = "persistent operation conflict must return operation_in_progress"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
                    self.ensure_persistent_monitor(existing.clone(), sink).await;
                    return Ok(existing);
                }
                return Err(CoreOperationError::OperationInProgress);
            }
            if existing.operation == CoreOperationKind::Install
"@
            New = @"
                    self.ensure_persistent_monitor(existing.clone(), sink).await;
                    return Ok(existing);
                }
                return Err(CoreOperationError::InvalidProgress);
            }
            if existing.operation == CoreOperationKind::Install
"@
            Expected = "Persistent operation arbitration"
        },
        @{
            Name = "production coordinator cannot disable persistent fail-closed recovery"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
            authority: Arc::new(SystemTrustedRuntimeAuthority::discover()),
            persistent_operations: true,
            journal,
"@
            New = @"
            authority: Arc::new(SystemTrustedRuntimeAuthority::discover()),
            persistent_operations: false,
            journal,
"@
            Expected = "fail closed"
        },
        @{
            Name = "journal safe snapshot must reject unknown fields"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '#[serde(deny_unknown_fields)]'
            New = '#[serde(default)]'
            Expected = "strict safe projection"
        },
        @{
            Name = "hidden helper cannot replace the canonical operation id argument"
            Path = "apps/desktop/src-tauri/src/core_operation/helper.rs"
            Old = '        .arg(operation_id.to_string())'
            New = '        .arg("caller-supplied-path")'
            Expected = "Hidden core-operation helper spawn"
        },
        @{
            Name = "update helper cannot fall back to PATH discovery"
            Path = "apps/desktop/src-tauri/src/core_operation/helper.rs"
            Old = 'discover_recorded_wokcore_executable(&paths.wokcore_install_record)'
            New = 'discover_wokcore_executable(&paths.wokcore_install_record)'
            Expected = "trusted WokCore install record"
        },
        @{
            Name = "hidden helper cannot append bulk path or token arguments"
            Path = "apps/desktop/src-tauri/src/core_operation/helper.rs"
            Old = @"
        })
        .stdin(Stdio::null())
"@
            New = @"
        })
        .args(["caller-path", "caller-token"])
        .stdin(Stdio::null())
"@
            Expected = "Hidden core-operation helper spawn"
        },
        @{
            Name = "hidden helper must dispatch before Tauri"
            Path = "apps/desktop/src-tauri/src/main.rs"
            Old = 'wokrouter_desktop::run_core_operation_helper_if_requested()'
            New = 'wokrouter_desktop::skip_core_operation_helper()'
            Expected = "hidden core-operation helper dispatch"
        },
        @{
            Name = "journal writer cannot use a shared record lock"
            Path = "apps/desktop/src-tauri/src/core_operation/journal.rs"
            Old = '        let _record_lock = self.lock_record_exclusive()?;'
            New = '        let _record_lock = self.lock_record_shared()?;'
            Expected = "exclusive record lock"
        },
        @{
            Name = "post-spawn readiness errors cannot abandon the helper child"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
            Ok(Err(error)) => {
                drop(claim);
                reap_helper(helper);
                return Err(error);
            }
"@
            New = @"
            Ok(Err(error)) => {
                drop(claim);
                drop(helper);
                return Err(error);
            }
"@
            Expected = "explicitly reaped"
        },
        @{
            Name = "helper readiness timeout cannot replace the operation lease fence"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
fn fence_helper_timeout(
    journal: &OperationJournal,
    expected: &CoreOperationSnapshot,
) -> Result<HelperTimeoutResolution, CoreOperationError> {
    match journal.try_operation_lease()? {
"@
            New = @"
fn fence_helper_timeout(
    journal: &OperationJournal,
    expected: &CoreOperationSnapshot,
) -> Result<HelperTimeoutResolution, CoreOperationError> {
    match journal.try_claim()? {
"@
            Expected = "atomically attach"
        },
        @{
            Name = "helper timeout fence cannot overwrite a newly written terminal"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '            if current != *expected {'
            New = '            if false {'
            Expected = "atomically attach"
        },
        @{
            Name = "initial helper timeout cannot skip its final lease fence"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
            Err(_) => {
                let resolution = match fence_helper_timeout(journal, &snapshot) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        drop(claim);
"@
            New = @"
            Err(_) => {
                let resolution = match skip_helper_timeout_fence(journal, &snapshot) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        drop(claim);
"@
            Expected = "readiness timeout must fence"
        },
        @{
            Name = "released initial handoff cannot skip the same-id restart"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = 'self.restart_initial_handoff(journal, &snapshot).await?'
            New = 'self.skip_initial_handoff(journal, &snapshot).await?'
            Expected = "restart an unready same-id handoff"
        },
        @{
            Name = "terminal recovery cannot use the claim lock as its fence"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = 'let recovery_lease = match journal.try_operation_lease()? {'
            New = 'let recovery_lease = match journal.try_claim()? {'
            Expected = "fence terminal recovery"
        },
        @{
            Name = "reopened helper timeout cannot skip its final lease fence"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        match observed {
            Ok(result) => {
                reap_helper(helper);
                result
            }
            Err(_) => {
                let resolution = match fence_helper_timeout(journal, snapshot) {
"@
            New = @"
        match observed {
            Ok(result) => {
                reap_helper(helper);
                result
            }
            Err(_) => {
                let resolution = match skip_helper_timeout_fence(journal, snapshot) {
"@
            Expected = "Initial helper handoff recovery"
        },
        @{
            Name = "active external install cannot disappear from operation arbitration"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
            return Ok(self.persistent_status().await?.is_some_and(|snapshot| {
                snapshot.state == CoreOperationState::Running
                    || (snapshot.operation == CoreOperationKind::Install
                        && snapshot.state == CoreOperationState::Failed
                        && snapshot.error_code.as_deref() == Some("install_in_progress"))
            }));
"@
            New = @"
            return Ok(self
                .persistent_status()
                .await?
                .is_some_and(|snapshot| snapshot.state == CoreOperationState::Running));
"@
            Expected = "active external install lease"
        },
        @{
            Name = "active external install cannot allow a persistent update start"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
            if existing.operation == CoreOperationKind::Install
                && existing.state == CoreOperationState::Failed
                && existing.error_code.as_deref() == Some("install_in_progress")
            {
                drop(claim);
                self.store_persistent_snapshot(existing.clone()).await;
                if operation == CoreOperationKind::Install {
                    return Ok(existing);
                }
                return Err(CoreOperationError::OperationInProgress);
            }
"@
            New = @"
            if existing.operation == CoreOperationKind::Install
                && existing.state == CoreOperationState::Failed
                && existing.error_code.as_deref() == Some("install_in_progress")
            {
                drop(claim);
                self.store_persistent_snapshot(existing.clone()).await;
                return Ok(existing);
            }
"@
            Expected = "active external install lease"
        },
        @{
            Name = "recovered update cannot accept a non-target running version"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = 'if snapshot.operation == CoreOperationKind::Update && !update_matches_target {'
            New = 'if snapshot.operation == CoreOperationKind::Update && false {'
            Expected = "exact update target"
        },
        @{
            Name = "released external install lease must become retryable"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '            if probe.install_lease_active()? {'
            New = '            if true {'
            Expected = "External install recovery"
        },
        @{
            Name = "missing recovered install must retain install_failed"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '                        CoreOperationKind::Install => "install_failed",'
            New = '                        CoreOperationKind::Install => "start_failed",'
            Expected = "stable install/update failure code"
        },
        @{
            Name = "external install frontend poll cannot omit operation recovery"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = '      void Promise.allSettled([getCoreOperation(), status.refetch()]).then('
            New = '      void Promise.allSettled([status.refetch()]).then('
            Expected = "operation and core refresh"
        },
        @{
            Name = "real helper recovery fixture cannot survive only in a comment"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = '    async fn real_helper_survives_coordinator_reopen_and_prevents_a_second_launch() {'
            New = @'
    // Former fixture: real_helper_survives_coordinator_reopen_and_prevents_a_second_launch
    async fn renamed_real_helper_behavior() {
'@
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "cross-platform process fixture cannot reintroduce PowerShell"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
    impl OperationRunner for ProcessFixtureRunner {
        fn run(
"@
            New = @"
    impl OperationRunner for ProcessFixtureRunner {
        const WINDOWS_ONLY_SHELL: &str = "powershell.exe";

        fn run(
"@
            Expected = "Windows-only shell"
        },
        @{
            Name = "transactional child cannot enable kill-on-drop"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = "        .kill_on_drop(policy.kill_on_drop);"
            New = "        .kill_on_drop(true);"
            Expected = "kill_on_drop(true)"
        },
        @{
            Name = "backend trusted executable must reject Development"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        if runtime.channel() == WokCoreRuntimeChannel::Development {
            return Err(CoreOperationError::DevelopmentRuntimeManagedByIde);
        }
        if let Some(executable) = runtime.executable() {
"@
            New = @"
        if let Some(executable) = runtime.executable() {
"@
            Expected = "Backend development update gate"
        },
        @{
            Name = "backend Development gate must precede executable reuse"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        if runtime.channel() == WokCoreRuntimeChannel::Development {
            return Err(CoreOperationError::DevelopmentRuntimeManagedByIde);
        }
        if let Some(executable) = runtime.executable() {
            return Ok(executable.to_path_buf());
        }
"@
            New = @"
        if let Some(executable) = runtime.executable() {
            return Ok(executable.to_path_buf());
        }
        if runtime.channel() == WokCoreRuntimeChannel::Development {
            return Err(CoreOperationError::DevelopmentRuntimeManagedByIde);
        }
"@
            Expected = "Backend development update gate must dominate"
        },
        @{
            Name = "backend check must use the production-gated trusted executable"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self.trusted_production_executable().await?;
        let completion = self
"@
            New = @"
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self
            .authority
            .discover()?
            .ok_or(CoreOperationError::UpdateUnavailable)?;
        let completion = self
"@
            Expected = "check_update must obtain a production-gated trusted executable"
        },
        @{
            Name = "backend install-update must reject Development first"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        self.require_production_channel().await?;
        Version::parse(expected_version).map_err(|_| CoreOperationError::InvalidProgress)?;
"@
            New = @"
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        Version::parse(expected_version).map_err(|_| CoreOperationError::InvalidProgress)?;
"@
            Expected = "install_update must reject Development before validation or child work"
        },
        @{
            Name = "frontend eligibility helper must require Production"
            Path = "apps/desktop/src/coreUpdateEligibility.ts"
            Old = '    status?.runtime_channel === "production" &&'
            New = "    status !== undefined &&"
            Expected = "Frontend update eligibility must require the production runtime channel"
        },
        @{
            Name = "automatic update check cannot bypass frontend eligibility"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
      operation !== undefined ||
      !isCoreUpdateEligible(status.data)
"@
            New = @"
      operation !== undefined ||
      status.data === undefined
"@
            Expected = "Automatic update check must use the shared eligibility gate"
        },
        @{
            Name = "manual update check cannot bypass frontend eligibility"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
        activeUpdateCheckRequestId.current !== undefined ||
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current)
"@
            New = @"
        activeUpdateCheckRequestId.current !== undefined ||
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current)
"@
            Expected = "Manual update check must use the shared eligibility gate"
        },
        @{
            Name = "manual update gate must precede the update-check side effect"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
    (openConfirmation: boolean, trigger?: HTMLButtonElement) => {
      if (
        activeUpdateCheckRequestId.current !== undefined ||
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current)
      ) {
        return;
      }
      nextUpdateCheckRequestId.current += 1;
      const requestId = nextUpdateCheckRequestId.current;
      activeUpdateCheckRequestId.current = requestId;
      startupCheckConsumed.current = true;
      setUpdateCheckPending(true);
      const revision = startupCheckRevision.current;
      void retryCoreUpdateCheck()
"@
            New = @"
    (openConfirmation: boolean, trigger?: HTMLButtonElement) => {
      const updateCheckRequest = retryCoreUpdateCheck();
      if (
        activeUpdateCheckRequestId.current !== undefined ||
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current)
      ) {
        return;
      }
      nextUpdateCheckRequestId.current += 1;
      const requestId = nextUpdateCheckRequestId.current;
      activeUpdateCheckRequestId.current = requestId;
      startupCheckConsumed.current = true;
      setUpdateCheckPending(true);
      const revision = startupCheckRevision.current;
      void updateCheckRequest
"@
            Expected = "Manual update gate must dominate retryCoreUpdateCheck"
        },
        @{
            Name = "automatic update check must wait for bridge readiness"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
  useEffect(() => {
    if (
      !bridgeReady ||
      startupCheckConsumed.current ||
"@
            New = @"
  useEffect(() => {
    if (
      startupCheckConsumed.current ||
"@
            Expected = "Automatic update check must require bridgeReady"
        },
        @{
            Name = "automatic install must wait for bridge readiness"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
  useEffect(() => {
    if (
      !bridgeReady ||
      installRequested.current ||
"@
            New = @"
  useEffect(() => {
    if (
      installRequested.current ||
"@
            Expected = "Automatic install must require bridgeReady"
        },
        @{
            Name = "update prompt cannot bypass frontend eligibility"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current) ||
        updateCheck?.code !== "update_available" ||
        updateCheck.targetVersion === undefined ||
"@
            New = @"
        blocksUpdateInteraction(latestOperation.current) ||
        updateCheck?.code !== "update_available" ||
        updateCheck.targetVersion === undefined ||
"@
            Expected = "Update prompt must use the shared eligibility gate"
        },
        @{
            Name = "update confirmation cannot bypass frontend eligibility"
            Path = "apps/desktop/src/components/CoreLifecycle.tsx"
            Old = @"
      updateRequested.current ||
      !latestBridgeReady.current ||
      blocksUpdateInteraction(latestOperation.current) ||
      !isCoreUpdateEligible(latestStatus.current) ||
      updateCheck?.code !== "update_available" ||
"@
            New = @"
      updateRequested.current ||
      !latestBridgeReady.current ||
      blocksUpdateInteraction(latestOperation.current) ||
      updateCheck?.code !== "update_available" ||
"@
            Expected = "Update confirmation must use the shared eligibility gate"
        },
        @{
            Name = "lifecycle acceptance docs must retain executable evidence"
            Path = "docs/operations/development.md"
            Old = "pwsh tests/scripts/check-foundation-contract.tests.ps1"
            New = "pwsh tests/scripts/not-the-foundation-contract.tests.ps1"
            Expected = "lifecycle acceptance evidence"
        },
        @{
            Name = "lifecycle acceptance docs cannot cite a missing fixture"
            Path = "apps/desktop/src/components/CoreLifecycle.test.tsx"
            Old = 'it("starts one production install in StrictMode and restores normal content after success", async () => {'
            New = 'it("renamed lifecycle behavior", async () => {'
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "lifecycle acceptance fixture cannot survive only in a comment"
            Path = "apps/desktop/src/components/CoreLifecycle.test.tsx"
            Old = 'it("starts one production install in StrictMode and restores normal content after success", async () => {'
            New = @'
// Former fixture: starts one production install in StrictMode and restores normal content after success
it("renamed lifecycle behavior", async () => {
'@
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "lifecycle acceptance fixture cannot live in a disabled test module"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
#[cfg(test)]
mod tests {
"@
            New = @"
#[cfg(test)]
#[cfg(any())]
mod tests {
"@
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "lifecycle acceptance fixture cannot live behind a module inner cfg"
            Path = "apps/desktop/src-tauri/src/core_operation.rs"
            Old = @"
#[cfg(test)]
mod tests {
    use std::{
"@
            New = @"
#[cfg(test)]
mod tests {
    #![cfg(any())]

    use std::{
"@
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "lifecycle acceptance fixture cannot live behind an integration inner cfg"
            Path = "crates/wokrouter-platform/tests/wokcore_install.rs"
            Old = @"
#![cfg(feature = "test-support")]

use std::{fs, path::Path, sync::mpsc};
"@
            New = @"
#![cfg(feature = "test-support")]
#![cfg(any())]

use std::{fs, path::Path, sync::mpsc};
"@
            Expected = "lifecycle acceptance fixture"
        },
        @{
            Name = "lifecycle acceptance fixture requires the exact test-support feature"
            Path = "crates/wokrouter-platform/tests/wokcore_install.rs"
            Old = '#![cfg(feature = "test-support")]'
            New = '#![cfg(feature = "test - support")]'
            Expected = "lifecycle acceptance fixture"
        }
    )
    foreach ($mutation in $lifecycleMutations) {
        Invoke-Scenario -Name $mutation.Name -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath $mutation.Path `
                -OldText $mutation.Old `
                -NewText $mutation.New
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText $mutation.Expected `
                -Scenario $mutation.Name
        }
    }

    Invoke-Scenario -Name "installCoreUpdate must remain owned by confirmation" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/CoreLifecycle.tsx" `
            -OldText @"
  const requestUpdate = useCallback(
    (trigger?: HTMLButtonElement) => {
      if (
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current) ||
        updateCheck?.code !== "update_available" ||
        updateCheck.targetVersion === undefined ||
        updateRequested.current
      ) {
        return;
      }
      if (trigger) {
        updateTrigger.current = trigger;
      }
      setUpdateConfirmationOpen(true);
    },
    [updateCheck],
  );
"@ `
            -NewText @"
  const requestUpdate = useCallback(
    (trigger?: HTMLButtonElement) => {
      if (
        !latestBridgeReady.current ||
        blocksUpdateInteraction(latestOperation.current) ||
        !isCoreUpdateEligible(latestStatus.current) ||
        updateCheck?.code !== "update_available" ||
        updateCheck.targetVersion === undefined ||
        updateRequested.current
      ) {
        return;
      }
      if (trigger) {
        updateTrigger.current = trigger;
      }
      void installCoreUpdate(updateCheck.targetVersion);
      setUpdateConfirmationOpen(true);
    },
    [updateCheck],
  );
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/CoreLifecycle.tsx" `
            -OldText "    void installCoreUpdate(targetVersion)" `
            -NewText "    void Promise.resolve(targetVersion)"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "confirmation-only installCoreUpdate ownership" `
            -Scenario "installCoreUpdate outside confirmation"
    }

    Invoke-Scenario -Name "installCoreUpdate must follow the complete confirmation guard" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/CoreLifecycle.tsx" `
            -OldText @"
      updateCheck?.code !== "update_available" ||
      targetVersion === undefined
"@ `
            -NewText @"
      updateCheck?.code !== "update_available" ||
      (void installCoreUpdate(targetVersion ?? ""), targetVersion === undefined)
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src/components/CoreLifecycle.tsx" `
            -OldText "    void installCoreUpdate(targetVersion)" `
            -NewText "    void Promise.resolve(targetVersion)"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "complete confirmation guard" `
            -Scenario "installCoreUpdate before confirmation guard completion"
    }

    foreach ($secretFile in @(
            @{
                Name = "Minisign private key header is rejected in a .key product file"
                Path = "apps/desktop/src/product-signing.key"
            },
            @{
                Name = "Minisign private key header is rejected in an extensionless product file"
                Path = "crates/wokrouter-platform/PRODUCT_SIGNING"
            }
        )) {
        Invoke-Scenario -Name $secretFile.Name -Test {
            $root = New-ContractFixture
            Add-FixtureTextFile `
                -Root $root `
                -RelativePath $secretFile.Path `
                -Content (
                    "untrusted comment: minisign " +
                    "secret key 0000000000000000"
                )
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "Minisign private or encrypted secret key header" `
                -Scenario $secretFile.Name
        }
    }

    Invoke-Scenario -Name "generated directories are excluded from product private-key scanning" -Test {
        $root = New-ContractFixture
        Add-FixtureTextFile `
            -Root $root `
            -RelativePath "apps/desktop/target/generated-signing.key" `
            -Content (
                "untrusted comment: minisign " +
                "secret key 0000000000000000"
            )
        Assert-ContractPasses `
            -Root $root `
            -Scenario "generated private-key fixture exclusion"
    }

    Invoke-Scenario -Name "development debug gate permits non-semantic trivia" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(debug_assertions)]
mod development {
"@ `
            -NewText @"
#[cfg(debug_assertions)]

/// Development-only candidate parsing.
#[allow(clippy::missing_errors_doc)]
mod development {
"@
        Assert-ContractPasses `
            -Root $root `
            -Scenario "development debug gate with trivia"
    }

    Invoke-Scenario -Name "Rust literals do not change braced item boundaries" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    use tokio::time::Instant;

    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(5);
"@ `
            -NewText @"
    use tokio::time::Instant;

    let _normal = "{ normal string brace }";
    let _byte = b"{ byte string brace }";
    let _raw = r###"raw " quote { brace }"###;
    let _raw_byte = br##"raw byte " quote { brace }"##;
    let _character = '}';
    let _byte_character = b'x';

    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(5);
"@
        Assert-ContractPasses `
            -Root $root `
            -Scenario "Rust literal brace trivia"
    }

    Invoke-Scenario -Name "Rust parameter delimiters do not become an item opener" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    paths: &AppPaths,
    candidate: Option<PathBuf>,
"@ `
            -NewText @"
    paths: &AppPaths,
    _marker: Option<[u8; { 1 }]>,
    candidate: Option<PathBuf>,
"@
        Assert-ContractPasses `
            -Root $root `
            -Scenario "Rust parameter and array delimiter trivia"
    }

    foreach ($braceMutation in @(
        @{
            Name = "negative brace depth"
            Text = "}"
        },
        @{
            Name = "unbalanced brace depth"
            Text = "{"
        }
    )) {
        Invoke-Scenario -Name "Rust $($braceMutation.Name) fails closed" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
                -OldText @"
#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@ `
                -NewText @"
$($braceMutation.Text)
#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "balanced braces" `
                -Scenario "Rust $($braceMutation.Name)"
        }
    }

    foreach ($unterminated in @(
        @{
            Name = "normal string"
            Text = '"unterminated'
        },
        @{
            Name = "raw string"
            Text = 'r###"unterminated'
        },
        @{
            Name = "nested block comment"
            Text = "/* outer /* inner */"
        },
        @{
            Name = "character"
            Text = "'{"
        }
    )) {
        Invoke-Scenario -Name "unterminated Rust $($unterminated.Name) fails closed" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
                -OldText @"
#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@ `
                -NewText @"
$($unterminated.Text)
#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "lexically valid" `
                -Scenario "unterminated Rust $($unterminated.Name)"
        }
    }

    Invoke-Scenario -Name "development environment parsing must remain debug-only" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(debug_assertions)]
mod development {
"@ `
            -NewText @"
mod development {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "debug_assertions" `
            -Scenario "missing development debug gate"
    }

    Invoke-Scenario -Name "development parsing cannot survive only as inert text" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";' `
            -NewText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DISABLED_WOKCORE_EXECUTABLE";'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "        candidate_from_value(std::env::var_os(EXECUTABLE_ENV))" `
            -NewText '        candidate_from_value(std::env::var_os("WOKROUTER_DISABLED_WOKCORE_EXECUTABLE"))'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText "    let candidate = None;"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();" `
            -NewText @"
// "WOKROUTER_DEV_WOKCORE_EXECUTABLE"
#[allow(dead_code)]
fn inert_development_parser() {
    let _ = std::env::var_os(development::EXECUTABLE_ENV);
    let _ = development::candidate_from_environment();
}

static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "development module" `
            -Scenario "development parsing retained only in comments and a dead helper"
    }

    Invoke-Scenario -Name "development module opener cannot bind past a semicolon" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";' `
            -NewText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DISABLED_WOKCORE_EXECUTABLE";'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "mod development {" `
            -NewText @"
mod development;
#[allow(dead_code)]
fn inert_development_body() {
    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";
    pub(super) fn candidate_from_environment() -> Option<PathBuf> {
        candidate_from_value(std::env::var_os(EXECUTABLE_ENV))
    }
}
{
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "braced body" `
            -Scenario "semicolon development module bound to a later brace"
    }

    Invoke-Scenario -Name "character braces cannot extend the development module body" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";' `
            -NewText '    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DISABLED_WOKCORE_EXECUTABLE";'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
mod development {
    use std::{
"@ `
            -NewText @"
mod development {
    const INERT_OPEN_BRACE: char = '{';
    use std::{
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@ `
            -NewText @"
pub(super) const INERT_EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";
const INERT_CLOSE_BRACE: char = '}';

#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "environment constant" `
            -Scenario "character braces extending the development module body"
    }

    Invoke-Scenario -Name "development environment lookup must stay in its active function" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "        candidate_from_value(std::env::var_os(EXECUTABLE_ENV))" `
            -NewText '        candidate_from_value(std::env::var_os("WOKROUTER_DISABLED_WOKCORE_EXECUTABLE"))'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    pub(super) fn candidate_from_value(value: Option<OsString>) -> Option<PathBuf> {" `
            -NewText @"
    #[allow(dead_code)]
    fn inert_environment_lookup() {
        let _ = std::env::var_os(EXECUTABLE_ENV);
    }

    pub(super) fn candidate_from_value(value: Option<OsString>) -> Option<PathBuf> {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "environment lookup" `
            -Scenario "environment lookup retained only in a dead helper"
    }

    Invoke-Scenario -Name "debug selector must actively read the development candidate" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText "    let candidate = None;"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();" `
            -NewText @"
#[allow(dead_code)]
fn inert_development_candidate() {
    let _ = development::candidate_from_environment();
}

static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Debug select_once development candidate call" `
            -Scenario "development candidate call retained only in a dead helper"
    }

    Invoke-Scenario -Name "debug selector candidate call cannot survive in a normal string" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText @"
    let candidate = None;
    let _inert = "development::candidate_from_environment()";
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "development candidate call" `
            -Scenario "development candidate call retained in a normal string"
    }

    Invoke-Scenario -Name "release selector cannot read the development environment literal" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@ `
            -NewText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let _ = std::env::var_os("WOKROUTER_DEV_WOKCORE_EXECUTABLE");
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "release select_once" `
            -Scenario "release selector directly reading the development environment literal"
    }

    Invoke-Scenario -Name "release selector cannot read an indirect development environment constant" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@ `
            -NewText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    const RELEASE_EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";
    let _ = std::env::var_os(RELEASE_EXECUTABLE_ENV);
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "environment literal" `
            -Scenario "release selector indirectly reading a duplicate development environment constant"
    }

    Invoke-Scenario -Name "release selector cannot access the development module" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@ `
            -NewText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let _ = std::env::var_os(development::EXECUTABLE_ENV);
    let _ = development::candidate_from_environment();
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "release select_once" `
            -Scenario "release selector accessing the development module"
    }

    Invoke-Scenario -Name "release selector cannot hide behind a disabled exact-cfg decoy" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@ `
            -NewText @"
#[cfg(not(debug_assertions))]
#[cfg(any())]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}

#[cfg(all(not(debug_assertions), not(any())))]
async fn select_once(_paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    panic!("active release selector is wrong")
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "select_once" `
            -Scenario "disabled exact release decoy with an alternate active selector"
    }

    Invoke-Scenario -Name "debug selector cannot hide behind a disabled exact-cfg decoy" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(debug_assertions)]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let candidate = development::candidate_from_environment();
    select_with_dependencies(
        paths,
        candidate,
        Arc::new(crate::system::process_executable_matches),
        &probe_connection,
        Arc::new(discover_wokcore_executable),
    )
    .await
}
"@ `
            -NewText @"
#[cfg(debug_assertions)]
#[cfg(any())]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let candidate = development::candidate_from_environment();
    select_with_dependencies(
        paths,
        candidate,
        Arc::new(crate::system::process_executable_matches),
        &probe_connection,
        Arc::new(discover_wokcore_executable),
    )
    .await
}

#[cfg(all(debug_assertions, not(any())))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "select_once" `
            -Scenario "disabled exact debug decoy with an alternate active selector"
    }

    Invoke-Scenario -Name "debug selector cannot carry an additional cfg" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(debug_assertions)]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
"@ `
            -NewText @"
#[cfg(debug_assertions)]
#[cfg(any())]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "selector attributes" `
            -Scenario "debug selector disabled by an additional cfg"
    }

    Invoke-Scenario -Name "release selector cannot carry cfg_attr" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
"@ `
            -NewText @"
#[cfg(not(debug_assertions))]
#[cfg_attr(not(any()), cfg(any()))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "selector attributes" `
            -Scenario "release selector disabled through cfg_attr"
    }

    Invoke-Scenario -Name "development environment lookup cannot hide in a nested closure" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    pub(super) fn candidate_from_environment() -> Option<PathBuf> {
        candidate_from_value(std::env::var_os(EXECUTABLE_ENV))
    }
"@ `
            -NewText @"
    pub(super) fn candidate_from_environment() -> Option<PathBuf> {
        let _inert = || candidate_from_value(std::env::var_os(EXECUTABLE_ENV));
        None
    }
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "environment lookup" `
            -Scenario "development environment lookup retained only in a nested closure"
    }

    Invoke-Scenario -Name "debug selector candidate call cannot hide in a nested closure" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText @"
    let _inert = || development::candidate_from_environment();
    let candidate = None;
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "development candidate call" `
            -Scenario "debug selector candidate call retained only in a nested closure"
    }

    Invoke-Scenario -Name "debug selector candidate cannot survive only in macro tokens" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText @"
    let candidate = None;
    let _inert = stringify!(
        let candidate = development::candidate_from_environment();
    );
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "development candidate call" `
            -Scenario "debug selector candidate retained only in stringify macro tokens"
    }

    Invoke-Scenario -Name "debug selector must pass the environment candidate unchanged" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let candidate = development::candidate_from_environment();" `
            -NewText @"
    let candidate = development::candidate_from_environment();
    let candidate = candidate.and(None);
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "candidate flow" `
            -Scenario "debug selector discarding the environment candidate before selection"
    }

    Invoke-Scenario -Name "development executable environment name cannot change" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText '"WOKROUTER_DEV_WOKCORE_EXECUTABLE"' `
            -NewText '"WOKROUTER_WOKCORE_EXECUTABLE"'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "environment constant" `
            -Scenario "wrong development executable environment name"
    }

    Invoke-Scenario -Name "development selection deadline must remain five seconds" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "Duration::from_secs(5)" `
            -NewText "Duration::from_secs(10)"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "five-second" `
            -Scenario "wrong development selection deadline"
    }

    Invoke-Scenario -Name "development retry interval must remain 50 milliseconds" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "Duration::from_millis(50)" `
            -NewText "Duration::from_millis(100)"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "50-ms" `
            -Scenario "wrong development retry interval"
    }

    Invoke-Scenario -Name "IDE-managed lifecycle error code cannot change" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/src/control.rs" `
            -OldText '#[error("development_runtime_managed_by_ide")]' `
            -NewText '#[error("development_runtime_unavailable")]'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "IDE-managed variant" `
            -Scenario "missing IDE-managed lifecycle contract"
    }

    Invoke-Scenario -Name "runtime status must retain the runtime channel" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/cli/src/commands/mod.rs" `
            -OldText "    pub runtime_channel: WokCoreRuntimeChannel," `
            -NewText "    pub channel: WokCoreRuntimeChannel,"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "runtime_channel" `
            -Scenario "missing runtime channel"
    }

    Invoke-Scenario -Name "development selector must compare process identity" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && process_matches(identity.process_id(), &candidate)
"@ `
            -NewText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && true
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "process identity" `
            -Scenario "missing process identity comparison"
    }

    Invoke-Scenario -Name "development selector must recheck process identity after connecting" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                && process_matches(identity.process_id(), &candidate);" `
            -NewText "                && true;"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "process identity" `
            -Scenario "missing process identity recheck"
    }

    Invoke-Scenario -Name "development client must remain bound to the discovered runtime identity" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
            let candidate_for_validator = candidate.clone();
            let matcher_for_validator = Arc::clone(&process_matches);
            let bound = client.bound_to_runtime(
                identity,
                Arc::new(move |process_id| {
                    matcher_for_validator(process_id, &candidate_for_validator)
                }),
            );
"@ `
            -NewText "            let bound = client.clone();"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "runtime-bound" `
            -Scenario "unbound development client"
    }

    Invoke-Scenario -Name "development selector tokens cannot survive only in comments" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "Duration::from_secs(5)" `
            -NewText "Duration::from_secs(10)"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "Duration::from_millis(50)" `
            -NewText "Duration::from_millis(100)"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && process_matches(identity.process_id(), &candidate)
"@ `
            -NewText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && true
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
            let candidate_for_validator = candidate.clone();
            let matcher_for_validator = Arc::clone(&process_matches);
            let bound = client.bound_to_runtime(
                identity,
                Arc::new(move |process_id| {
                    matcher_for_validator(process_id, &candidate_for_validator)
                }),
            );
"@ `
            -NewText "            let bound = client.clone();"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await" `
            -NewText "                Ok(connection_probe(bound.clone()).await)"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                && process_matches(identity.process_id(), &candidate);" `
            -NewText "                && true;"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();" `
            -NewText @"
// Duration::from_secs(5)
// Duration::from_millis(50)
// false && process_matches(identity.process_id(), &candidate)
// let bound = client.bound_to_runtime(identity, validator);
// tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await
// client.discovered_runtime_identity() == Some(identity)
static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "select_with_dependencies" `
            -Scenario "selector requirements retained only in comments"
    }

    Invoke-Scenario -Name "selector constants reject unused raw-string and local decoys" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(5);" `
            -NewText @"
    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(10);
    let _raw_decoy = r#"Duration::from_secs(5)"#;
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    const DEVELOPMENT_RETRY_DELAY: Duration = Duration::from_millis(50);" `
            -NewText @"
    const DEVELOPMENT_RETRY_DELAY: Duration = Duration::from_millis(100);
    let _unused_decoy = Duration::from_millis(50);
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "named constants" `
            -Scenario "selector values retained only in inert decoys"
    }

    Invoke-Scenario -Name "selector must use its timeout and retry constants" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let deadline = Instant::now() + DEVELOPMENT_TIMEOUT;" `
            -NewText "    let deadline = Instant::now() + Duration::from_secs(10);"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "        tokio::time::sleep(DEVELOPMENT_RETRY_DELAY.min(deadline - now)).await;" `
            -NewText "        tokio::time::sleep(Duration::from_millis(100).min(deadline - now)).await;"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "constant references" `
            -Scenario "selector declarations are unused by deadline and sleep"
    }

    Invoke-Scenario -Name "selector sequence cannot survive in an unused closure" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && process_matches(identity.process_id(), &candidate)
"@ `
            -NewText @"
        if let Some(identity) = client.discovered_runtime_identity()
            && true
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
            let candidate_for_validator = candidate.clone();
            let matcher_for_validator = Arc::clone(&process_matches);
            let bound = client.bound_to_runtime(
                identity,
                Arc::new(move |process_id| {
                    matcher_for_validator(process_id, &candidate_for_validator)
                }),
            );
"@ `
            -NewText "            let bound = client.clone();"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await" `
            -NewText "                Ok(connection_probe(bound.clone()).await)"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                && process_matches(identity.process_id(), &candidate);" `
            -NewText "                && true;"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    loop {" `
            -NewText @"
    let _inert = || {
        let _ = false && process_matches(identity.process_id(), &candidate);
        let bound = client.bound_to_runtime(identity, validator);
        let Ok(connection) =
            tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await;
        let still_matches = client.discovered_runtime_identity() == Some(identity);
    };
    loop {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "selection loop" `
            -Scenario "selector sequence retained only in an unused closure"
    }

    Invoke-Scenario -Name "complete selector loop cannot survive in an unused closure" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    let Some(candidate) = candidate else {" `
            -NewText @"
    let deadline = Instant::now() + DEVELOPMENT_TIMEOUT;
    let _inert = || async {
    let Some(candidate) = candidate else {
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    select_production(paths, process_matches, discover)
}
"@ `
            -NewText @"
    select_production(paths, process_matches, discover)
    };
    select_production(paths, process_matches, discover)
}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "selection loop" `
            -Scenario "complete selector loop retained only in an unused closure"
    }

    Invoke-Scenario -Name "identity branch cannot survive in an unused loop closure" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    loop {
        if let Some(identity) = client.discovered_runtime_identity()
"@ `
            -NewText @"
    loop {
        let _inert = || async {
        if let Some(identity) = client.discovered_runtime_identity()
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
        }

        let now = Instant::now();
"@ `
            -NewText @"
        }
        };

        let now = Instant::now();
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "initial process identity check" `
            -Scenario "identity branch retained only in an unused loop closure"
    }

    Invoke-Scenario -Name "selector loop cannot survive as an async closure expression" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "    loop {" `
            -NewText @"
    let _inert = async ||
        loop {
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    }
    select_production(paths, process_matches, discover)
"@ `
            -NewText @"
        };
    select_production(paths, process_matches, discover)
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "selection loop" `
            -Scenario "complete selector loop retained as an uncalled async closure expression"
    }

    Invoke-Scenario -Name "identity branch cannot survive as an async closure expression" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
    loop {
        if let Some(identity) = client.discovered_runtime_identity()
"@ `
            -NewText @"
    loop {
        let _inert = async ||
            if let Some(identity) = client.discovered_runtime_identity()
"@
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
        }

        let now = Instant::now();
"@ `
            -NewText @"
            };

        let now = Instant::now();
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "initial process identity check" `
            -Scenario "identity branch retained as an uncalled async closure expression"
    }

    Invoke-Scenario -Name "development connection probe must remain deadline-bound" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "                tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await" `
            -NewText "                Ok(connection_probe(bound.clone()).await)"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "deadline-bound connection probe" `
            -Scenario "connection probe without selector deadline"
    }

    Invoke-Scenario -Name "development selector operations must retain runtime binding" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText @"
            let candidate_for_validator = candidate.clone();
            let matcher_for_validator = Arc::clone(&process_matches);
            let bound = client.bound_to_runtime(
                identity,
                Arc::new(move |process_id| {
                    matcher_for_validator(process_id, &candidate_for_validator)
                }),
            );
"@ `
            -NewText "            let bound = client.clone();"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "runtime-bound" `
            -Scenario "runtime binding removed from the identity branch"
    }

    Invoke-Scenario -Name "runtime identity authorization cannot be forged by comments or strings" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-wokcore-client/src/lib.rs" `
            -OldText "                    && identity.instance_id == record.instance_id" `
            -NewText "                    && true"
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-wokcore-client/src/lib.rs" `
            -OldText "pub type WokCoreRuntimeValidator = dyn Fn(NonZeroU32) -> bool + Send + Sync;" `
            -NewText @"
pub type WokCoreRuntimeValidator = dyn Fn(NonZeroU32) -> bool + Send + Sync;
// identity.process_id == record.process_id && identity.instance_id == record.instance_id && validator(record.process_id)
const INERT_RUNTIME_IDENTITY_DECOY: &str = r#"identity.process_id == record.process_id && identity.instance_id == record.instance_id && validator(record.process_id)"#;
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "jointly recheck PID, instance ID, and executable identity" `
            -Scenario "instance authorization retained only in comments and strings"
    }

    Invoke-Scenario -Name "production selection cannot bypass pending trusted authorization" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "        client: bound_client," `
            -NewText @"
        // client: bound_client,
        let _inert = "client: bound_client,";
        client: probe_client.clone(),
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "keep Missing pending" `
            -Scenario "production client replaced by an unrestricted probe clone"
    }

    Invoke-Scenario -Name "production start cannot probe before trusted binding" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/cli/src/commands/start.rs" `
            -OldText "        let connection = if runtime.establish_production_binding(executable) {" `
            -NewText @"
        // runtime.establish_production_binding(executable)
        let _inert = "runtime.establish_production_binding(executable)";
        let connection = if true {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "establish trusted runtime binding" `
            -Scenario "start readiness request bypassed atomic binding"
    }

    Invoke-Scenario -Name "IDE-managed error code cannot survive in a dead constant" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/src/control.rs" `
            -OldText '#[error("development_runtime_managed_by_ide")]' `
            -NewText '#[error("development_runtime_unavailable")]'
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "apps/desktop/src-tauri/src/control.rs" `
            -OldText "pub(crate) enum DesktopControlError {" `
            -NewText @"
const INERT_DEVELOPMENT_ERROR: &str = "development_runtime_managed_by_ide";

pub(crate) enum DesktopControlError {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "DesktopControlError" `
            -Scenario "IDE-managed error retained only in a dead constant"
    }

    Invoke-Scenario -Name "development no-switch regression test cannot be removed" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
            -OldText "async fn a_selected_development_session_never_switches_to_production()" `
            -NewText "async fn selected_development_runtime_can_switch_to_production()"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "no-switch" `
            -Scenario "missing development no-switch regression"
    }

    Invoke-Scenario -Name "development no-switch regression test cannot be ignored" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
            -OldText @"
#[tokio::test]
async fn a_selected_development_session_never_switches_to_production() {
"@ `
            -NewText @"
#[tokio::test]
#[ignore]
async fn a_selected_development_session_never_switches_to_production() {
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "must not be ignored" `
            -Scenario "ignored development no-switch regression"
    }

    foreach ($attributeMutation in @(
        @{
            Name = "cfg exclusion"
            Attribute = "#[cfg(any())]"
        },
        @{
            Name = "conditional ignore"
            Attribute = "#[cfg_attr(debug_assertions, ignore)]"
        },
        @{
            Name = "reasoned ignore"
            Attribute = '#[ignore = "reason"]'
        },
        @{
            Name = "should panic"
            Attribute = "#[should_panic]"
        }
    )) {
        Invoke-Scenario -Name "development no-switch test rejects $($attributeMutation.Name)" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
                -OldText @"
#[tokio::test]
async fn a_selected_development_session_never_switches_to_production() {
"@ `
                -NewText @"
#[tokio::test]
$($attributeMutation.Attribute)
async fn a_selected_development_session_never_switches_to_production() {
"@
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "execution-changing attributes" `
                -Scenario "no-switch test with $($attributeMutation.Name)"
        }
    }

    foreach ($attributeMutation in @(
        @{
            Name = "spaced cfg exclusion"
            Attribute = "# [ cfg(any()) ]"
        },
        @{
            Name = "multiline cfg exclusion"
            Attribute = @"
#[cfg(
    any()
)]
"@
        },
        @{
            Name = "multiline conditional ignore"
            Attribute = @"
#[cfg_attr(
    debug_assertions,
    ignore
)]
"@
        }
    )) {
        Invoke-Scenario -Name "development no-switch test rejects $($attributeMutation.Name)" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
                -OldText @"
#[tokio::test]
async fn a_selected_development_session_never_switches_to_production() {
"@ `
                -NewText @"
$($attributeMutation.Attribute)
#[tokio::test]
async fn a_selected_development_session_never_switches_to_production() {
"@
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "execution-changing attributes" `
                -Scenario "no-switch test with $($attributeMutation.Name)"
        }
    }

    Invoke-Scenario -Name "no-switch assertion cannot survive in a byte string" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
            -OldText @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@ `
            -NewText @'
    assert_eq!(selected.executable(), Some(development.as_path()));
    let _inert = b"assert_eq!(selected.connection().await, CoreConnection::Stopped);";
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "stopped retained connection" `
            -Scenario "stopped assertion retained only in a byte string"
    }

    Invoke-Scenario -Name "no-switch assertion cannot survive in a raw byte string" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
            -OldText @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@ `
            -NewText @'
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    let _inert = br##"assert!(replacement.received_requests().await.unwrap().is_empty());"##;
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
'@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "replacement zero requests" `
            -Scenario "replacement assertion retained only in a raw byte string"
    }

    $noSwitchAssertions = @(
        @{
            Name = "development channel"
            OldText = @"
    fixture.write_discovery_at(42, 1, &replacement.uri());

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
"@
            NewText = @"
    fixture.write_discovery_at(42, 1, &replacement.uri());
"@
        },
        @{
            Name = "selected executable"
            OldText = @"
    fixture.write_discovery_at(42, 1, &replacement.uri());

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
"@
            NewText = @"
    fixture.write_discovery_at(42, 1, &replacement.uri());

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
"@
        },
        @{
            Name = "stopped retained connection"
            OldText = @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@
            NewText = @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@
        },
        @{
            Name = "replacement zero requests"
            OldText = @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@
            NewText = @"
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@
        },
        @{
            Name = "production discovery zero calls"
            OldText = @"
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
"@
            NewText = @"
    assert!(replacement.received_requests().await.unwrap().is_empty());
"@
        }
    )
    foreach ($assertion in $noSwitchAssertions) {
        Invoke-Scenario -Name "development no-switch test retains $($assertion.Name) assertion" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
                -OldText $assertion.OldText `
                -NewText $assertion.NewText
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText $assertion.Name `
                -Scenario "no-switch test missing $($assertion.Name)"
        }
    }

    Invoke-Scenario -Name "development no-switch regression test cannot have an empty body" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/tests/wokcore_runtime.rs" `
            -OldText @"
async fn a_selected_development_session_never_switches_to_production() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    fixture.write_discovery(41, 2);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.clone().into_os_string()),
        true,
        None,
        Arc::clone(&discoveries),
    );
    let selected = selector.select(&fixture.paths).await.unwrap();

    let replacement = MockServer::start().await;
    mount_running_runtime(&replacement).await;
    fixture.write_discovery_at(42, 1, &replacement.uri());

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}
"@ `
            -NewText @"
async fn a_selected_development_session_never_switches_to_production() {}
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "development channel" `
            -Scenario "empty development no-switch regression"
    }

    foreach ($privateField in @("pid", "path", "executable")) {
        Invoke-Scenario -Name "Rust runtime status cannot expose $privateField" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "apps/cli/src/commands/mod.rs" `
                -OldText "pub struct CoreStatus {`n    pub state: CoreUiState," `
                -NewText "pub struct CoreStatus {`n    pub ${privateField}: Option<String>,`n    pub state: CoreUiState,"
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "private runtime field" `
                -Scenario "Rust status exposing $privateField"
        }

        Invoke-Scenario -Name "frontend runtime status cannot expose $privateField" -Test {
            $root = New-ContractFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath "apps/desktop/src/control.ts" `
                -OldText "  .object({`n    state: z.enum([" `
                -NewText "  .object({`n    ${privateField}: z.string().optional(),`n    state: z.enum(["
            Assert-ContractRejects `
                -Root $root `
                -ExpectedText "private runtime field" `
                -Scenario "frontend status exposing $privateField"
        }
    }

    Invoke-Scenario -Name "macOS arm64 must use the macos-14 runner" -Test {
        $root = New-ContractFixture
        Edit-WorkflowJob `
            -Root $root `
            -JobName "native-test-matrix" `
            -OldText @"
          - os: macos-14
            target: aarch64-apple-darwin
"@ `
            -NewText @"
          - os: macos-15
            target: aarch64-apple-darwin
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "macos-14" `
            -Scenario "wrong macOS arm64 runner"
    }

    Invoke-Scenario -Name "fixed host self-test cannot run on both Windows targets" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText @"
      - name: Self-test fixed Windows test host
        if: runner.os == 'Windows' && matrix.target == 'x86_64-pc-windows-msvc'
"@ `
            -NewText @"
      - name: Self-test fixed Windows test host
        if: runner.os == 'Windows'
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "self-test" `
            -Scenario "over-wide fixed host self-test condition"
    }

    Invoke-Scenario -Name "fixed host cannot run on both Windows targets" -Test {
        $root = New-ContractFixture
        Set-FixedHostCondition `
            -Root $root `
            -Condition "runner.os == 'Windows'"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Windows x64 target" `
            -Scenario "over-wide fixed host condition"
    }

    Invoke-Scenario -Name "fixed host cannot run on Windows arm64" -Test {
        $root = New-ContractFixture
        Set-FixedHostCondition `
            -Root $root `
            -Condition "runner.os == 'Windows' && matrix.target == 'aarch64-pc-windows-msvc'"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Windows x64 target" `
            -Scenario "Windows arm64 fixed host condition"
    }

    Invoke-Scenario -Name "fixed host cannot name another target" -Test {
        $root = New-ContractFixture
        Set-FixedHostCondition `
            -Root $root `
            -Condition "runner.os == 'Windows' && matrix.target == 'x86_64-unknown-linux-gnu'"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Windows x64 target" `
            -Scenario "non-Windows target fixed host condition"
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

    Invoke-Scenario -Name "an additional direct Windows Cargo test cannot be added" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText @"
      - name: Test workspace natively
        if: runner.os != 'Windows'
"@ `
            -NewText @"
      - name: Direct Windows Cargo test
        if: runner.os == 'Windows'
        run: cargo test -p wokrouter-platform --locked
      - name: Test workspace natively
        if: runner.os != 'Windows'
"@
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Direct Windows Cargo tests" `
            -Scenario "additional direct Windows Cargo test"
    }

    Invoke-Scenario -Name "Cargo hash test executables cannot run directly" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "        run: cargo test --workspace --all-features --locked --no-run --target `${{ matrix.target }}" `
            -NewText "        run: ./target/debug/deps/wokrouter-0123456789abcdef.exe"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "hash test executables" `
            -Scenario "direct Cargo hash test executable"
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

    Invoke-Scenario -Name "provider credentials must be cleared before the fixed host" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText '          $env:GEMINI_API_KEY = ""' `
            -NewText '          $env:GEMINI_API_KEY = "inherited"'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "all four Provider keys" `
            -Scenario "missing fixed-host Provider clearing"
    }

    Invoke-Scenario -Name "Windows arm64 tests cannot execute" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText " --no-run --target `${{ matrix.target }}" `
            -NewText " --target `${{ matrix.target }}"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "compile without running" `
            -Scenario "Windows arm64 Cargo tests without no-run"
    }

    Invoke-Scenario -Name "Windows arm64 tools cannot be removed" -Test {
        $root = New-ContractFixture
        Edit-WorkflowJob `
            -Root $root `
            -JobName "native-test-matrix" `
            -OldText "Microsoft.VisualStudio.Component.VC.Tools.ARM64" `
            -NewText "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "Visual C++ ARM64 tools" `
            -Scenario "missing Windows arm64 tool installation"
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

    Invoke-Scenario -Name "six-target matrix cannot lose Windows arm64" -Test {
        $root = New-ContractFixture
        Edit-WorkflowJob `
            -Root $root `
            -JobName "target-check-matrix" `
            -OldText @"
          - os: windows-latest
            target: aarch64-pc-windows-msvc
"@ `
            -NewText ""
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "aarch64-pc-windows-msvc" `
            -Scenario "missing Windows arm64 target"
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

    Invoke-Scenario -Name "compatibility matrix cannot lose WokCore v2 preference coverage" -Test {
        $root = New-ContractFixture
        Edit-Workflow `
            -Root $root `
            -OldText "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1" `
            -NewText "redirects_are_not_followed"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1" `
            -Scenario "missing WokCore v2 preference compatibility"
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
