[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $BundleDirectory,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $Target,
    [string] $ToolAdapterPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1") -Force

function Assert-RegularPath {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][ValidateSet("File", "Directory")]
        [string] $Kind,
        [Parameter(Mandatory)][string] $Description
    )

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Description must not be a reparse point."
    }
    if (($Kind -ceq "Directory") -ne $item.PSIsContainer) {
        throw "$Description must be a regular $($Kind.ToLowerInvariant())."
    }
    return $item
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory)][string] $Command,
        [Parameter(Mandatory)][string[]] $Arguments,
        [Parameter(Mandatory)][string] $Failure
    )

    $result = & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw $Failure
    }
    return $result
}

function Get-NativeLinkTarget {
    param([Parameter(Mandatory)] $Item)

    $targets = @($Item.Target)
    if ($Item.LinkType -cne "SymbolicLink" -or $targets.Count -ne 1) {
        throw "macOS inventory contains an unsupported reparse point."
    }
    $target = [string] $targets[0]
    if ([string]::IsNullOrWhiteSpace($target)) {
        throw "macOS inventory contains a symlink without a target."
    }
    return $target
}

function Add-NativeAppInventory {
    param(
        [Parameter(Mandatory)][string] $Directory,
        [Parameter(Mandatory)][string] $Prefix,
        [Parameter(Mandatory)]
        [Collections.Generic.List[object]] $Records
    )

    foreach ($item in Get-ChildItem -LiteralPath $Directory -Force) {
        $relative = if ([string]::IsNullOrEmpty($Prefix)) {
            $item.Name
        } else {
            "$Prefix/$($item.Name)"
        }
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            $Records.Add([pscustomobject]@{
                Kind = "Link"
                Relative = $relative
                Target = (Get-NativeLinkTarget -Item $item)
                Sha256 = $null
            })
        } elseif ($item.PSIsContainer) {
            $Records.Add([pscustomobject]@{
                Kind = "Directory"
                Relative = $relative
                Target = $null
                Sha256 = $null
            })
            Add-NativeAppInventory `
                -Directory $item.FullName `
                -Prefix $relative `
                -Records $Records
        } else {
            $Records.Add([pscustomobject]@{
                Kind = "File"
                Relative = $relative
                Target = $null
                Sha256 = (
                    Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName
                ).Hash
            })
        }
    }
}

function Get-NativeAppInventory {
    param([Parameter(Mandatory)][string] $App)

    $records = [Collections.Generic.List[object]]::new()
    Add-NativeAppInventory -Directory $App -Prefix "" -Records $records
    return $records.ToArray()
}

function Get-NativeDmgRootInventory {
    param([Parameter(Mandatory)][string] $Mount)

    $records = [Collections.Generic.List[object]]::new()
    foreach ($item in Get-ChildItem -LiteralPath $Mount -Force) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            $records.Add([pscustomobject]@{
                Kind = "Link"
                Name = $item.Name
                Target = (Get-NativeLinkTarget -Item $item)
            })
        } else {
            $records.Add([pscustomobject]@{
                Kind = if ($item.PSIsContainer) { "Directory" } else { "File" }
                Name = $item.Name
                Target = $null
            })
        }
    }
    return $records.ToArray()
}

function Get-NativeMacMetadata {
    param([Parameter(Mandatory)][string] $App)

    $plist = Join-Path $App "Contents/Info.plist"
    $readValue = {
        param([Parameter(Mandatory)][string] $Key, [switch] $Optional)

        $value = & /usr/libexec/PlistBuddy `
            -c "Print :$Key" `
            $plist 2>$null
        if ($LASTEXITCODE -ne 0) {
            if ($Optional) {
                return $null
            }
            throw "Could not inspect required macOS bundle metadata '$Key'."
        }
        return ([string] $value).Trim()
    }
    return [pscustomobject]@{
        CFBundleIdentifier = & $readValue "CFBundleIdentifier"
        CFBundleExecutable = & $readValue "CFBundleExecutable"
        CFBundleShortVersionString = & $readValue "CFBundleShortVersionString"
        CFBundleName = & $readValue "CFBundleName" -Optional
    }
}

function Invoke-Adapter {
    param(
        [Parameter(Mandatory)][string] $Operation,
        [string] $Source,
        [string] $Destination
    )

    if (-not [string]::IsNullOrWhiteSpace($ToolAdapterPath)) {
        $null = Assert-RegularPath `
            -Path $ToolAdapterPath `
            -Kind File `
            -Description "Tool adapter"
        return & $ToolAdapterPath `
            -Operation $Operation `
            -Source $Source `
            -Destination $Destination
    }

    switch ($Operation) {
        "mac-app-metadata" {
            return (
                Get-NativeMacMetadata -App $Source |
                    ConvertTo-Json -Compress
            )
        }
        "mac-app-inventory" {
            return @(
                Get-NativeAppInventory -App $Source
            ) | ConvertTo-Json -Compress -Depth 4
        }
        "mac-dmg-root-inventory" {
            return @(
                Get-NativeDmgRootInventory -Mount $Source
            ) | ConvertTo-Json -Compress -Depth 4
        }
        "mac-attach" {
            [IO.Directory]::CreateDirectory($Destination) | Out-Null
            $null = Invoke-NativeCommand `
                -Command "hdiutil" `
                -Arguments @(
                    "attach",
                    "-readonly",
                    "-nobrowse",
                    "-mountpoint",
                    $Destination,
                    $Source
                ) `
                -Failure "Could not attach DMG read-only."
            return $Destination
        }
        "mac-detach" {
            $null = Invoke-NativeCommand `
                -Command "hdiutil" `
                -Arguments @("detach", $Source) `
                -Failure "Could not detach DMG."
            return
        }
        "mac-create-tar" {
            $parent = Split-Path -Parent $Source
            $leaf = Split-Path -Leaf $Source
            $null = Invoke-NativeCommand `
                -Command "tar" `
                -Arguments @("-C", $parent, "-czf", $Destination, $leaf) `
                -Failure "Could not create macOS tar.gz."
            return
        }
        "mac-create-zip" {
            $null = Invoke-NativeCommand `
                -Command "ditto" `
                -Arguments @(
                    "-c",
                    "-k",
                    "--sequesterRsrc",
                    "--keepParent",
                    $Source,
                    $Destination
                ) `
                -Failure "Could not create macOS ZIP."
            return
        }
        "mac-lipo-architecture" {
            $description = Invoke-NativeCommand `
                -Command "lipo" `
                -Arguments @("-archs", $Source) `
                -Failure "Could not inspect Mach-O architecture."
            $architectures = @(
                ([string] $description).Trim() -split "\s+" |
                    Where-Object { $_ -cne "" }
            )
            if ($architectures.Count -ne 1) {
                throw "Mach-O binary must contain exactly one architecture."
            }
            return $(if ($architectures[0] -ceq "arm64") {
                "arm64"
            } elseif ($architectures[0] -ceq "x86_64") {
                "x86_64"
            } else {
                throw "Unsupported Mach-O architecture '$($architectures[0])'."
            })
        }
        default {
            throw "Unsupported native tool operation '$Operation'."
        }
    }
}

