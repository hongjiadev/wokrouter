[CmdletBinding()]
param(
    [string] $MinisignPath = $env:WOKROUTER_MINISIGN_PATH
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1"
$normalizer = Join-Path $PSScriptRoot "normalize-minisign-public-key.ps1"
$signer = Join-Path $PSScriptRoot "sign-release-bundle.ps1"
$verifier = Join-Path $PSScriptRoot "verify-release-bundle.ps1"
$version = "1.2.3"
$fixtureRoots = [Collections.Generic.List[string]]::new()
$scenarioCount = 0
$failures = [Collections.Generic.List[string]]::new()
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
    [IO.Path]::DirectorySeparatorChar,
    [IO.Path]::AltDirectorySeparatorChar
)

function New-FixtureRoot {
    $root = Join-Path $temporaryRoot (
        "wokrouter-release-bundle-" + [Guid]::NewGuid().ToString("N")
    )
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $script:fixtureRoots.Add($root)
    return $root
}

function Copy-FixtureBundle {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Root
    )

    $destination = Join-Path $Root "bundle"
    [IO.Directory]::CreateDirectory($destination) | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $destination -Force
    return $destination
}

function Invoke-Scenario {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][scriptblock] $Test
    )

    $script:scenarioCount++
    try {
        & $Test
        Write-Host "PASS: $Name"
    }
    catch {
        $script:failures.Add("${Name}: $($_.Exception.Message)")
    }
}

function Assert-Rejects {
    param(
        [Parameter(Mandatory)][scriptblock] $Action,
        [Parameter(Mandatory)][string] $ExpectedText
    )

    try {
        & $Action
    }
    catch {
        if (
            $_.Exception.Message.IndexOf(
                $ExpectedText,
                [StringComparison]::OrdinalIgnoreCase
            ) -lt 0
        ) {
            throw (
                "Expected rejection containing '$ExpectedText', got: " +
                $_.Exception.Message
            )
        }
        return
    }
    throw "Expected rejection containing '$ExpectedText'."
}

function New-EphemeralKeyPair {
    param([Parameter(Mandatory)][string] $Root)

    $public = Join-Path $Root "fixture-minisign.pub"
    $secret = Join-Path $Root "fixture-minisign.key"
    & $MinisignPath -G -W -f -p $public -s $secret | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to generate an ephemeral Minisign key."
    }
    $rawPublic = "$public.raw"
    Move-Item -LiteralPath $public -Destination $rawPublic
    & $normalizer -InputPath $rawPublic -OutputPath $public | Out-Null
    Remove-Item -LiteralPath $rawPublic
    return [pscustomobject]@{
        Public = $public
        Secret = $secret
    }
}

function Invoke-Verify {
    param(
        [Parameter(Mandatory)][string] $Bundle,
        [Parameter(Mandatory)][string] $PublicKey
    )

    & $verifier `
        -ArtifactDirectory $Bundle `
        -Version $version `
        -PublicKeyPath $PublicKey `
        -MinisignPath $MinisignPath | Out-Null
}

if ([string]::IsNullOrWhiteSpace($MinisignPath)) {
    $command = Get-Command minisign -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        $MinisignPath = $command.Source
    }
}
if (
    [string]::IsNullOrWhiteSpace($MinisignPath) -or
    -not (Test-Path -LiteralPath $MinisignPath -PathType Leaf)
) {
    throw (
        "Set WOKROUTER_MINISIGN_PATH or install minisign before running " +
        "release bundle tests."
    )
}
foreach ($required in @($modulePath, $normalizer, $signer, $verifier)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Missing release signing component: $required"
    }
}

