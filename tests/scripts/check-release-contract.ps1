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
$releaseContractPath = Join-Path `
    $rootPath `
    "tests/release/WokRouter.ReleaseContract.psm1"
$linuxPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-linux-assets.ps1"
$macPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-macos-assets.ps1"
$windowsPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-windows-assets.ps1"
$signerPath = Join-Path $rootPath "tests/release/sign-release-bundle.ps1"
$verifierPath = Join-Path $rootPath "tests/release/verify-release-bundle.ps1"
$publicKeyPath = Join-Path $rootPath "release/minisign.pub"
$cargoManifestPath = Join-Path $rootPath "Cargo.toml"
$cargoLockPath = Join-Path $rootPath "Cargo.lock"
$packageManifestPath = Join-Path $rootPath "apps/desktop/package.json"
$tauriConfigurationPath = Join-Path `
    $rootPath `
    "apps/desktop/src-tauri/tauri.conf.json"
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

function Get-SourceMatchIndex {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Pattern
    )

    $match = [regex]::Match($Source, $Pattern)
    return $(if ($match.Success) { $match.Index } else { -1 })
}

function Read-BoundedUtf8Text {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][long]$MaximumBytes
    )

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -eq 0 -or $bytes.Length -gt $MaximumBytes) {
        throw "Release source file has an invalid size: $Path"
    }
    return [Text.UTF8Encoding]::new($false, $true).GetString($bytes).Replace(
        "`r`n",
        "`n"
    )
}

function Get-CargoWorkspaceVersion {
    param([Parameter(Mandatory)][string]$Text)

    $sections = @(
        [regex]::Matches(
            $Text,
            '(?ms)^\[workspace\.package\][ \t]*\n(?<body>.*?)(?=^\[[^\r\n]+\][ \t]*$|\z)'
        )
    )
    if ($sections.Count -ne 1) {
        throw "Cargo.toml must contain exactly one [workspace.package] section."
    }
    $versions = @(
        [regex]::Matches(
            $sections[0].Groups["body"].Value,
            '(?m)^version[ \t]*=[ \t]*"(?<value>[^"]+)"[ \t]*$'
        )
    )
    if ($versions.Count -ne 1) {
        throw "Cargo.toml workspace package must contain exactly one version."
    }
    return $versions[0].Groups["value"].Value
}

function Get-JsonReleaseVersion {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][string]$Name
    )

    $members = @([regex]::Matches($Text, '(?m)"version"[ \t\r\n]*:'))
    if ($members.Count -ne 1) {
        throw "$Name must contain exactly one version member."
    }
    $document = $Text | ConvertFrom-Json
    $properties = @(
        $document.PSObject.Properties |
            Where-Object { $_.Name -ieq "version" }
    )
    if (
        $properties.Count -ne 1 -or
        $properties[0].Name -cne "version" -or
        $properties[0].Value -isnot [string]
    ) {
        throw "$Name must contain one exact string version member."
    }
    return [string]$properties[0].Value
}

function Get-WokRouterLockVersions {
    param([Parameter(Mandatory)][string]$Text)

    $wanted = @(
        "wokrouter-cli",
        "wokrouter-desktop",
        "wokrouter-platform",
        "wokrouter-storage",
        "wokrouter-wokcore-client"
    )
    $blocks = @(
        [regex]::Matches(
            $Text,
            '(?ms)^\[\[package\]\][ \t]*\n.*?(?=^\[\[package\]\][ \t]*$|\z)'
        )
    )
    $versions = [System.Collections.Generic.List[string]]::new()
    foreach ($packageName in $wanted) {
        $matching = @(
            $blocks |
                Where-Object {
                    $_.Value -match (
                        '(?m)^name[ \t]*=[ \t]*"' +
                        [regex]::Escape($packageName) +
                        '"[ \t]*$'
                    )
                }
        )
        if ($matching.Count -ne 1) {
            throw "Cargo.lock must contain exactly one '$packageName' package."
        }
        $version = @(
            [regex]::Matches(
                $matching[0].Value,
                '(?m)^version[ \t]*=[ \t]*"(?<value>[^"]+)"[ \t]*$'
            )
        )
        if ($version.Count -ne 1) {
            throw "Cargo.lock package '$packageName' must contain exactly one version."
        }
        $versions.Add($version[0].Groups["value"].Value)
    }
    return $versions.ToArray()
}

