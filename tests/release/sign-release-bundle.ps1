[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $ArtifactDirectory,
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $SecretKeyPath,
    [Parameter(Mandatory)][string] $PublicKeyPath,
    [Parameter(Mandatory)][string] $MinisignPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1"
$normalizerPath = Join-Path $PSScriptRoot "normalize-minisign-public-key.ps1"
$maximumArtifactBytes = 536870912
$maximumPublicKeyBytes = 1024
$maximumSecretKeyBytes = 4096
$semverPattern = "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\." +
    "(0|[1-9][0-9]*)(?:-(?:0|[1-9][0-9]*|" +
    "[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|" +
    "[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?" +
    "(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"

function Assert-NoReparsePath {
    param([Parameter(Mandatory)][string] $Path)

    $current = [IO.Path]::GetFullPath($Path)
    while ($null -ne $current) {
        if (
            [IO.File]::Exists($current) -or
            [IO.Directory]::Exists($current)
        ) {
            $item = Get-Item -LiteralPath $current -Force
            if (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                0
            ) {
                throw "Release signing paths must not contain reparse points."
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($null -eq $parent) { $null } else { $parent.FullName }
    }
}

function Get-RegularFilePath {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description is missing: $Path"
    }
    Assert-NoReparsePath -Path $Path
    $item = Get-Item -LiteralPath $Path -Force
    if (
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw "$Description must not be a reparse point: $Path"
    }
    return [IO.Path]::GetFullPath($item.FullName)
}

function Get-RegularDirectoryPath {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "$Description is missing: $Path"
    }
    Assert-NoReparsePath -Path $Path
    $item = Get-Item -LiteralPath $Path -Force
    if (
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw "$Description must not be a reparse point: $Path"
    }
    return [IO.Path]::GetFullPath($item.FullName).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
}

function Assert-BoundedFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][long] $MaximumBytes,
        [Parameter(Mandatory)][string] $Description
    )

    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Length -le 0 -or $item.Length -gt $MaximumBytes) {
        throw "$Description is empty or oversized."
    }
}

function Assert-OutsideArtifactDirectory {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Description,
        [Parameter(Mandatory)][string] $ArtifactRoot
    )

    $prefix = $ArtifactRoot + [IO.Path]::DirectorySeparatorChar
    if ($Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Description must remain outside the release bundle."
    }
}

function Assert-ExactInventory {
    param(
        [Parameter(Mandatory)][string] $Directory,
        [Parameter(Mandatory)][string[]] $ExpectedNames,
        [Parameter(Mandatory)][string] $Description
    )

    $items = @(Get-ChildItem -LiteralPath $Directory -Force)
    foreach ($item in $items) {
        if (
            $item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        ) {
            throw "$Description contains a directory or reparse point: $($item.Name)"
        }
    }
    $caseFolded = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($item in $items) {
        if (-not $caseFolded.Add($item.Name)) {
            throw "$Description contains case-insensitive duplicate names."
        }
    }
    [string[]] $actual = @($items | ForEach-Object Name)
    [Array]::Sort($actual, [StringComparer]::Ordinal)
    [string[]] $expected = @($ExpectedNames)
    [Array]::Sort($expected, [StringComparer]::Ordinal)
    if (
        [string]::Join("`n", $actual) -cne
        [string]::Join("`n", $expected)
    ) {
        $caseActual = @($actual | ForEach-Object { $_.ToLowerInvariant() })
        $caseExpected = @($expected | ForEach-Object { $_.ToLowerInvariant() })
        [Array]::Sort($caseActual, [StringComparer]::Ordinal)
        [Array]::Sort($caseExpected, [StringComparer]::Ordinal)
        if (
            [string]::Join("`n", $caseActual) -ceq
            [string]::Join("`n", $caseExpected)
        ) {
            throw "$Description has incorrect case."
        }
        throw "$Description inventory is not exact."
    }
}

