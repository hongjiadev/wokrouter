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
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/cli/src/commands") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src-tauri/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-platform/tests") -Force
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
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/cli/src/commands/mod.rs") `
        -Destination (Join-Path $root "apps/cli/src/commands/mod.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src/control.ts") `
        -Destination (Join-Path $root "apps/desktop/src/control.ts")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "apps/desktop/src-tauri/src/control.rs") `
        -Destination (Join-Path $root "apps/desktop/src-tauri/src/control.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/src/wokcore_runtime.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/src/wokcore_runtime.rs")
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot "crates/wokrouter-platform/tests/wokcore_runtime.rs") `
        -Destination (Join-Path $root "crates/wokrouter-platform/tests/wokcore_runtime.rs")
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
    if (-not $content.Contains($OldText)) {
        throw "Fixture mutation source was not found in ${RelativePath}: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($OldText, $NewText) -Encoding UTF8
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

    Invoke-Scenario -Name "development executable environment name cannot change" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText '"WOKROUTER_DEV_WOKCORE_EXECUTABLE"' `
            -NewText '"WOKROUTER_WOKCORE_EXECUTABLE"'
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "WOKROUTER_DEV_WOKCORE_EXECUTABLE" `
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
            -ExpectedText "development_runtime_managed_by_ide" `
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
            -OldText "            && process_matches(process_id, &candidate)" `
            -NewText "            && true"
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
            -OldText "            let still_matches = process_matches(process_id, &candidate);" `
            -NewText "            let still_matches = true;"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "process identity" `
            -Scenario "missing process identity recheck"
    }

    Invoke-Scenario -Name "development client must remain bound to the discovered PID" -Test {
        $root = New-ContractFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "crates/wokrouter-platform/src/wokcore_runtime.rs" `
            -OldText "            let bound = client.bound_to_process(process_id);" `
            -NewText "            let bound = client.clone();"
        Assert-ContractRejects `
            -Root $root `
            -ExpectedText "PID-bound" `
            -Scenario "unbound development client"
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
        Edit-Workflow `
            -Root $root `
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
        Edit-Workflow `
            -Root $root `
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
        Edit-Workflow `
            -Root $root `
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
