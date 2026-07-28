[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$releasePath = Join-Path $rootPath ".github/workflows/release.yml"
$ciPath = Join-Path $rootPath ".github/workflows/ci.yml"
$developmentPath = Join-Path $rootPath "docs/operations/development.md"
$failures = [System.Collections.Generic.List[string]]::new()

function Add-Failure {
    param([Parameter(Mandatory)][string]$Message)

    if (-not $failures.Contains($Message)) {
        $failures.Add($Message)
    }
}

function Get-JobBlock {
    param(
        [Parameter(Mandatory)][string]$Workflow,
        [Parameter(Mandatory)][string]$Name
    )

    $pattern = "(?ms)^  $([regex]::Escape($Name)):\s*$.*?(?=^  [A-Za-z0-9_-]+:\s*$|\z)"
    $matches = [regex]::Matches($Workflow, $pattern)
    if ($matches.Count -ne 1) {
        Add-Failure -Message "Release workflow must define job '$Name' exactly once."
        return ""
    }
    return $matches[0].Value
}

foreach ($path in @($releasePath, $ciPath, $developmentPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Add-Failure -Message "Required release contract file is missing: $path"
    }
}

if ($failures.Count -eq 0) {
    $release = (Get-Content -LiteralPath $releasePath -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $ci = (Get-Content -LiteralPath $ciPath -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $development = Get-Content -LiteralPath $developmentPath -Raw -Encoding UTF8

    if ($release -notmatch '(?m)^      - "v\*"$') {
        Add-Failure -Message "Release workflow must verify WokRouter v* tag pushes."
    }
    if (
        $release -notmatch '(?m)^  workflow_dispatch:\s*$' -or
        $release -notmatch '(?m)^      release_tag:\s*$' -or
        $release -notmatch '(?m)^        required: true\s*$'
    ) {
        Add-Failure -Message "Release workflow must require a release_tag for manual verification."
    }
    if ($release -notmatch '(?m)^permissions:\n  contents: read\s*$') {
        Add-Failure -Message "Release workflow root permissions must be contents: read."
    }

    foreach ($providerVariable in @(
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY"
        )) {
        foreach ($workflow in @($release, $ci)) {
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
                Add-Failure `
                    -Message "CI and release workflows must define '$providerVariable' exactly once as empty."
            }
        }
    }

    $versionJob = Get-JobBlock -Workflow $release -Name "release-version"
    $tagCheckout = @'
      - uses: actions/checkout@v6
        with:
          fetch-depth: 1
          ref: ${{ github.event_name == 'workflow_dispatch' && format('refs/tags/{0}', inputs.release_tag) || github.ref }}
'@
    if (
        $versionJob -notmatch [regex]::Escape('${{ inputs.release_tag }}') -or
        $versionJob -notmatch [regex]::Escape('${{ github.ref_name }}') -or
        $versionJob -notmatch "canonical WokRouter semver tag" -or
        $versionJob -notmatch [regex]::Escape('$tag.Substring(1)') -or
        -not $versionJob.Contains($tagCheckout) -or
        $versionJob -notmatch '(?m)^          "source_sha=\$sourceSha" \|$'
    ) {
        Add-Failure `
            -Message "Release source and version must be resolved from the requested WokRouter tag commit."
    }
    if ($release -match '(?m)^\s*WOKCORE_[A-Z_]*VERSION:') {
        Add-Failure -Message "WokRouter release version must not depend on a WokCore version."
    }

    $buildJob = Get-JobBlock -Workflow $release -Name "release-build"
    $sourceCheckout = @'
      - uses: actions/checkout@v6
        with:
          ref: ${{ needs.release-version.outputs.source_sha }}
'@
    if (-not $buildJob.Contains($sourceCheckout)) {
        Add-Failure `
            -Message "Release builds must checkout the commit resolved from the requested WokRouter tag."
    }
    $expectedPairs = @(
        @("windows-latest", "x86_64-pc-windows-msvc", "zip"),
        @("macos-15-intel", "x86_64-apple-darwin", "tar.gz"),
        @("macos-15", "aarch64-apple-darwin", "tar.gz"),
        @("ubuntu-24.04", "x86_64-unknown-linux-gnu", "tar.gz"),
        @("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu", "tar.gz")
    )
    foreach ($pair in $expectedPairs) {
        $pattern = "(?m)^          - os: $([regex]::Escape($pair[0]))\n            target: $([regex]::Escape($pair[1]))\n            extension: $([regex]::Escape($pair[2]))$"
        if ($buildJob -notmatch $pattern) {
            Add-Failure `
                -Message "Release matrix is missing '$($pair[1])' on '$($pair[0])'."
        }
    }
    if (@([regex]::Matches($buildJob, '(?m)^            target: ')).Count -ne 5) {
        Add-Failure -Message "Release build matrix must contain exactly five targets."
    }
    foreach ($requiredText in @(
            "WOKROUTER_BUNDLE_KIND: online",
            'WOKROUTER_RELEASE_VERSION: ${{ needs.release-version.outputs.version }}',
            'WOKROUTER_TARGET_TRIPLE: ${{ matrix.target }}',
            "Build target-specific online bundle",
            "Package release artifact and enforce online boundary",
            "wokrouter-online-",
            "RELEASE-METADATA.json",
            "contains a WokCore or legacy daemon payload"
        )) {
        if (-not $buildJob.Contains($requiredText)) {
            Add-Failure -Message "Release build is missing required boundary text '$requiredText'."
        }
    }

    $compatibilityJob = Get-JobBlock -Workflow $release -Name "release-compatibility"
    if (-not $compatibilityJob.Contains($sourceCheckout)) {
        Add-Failure `
            -Message "Release compatibility tests must checkout the requested WokRouter tag commit."
    }
    foreach ($testName in @(
            "current_wokrouter_accepts_current_wokcore",
            "compatible_handshake_accepts_unknown_same_major_fields",
            "legacy_same_major_runtime_without_installation_id_remains_running",
            "non_overlapping_api_major_is_incompatible_without_http_fallback",
            "an_existing_compatible_install_is_never_overwritten",
            "installing_wokcore_does_not_modify_wokrouter_binary_or_version"
        )) {
        $testPattern = "(?m)^        run: cargo test .* $([regex]::Escape($testName)) --locked$"
        if ($compatibilityJob -notmatch $testPattern) {
            Add-Failure `
                -Message "Release compatibility matrix must execute '$testName' as a Cargo test."
        }
    }

    $verifyJob = Get-JobBlock -Workflow $release -Name "release-verify"
    foreach ($requiredText in @(
            "release-build",
            "release-compatibility",
            "Expected five release archives",
            "RELEASE-METADATA.json",
            "contains a WokCore or legacy daemon payload"
        )) {
        if (-not $verifyJob.Contains($requiredText)) {
            Add-Failure -Message "Release verification is missing '$requiredText'."
        }
    }

    $publishJob = Get-JobBlock -Workflow $release -Name "publish"
    if (
        $publishJob -notmatch [regex]::Escape("startsWith(github.ref, 'refs/tags/')") -or
        $publishJob -notmatch '(?m)^    permissions:\n      contents: write\s*$' -or
        $publishJob -notmatch 'gh release create "\$RELEASE_TAG".*--verify-tag' -or
        $publishJob -notmatch [regex]::Escape('--repo "$GITHUB_REPOSITORY"')
    ) {
        Add-Failure -Message "Publishing must be tag-only, verified, scoped to contents: write, and use an explicit GitHub repository."
    }

    foreach ($requiredFact in @(
            "wokrouter-test-host.exe",
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "online WokRouter",
            "WokRouter tag",
            "independent"
        )) {
        if ($development -notmatch [regex]::Escape($requiredFact)) {
            Add-Failure `
                -Message "Development docs must describe release fact '$requiredFact'."
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "RELEASE CONTRACT ERROR: $failure"
    }
    exit 1
}

Write-Host "Release contract passed."
