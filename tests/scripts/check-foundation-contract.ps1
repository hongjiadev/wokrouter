[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$workflowPath = Join-Path $rootPath ".github/workflows/ci.yml"
$denyPath = Join-Path $rootPath "deny.toml"
$developmentPath = Join-Path $rootPath "docs/operations/development.md"
$failures = [System.Collections.Generic.List[string]]::new()

function Add-ContractFailure {
    param(
        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $failures.Contains($Message)) {
        $failures.Add($Message)
    }
}

function Get-LineIndent {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Line
    )

    if ($Line -match "`t") {
        Add-ContractFailure -Message "Workflow YAML must not use tab indentation."
    }
    if ($Line -match "^( *)") {
        return $Matches[1].Length
    }
    return 0
}

function Get-WorkflowJobs {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$Lines
    )

    $jobsRoots = [System.Collections.Generic.List[int]]::new()
    for ($index = 0; $index -lt $Lines.Count; $index += 1) {
        if ($Lines[$index] -match "^jobs:\s*$") {
            $jobsRoots.Add($index)
        }
    }
    if ($jobsRoots.Count -ne 1) {
        Add-ContractFailure -Message "Workflow must contain exactly one root 'jobs' mapping."
        return @{}
    }

    $jobs = @{}
    $index = $jobsRoots[0] + 1
    while ($index -lt $Lines.Count) {
        $line = $Lines[$index]
        if ([string]::IsNullOrWhiteSpace($line)) {
            $index += 1
            continue
        }

        $indent = Get-LineIndent -Line $line
        if ($indent -eq 0) {
            break
        }
        if ($indent -ne 2 -or $line -notmatch "^  (?<name>[A-Za-z0-9_-]+):\s*$") {
            Add-ContractFailure `
                -Message "Workflow jobs mapping contains invalid structure at line $($index + 1)."
            $index += 1
            continue
        }

        $jobName = $Matches["name"]
        $start = $index
        $index += 1
        while ($index -lt $Lines.Count) {
            if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                $index += 1
                continue
            }
            if ((Get-LineIndent -Line $Lines[$index]) -le 2) {
                break
            }
            $index += 1
        }
        if ($jobs.ContainsKey($jobName)) {
            Add-ContractFailure -Message "Workflow job '$jobName' is defined more than once."
            continue
        }
        $jobs[$jobName] = [pscustomobject]@{
            Name = $jobName
            Lines = @($Lines[$start..($index - 1)])
        }
    }

    return $jobs
}

function Get-JobScalar {
    param(
        [Parameter(Mandatory)]
        [object]$Job,

        [Parameter(Mandatory)]
        [string]$Key
    )

    $pattern = "^    $([regex]::Escape($Key)):\s*(?<value>.*?)\s*$"
    $values = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $Job.Lines) {
        if ($line -match $pattern) {
            $values.Add($Matches["value"])
        }
    }
    if ($values.Count -gt 1) {
        Add-ContractFailure `
            -Message "Workflow job '$($Job.Name)' defines '$Key' more than once."
    }
    if ($values.Count -eq 0) {
        return $null
    }
    return $values[0]
}

function Set-StepScalar {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Fields,

        [Parameter(Mandatory)]
        [string]$Payload,

        [Parameter(Mandatory)]
        [int]$LineNumber
    )

    if ($Payload -notmatch "^(?<key>[A-Za-z0-9_-]+):\s*(?<value>.*)$") {
        Add-ContractFailure `
            -Message "Workflow step contains invalid YAML at line $LineNumber."
        return
    }
    $key = $Matches["key"]
    if ($Fields.ContainsKey($key)) {
        Add-ContractFailure `
            -Message "Workflow step defines '$key' more than once at line $LineNumber."
        return
    }
    $Fields[$key] = $Matches["value"]
}

