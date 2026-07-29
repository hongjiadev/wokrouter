[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $InputPath,
    [Parameter(Mandatory)][string] $OutputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

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
                throw "Public-key paths must not contain reparse points."
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

$source = Get-RegularFilePath `
    -Path $InputPath `
    -Description "Minisign public key"
$destination = [IO.Path]::GetFullPath($OutputPath)
$destinationParent = Split-Path -Parent $destination
Assert-NoReparsePath -Path $destination
if (-not (Test-Path -LiteralPath $destinationParent -PathType Container)) {
    throw "Public-key output directory is missing: $destinationParent"
}
$parentItem = Get-Item -LiteralPath $destinationParent -Force
if (
    ($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
) {
    throw "Public-key output directory must not be a reparse point."
}
if (Test-Path -LiteralPath $destination) {
    $existing = Get-Item -LiteralPath $destination -Force
    if (
        $existing.PSIsContainer -or
        ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    ) {
        throw "Public-key output must be a regular file path."
    }
}

$bytes = [IO.File]::ReadAllBytes($source)
if ($bytes.Length -eq 0 -or $bytes.Length -gt 1024) {
    throw "Minisign public key is empty or oversized."
}
$encoding = [Text.UTF8Encoding]::new($false, $true)
try {
    $text = $encoding.GetString($bytes)
}
catch {
    throw "Minisign public key must be valid UTF-8."
}
if ($text.Length -gt 0 -and $text[0] -eq [char] 0xfeff) {
    $text = $text.Substring(1)
}
$text = $text.Replace("`r`n", "`n").Replace("`r", "`n").TrimEnd("`n")
$lines = $text.Split("`n")
if ($lines.Count -ne 2) {
    throw "Minisign public key must contain exactly two lines."
}
if (
    $lines[0] -cnotmatch (
        "^untrusted comment: minisign public key (?<id>[0-9A-F]{1,16})$"
    )
) {
    throw "Minisign public key comment is malformed."
}
$commentId = $Matches.id
try {
    $keyBytes = [Convert]::FromBase64String($lines[1])
}
catch {
    throw "Minisign public key payload is not valid base64."
}
if (
    $keyBytes.Length -ne 42 -or
    $keyBytes[0] -ne [byte][char]"E" -or
    $keyBytes[1] -ne [byte][char]"d"
) {
    throw "Minisign public key payload has an unsupported format."
}
$payloadId = [BitConverter]::ToUInt64($keyBytes, 2).ToString("X16")
$prefixLength = $payloadId.Length - $commentId.Length
if (
    $prefixLength -lt 0 -or
    -not $payloadId.EndsWith(
        $commentId,
        [StringComparison]::Ordinal
    ) -or
    $payloadId.Substring(0, $prefixLength) -cnotmatch "^0*$"
) {
    throw "Minisign public key id does not match its payload."
}

$normalized = (
    "untrusted comment: minisign public key $payloadId`n" +
    "$($lines[1])`n"
)
[IO.File]::WriteAllBytes(
    $destination,
    [Text.UTF8Encoding]::new($false).GetBytes($normalized)
)
Write-Output $payloadId
