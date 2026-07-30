[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptUnderTest = Join-Path $PSScriptRoot "check-release-contract.ps1"
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).Path
$shell = (Get-Process -Id $PID).Path
$fixtureRoots = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$scenarioCount = 0
$fixtureBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)

function New-ReleaseFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) (
        "wokrouter-release-contract-" + [guid]::NewGuid()
    )
    $null = New-Item -ItemType Directory -Path (Join-Path $root ".github/workflows") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "apps/desktop/src-tauri") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs/operations") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "tests/release") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "release") -Force
    foreach ($relativePath in @(
            ".github/workflows/ci.yml",
            ".github/workflows/release.yml",
            "Cargo.toml",
            "Cargo.lock",
            "apps/desktop/package.json",
            "apps/desktop/src-tauri/tauri.conf.json",
            "docs/operations/development.md",
            "tests/release/WokRouter.ReleaseContract.psm1",
            "tests/release/package-linux-assets.ps1",
            "tests/release/package-macos-assets.ps1",
            "tests/release/package-windows-assets.ps1",
            "tests/release/sign-release-bundle.ps1",
            "tests/release/verify-release-bundle.ps1",
            "release/minisign.pub"
        )) {
        Copy-Item `
            -LiteralPath (Join-Path $repositoryRoot $relativePath) `
            -Destination (Join-Path $root $relativePath)
    }
    $fixtureRoots.Add($root)
    return $root
}

function Edit-FixtureFile {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$RelativePath,
        [Parameter(Mandatory)][string]$OldText,
        [Parameter(Mandatory)][AllowEmptyString()][string]$NewText
    )

    $path = Join-Path $Root $RelativePath
    $content = (Get-Content -LiteralPath $path -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $old = $OldText.Replace("`r`n", "`n")
    $new = $NewText.Replace("`r`n", "`n")
    if (-not $content.Contains($old)) {
        throw "Fixture mutation source was not found in ${RelativePath}: $OldText"
    }
    Set-Content -LiteralPath $path -Value $content.Replace($old, $new) -Encoding UTF8
}

function Invoke-Check {
    param(
        [Parameter(Mandatory)][string]$Root
    )

    $arguments = @("-NoProfile")
    if ($PSVersionTable.PSEdition -eq "Desktop") {
        $arguments += @("-ExecutionPolicy", "Bypass")
    }
    $arguments += @("-File", $scriptUnderTest, "-Root", $Root)
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $shell @arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return @{ ExitCode = $exitCode; Output = ($output | Out-String) }
}

function Assert-Passes {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Scenario
    )

    $result = Invoke-Check -Root $Root
    if ($result.ExitCode -ne 0) {
        throw "$Scenario should pass, but exited $($result.ExitCode): $($result.Output)"
    }
}

function Assert-Rejects {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$ExpectedText,
        [Parameter(Mandatory)][string]$Scenario
    )

    $result = Invoke-Check -Root $Root
    if ($result.ExitCode -ne 1) {
        throw "$Scenario should exit 1, but exited $($result.ExitCode): $($result.Output)"
    }
    if ($result.Output -notmatch [regex]::Escape($ExpectedText)) {
        throw "$Scenario did not identify '$ExpectedText': $($result.Output)"
    }
}