foreach ($path in @(
        $releasePath,
        $ciPath,
        $developmentPath,
        $releaseContractPath,
        $linuxPackagerPath,
        $macPackagerPath,
        $windowsPackagerPath,
        $signerPath,
        $verifierPath,
        $publicKeyPath,
        $cargoManifestPath,
        $cargoLockPath,
        $packageManifestPath,
        $tauriConfigurationPath
    )) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Add-Failure -Message "Required release contract file is missing: $path"
    }
}

if ($failures.Count -eq 0) {
    try {
        $workspaceVersion = Get-CargoWorkspaceVersion -Text (
            Read-BoundedUtf8Text -Path $cargoManifestPath -MaximumBytes 131072
        )
        [string[]]$sourceVersions = @(
            $workspaceVersion
            Get-JsonReleaseVersion `
                -Text (
                    Read-BoundedUtf8Text `
                        -Path $packageManifestPath `
                        -MaximumBytes 262144
                ) `
                -Name "apps/desktop/package.json"
            Get-JsonReleaseVersion `
                -Text (
                    Read-BoundedUtf8Text `
                        -Path $tauriConfigurationPath `
                        -MaximumBytes 262144
                ) `
                -Name "apps/desktop/src-tauri/tauri.conf.json"
            Get-WokRouterLockVersions -Text (
                Read-BoundedUtf8Text -Path $cargoLockPath -MaximumBytes 8388608
            )
        )
        foreach ($sourceVersion in $sourceVersions) {
            if ($sourceVersion -cne $workspaceVersion) {
                throw "WokRouter product source versions must match exactly."
            }
        }
    }
    catch {
        Add-Failure -Message "WokRouter source versions are invalid: $($_.Exception.Message)"
    }

    Import-Module $releaseContractPath -Force
    try {
        [string[]] $expectedTargets = @(
            "aarch64-apple-darwin",
            "aarch64-pc-windows-msvc",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu"
        )
        [string[]] $expectedPayloads = @(
            "WokRouter-v0.1.6-Linux-arm64.AppImage",
            "WokRouter-v0.1.6-Linux-arm64.deb",
            "WokRouter-v0.1.6-Linux-arm64.rpm",
            "WokRouter-v0.1.6-Linux-x86_64.AppImage",
            "WokRouter-v0.1.6-Linux-x86_64.deb",
            "WokRouter-v0.1.6-Linux-x86_64.rpm",
            "WokRouter-v0.1.6-Windows-arm64-Portable.zip",
            "WokRouter-v0.1.6-Windows-arm64.msi",
            "WokRouter-v0.1.6-Windows-x86_64-Portable.zip",
            "WokRouter-v0.1.6-Windows-x86_64.msi",
            "WokRouter-v0.1.6-macOS-arm64.dmg",
            "WokRouter-v0.1.6-macOS-arm64.tar.gz",
            "WokRouter-v0.1.6-macOS-arm64.zip",
            "WokRouter-v0.1.6-macOS-x86_64.dmg",
            "WokRouter-v0.1.6-macOS-x86_64.tar.gz",
            "WokRouter-v0.1.6-macOS-x86_64.zip"
        )
        [string[]] $actualTargets = @(
            Get-WokRouterTargetContracts -Version "0.1.6" |
                ForEach-Object Target
        )
        [string[]] $actualPayloads = @(
            Get-WokRouterPayloadNames -Version "0.1.6"
        )
        if (
            [string]::Join("`n", $actualTargets) -cne
            [string]::Join("`n", $expectedTargets)
        ) {
            Add-Failure `
                -Message "Release contract must return the exact 6 ordinal target names."
        }
        if (
            [string]::Join("`n", $actualPayloads) -cne
            [string]::Join("`n", $expectedPayloads) -or
            $actualPayloads -match "unknown|pc-windows|apple-darwin"
        ) {
            Add-Failure `
                -Message "Release contract must return the exact 16 friendly payload names."
        }
    }
    catch {
        Add-Failure `
            -Message "Release asset contract could not be evaluated: $($_.Exception.Message)"
    }
    finally {
        Remove-Module WokRouter.ReleaseContract -ErrorAction SilentlyContinue
    }

    foreach ($packagerPath in @(
            $linuxPackagerPath,
            $macPackagerPath,
            $windowsPackagerPath
        )) {
        $packagerSource = Get-Content `
            -LiteralPath $packagerPath `
            -Raw `
            -Encoding UTF8
        if ($packagerSource -match "(?i)\b(?:skip|bypass)\b") {
            Add-Failure `
                -Message "Release packagers must not contain Skip or Bypass production paths."
        }
    }

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
    $concurrencyBlock = @'
concurrency:
  group: wokrouter-release-${{ github.event_name == 'workflow_dispatch' && inputs.release_tag || github.ref_name }}
  cancel-in-progress: false
'@
    if (-not $release.Contains($concurrencyBlock)) {
        Add-Failure `
            -Message "Release workflow must serialize the same release tag without cancellation."
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
        $versionJob -notmatch '(?m)^          "source_sha=\$sourceSha" \|$' -or
        -not $versionJob.Contains("Read-ExactUtf8File") -or
        -not $versionJob.Contains("Get-CargoWorkspaceVersion") -or
        -not $versionJob.Contains("Get-JsonVersion") -or
        -not $versionJob.Contains("Get-LockPackageVersions") -or
        -not $versionJob.Contains(
            "WokRouter source version does not match release tag."
        )
    ) {
        Add-Failure `
            -Message "Release source and version must be resolved from the requested WokRouter tag commit and match every product source."
    }
    if ($release -match '(?m)^\s*WOKCORE_[A-Z_]*VERSION:') {
        Add-Failure -Message "WokRouter release version must not depend on a WokCore version."
    }

    $buildJob = Get-JobBlock -Workflow $release -Name "release-build"
    $sourceCheckout = @'
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
          ref: ${{ needs.release-version.outputs.source_sha }}
'@
    if (-not $buildJob.Contains($sourceCheckout)) {
        Add-Failure `
            -Message "Release builds must checkout the commit resolved from the requested WokRouter tag."
    }
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
        if ($buildJob -notmatch $pattern) {
            Add-Failure `
                -Message "Release matrix is missing '$($pair[1])' on '$($pair[0])'."
        }
    }
    if (
        @([regex]::Matches($buildJob, '(?m)^            target: ')).Count -ne
        6
    ) {
        Add-Failure `
            -Message "Release build matrix must contain exactly 6 targets."
    }
    foreach ($requiredText in @(
            "WOKROUTER_BUNDLE_KIND: online",
            'WOKROUTER_RELEASE_VERSION: ${{ needs.release-version.outputs.version }}',
            'WOKROUTER_TARGET_TRIPLE: ${{ matrix.target }}',
            "sudo apt-get install --yes --no-install-recommends",
            'name: wokrouter-payload-${{ matrix.target }}',
            'path: target/wokrouter-public-${{ matrix.target }}/*'
        )) {
        if (-not $buildJob.Contains($requiredText)) {
            Add-Failure -Message "Release build is missing required boundary text '$requiredText'."
        }
    }
    $arm64ToolCondition = (
        "if: runner.os == 'Windows' && " +
        "matrix.target == 'aarch64-pc-windows-msvc'"
    )
    if (
        @(
            $buildJob -split "`n" |
                Where-Object {
                    $_.Trim() -ceq 'pnpm --dir apps/desktop tauri build `'
                }
        ).Count -ne 3 -or
        -not $buildJob.Contains($arm64ToolCondition) -or
        -not $buildJob.Contains(
            "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
        ) -or
        -not $buildJob.Contains("-WindowStyle Hidden") -or
        $buildJob.Contains("ToolAdapterPath")
    ) {
        Add-Failure `
            -Message "Release builds must run one explicit native packager path per platform and install Windows ARM64 tools."
    }
    foreach ($platformBuild in @(
            @("runner.os == 'Linux'", "--bundles appimage,deb,rpm"),
            @("runner.os == 'macOS'", "--bundles app,dmg"),
            @("runner.os == 'Windows'", "--bundles msi")
        )) {
        $pattern = (
            "(?ms)^      - name: Build .*?`n" +
            "        if: $([regex]::Escape($platformBuild[0]))`n" +
            ".*?$([regex]::Escape($platformBuild[1]))"
        )
        if ($buildJob -notmatch $pattern) {
            Add-Failure `
                -Message "Release build is missing the scoped '$($platformBuild[1])' command."
        }
        if (
            @(
                $buildJob -split "`n" |
                    Where-Object {
                        $_.Trim() -ceq "$($platformBuild[1]) ``"
                    }
            ).Count -ne 1
        ) {
            Add-Failure `
                -Message "Release build must contain one executable '$($platformBuild[1])' line."
        }
    }
    foreach ($packager in @(
            "package-linux-assets.ps1",
            "package-macos-assets.ps1",
            "package-windows-assets.ps1"
        )) {
        if (
            @(
                $buildJob -split "`n" |
                    Where-Object {
                        $_.Trim() -ceq "& tests/release/$packager ``"
                    }
            ).Count -ne 1
        ) {
            Add-Failure `
                -Message "Release build must execute '$packager' exactly once."
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
            "installing_wokcore_does_not_modify_wokrouter_binary_or_version",
            "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1",
            "wokcore_install_missing_v2_manifest_falls_back_to_the_signed_v1_release",
            "wokcore_install_present_invalid_v2_manifest_never_downgrades_to_v1",
            "wokcore_install_rejects_a_signed_v1_schema_at_the_v2_endpoint_without_downgrading"
        )) {
        $testPattern = "(?m)^        run: cargo test .* $([regex]::Escape($testName)) --locked$"
        if ($compatibilityJob -notmatch $testPattern) {
            Add-Failure `
                -Message "Release compatibility matrix must execute '$testName' as a Cargo test."
        }
    }

    $verifyJob = Get-JobBlock -Workflow $release -Name "release-assemble"
    foreach ($requiredText in @(
            "release-build",
            "release-compatibility",
            "merge-multiple: true",
            "Get-WokRouterPayloadNames",
            "WOKROUTER_MINISIGN_SECRET_KEY",
            "sign-release-bundle.ps1",
            "verify-release-bundle.ps1"
        )) {
        if (-not $verifyJob.Contains($requiredText)) {
            Add-Failure -Message "Release verification is missing '$requiredText'."
        }
    }
    $assembleCheckout = @'
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
          ref: ${{ needs.release-version.outputs.source_sha }}