function Get-ExactChild {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Subdirectory,
        [Parameter(Mandatory)][string] $Suffix,
        [Parameter(Mandatory)][ValidateSet("File", "Directory")]
        [string] $Kind,
        [string[]] $AllowedAuxiliaryNames = @()
    )

    $directory = Join-Path $Root $Subdirectory
    $null = Assert-RegularPath `
        -Path $directory `
        -Kind Directory `
        -Description "$Subdirectory source directory"
    $items = @(Get-ChildItem -LiteralPath $directory -Force)
    $sources = @($items | Where-Object {
        ($Kind -ceq "Directory") -eq $_.PSIsContainer -and
        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0 -and
        $_.Name.EndsWith($Suffix, [StringComparison]::Ordinal)
    })
    $auxiliary = @($items | Where-Object {
        $AllowedAuxiliaryNames -contains $_.Name
    })
    $unknown = @($items | Where-Object {
        $sources -notcontains $_ -and $auxiliary -notcontains $_
    })
    if (
        $sources.Count -ne 1 -or
        $unknown.Count -ne 0 -or
        @($auxiliary | Where-Object {
            ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        }).Count -ne 0
    ) {
        throw "$Subdirectory must contain exactly one regular $Suffix source."
    }
    foreach ($item in $auxiliary | Where-Object PSIsContainer) {
        foreach ($nested in Get-ChildItem -LiteralPath $item.FullName -Force -Recurse) {
            if (($nested.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Subdirectory auxiliary tree contains a reparse point."
            }
        }
    }
    return $sources[0].FullName
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory)] $Value,
        [Parameter(Mandatory)][string[]] $Expected,
        [Parameter(Mandatory)][string] $Description
    )

    [string[]] $actual = @($Value.PSObject.Properties.Name)
    [Array]::Sort($actual, [StringComparer]::Ordinal)
    [string[]] $orderedExpected = @($Expected)
    [Array]::Sort($orderedExpected, [StringComparer]::Ordinal)
    if (
        [string]::Join("|", $actual) -cne
        [string]::Join("|", $orderedExpected)
    ) {
        throw (
            "$Description has unexpected fields: " +
            [string]::Join("|", $actual)
        )
    }
}

function ConvertTo-FingerprintField {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Value)

    return [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value))
}

function Get-LexicalUnixAbsolutePath {
    param([Parameter(Mandatory)][string] $Path)

    if (-not $Path.StartsWith("/", [StringComparison]::Ordinal)) {
        throw "macOS absolute symlink target must start with '/'."
    }
    $parts = [Collections.Generic.List[string]]::new()
    foreach ($part in $Path.Split("/")) {
        if ($part -ceq "" -or $part -ceq ".") {
            continue
        }
        if ($part -ceq "..") {
            if ($parts.Count -eq 0) {
                throw "macOS symlink uses a forbidden absolute target '$Path'."
            }
            $parts.RemoveAt($parts.Count - 1)
            continue
        }
        $parts.Add($part)
    }
    return "/" + [string]::Join("/", $parts)
}

function Resolve-InternalLinkTarget {
    param(
        [Parameter(Mandatory)][string] $Relative,
        [Parameter(Mandatory)][string] $Target,
        [Parameter(Mandatory)][object] $KnownPaths
    )

    if (
        $Target.IndexOf([char] 0) -ge 0 -or
        $Target.Contains("`r") -or
        $Target.Contains("`n")
    ) {
        throw "macOS symlink target is invalid."
    }
    if ($Target.StartsWith("/", [StringComparison]::Ordinal)) {
        $normalizedTarget = Get-LexicalUnixAbsolutePath -Path $Target
        if (
            $normalizedTarget -ceq "/System/Library" -or
            $normalizedTarget.StartsWith(
                "/System/Library/",
                [StringComparison]::Ordinal
            ) -or
            $normalizedTarget -ceq "/usr/lib" -or
            $normalizedTarget.StartsWith(
                "/usr/lib/",
                [StringComparison]::Ordinal
            )
        ) {
            return $normalizedTarget
        }
        throw "macOS symlink uses a forbidden absolute target '$Target'."
    }
    if ([string]::IsNullOrWhiteSpace($Target)) {
        throw "macOS symlink target is empty."
    }

    $relativeParts = @($Relative.Split("/"))
    $parts = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $relativeParts.Count - 1; $index += 1) {
        $parts.Add($relativeParts[$index])
    }
    foreach ($part in $Target.Split("/")) {
        if ($part -ceq "" -or $part -ceq ".") {
            continue
        }
        if ($part -ceq "..") {
            if ($parts.Count -eq 0) {
                throw "macOS symlink target escapes the app root."
            }
            $parts.RemoveAt($parts.Count - 1)
            continue
        }
        $parts.Add($part)
    }
    $resolved = [string]::Join("/", $parts)
    if (-not $KnownPaths.Contains($resolved)) {
        throw (
            "macOS symlink target '$resolved' does not exist inside " +
            "the app inventory."
        )
    }
    return $resolved
}