function ConvertTo-WorkflowStep {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$Lines,

        [Parameter(Mandatory)]
        [int]$StartLine
    )

    $fields = @{}
    $nested = @{}
    $firstPayload = $Lines[0].Substring(8)
    Set-StepScalar -Fields $fields -Payload $firstPayload -LineNumber $StartLine

    $index = 1
    while ($index -lt $Lines.Count) {
        $line = $Lines[$index]
        if ([string]::IsNullOrWhiteSpace($line)) {
            $index += 1
            continue
        }
        $indent = Get-LineIndent -Line $line
        if ($indent -ne 8 -or $line -notmatch "^        (?<payload>.+)$") {
            Add-ContractFailure `
                -Message "Workflow step contains invalid indentation at line $($StartLine + $index)."
            $index += 1
            continue
        }
        $payload = $Matches["payload"]
        if ($payload -notmatch "^(?<key>[A-Za-z0-9_-]+):\s*(?<value>.*)$") {
            Add-ContractFailure `
                -Message "Workflow step contains invalid YAML at line $($StartLine + $index)."
            $index += 1
            continue
        }

        $key = $Matches["key"]
        $value = $Matches["value"]
        if ($value -eq "|" -or $value -eq ">") {
            $blockLines = [System.Collections.Generic.List[string]]::new()
            $index += 1
            while ($index -lt $Lines.Count) {
                if (
                    -not [string]::IsNullOrWhiteSpace($Lines[$index]) -and
                    (Get-LineIndent -Line $Lines[$index]) -lt 10
                ) {
                    break
                }
                if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                    $blockLines.Add("")
                }
                else {
                    $blockLines.Add($Lines[$index].Substring(10))
                }
                $index += 1
            }
            if ($value -eq "|") {
                $fields[$key] = $blockLines -join "`n"
            }
            else {
                $fields[$key] = $blockLines -join " "
            }
            continue
        }

        if ([string]::IsNullOrEmpty($value)) {
            $childValues = @{}
            $index += 1
            while ($index -lt $Lines.Count) {
                if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                    $index += 1
                    continue
                }
                $childIndent = Get-LineIndent -Line $Lines[$index]
                if ($childIndent -le 8) {
                    break
                }
                if (
                    $childIndent -ne 10 -or
                    $Lines[$index] -notmatch
                    "^          (?<childKey>[A-Za-z0-9_-]+):\s*(?<childValue>.*)$"
                ) {
                    Add-ContractFailure `
                        -Message "Workflow step '$key' mapping is invalid at line $($StartLine + $index)."
                    $index += 1
                    continue
                }
                $childValues[$Matches["childKey"]] = $Matches["childValue"]
                $index += 1
            }
            $nested[$key] = $childValues
            continue
        }

        if ($fields.ContainsKey($key)) {
            Add-ContractFailure `
                -Message "Workflow step defines '$key' more than once at line $($StartLine + $index)."
        }
        else {
            $fields[$key] = $value
        }
        $index += 1
    }

    return [pscustomobject]@{
        Fields = $fields
        Nested = $nested
    }
}