function Invoke-MinisignSign {
    param(
        [Parameter(Mandatory)][string] $MessagePath,
        [Parameter(Mandatory)][string] $SignaturePath,
        [Parameter(Mandatory)][string] $TrustedComment,
        [Parameter(Mandatory)][string] $UntrustedComment
    )

    & $minisign `
        -S `
        -W `
        -s $secret `
        -m $MessagePath `
        -x $SignaturePath `
        -c $UntrustedComment `
        -t $TrustedComment `
        -q
    if ($LASTEXITCODE -ne 0) {
        throw "Minisign signing failed for '$([IO.Path]::GetFileName($MessagePath))'."
    }
    $null = Get-RegularFilePath `
        -Path $SignaturePath `
        -Description "Generated Minisign signature"
}

Import-Module $modulePath -Force
try {
    if ($Version.Length -gt 128 -or $Version -cnotmatch $semverPattern) {
        throw "WokRouter release version must be canonical SemVer."
    }
    $payloads = @(Get-WokRouterPayloadNames -Version $Version)
    if ($payloads.Count -ne 16) {
        throw "WokRouter release contract must contain exactly 16 payloads."
    }
    $artifactRoot = Get-RegularDirectoryPath `
        -Path $ArtifactDirectory `
        -Description "Artifact directory"
    $secret = Get-RegularFilePath `
        -Path $SecretKeyPath `
        -Description "Minisign secret key"
    $public = Get-RegularFilePath `
        -Path $PublicKeyPath `
        -Description "Minisign public key"
    $minisign = Get-RegularFilePath `
        -Path $MinisignPath `
        -Description "Minisign executable"
    Assert-BoundedFile `
        -Path $secret `
        -MaximumBytes $maximumSecretKeyBytes `
        -Description "Minisign secret key"
    Assert-BoundedFile `
        -Path $public `
        -MaximumBytes $maximumPublicKeyBytes `
        -Description "Minisign public key"
    Assert-OutsideArtifactDirectory `
        -Path $secret `
        -Description "Minisign secret key" `
        -ArtifactRoot $artifactRoot
    Assert-OutsideArtifactDirectory `
        -Path $public `
        -Description "Minisign public key" `
        -ArtifactRoot $artifactRoot

    Assert-ExactInventory `
        -Directory $artifactRoot `
        -ExpectedNames $payloads `
        -Description "Unsigned WokRouter release bundle"

    foreach ($name in $payloads) {
        $payload = Get-RegularFilePath `
            -Path (Join-Path $artifactRoot $name) `
            -Description "WokRouter payload '$name'"
        Assert-BoundedFile `
            -Path $payload `
            -MaximumBytes $maximumArtifactBytes `
            -Description "WokRouter payload '$name'"
        Invoke-MinisignSign `
            -MessagePath $payload `
            -SignaturePath (Join-Path $artifactRoot "$name.minisig") `
            -UntrustedComment "WokRouter release asset" `
            -TrustedComment "WokRouter v$Version"
    }

    $checksumLines = [Collections.Generic.List[string]]::new()
    foreach ($name in $payloads) {
        $hash = (
            Get-FileHash `
                -LiteralPath (Join-Path $artifactRoot $name) `
                -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        $checksumLines.Add("$hash  $name")
    }
    $checksumPath = Join-Path $artifactRoot "SHA256SUMS"
    [IO.File]::WriteAllBytes(
        $checksumPath,
        [Text.UTF8Encoding]::new($false).GetBytes(
            [string]::Join("`n", $checksumLines)
        )
    )
    Invoke-MinisignSign `
        -MessagePath $checksumPath `
        -SignaturePath "$checksumPath.minisig" `
        -UntrustedComment "WokRouter checksums" `
        -TrustedComment "WokRouter v$Version"

    $bundlePublicKey = Join-Path $artifactRoot "WokRouter-Minisign.pub"
    & $normalizerPath -InputPath $public -OutputPath $bundlePublicKey | Out-Null

    $expected = [Collections.Generic.List[string]]::new()
    foreach ($name in $payloads) {
        $expected.Add($name)
        $expected.Add("$name.minisig")
    }
    $expected.Add("SHA256SUMS")
    $expected.Add("SHA256SUMS.minisig")
    $expected.Add("WokRouter-Minisign.pub")
    Assert-ExactInventory `
        -Directory $artifactRoot `
        -ExpectedNames $expected.ToArray() `
        -Description "Signed WokRouter release bundle"
    Write-Output $expected.ToArray()
}
finally {
    Remove-Module WokRouter.ReleaseContract -ErrorAction SilentlyContinue
}