function Get-ValidatedAppFingerprint {
    param([Parameter(Mandatory)][object[]] $Inventory)

    if ($Inventory.Count -eq 0) {
        throw "macOS app inventory is empty."
    }
    $knownPaths = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    $null = $knownPaths.Add("")
    $byPath = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    foreach ($entry in $Inventory) {
        Assert-ExactProperties `
            -Value $entry `
            -Expected @("Kind", "Relative", "Target", "Sha256") `
            -Description "macOS app inventory entry"
        $relative = [string] $entry.Relative
        if (
            [string]::IsNullOrWhiteSpace($relative) -or
            $relative.StartsWith("/", [StringComparison]::Ordinal) -or
            $relative.EndsWith("/", [StringComparison]::Ordinal) -or
            $relative.Contains("//") -or
            @($relative.Split("/") | Where-Object {
                    $_ -ceq "." -or $_ -ceq ".." -or $_ -ceq ""
                }).Count -ne 0
        ) {
            throw "macOS app inventory contains an invalid relative path."
        }
        if (-not $knownPaths.Add($relative)) {
            throw "macOS app inventory contains a duplicate path."
        }
        if ($forbidden.IsMatch($relative)) {
            throw "macOS app contains a forbidden payload."
        }
        $byPath.Add($relative, $entry)
    }
    foreach ($required in @(
            "Contents",
            "Contents/MacOS",
            "Contents/MacOS/wokrouter-desktop",
            "Contents/MacOS/wokrouter"
        )) {
        if (-not $knownPaths.Contains($required)) {
            throw "macOS app inventory is missing '$required'."
        }
    }

    $records = [Collections.Generic.List[string]]::new()
    foreach ($relative in $byPath.Keys) {
        $entry = $byPath[$relative]
        $relativeField = ConvertTo-FingerprintField -Value $relative
        switch ([string] $entry.Kind) {
            "Directory" {
                if ($null -ne $entry.Target -or $null -ne $entry.Sha256) {
                    throw "macOS directory inventory entry is invalid."
                }
                $records.Add("D|$relativeField")
            }
            "File" {
                $hash = [string] $entry.Sha256
                if (
                    $null -ne $entry.Target -or
                    $hash -cnotmatch "^[0-9A-Fa-f]{64}$"
                ) {
                    throw "macOS file inventory entry is invalid."
                }
                $records.Add("F|$relativeField|$($hash.ToUpperInvariant())")
            }
            "Link" {
                if ($null -ne $entry.Sha256) {
                    throw "macOS symlink inventory entry is invalid."
                }
                $target = [string] $entry.Target
                $null = Resolve-InternalLinkTarget `
                    -Relative $relative `
                    -Target $target `
                    -KnownPaths $knownPaths
                $targetField = ConvertTo-FingerprintField -Value $target
                $records.Add("L|$relativeField|$targetField")
            }
            default {
                throw "macOS app inventory contains an unsupported entry kind."
            }
        }
    }
    [string[]] $ordered = $records.ToArray()
    [Array]::Sort($ordered, [StringComparer]::Ordinal)
    return [string]::Join("`n", $ordered)
}