function Get-JobSteps {
    param(
        [Parameter(Mandatory)]
        [object]$Job
    )

    $stepsLine = -1
    for ($index = 0; $index -lt $Job.Lines.Count; $index += 1) {
        if ($Job.Lines[$index] -match "^    steps:\s*$") {
            if ($stepsLine -ge 0) {
                Add-ContractFailure `
                    -Message "Workflow job '$($Job.Name)' defines steps more than once."
            }
            $stepsLine = $index
        }
    }
    if ($stepsLine -lt 0) {
        Add-ContractFailure -Message "Workflow job '$($Job.Name)' is missing steps."
        return @()
    }

    $steps = [System.Collections.Generic.List[object]]::new()
    $index = $stepsLine + 1
    while ($index -lt $Job.Lines.Count) {
        if ([string]::IsNullOrWhiteSpace($Job.Lines[$index])) {
            $index += 1
            continue
        }
        $indent = Get-LineIndent -Line $Job.Lines[$index]
        if ($indent -le 4) {
            break
        }
        if ($Job.Lines[$index] -notmatch "^      - ") {
            Add-ContractFailure `
                -Message "Workflow job '$($Job.Name)' has an invalid step at line $($index + 1)."
            $index += 1
            continue
        }

        $start = $index
        $index += 1
        while ($index -lt $Job.Lines.Count) {
            if (
                -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
                (
                    (Get-LineIndent -Line $Job.Lines[$index]) -le 4 -or
                    $Job.Lines[$index] -match "^      - "
                )
            ) {
                break
            }
            $index += 1
        }
        $stepLines = @($Job.Lines[$start..($index - 1)])
        $steps.Add((
                ConvertTo-WorkflowStep `
                    -Lines $stepLines `
                    -StartLine ($start + 1)
            ))
    }

    return @($steps)
}

function Get-PlatformMatrixRunners {
    param(
        [Parameter(Mandatory)]
        [object]$Job
    )

    $strategyIndex = -1
    for ($index = 0; $index -lt $Job.Lines.Count; $index += 1) {
        if ($Job.Lines[$index] -match "^    strategy:\s*$") {
            $strategyIndex = $index
            break
        }
    }
    if ($strategyIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy."
        return @()
    }

    $matrixIndex = -1
    for ($index = $strategyIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if (
            -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
            (Get-LineIndent -Line $Job.Lines[$index]) -le 4
        ) {
            break
        }
        if ($Job.Lines[$index] -match "^      matrix:\s*$") {
            $matrixIndex = $index
            break
        }
    }
    if ($matrixIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy.matrix."
        return @()
    }

    $osIndex = -1
    for ($index = $matrixIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if (
            -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
            (Get-LineIndent -Line $Job.Lines[$index]) -le 6
        ) {
            break
        }
        if ($Job.Lines[$index] -match "^        os:\s*$") {
            $osIndex = $index
            break
        }
    }
    if ($osIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy.matrix.os."
        return @()
    }

    $runners = [System.Collections.Generic.List[string]]::new()
    for ($index = $osIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if ([string]::IsNullOrWhiteSpace($Job.Lines[$index])) {
            continue
        }
        if ((Get-LineIndent -Line $Job.Lines[$index]) -le 8) {
            break
        }
        if ($Job.Lines[$index] -match "^          - (?<runner>[A-Za-z0-9_.-]+)\s*$") {
            $runners.Add($Matches["runner"])
        }
        else {
            Add-ContractFailure `
                -Message "Platform matrix contains invalid runner structure."
        }
    }
    return @($runners)
}

function Assert-JobRunStep {
    param(
        [Parameter(Mandatory)]
        [string]$JobName,

        [Parameter(Mandatory)]
        [object[]]$Steps,

        [Parameter(Mandatory)]
        [string]$Command
    )

    $matches = @(
        $Steps | Where-Object {
            $_.Fields.ContainsKey("run") -and $_.Fields["run"] -eq $Command
        }
    )
    if ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must contain one independent '$Command' step."
        return
    }
    if ($matches[0].Fields.ContainsKey("continue-on-error")) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must propagate failure from '$Command'."
    }
}

function Get-ActionStep {
    param(
        [Parameter(Mandatory)]
        [object[]]$Steps,

        [Parameter(Mandatory)]
        [string]$Action
    )

    $matches = @(
        $Steps | Where-Object {
            $_.Fields.ContainsKey("uses") -and $_.Fields["uses"] -eq $Action
        }
    )
    if ($matches.Count -eq 1) {
        return $matches[0]
    }
    return $null
}

function Assert-NestedValue {
    param(
        [Parameter(Mandatory)]
        [object]$Step,

        [Parameter(Mandatory)]
        [string]$Mapping,

        [Parameter(Mandatory)]
        [string]$Key,

        [Parameter(Mandatory)]
        [string]$Value,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (
        -not $Step.Nested.ContainsKey($Mapping) -or
        -not $Step.Nested[$Mapping].ContainsKey($Key) -or
        $Step.Nested[$Mapping][$Key] -ne $Value
    ) {
        Add-ContractFailure -Message $Message
    }
}

$workflowLines = @(Get-Content -LiteralPath $workflowPath -Encoding UTF8)
$workflow = $workflowLines -join "`n"
$deny = Get-Content -LiteralPath $denyPath -Raw -Encoding UTF8
$development = Get-Content -LiteralPath $developmentPath -Raw -Encoding UTF8
$jobs = Get-WorkflowJobs -Lines $workflowLines

$requiredJobs = @(
    "rust",
    "frontend",
    "native-test-matrix",
    "target-check-matrix",
    "compatibility",
    "platform-check"
)
foreach ($jobName in $requiredJobs) {
    if (-not $jobs.ContainsKey($jobName)) {
        Add-ContractFailure -Message "Workflow jobs mapping is missing '$jobName'."
    }
}

foreach ($providerVariable in @(
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY"
    )) {
    $providerDefinitions = @(
        [regex]::Matches(
            $workflow,
            "(?m)^\s*$providerVariable`:\s*.*$"
        )
    )
    if (
        $workflow -notmatch "(?m)^  $providerVariable`: `"`"`$" -or
        $providerDefinitions.Count -ne 1
    ) {
        Add-ContractFailure `
            -Message "Workflow must define provider environment variable '$providerVariable' exactly once as empty."
    }
}

if ($jobs.ContainsKey("rust")) {
    $rustSteps = @(Get-JobSteps -Job $jobs["rust"])
    foreach ($command in @(
            "node apps/desktop/scripts/stage-sidecars.mjs",
            "pwsh tests/scripts/check-public-repo-hygiene.tests.ps1",
            "pwsh tests/scripts/check-public-repo-hygiene.ps1",
            "pwsh tests/scripts/check-core-boundary.tests.ps1",
            "pwsh tests/scripts/check-core-boundary.ps1",
            "pwsh tests/scripts/check-no-body-persistence.tests.ps1",
            "pwsh tests/scripts/check-no-body-persistence.ps1",
            "pwsh tests/scripts/check-foundation-contract.tests.ps1",
            "pwsh tests/scripts/check-foundation-contract.ps1",
            "pwsh tests/scripts/check-release-contract.tests.ps1",
            "pwsh tests/scripts/check-release-contract.ps1",
            "cargo fmt --all -- --check",
            "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings"
        )) {
        Assert-JobRunStep -JobName "rust" -Steps $rustSteps -Command $command
    }

    $rustToolchain = Get-ActionStep `
        -Steps $rustSteps `
        -Action "actions-rust-lang/setup-rust-toolchain@v1"
    if ($null -eq $rustToolchain) {
        Add-ContractFailure -Message "Rust job must set up the pinned Rust toolchain."
    }
    else {
        Assert-NestedValue `
            -Step $rustToolchain `
            -Mapping "with" `
            -Key "toolchain" `
            -Value "1.97.1" `
            -Message "Rust job must pin Rust 1.97.1."
    }

    $denyStep = Get-ActionStep `
        -Steps $rustSteps `
        -Action "EmbarkStudios/cargo-deny-action@v2.1.1"
    if ($null -eq $denyStep) {
        Add-ContractFailure `
            -Message "Rust job must run cargo-deny-action v2.1.1."
    }
    else {
        Assert-NestedValue `
            -Step $denyStep `
            -Mapping "with" `
            -Key "command" `
            -Value "check" `
            -Message "Rust cargo-deny step must run the check command."
        Assert-NestedValue `
            -Step $denyStep `
            -Mapping "with" `
            -Key "arguments" `
            -Value "--all-features" `
            -Message "Rust cargo-deny step must check all features."
    }
}

if ($jobs.ContainsKey("frontend")) {
    $frontendSteps = @(Get-JobSteps -Job $jobs["frontend"])
    foreach ($command in @(
            "pnpm --dir apps/desktop install --frozen-lockfile",
            "pnpm --dir apps/desktop typecheck",
            "pnpm --dir apps/desktop test:unit",
            "pnpm --dir apps/desktop build"
        )) {
        Assert-JobRunStep -JobName "frontend" -Steps $frontendSteps -Command $command
    }

    $pnpmStep = Get-ActionStep -Steps $frontendSteps -Action "pnpm/action-setup@v6"
    if ($null -eq $pnpmStep) {
        Add-ContractFailure -Message "Frontend job must set up pnpm."
    }
    else {
        Assert-NestedValue `
            -Step $pnpmStep `
            -Mapping "with" `
            -Key "version" `
            -Value "11.17.0" `
            -Message "Frontend job must pin pnpm 11.17.0."
    }
}

function Assert-TargetMatrix {
    param(
        [Parameter(Mandatory)]
        [object]$Job,

        [Parameter(Mandatory)]
        [string]$JobName
    )

    if ((Get-JobScalar -Job $Job -Key "runs-on") -ne '${{ matrix.os }}') {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' runs-on must be '`${{ matrix.os }}'."
    }
    $jobText = $Job.Lines -join "`n"
    $expectedPairs = @(
        @("windows-latest", "x86_64-pc-windows-msvc"),
        @("windows-latest", "aarch64-pc-windows-msvc"),
        @("macos-15-intel", "x86_64-apple-darwin"),
        @("macos-14", "aarch64-apple-darwin"),
        @("ubuntu-24.04", "x86_64-unknown-linux-gnu"),
        @("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu")
    )
    foreach ($pair in $expectedPairs) {
        $pattern = "(?m)^          - os: $([regex]::Escape($pair[0]))\n            target: $([regex]::Escape($pair[1]))$"
        if ($jobText -notmatch $pattern) {
            Add-ContractFailure `
                -Message "Workflow job '$JobName' is missing native runner '$($pair[0])' for '$($pair[1])'."
        }
    }
    $targetCount = @(
        $Job.Lines | Where-Object { $_ -match "^            target: " }
    ).Count
    if ($targetCount -ne 6) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must contain exactly 6 target entries."
    }
}

if ($jobs.ContainsKey("native-test-matrix")) {
    $nativeJob = $jobs["native-test-matrix"]
    Assert-TargetMatrix -Job $nativeJob -JobName "native-test-matrix"
    $nativeSteps = @(Get-JobSteps -Job $nativeJob)
    Assert-JobRunStep `
        -JobName "native-test-matrix" `
        -Steps $nativeSteps `
        -Command "./tests/scripts/run-fixed-test-host.tests.ps1"
    $fixedHostSelfTests = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq "./tests/scripts/run-fixed-test-host.tests.ps1"
        }
    )
    $windowsX64Condition = (
        "runner.os == 'Windows' && " +
        "matrix.target == 'x86_64-pc-windows-msvc'"
    )
    if (
        $fixedHostSelfTests.Count -ne 1 -or
        -not $fixedHostSelfTests[0].Fields.ContainsKey("if") -or
        $fixedHostSelfTests[0].Fields["if"] -ne $windowsX64Condition
    ) {
        Add-ContractFailure `
            -Message "The fixed test host self-test must run only for the Windows x64 target."
    }
    $fixedHostSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "run-fixed-test-host\.ps1" -and
            $_.Fields["run"] -match "HarnessArguments @\(`"--nocapture`"\)"
        }
    )
    if ($fixedHostSteps.Count -ne 1) {
        Add-ContractFailure `
            -Message "Windows native tests must execute the workspace through the fixed test host."
    }
    else {
        if (
            -not $fixedHostSteps[0].Fields.ContainsKey("if") -or
            $fixedHostSteps[0].Fields["if"] -ne $windowsX64Condition
        ) {
            Add-ContractFailure `
                -Message "The fixed test host step must run only for the Windows x64 target."
        }
        $providerClearPattern = (
            '(?ms)\$env:OPENAI_API_KEY = ""\n' +
            '\$env:ANTHROPIC_API_KEY = ""\n' +
            '\$env:GEMINI_API_KEY = ""\n' +
            '\$env:GOOGLE_API_KEY = ""\n' +
            '\s*& \./tests/scripts/run-fixed-test-host\.ps1'
        )
        if ($fixedHostSteps[0].Fields["run"] -notmatch $providerClearPattern) {
            Add-ContractFailure `
                -Message "Windows Rust tests must clear all four Provider keys immediately before the fixed host."
        }
    }
    $arm64ToolSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "Microsoft\.VisualStudio\.Component\.VC\.Tools\.ARM64"
        }
    )
    $windowsArm64Condition = (
        "runner.os == 'Windows' && " +
        "matrix.target == 'aarch64-pc-windows-msvc'"
    )
    if (
        $arm64ToolSteps.Count -ne 1 -or
        -not $arm64ToolSteps[0].Fields.ContainsKey("if") -or
        $arm64ToolSteps[0].Fields["if"] -ne $windowsArm64Condition -or
        $arm64ToolSteps[0].Fields["run"] -notmatch "-WindowStyle Hidden"
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 native checks must install the Visual C++ ARM64 tools."
    }
    $arm64CompileSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq (
                'cargo test --workspace --all-features --locked --no-run ' +
                '--target ${{ matrix.target }}'
            )
        }
    )
    if (
        $arm64CompileSteps.Count -ne 1 -or
        -not $arm64CompileSteps[0].Fields.ContainsKey("if") -or
        $arm64CompileSteps[0].Fields["if"] -ne $windowsArm64Condition
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 tests must compile without running."
    }
    $allowedCargoRuns = @(
        'cargo test --workspace --all-features --locked',
        (
            'cargo test --workspace --all-features --locked --no-run ' +
            '--target ${{ matrix.target }}'
        )
    )
    $unexpectedCargoTests = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "(?m)(^|\n)\s*cargo test " -and
            $_.Fields["run"] -notin $allowedCargoRuns
        }
    )
    if ($unexpectedCargoTests.Count -ne 0) {
        Add-ContractFailure `
            -Message "Direct Windows Cargo tests are forbidden outside the two exact native matrix paths."
    }
    $hashedExecutableSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match (
                "(?im)(^|\n).*target.*[\\/]deps[\\/].*\.exe(?:\s|$)"
            )
        }
    )
    if ($hashedExecutableSteps.Count -ne 0) {
        Add-ContractFailure `
            -Message "Cargo hash test executables must never run directly on Windows."
    }
    $nativeCargoSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq "cargo test --workspace --all-features --locked"
        }
    )
    if (
        $nativeCargoSteps.Count -ne 1 -or
        -not $nativeCargoSteps[0].Fields.ContainsKey("if") -or
        $nativeCargoSteps[0].Fields["if"] -ne "runner.os != 'Windows'"
    ) {
        Add-ContractFailure `
            -Message "Direct Cargo workspace tests must be restricted to non-Windows native runners."
    }
}

if ($jobs.ContainsKey("target-check-matrix")) {
    $targetJob = $jobs["target-check-matrix"]
    Assert-TargetMatrix -Job $targetJob -JobName "target-check-matrix"
    $targetSteps = @(Get-JobSteps -Job $targetJob)
    Assert-JobRunStep `
        -JobName "target-check-matrix" `
        -Steps $targetSteps `
        -Command 'cargo check --workspace --all-features --locked --target ${{ matrix.target }}'
    $arm64ToolSteps = @(
        $targetSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "Microsoft\.VisualStudio\.Component\.VC\.Tools\.ARM64"
        }
    )
    if (
        $arm64ToolSteps.Count -ne 1 -or
        -not $arm64ToolSteps[0].Fields.ContainsKey("if") -or
        $arm64ToolSteps[0].Fields["if"] -ne $windowsArm64Condition -or
        $arm64ToolSteps[0].Fields["run"] -notmatch "-WindowStyle Hidden"
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 target checks must install the Visual C++ ARM64 tools."
    }
}

if ($jobs.ContainsKey("compatibility")) {
    $compatibilitySteps = @(Get-JobSteps -Job $jobs["compatibility"])
    foreach ($command in @(
            "cargo test -p wokrouter-wokcore-client --test handshake current_wokrouter_accepts_current_wokcore --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake compatible_handshake_accepts_unknown_same_major_fields --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake legacy_same_major_runtime_without_installation_id_remains_running --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake non_overlapping_api_major_is_incompatible_without_http_fallback --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install an_existing_compatible_install_is_never_overwritten --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install installing_wokcore_does_not_modify_wokrouter_binary_or_version --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1 --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_missing_v2_manifest_falls_back_to_the_signed_v1_release --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_present_invalid_v2_manifest_never_downgrades_to_v1 --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_rejects_a_signed_v1_schema_at_the_v2_endpoint_without_downgrading --locked"
        )) {
        Assert-JobRunStep `
            -JobName "compatibility" `
            -Steps $compatibilitySteps `
            -Command $command
    }
}