function Invoke-Scenario {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][scriptblock]$Test)

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
    Invoke-Scenario -Name "real release workflow satisfies the contract" -Test {
        $root = New-ReleaseFixture
        Assert-Passes -Root $root -Scenario "real release fixture"
    }

    Invoke-Scenario -Name "all product source versions must remain identical" -Test {
        $mutations = @(
            @{
                Path = "Cargo.toml"
                Old = "[workspace.package]`nversion = `"0.1.15`""
                New = "[workspace.package]`nversion = `"0.1.0`""
            },
            @{
                Path = "apps/desktop/package.json"
                Old = '  "version": "0.1.15",'
                New = '  "version": "0.1.0",'
            },
            @{
                Path = "apps/desktop/src-tauri/tauri.conf.json"
                Old = '  "version": "0.1.15",'
                New = '  "version": "0.1.0",'
            },
            @{
                Path = "Cargo.lock"
                Old = "[[package]]`nname = `"wokrouter-cli`"`nversion = `"0.1.15`""
                New = "[[package]]`nname = `"wokrouter-cli`"`nversion = `"0.1.0`""
            }
        )
        foreach ($mutation in $mutations) {
            $root = New-ReleaseFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath $mutation.Path `
                -OldText $mutation.Old `
                -NewText $mutation.New
            Assert-Rejects `
                -Root $root `
                -ExpectedText "source versions" `
                -Scenario "mismatched $($mutation.Path) version"
        }
    }

    Invoke-Scenario -Name "release matrix must retain Windows arm64" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText @"
          - os: windows-latest
            target: aarch64-pc-windows-msvc
"@ `
            -NewText ""
        Assert-Rejects `
            -Root $root `
            -ExpectedText "aarch64-pc-windows-msvc" `
            -Scenario "missing Windows arm64 target"
    }

    Invoke-Scenario -Name "macOS arm64 must use macos-14" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText @"
          - os: macos-14
            target: aarch64-apple-darwin
"@ `
            -NewText @"
          - os: macos-15
            target: aarch64-apple-darwin
"@
        Assert-Rejects `
            -Root $root `
            -ExpectedText "macos-14" `
            -Scenario "wrong macOS arm64 runner"
    }

    Invoke-Scenario -Name "friendly asset contract module must remain complete" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath "tests/release/WokRouter.ReleaseContract.psm1" `
            -OldText '"WokRouter-v$Version-$($contract.System)-"' `
            -NewText '"WokRouter-v$Version-$($contract.Target)-"'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "exact 16 friendly payload names" `
            -Scenario "public names exposing target triples"
    }

    Invoke-Scenario -Name "all platform packagers must remain present" -Test {
        $root = New-ReleaseFixture
        Remove-Item -LiteralPath (
            Join-Path $root "tests/release/package-macos-assets.ps1"
        )
        Assert-Rejects `
            -Root $root `
            -ExpectedText "Required release contract file is missing" `
            -Scenario "missing macOS packager"
    }

    Invoke-Scenario -Name "release matrix must retain Linux arm64" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile -Root $root -RelativePath ".github/workflows/release.yml" -OldText @"
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
"@ -NewText ""
        Assert-Rejects -Root $root -ExpectedText "aarch64-unknown-linux-gnu" -Scenario "missing target"
    }

    Invoke-Scenario -Name "release version cannot couple to WokCore" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "      WOKROUTER_RELEASE_VERSION: `${{ needs.release-version.outputs.version }}" `
            -NewText "      WOKCORE_RELEASE_VERSION: 1.2.3"
        Assert-Rejects -Root $root -ExpectedText "WokCore version" -Scenario "WokCore version coupling"
    }

    Invoke-Scenario -Name "manual release verification must remain available" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "  workflow_dispatch:" `
            -NewText "  disabled_dispatch:"
        Assert-Rejects -Root $root -ExpectedText "manual verification" -Scenario "missing dispatch"
    }

    Invoke-Scenario -Name "manual verification must checkout the requested tag commit" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '          ref: ${{ needs.release-version.outputs.source_sha }}' `
            -NewText '          ref: ${{ github.sha }}'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "requested WokRouter tag" `
            -Scenario "release jobs checking out the dispatch branch"
    }

    Invoke-Scenario -Name "online artifact boundary cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "      WOKROUTER_BUNDLE_KIND: online" `
            -NewText "      WOKROUTER_BUNDLE_KIND: offline"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "missing required boundary text" `
            -Scenario "missing online boundary"
    }

    Invoke-Scenario -Name "compatibility matrix must retain older same-major coverage" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "legacy_same_major_runtime_without_installation_id_remains_running" `
            -NewText "redirects_are_not_followed"
        Assert-Rejects -Root $root -ExpectedText "legacy_same_major" -Scenario "missing compatibility case"
    }

    Invoke-Scenario -Name "compatibility matrix must retain WokCore v2 preference" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1" `
            -NewText "redirects_are_not_followed"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1" `
            -Scenario "missing WokCore v2 preference"
    }

    Invoke-Scenario -Name "provider credentials must be empty in release" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '  OPENAI_API_KEY: ""' `
            -NewText "  OPENAI_API_KEY: inherited"
        Assert-Rejects -Root $root -ExpectedText "OPENAI_API_KEY" -Scenario "provider credential inheritance"
    }

    Invoke-Scenario -Name "write permission must remain publish-only" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "permissions:`n  contents: read" `
            -NewText "permissions:`n  contents: write"
        Assert-Rejects -Root $root -ExpectedText "contents: read" -Scenario "broad write permission"
    }

    foreach ($bundleSet in @(
            "appimage,deb,rpm",
            "app,dmg",
            "msi"
        )) {
        Invoke-Scenario -Name "release build must retain --bundles $bundleSet" -Test {
            $root = New-ReleaseFixture
            Edit-FixtureFile `
                -Root $root `
                -RelativePath ".github/workflows/release.yml" `
                -OldText "--bundles $bundleSet" `
                -NewText "--bundles all"
            Assert-Rejects `
                -Root $root `
                -ExpectedText "one executable '--bundles $bundleSet' line" `
                -Scenario "missing explicit $bundleSet bundle set"
        }
    }

    Invoke-Scenario -Name "Minisign secret cannot escape the signing step" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText @'
    runs-on: ubuntu-24.04
    outputs:
'@ `
            -NewText @'
    runs-on: ubuntu-24.04
    env:
      LEAKED_KEY: ${{ secrets.WOKROUTER_MINISIGN_SECRET_KEY }}
    outputs:
'@
        Assert-Rejects `
            -Root $root `
            -ExpectedText "secret must appear only" `
            -Scenario "secret outside signing step"
    }

    Invoke-Scenario -Name "old five-archive verification cannot return" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "name: Release" `
            -NewText "name: Release`n# Expected five release archives"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "old five-archive" `
            -Scenario "legacy five-archive path"
    }

    Invoke-Scenario -Name "local signed bundle verification cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "            & tests/release/verify-release-bundle.ps1 ``" `
            -NewText "            & tests/release/not-verify-release-bundle.ps1 ``"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "locally verify" `
            -Scenario "missing local signed bundle verification"
    }

    Invoke-Scenario -Name "private key write must remain inside cleanup try" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText @"
          try {
            [IO.File]::WriteAllText(
              `$secretPath,
              `$env:WOKROUTER_MINISIGN_SECRET_KEY,
              [Text.UTF8Encoding]::new(`$false)
            )