function Assert-App {
    param(
        [Parameter(Mandatory)][string] $App,
        [Parameter(Mandatory)][string] $ExpectedArchitecture
    )

    if ((Split-Path -Leaf $App) -cne "WokRouter.app") {
        throw "macOS bundle app must be named exactly WokRouter.app."
    }
    $null = Assert-RegularPath `
        -Path $App `
        -Kind Directory `
        -Description "macOS app"
    try {
        $metadata = Invoke-Adapter `
            -Operation "mac-app-metadata" `
            -Source $App |
            ConvertFrom-Json
    }
    catch {
        throw "macOS bundle metadata inspection failed: $($_.Exception.Message)"
    }
    Assert-ExactProperties `
        -Value $metadata `
        -Expected @(
            "CFBundleIdentifier",
            "CFBundleExecutable",
            "CFBundleShortVersionString",
            "CFBundleName"
        ) `
        -Description "macOS bundle metadata"
    if ([string] $metadata.CFBundleIdentifier -cne "dev.wokrouter.desktop") {
        throw "macOS bundle identifier is invalid."
    }
    if ([string] $metadata.CFBundleExecutable -cne "wokrouter-desktop") {
        throw "macOS bundle executable is invalid."
    }
    if ([string] $metadata.CFBundleShortVersionString -cne $Version) {
        throw "macOS app version does not match '$Version'."
    }
    if (
        $null -ne $metadata.CFBundleName -and
        [string] $metadata.CFBundleName -cne "WokRouter"
    ) {
        throw "macOS bundle name is invalid."
    }

    try {
        $inventoryDocument = Invoke-Adapter `
            -Operation "mac-app-inventory" `
            -Source $App |
            ConvertFrom-Json
        [object[]] $inventory = @(
            $inventoryDocument | ForEach-Object { $_ }
        )
    }
    catch {
        throw "macOS app inventory inspection failed: $($_.Exception.Message)"
    }
    $fingerprint = Get-ValidatedAppFingerprint -Inventory $inventory
    foreach ($name in @("wokrouter-desktop", "wokrouter")) {
        $binary = Join-Path $App "Contents/MacOS/$name"
        $null = Assert-RegularPath `
            -Path $binary `
            -Kind File `
            -Description "macOS app binary"
        $actualArchitecture = (
            Invoke-Adapter `
                -Operation "mac-lipo-architecture" `
                -Source $binary
        ).Trim()
        if ($actualArchitecture -cne $ExpectedArchitecture) {
            throw "macOS binary architecture does not match '$ExpectedArchitecture'."
        }
    }
    Write-Output $fingerprint
}

function Assert-DmgRoot {
    param([Parameter(Mandatory)][string] $Mount)

    try {
        $inventoryDocument = Invoke-Adapter `
            -Operation "mac-dmg-root-inventory" `
            -Source $Mount |
            ConvertFrom-Json
        [object[]] $inventory = @(
            $inventoryDocument | ForEach-Object { $_ }
        )
    }
    catch {
        throw "DMG root inventory inspection failed: $($_.Exception.Message)"
    }
    $allowed = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::Ordinal
    )
    $allowed.Add(".DS_Store", "File")
    $allowed.Add(".VolumeIcon.icns", "File")
    $allowed.Add(".background", "Directory")
    $allowed.Add("Applications", "Link")
    $allowed.Add("WokRouter.app", "Directory")
    $seen = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    foreach ($entry in $inventory) {
        Assert-ExactProperties `
            -Value $entry `
            -Expected @("Kind", "Name", "Target") `
            -Description "DMG root inventory entry"
        $name = [string] $entry.Name
        if ($forbidden.IsMatch($name)) {
            throw "DMG root contains a forbidden payload."
        }
        if (
            $name.EndsWith(".app", [StringComparison]::Ordinal) -and
            $name -cne "WokRouter.app"
        ) {
            throw "DMG root app must be named exactly WokRouter.app."
        }
        if (
            [string]::IsNullOrWhiteSpace($name) -or
            -not $allowed.ContainsKey($name) -or
            -not $seen.Add($name)
        ) {
            throw "DMG root inventory contains an unexpected entry."
        }
        if ([string] $entry.Kind -cne $allowed[$name]) {
            throw "DMG root inventory entry '$name' has the wrong type."
        }
        if ($name -ceq "Applications") {
            if ([string] $entry.Target -cne "/Applications") {
                throw "DMG Applications symlink must target /Applications."
            }
        } elseif ($null -ne $entry.Target) {
            throw "DMG root non-link entry '$name' has a link target."
        }
    }
    if (-not $seen.Contains("WokRouter.app")) {
        throw "DMG root inventory must contain exactly WokRouter.app."
    }
    if ($seen.Contains(".background")) {
        $background = Join-Path $Mount ".background"
        if (Test-Path -LiteralPath $background) {
            $null = Assert-RegularPath `
                -Path $background `
                -Kind Directory `
                -Description "DMG background metadata"
            $forbiddenMetadata = [regex]::new(
                "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
            )
            foreach (
                $item in Get-ChildItem `
                    -LiteralPath $background `
                    -Force `
                    -Recurse
            ) {
                if (
                    ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                    0
                ) {
                    throw "DMG background metadata contains a reparse point."
                }
                if ($forbiddenMetadata.IsMatch($item.Name)) {
                    throw "DMG background metadata contains a forbidden payload."
                }
            }
        } elseif ([string]::IsNullOrWhiteSpace($ToolAdapterPath)) {
            throw "DMG background metadata directory is missing."
        }
    }
    return Join-Path $Mount "WokRouter.app"
}