if ($jobs.ContainsKey("platform-check")) {
    $aggregator = $jobs["platform-check"]
    if ((Get-JobScalar -Job $aggregator -Key "if") -ne "always()") {
        Add-ContractFailure -Message "Platform aggregator if must be 'always()'."
    }
    $aggregatorText = $aggregator.Lines -join "`n"
    foreach ($dependency in @(
            "rust",
            "frontend",
            "native-test-matrix",
            "target-check-matrix",
            "compatibility"
        )) {
        if ($aggregatorText -notmatch "(?m)^      - $([regex]::Escape($dependency))$") {
            Add-ContractFailure `
                -Message "Platform aggregator must require '$dependency'."
        }
        if ($aggregatorText -notmatch [regex]::Escape("needs.$dependency.result")) {
            Add-ContractFailure `
                -Message "Platform aggregator must verify the result of '$dependency'."
        }
    }
}

if ($deny -notmatch "(?m)^yanked\s*=\s*`"deny`"\s*$") {
    Add-ContractFailure -Message "deny.toml must deny yanked dependencies."
}
if ($deny -notmatch '(?m)^\s*"aarch64-apple-darwin",\s*$') {
    Add-ContractFailure -Message "deny.toml must include the Apple Silicon target."
}
if ($development -notmatch "cargo-deny 0\.20\.2") {
    Add-ContractFailure -Message "Development docs must pin cargo-deny 0.20.2."
}
if (
    $development -notmatch
    "cargo install --locked cargo-deny --version 0\.20\.2"
) {
    Add-ContractFailure `
        -Message "Development docs must give the CI-matching cargo-deny install command."
}
if ($development -notmatch "cargo deny --version") {
    Add-ContractFailure `
        -Message "Development docs must require cargo-deny version verification."
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "CONTRACT ERROR: $failure"
    }
    exit 1
}

Write-Host "Foundation CI/configuration contract passed."