'@
    if (
        -not $verifyJob.Contains($assembleCheckout) -or
        -not $verifyJob.Contains("sudo apt-get install --yes --no-install-recommends minisign") -or
        -not $verifyJob.Contains("pattern: wokrouter-payload-*") -or
        -not $verifyJob.Contains(
            'name: wokrouter-${{ needs.release-version.outputs.tag }}-signed'
        )
    ) {
        Add-Failure `
            -Message "Release assembly must checkout the verified source and produce one exact signed bundle artifact."
    }
    $payloadIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            Get-WokRouterPayloadNames -Version '
    $secretIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^          WOKROUTER_MINISIGN_SECRET_KEY: '
    $signIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            & tests/release/sign-release-bundle\.ps1 `\s*$'
    $localVerifyIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            & tests/release/verify-release-bundle\.ps1 `\s*$'
    $signedUploadIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^          name: wokrouter-\$\{\{ needs\.release-version\.outputs\.tag \}\}-signed$'
    if (
        -not $verifyJob.Contains('$items.Count -ne 16') -or
        $payloadIndex -lt 0 -or
        $secretIndex -le $payloadIndex -or
        $signIndex -le $secretIndex -or
        $localVerifyIndex -le $signIndex -or
        $signedUploadIndex -le $localVerifyIndex -or
        -not $verifyJob.Contains("-PublicKeyPath release/minisign.pub")
    ) {
        Add-Failure `
            -Message "Release assembly must require 16 payloads before reading the secret, then sign and locally verify before upload."
    }

    $publishJob = Get-JobBlock -Workflow $release -Name "publish"
    $draftCreateBlock = @'
            gh release create "$RELEASE_TAG" \
              --repo "$GITHUB_REPOSITORY" \
              --verify-tag \
              --draft \