$contract = @(
    Get-WokRouterTargetContracts -Version $Version |
        Where-Object Target -CEQ $Target
)
if ($contract.Count -ne 1 -or $contract[0].System -cne "macOS") {
    throw "Unsupported macOS release target '$Target'."
}
$architecture = [string] $contract[0].Architecture

$bundle = (Assert-RegularPath `
    -Path $BundleDirectory `
    -Kind Directory `
    -Description "Bundle directory").FullName
$rootItems = @(Get-ChildItem -LiteralPath $bundle -Force)
$allowedRootAuxiliary = @(".DS_Store", ".localized")
$allowedRootDirectories = @("share")
$rootNames = @(
    $rootItems |
        Where-Object {
            $allowedRootAuxiliary -notcontains $_.Name -and
            $allowedRootDirectories -notcontains $_.Name
        } |
        ForEach-Object Name
)
[Array]::Sort($rootNames, [StringComparer]::Ordinal)
$unexpectedFiles = @($rootItems | Where-Object {
    $allowedRootAuxiliary -notcontains $_.Name -and
    $allowedRootDirectories -notcontains $_.Name -and
    -not $_.PSIsContainer
})
$invalidAuxiliary = @($rootItems | Where-Object {
    $allowedRootAuxiliary -contains $_.Name -and (
        $_.PSIsContainer -or
        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    )
})
$invalidAllowedDirectories = @($rootItems | Where-Object {
    $allowedRootDirectories -contains $_.Name -and (
        -not $_.PSIsContainer -or
        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    )
})
if (
    [string]::Join("|", $rootNames) -cne "dmg|macos" -or
    $unexpectedFiles.Count -ne 0 -or
    $invalidAuxiliary.Count -ne 0 -or
    $invalidAllowedDirectories.Count -ne 0
) {
    $details = @(
        $rootItems | ForEach-Object {
            $kind = if ($_.PSIsContainer) { "Directory" } else { "File" }
            $reparse = if (($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { "Reparse" } else { "Regular" }
            "$($_.Name):${kind}:${reparse}"
        }
    )
    [Array]::Sort($details, [StringComparer]::Ordinal)
    throw "macOS bundle must contain exact dmg and macos source directories (root entries: $([string]::Join('|', $details)))."
}
$dmg = Get-ExactChild `
    -Root $bundle `
    -Subdirectory "dmg" `
    -Suffix ".dmg" `
    -Kind File `
    -AllowedAuxiliaryNames @("bundle_dmg.sh", "share")
$app = Get-ExactChild `
    -Root $bundle `
    -Subdirectory "macos" `
    -Suffix ".app" `
    -Kind Directory
$sourceFingerprint = Assert-App `
    -App $app `
    -ExpectedArchitecture $architecture

if ([IO.File]::Exists($OutputDirectory)) {
    throw "Output directory must not be a file."
}
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
$output = (Assert-RegularPath `
    -Path $OutputDirectory `
    -Kind Directory `
    -Description "Output directory").FullName
if (@(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) {
    throw "Output directory must be empty."
}

$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporary = Join-Path $temporaryParent (
    "wokrouter-macos-package-" + [Guid]::NewGuid().ToString("N")
)
[IO.Directory]::CreateDirectory($temporary) | Out-Null
$mounted = $null
try {
    $mountRequest = Join-Path $temporary "mount"
    $mounted = (
        Invoke-Adapter `
            -Operation "mac-attach" `
            -Source $dmg `
            -Destination $mountRequest
    ).Trim()
    $null = Assert-RegularPath `
        -Path $mounted `
        -Kind Directory `
        -Description "Mounted DMG"
    $mountedApp = Assert-DmgRoot -Mount $mounted
    $mountedFingerprint = Assert-App `
        -App $mountedApp `
        -ExpectedArchitecture $architecture
    if ($sourceFingerprint -cne $mountedFingerprint) {
        throw "DMG app does not match the source app."
    }

    $prefix = "WokRouter-v$Version-macOS-$architecture"
    $dmgOutput = Join-Path $output "$prefix.dmg"
    $tarOutput = Join-Path $output "$prefix.tar.gz"
    $zipOutput = Join-Path $output "$prefix.zip"
    [IO.File]::Copy($dmg, $dmgOutput, $false)
    $null = Invoke-Adapter `
        -Operation "mac-create-tar" `
        -Source $app `
        -Destination $tarOutput
    $null = Invoke-Adapter `
        -Operation "mac-create-zip" `
        -Source $app `
        -Destination $zipOutput
    foreach ($path in @($dmgOutput, $tarOutput, $zipOutput)) {
        $null = Assert-RegularPath `
            -Path $path `
            -Kind File `
            -Description "macOS output"
        Write-Output $path
    }
}
finally {
    if (-not [string]::IsNullOrWhiteSpace($mounted)) {
        $null = Invoke-Adapter -Operation "mac-detach" -Source $mounted
    }
    $fullTemporary = [IO.Path]::GetFullPath($temporary)
    if (
        $fullTemporary.StartsWith(
            $temporaryParent,
            [StringComparison]::OrdinalIgnoreCase
        ) -and
        [IO.Path]::GetFileName($fullTemporary) -cmatch (
            "^wokrouter-macos-package-[0-9a-f]{32}$"
        ) -and
        [IO.Directory]::Exists($fullTemporary)
    ) {
        [IO.Directory]::Delete($fullTemporary, $true)
    }
}