"@ `
            -NewText @"
          [IO.File]::WriteAllText(
            `$secretPath,
            `$env:WOKROUTER_MINISIGN_SECRET_KEY,
            [Text.UTF8Encoding]::new(`$false)
          )
          try {
"@
        Assert-Rejects `
            -Root $root `
            -ExpectedText "covered by secure finally cleanup" `
            -Scenario "private key write before cleanup try"
    }

    Invoke-Scenario -Name "exact unsigned inventory cannot change" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '$items.Count -ne 16' `
            -NewText '$items.Count -ne 15'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "require 16 payloads" `
            -Scenario "wrong unsigned payload count"
    }

    Invoke-Scenario -Name "draft guard cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText ".isDraft" `
            -NewText ".isPublished"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "guard a draft" `
            -Scenario "missing draft guard"
    }

    Invoke-Scenario -Name "stale draft asset cleanup cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText 'gh release delete-asset "$RELEASE_TAG"' `
            -NewText 'echo keep-stale-asset "$RELEASE_TAG"'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "tag-only" `
            -Scenario "missing draft asset cleanup"
    }

    Invoke-Scenario -Name "draft must be rechecked before upload" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "The WokRouter draft became public before upload." `
            -NewText "Upload without rechecking the draft."
        Assert-Rejects `
            -Root $root `
            -ExpectedText "guard a draft" `
            -Scenario "missing pre-upload draft recheck"
    }

    Invoke-Scenario -Name "remote draft download cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText 'gh release download "$RELEASE_TAG"' `
            -NewText 'echo skip-download "$RELEASE_TAG"'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "tag-only" `
            -Scenario "missing remote draft download"
    }

    Invoke-Scenario -Name "remote draft verification cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "          pwsh tests/release/verify-release-bundle.ps1 \" `
            -NewText "          pwsh tests/release/not-verify-release-bundle.ps1 \"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "upload, re-download, verify" `
            -Scenario "missing remote draft verification"
    }

    Invoke-Scenario -Name "remote tag identity must guard the first release mutation" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "              require_remote_tag_commit" `
            -NewText "              # require_remote_tag_commit"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "tag-only" `
            -Scenario "comment replacing the pre-mutation tag identity guard"
    }

    Invoke-Scenario -Name "remote tag identity must be rechecked before publication" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "          require_remote_tag_commit`n          gh release edit" `
            -NewText "          # require_remote_tag_commit`n          gh release edit"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "tag-only" `
            -Scenario "comment replacing the pre-publication tag identity guard"
    }

    Invoke-Scenario -Name "draft publication cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "--draft=false" `
            -NewText "--draft=true`n          # --draft=false"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "tag-only" `
            -Scenario "missing draft publication"
    }

    Invoke-Scenario -Name "exact signed inventory cannot change" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '"${#local_assets[@]}" -ne 35' `
            -NewText '"${#local_assets[@]}" -ne 34'
        Assert-Rejects `
            -Root $root `
            -ExpectedText "35-file bundle" `
            -Scenario "wrong signed asset count"
    }

    Invoke-Scenario -Name "release concurrency cannot be removed" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText "  cancel-in-progress: false" `
            -NewText "  cancel-in-progress: true"
        Assert-Rejects `
            -Root $root `
            -ExpectedText "without cancellation" `
            -Scenario "release transaction cancellation"
    }

    Invoke-Scenario -Name "publish must name the repository" -Test {
        $root = New-ReleaseFixture
        Edit-FixtureFile `
            -Root $root `
            -RelativePath ".github/workflows/release.yml" `
            -OldText '--repo "$GITHUB_REPOSITORY" ' `
            -NewText ""
        Assert-Rejects `
            -Root $root `
            -ExpectedText "explicit GitHub repository" `
            -Scenario "publish without an explicit repository"
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "RELEASE CONTRACT SELF-TEST ERROR: $failure"
        }
        Write-Host "Release contract self-tests failed: $($failures.Count) of $scenarioCount scenario(s)."
        exit 1
    }

    Write-Host "Release contract self-tests passed: $scenarioCount scenario(s)."
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
                "wokrouter-release-contract-",
                [System.StringComparison]::Ordinal
            )
        ) {
            throw "Refusing to remove unexpected release fixture path '$resolvedRoot'."
        }
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