'@
    $publishEditBlock = @'
          gh release edit "$RELEASE_TAG" \
            --repo "$GITHUB_REPOSITORY" \
            --draft=false
'@
    $preMutationIdentityBlock = @'
          begin_release_mutation() {
            if [[ "$release_mutation_started" == "false" ]]; then
              require_remote_tag_commit
              release_mutation_started=true
            fi
          }
'@
    $preCreateIdentityBlock = @'
            begin_release_mutation
            gh release create "$RELEASE_TAG" \
'@
    $preDeleteIdentityBlock = @'
            begin_release_mutation
            gh release delete-asset "$RELEASE_TAG" "$asset" \
'@
    $preUploadIdentityBlock = @'
          begin_release_mutation
          gh release upload "$RELEASE_TAG" dist/* \
'@
    $prePublicationIdentityBlock = @'
          require_remote_tag_commit
          gh release edit "$RELEASE_TAG" \
'@
    if (
        $publishJob -notmatch [regex]::Escape("startsWith(github.ref, 'refs/tags/')") -or
        $publishJob -notmatch '(?m)^    permissions:\n      contents: write\s*$' -or
        $publishJob -notmatch 'gh release create "\$RELEASE_TAG"' -or
        $publishJob -notmatch '--verify-tag' -or
        $publishJob -notmatch 'isDraft' -or
        $publishJob -notmatch 'gh release delete-asset' -or
        $publishJob -notmatch 'gh release download' -or
        $publishJob -notmatch 'verify-release-bundle\.ps1' -or
        $publishJob -notmatch 'gh release edit "\$RELEASE_TAG"' -or
        $publishJob -notmatch '--draft=false' -or
        -not $publishJob.Contains($draftCreateBlock) -or
        -not $publishJob.Contains($publishEditBlock) -or
        -not $publishJob.Contains($preMutationIdentityBlock) -or
        -not $publishJob.Contains($preCreateIdentityBlock) -or
        -not $publishJob.Contains($preDeleteIdentityBlock) -or
        -not $publishJob.Contains($preUploadIdentityBlock) -or
        -not $publishJob.Contains($prePublicationIdentityBlock) -or
        -not $publishJob.Contains("gh api") -or
        -not $publishJob.Contains("SOURCE_SHA") -or
        -not $publishJob.Contains("Remote WokRouter tag commit does not match source SHA.") -or
        $publishJob -notmatch [regex]::Escape('--repo "$GITHUB_REPOSITORY"') -or
        @([regex]::Matches($publishJob, '\bgh release (?:view|create|delete-asset|upload|download|edit)\b')).Count -ne
        @([regex]::Matches(
                $publishJob,
                [regex]::Escape('--repo "$GITHUB_REPOSITORY"')
            )).Count
    ) {
        Add-Failure -Message "Publishing must be tag-only, verified, scoped to contents: write, and use an explicit GitHub repository."
    }
    if (
        @([regex]::Matches($release, '(?m)^\s+contents: write\s*$')).Count -ne
        1 -or
        $publishJob -notmatch (
            "(?m)^    if: github\.event_name == 'push' && " +
            "startsWith\(github\.ref, 'refs/tags/'\)$"
        ) -or
        -not $publishJob.Contains($assembleCheckout) -or
        -not $publishJob.Contains(
            'name: wokrouter-${{ needs.release-version.outputs.tag }}-signed'
        ) -or
        -not $publishJob.Contains("-PublicKeyPath release/minisign.pub") -or
        -not $publishJob.Contains("Expected exactly 35 signed WokRouter assets") -or
        -not $publishJob.Contains('"${#local_assets[@]}" -ne 35') -or
        -not $publishJob.Contains(
            "sudo apt-get install --yes --no-install-recommends minisign"
        ) -or
        -not $publishJob.Contains(
            "The WokRouter draft became public before asset cleanup."
        ) -or
        $publishJob -notmatch '(?m)^              --draft \\\s*$'
    ) {
        Add-Failure `
            -Message "Only a tag push may publish the exact externally verified 35-file bundle."
    }
    $preMutationVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^      - name: Verify the signed bundle before release mutation$'
    $firstPublishVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          & tests/release/verify-release-bundle\.ps1 `\s*$'
    $releaseView = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release view "\$RELEASE_TAG" \\$'
    $draftGuard = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            if \[\[ "\$\(jq -r ''\.isDraft'''
    $draftCreate = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release create "\$RELEASE_TAG" \\$'
    $assetCleanup = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release delete-asset "\$RELEASE_TAG" "\$asset" \\$'
    $preUploadGuard = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern (
            '(?ms)^          if \[\[ "\$\(\n' +
            '            gh release view "\$RELEASE_TAG" \\\n' +
            '              --repo "\$GITHUB_REPOSITORY" \\\n' +
            '              --json isDraft \\\n' +
            '              --jq ''\.isDraft''\n' +
            '          \)" != "true" \]\]; then\n' +
            '            echo "The WokRouter draft became public before upload\." >&2\n' +
            '            exit 1\n' +
            '          fi\n' +
            '          begin_release_mutation\n' +
            '          gh release upload "\$RELEASE_TAG" dist/\* \\$'
        )
    $assetUpload = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release upload "\$RELEASE_TAG" dist/\* \\$'
    $remoteDownload = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release download "\$RELEASE_TAG" \\$'
    $remoteVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          pwsh tests/release/verify-release-bundle\.ps1 \\$'
    $publishRelease = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release edit "\$RELEASE_TAG" \\$'
    if (
        $preMutationVerify -lt 0 -or
        $firstPublishVerify -le $preMutationVerify -or
        $releaseView -le $firstPublishVerify -or
        $draftGuard -le $releaseView -or
        $draftCreate -le $draftGuard -or
        $assetCleanup -le $draftGuard -or
        $preUploadGuard -le $assetCleanup -or
        $assetUpload -le $preUploadGuard -or
        $remoteDownload -le $assetUpload -or
        $remoteVerify -le $remoteDownload -or
        $publishRelease -le $remoteVerify
    ) {
        Add-Failure `
            -Message "Publishing must guard a draft, clear stale draft assets, upload, re-download, verify, and only then publish."
    }

    $signingSteps = @(
        [regex]::Matches(
            $verifyJob,
            "(?ms)^      - name: Sign and locally verify the release bundle\s*$.*?(?=^      - |\z)"
        )
    )
    $signingStep = if ($signingSteps.Count -eq 1) {
        $signingSteps[0].Value
    }
    else {
        ""
    }
    $releaseWithoutSigningStep = if ($signingStep -eq "") {
        $release
    }
    else {
        $release.Replace($signingStep, "")
    }
    if (
        $signingStep -eq "" -or
        -not $signingStep.Contains("WOKROUTER_MINISIGN_SECRET_KEY") -or
        $releaseWithoutSigningStep.Contains("WOKROUTER_MINISIGN_SECRET_KEY")
    ) {
        Add-Failure `
            -Message "The WOKROUTER_MINISIGN_SECRET_KEY secret must appear only in the release-assemble signing step."
    }
    if (
        $signingStep -notmatch (
            '(?ms)try \{\n\s+\[IO\.File\]::WriteAllText\(.*?' +
            'sign-release-bundle\.ps1.*?finally \{.*?' +
            '\[IO\.File\]::WriteAllBytes\(.*?' +
            'Remove-Item -LiteralPath \$secretPath -Force'
        )
    ) {
        Add-Failure `
            -Message "The plaintext Minisign key write must be covered by secure finally cleanup."
    }
    if ($release.Contains("Expected five release archives")) {
        Add-Failure `
            -Message "The old five-archive release verification path must be removed."
    }

    foreach ($requiredFact in @(
            "wokrouter-test-host.exe",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "online WokRouter",
            "WokRouter tag",
            "independent",
            "exactly 16",
            "exactly 35",
            "release/minisign.pub",
            "immutable"
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