Import-Module $modulePath -Force
$baseRoot = New-FixtureRoot
$keys = New-EphemeralKeyPair -Root $baseRoot
$bundle = Join-Path $baseRoot "bundle"
[IO.Directory]::CreateDirectory($bundle) | Out-Null
$payloads = @(Get-WokRouterPayloadNames -Version $version)
foreach ($name in $payloads) {
    [IO.File]::WriteAllText(
        (Join-Path $bundle $name),
        "fixture:$name",
        [Text.UTF8Encoding]::new($false)
    )
}
& $signer `
    -ArtifactDirectory $bundle `
    -Version $version `
    -SecretKeyPath $keys.Secret `
    -PublicKeyPath $keys.Public `
    -MinisignPath $MinisignPath | Out-Null

try {
    Invoke-Scenario -Name "signs and verifies the exact 35-file bundle" -Test {
        Invoke-Verify -Bundle $bundle -PublicKey $keys.Public
        $items = @(Get-ChildItem -LiteralPath $bundle -Force)
        if ($items.Count -ne 35) {
            throw "Expected exactly 35 release bundle entries."
        }
        if (@($items | Where-Object { -not $_.PSIsContainer }).Count -ne 35) {
            throw "Every release bundle entry must be a file."
        }
        $checksumBytes = [IO.File]::ReadAllBytes(
            (Join-Path $bundle "SHA256SUMS")
        )
        if (
            $checksumBytes.Length -ge 3 -and
            $checksumBytes[0] -eq 0xef -and
            $checksumBytes[1] -eq 0xbb -and
            $checksumBytes[2] -eq 0xbf
        ) {
            throw "SHA256SUMS contains a UTF-8 BOM."
        }
        if ($checksumBytes[-1] -eq 10) {
            throw "SHA256SUMS has a trailing newline."
        }
        $checksumText = [Text.UTF8Encoding]::new(
            $false,
            $true
        ).GetString($checksumBytes)
        if ($checksumText.Contains("`r")) {
            throw "SHA256SUMS does not use LF-only newlines."
        }
    }

    Invoke-Scenario -Name "rejects a tampered payload" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        [IO.File]::AppendAllText(
            (Join-Path $copy $payloads[0]),
            "tampered",
            [Text.UTF8Encoding]::new($false)
        )
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "checksum"
    }

    Invoke-Scenario -Name "rejects a removed signature" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        Remove-Item -LiteralPath (Join-Path $copy "$($payloads[0]).minisig")
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "inventory"
    }

    Invoke-Scenario -Name "rejects duplicate-case or case-variant names" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $original = Join-Path $copy "$($payloads[0]).minisig"
        $caseVariant = Join-Path $copy "$($payloads[0].ToLowerInvariant()).minisig"
        $expected = "duplicate"
        try {
            [IO.File]::Copy($original, $caseVariant, $false)
        }
        catch [IO.IOException] {
            $temporary = Join-Path $copy "case-rename.tmp"
            Move-Item -LiteralPath $original -Destination $temporary
            Move-Item -LiteralPath $temporary -Destination $caseVariant
            $expected = "case"
        }
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText $expected
    }

    Invoke-Scenario -Name "rejects an extra file" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        [IO.File]::WriteAllText((Join-Path $copy "extra.txt"), "extra")
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "inventory"
    }

    Invoke-Scenario -Name "rejects a reparse point" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $target = Join-Path $root "junction-target"
        [IO.Directory]::CreateDirectory($target) | Out-Null
        $null = New-Item `
            -ItemType Junction `
            -Path (Join-Path $copy "bundle-link") `
            -Target $target
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "reparse"
    }

    Invoke-Scenario -Name "rejects the wrong external trust anchor" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $wrong = New-EphemeralKeyPair -Root $root
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $wrong.Public } `
            -ExpectedText "public key"
    }

    Invoke-Scenario -Name "rejects a malformed checksum member" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $checksum = Join-Path $copy "SHA256SUMS"
        $text = [IO.File]::ReadAllText($checksum)
        [IO.File]::WriteAllText(
            $checksum,
            $text.Replace("  ", " "),
            [Text.UTF8Encoding]::new($false)
        )
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "checksum"
    }

    Invoke-Scenario -Name "rejects a duplicate checksum member" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $checksum = Join-Path $copy "SHA256SUMS"
        $lines = [IO.File]::ReadAllText($checksum).Split("`n")
        $lines[1] = $lines[0]
        [IO.File]::WriteAllText(
            $checksum,
            [string]::Join("`n", $lines),
            [Text.UTF8Encoding]::new($false)
        )
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "duplicate"
    }

    Invoke-Scenario -Name "rejects an invalid Minisign signature" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        $signature = Join-Path $copy "$($payloads[0]).minisig"
        $bytes = [IO.File]::ReadAllBytes($signature)
        $bytes[$bytes.Length - 2] = $bytes[$bytes.Length - 2] -bxor 1
        [IO.File]::WriteAllBytes($signature, $bytes)
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "signature"
    }

    Invoke-Scenario -Name "rejects a changed bundled public key" -Test {
        $root = New-FixtureRoot
        $copy = Copy-FixtureBundle -Source $bundle -Root $root
        [IO.File]::AppendAllText(
            (Join-Path $copy "WokRouter-Minisign.pub"),
            "changed",
            [Text.UTF8Encoding]::new($false)
        )
        Assert-Rejects `
            -Action { Invoke-Verify -Bundle $copy -PublicKey $keys.Public } `
            -ExpectedText "public key"
    }

    Invoke-Scenario -Name "rejects an unsupported release version" -Test {
        foreach ($invalidVersion in @("01.2.3", "1.2.3-01")) {
            Assert-Rejects -Action {
                & $verifier `
                    -ArtifactDirectory $bundle `
                    -Version $invalidVersion `
                    -PublicKeyPath $keys.Public `
                    -MinisignPath $MinisignPath
            } -ExpectedText "SemVer"
        }
    }

    Invoke-Scenario -Name "normalizes public keys to strict LF UTF-8" -Test {
        $root = New-FixtureRoot
        $input = Join-Path $root "input.pub"
        $output = Join-Path $root "output.pub"
        $text = [IO.File]::ReadAllText($keys.Public).
            Replace("`r`n", "`n").
            Replace("`r", "`n").
            Replace("`n", "`r`n")
        [IO.File]::WriteAllText(
            $input,
            [char] 0xfeff + $text,
            [Text.UTF8Encoding]::new($false)
        )
        & $normalizer -InputPath $input -OutputPath $output | Out-Null
        $normalized = [IO.File]::ReadAllBytes($output)
        if (
            $normalized[0] -eq 0xef -and
            $normalized[1] -eq 0xbb -and
            $normalized[2] -eq 0xbf
        ) {
            throw "Normalized key contains a BOM."
        }
        $normalizedText = [Text.Encoding]::UTF8.GetString($normalized)
        if ($normalizedText.Contains("`r") -or -not $normalizedText.EndsWith("`n")) {
            throw "Normalized key does not have canonical LF framing."
        }
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "RELEASE BUNDLE TEST ERROR: $failure"
        }
        throw "Release bundle tests failed: $($failures.Count) of $scenarioCount."
    }
    Write-Host "Release bundle tests passed: $scenarioCount scenario(s)."
}
finally {
    Remove-Module WokRouter.ReleaseContract -ErrorAction SilentlyContinue
    foreach ($root in $fixtureRoots) {
        if (-not [IO.Directory]::Exists($root)) {
            continue
        }
        $full = [IO.Path]::GetFullPath($root)
        $parent = [IO.Directory]::GetParent($full).FullName.TrimEnd(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        )
        $leaf = [IO.Path]::GetFileName($full)
        if (
            -not $parent.Equals(
                $temporaryRoot,
                [StringComparison]::OrdinalIgnoreCase
            ) -or
            $leaf -cnotmatch "^wokrouter-release-bundle-[0-9a-f]{32}$"
        ) {
            throw "Refusing to remove unexpected fixture root '$full'."
        }
        foreach ($reparse in @(
                Get-ChildItem -LiteralPath $full -Force -Recurse |
                    Where-Object {
                        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
                    }
            )) {
            if ($reparse.PSIsContainer) {
                [IO.Directory]::Delete($reparse.FullName, $false)
            } else {
                [IO.File]::Delete($reparse.FullName)
            }
        }
        [IO.Directory]::Delete($full, $true)
    }
}
