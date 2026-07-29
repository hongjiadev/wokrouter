[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $ArtifactDirectory,
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $PublicKeyPath,
    [Parameter(Mandatory)][string] $MinisignPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1"
$maximumArtifactBytes = 536870912
$maximumSignatureBytes = 4096
$maximumPublicKeyBytes = 1024
$maximumChecksumsBytes = 8192
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
                throw "Release verification paths must not contain reparse points."
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
        [Parameter(Mandatory)][string[]] $ExpectedNames
    )

    $items = @(Get-ChildItem -LiteralPath $Directory -Force)
    foreach ($item in $items) {
        if (
            $item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        ) {
            throw "Release bundle contains a directory or reparse point: $($item.Name)"
        }
    }
    $caseFolded = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($item in $items) {
        if (-not $caseFolded.Add($item.Name)) {
            throw "Release bundle contains case-insensitive duplicate names."
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
            throw "Release bundle inventory has incorrect case."
        }
        throw "Release bundle inventory is not exact."
    }
}

function Assert-CanonicalPublicKey {
    param([Parameter(Mandatory)][string] $Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    $encoding = [Text.UTF8Encoding]::new($false, $true)
    try {
        $text = $encoding.GetString($bytes)
    }
    catch {
        throw "Trusted Minisign public key must be valid UTF-8."
    }
    if (
        $text.Contains("`r") -or
        -not $text.EndsWith("`n") -or
        $text.EndsWith("`n`n") -or
        $text.TrimEnd("`n").Split("`n").Count -ne 2 -or
        $text[0] -eq [char] 0xfeff
    ) {
        throw "Trusted Minisign public key is not normalized."
    }
    $lines = $text.TrimEnd("`n").Split("`n")
    if (
        $lines[0] -cnotmatch (
            "^untrusted comment: minisign public key (?<id>[0-9A-F]{16})$"
        )
    ) {
        throw "Trusted Minisign public key comment is malformed."
    }
    $commentId = $Matches.id
    try {
        $keyBytes = [Convert]::FromBase64String($lines[1])
    }
    catch {
        throw "Trusted Minisign public key payload is malformed."
    }
    if (
        $keyBytes.Length -ne 42 -or
        $keyBytes[0] -ne [byte][char]"E" -or
        $keyBytes[1] -ne [byte][char]"d" -or
        [BitConverter]::ToUInt64($keyBytes, 2).ToString("X16") -cne $commentId
    ) {
        throw "Trusted Minisign public key id or format is invalid."
    }
}

function Assert-MinisignSignatureText {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $UntrustedComment,
        [Parameter(Mandatory)][string] $TrustedComment
    )

    Assert-BoundedFile `
        -Path $Path `
        -MaximumBytes $maximumSignatureBytes `
        -Description "Release signature"
    $bytes = [IO.File]::ReadAllBytes($Path)
    if (
        $bytes.Length -ge 3 -and
        $bytes[0] -eq 0xef -and
        $bytes[1] -eq 0xbb -and
        $bytes[2] -eq 0xbf
    ) {
        throw "Release signature must not contain a UTF-8 BOM."
    }
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    }
    catch {
        throw "Release signature must be valid UTF-8."
    }
    $lines = @($text.Replace("`r`n", "`n").Split("`n"))
    if (
        $lines.Count -ne 5 -or
        $lines[0] -cne "untrusted comment: $UntrustedComment" -or
        $lines[2] -cne "trusted comment: $TrustedComment" -or
        $lines[4] -cne ""
    ) {
        throw "Release signature has an invalid shape or comment."
    }
    try {
        $messageSignature = [Convert]::FromBase64String($lines[1])
        $trustedCommentSignature = [Convert]::FromBase64String($lines[3])
    }
    catch {
        throw "Release signature contains invalid base64."
    }
    if (
        $messageSignature.Length -ne 74 -or
        $messageSignature[0] -ne 0x45 -or
        $messageSignature[1] -ne 0x44 -or
        $trustedCommentSignature.Length -ne 64
    ) {
        throw "Release signature payload has an invalid shape."
    }
}

function Invoke-MinisignVerify {
    param(
        [Parameter(Mandatory)][string] $MessagePath,
        [Parameter(Mandatory)][string] $SignaturePath
    )

    & $minisign `
        -V `
        -p $trustedPublicKey `
        -m $MessagePath `
        -x $SignaturePath `
        -q 2>&1 |
        Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Invalid Minisign signature for '$([IO.Path]::GetFileName($MessagePath))'."
    }
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
    $trustedPublicKey = Get-RegularFilePath `
        -Path $PublicKeyPath `
        -Description "Trusted Minisign public key"
    $minisign = Get-RegularFilePath `
        -Path $MinisignPath `
        -Description "Minisign executable"
    Assert-BoundedFile `
        -Path $trustedPublicKey `
        -MaximumBytes $maximumPublicKeyBytes `
        -Description "Trusted Minisign public key"
    Assert-OutsideArtifactDirectory `
        -Path $trustedPublicKey `
        -Description "Trusted Minisign public key" `
        -ArtifactRoot $artifactRoot
    Assert-CanonicalPublicKey -Path $trustedPublicKey

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
        -ExpectedNames $expected.ToArray()

    $bundledPublicKey = Get-RegularFilePath `
        -Path (Join-Path $artifactRoot "WokRouter-Minisign.pub") `
        -Description "Bundled Minisign public key"
    Assert-BoundedFile `
        -Path $bundledPublicKey `
        -MaximumBytes $maximumPublicKeyBytes `
        -Description "Bundled Minisign public key"
    $trustedBytes = [IO.File]::ReadAllBytes($trustedPublicKey)
    $bundledBytes = [IO.File]::ReadAllBytes($bundledPublicKey)
    $publicKeyMatches = $trustedBytes.Length -eq $bundledBytes.Length
    if ($publicKeyMatches) {
        [int] $difference = 0
        for ($index = 0; $index -lt $trustedBytes.Length; $index++) {
            $difference = $difference -bor (
                $trustedBytes[$index] -bxor $bundledBytes[$index]
            )
        }
        $publicKeyMatches = $difference -eq 0
    }
    if (-not $publicKeyMatches) {
        throw "Bundled public key does not match the external trusted public key."
    }

    $checksumPath = Get-RegularFilePath `
        -Path (Join-Path $artifactRoot "SHA256SUMS") `
        -Description "SHA256SUMS"
    Assert-BoundedFile `
        -Path $checksumPath `
        -MaximumBytes $maximumChecksumsBytes `
        -Description "SHA256SUMS"
    $checksumBytes = [IO.File]::ReadAllBytes($checksumPath)
    if (
        $checksumBytes.Length -eq 0 -or
        $checksumBytes[$checksumBytes.Length - 1] -eq 10 -or
        (
            $checksumBytes.Length -ge 3 -and
            $checksumBytes[0] -eq 0xef -and
            $checksumBytes[1] -eq 0xbb -and
            $checksumBytes[2] -eq 0xbf
        )
    ) {
        throw "SHA256SUMS has invalid UTF-8 or newline framing."
    }
    try {
        $checksumText = [Text.UTF8Encoding]::new(
            $false,
            $true
        ).GetString($checksumBytes)
    }
    catch {
        throw "SHA256SUMS must be valid UTF-8."
    }
    if ($checksumText.Contains("`r")) {
        throw "SHA256SUMS must use LF-only newlines."
    }
    $checksumLines = $checksumText.Split("`n")
    if ($checksumLines.Count -ne $payloads.Count) {
        throw "SHA256SUMS must contain exactly 16 checksum members."
    }
    $members = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    $caseFoldedMembers = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    for ($index = 0; $index -lt $payloads.Count; $index++) {
        $line = $checksumLines[$index]
        if ($line -cnotmatch "^(?<hash>[0-9a-f]{64})  (?<name>.+)$") {
            throw "SHA256SUMS contains a malformed checksum line."
        }
        $member = $Matches.name
        if (-not $members.Add($member)) {
            throw "SHA256SUMS contains a duplicate checksum member."
        }
        if (-not $caseFoldedMembers.Add($member)) {
            throw "SHA256SUMS contains a case-insensitive duplicate checksum member."
        }
        if ($member -cne $payloads[$index]) {
            throw "SHA256SUMS checksum members are not in exact ordinal order."
        }
        $payloadPath = Get-RegularFilePath `
            -Path (Join-Path $artifactRoot $member) `
            -Description "WokRouter payload '$member'"
        Assert-BoundedFile `
            -Path $payloadPath `
            -MaximumBytes $maximumArtifactBytes `
            -Description "WokRouter payload '$member'"
        $actualHash = (
            Get-FileHash `
                -LiteralPath $payloadPath `
                -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        if ($actualHash -cne $Matches.hash) {
            throw "SHA256 checksum mismatch for '$member'."
        }
    }

    foreach ($name in $payloads) {
        $signaturePath = Join-Path $artifactRoot "$name.minisig"
        Assert-MinisignSignatureText `
            -Path $signaturePath `
            -UntrustedComment "WokRouter release asset" `
            -TrustedComment "WokRouter v$Version"
        Invoke-MinisignVerify `
            -MessagePath (Join-Path $artifactRoot $name) `
            -SignaturePath $signaturePath
    }
    $checksumSignature = Join-Path $artifactRoot "SHA256SUMS.minisig"
    Assert-MinisignSignatureText `
        -Path $checksumSignature `
        -UntrustedComment "WokRouter checksums" `
        -TrustedComment "WokRouter v$Version"
    Invoke-MinisignVerify `
        -MessagePath $checksumPath `
        -SignaturePath $checksumSignature
    Write-Output "WokRouter release bundle verification passed"
}
finally {
    Remove-Module WokRouter.ReleaseContract -ErrorAction SilentlyContinue
}
