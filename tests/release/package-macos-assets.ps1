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

function Assert-NoForbiddenPayload {
    param([Parameter(Mandatory)][string] $Root)

    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    foreach ($item in Get-ChildItem -LiteralPath $Root -Force -Recurse) {
        if ($forbidden.IsMatch($item.Name)) {
            throw "macOS app contains a forbidden payload."
        }
    }
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
        "mac-app-version" {
            return Invoke-NativeCommand `
                -Command "/usr/libexec/PlistBuddy" `
                -Arguments @(
                    "-c",
                    "Print :CFBundleShortVersionString",
                    (Join-Path $Source "Contents/Info.plist")
                ) `
                -Failure "Could not inspect macOS app version."
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
        [string] $Kind
    )

    $directory = Join-Path $Root $Subdirectory
    $null = Assert-RegularPath `
        -Path $directory `
        -Kind Directory `
        -Description "$Subdirectory source directory"
    $items = @(Get-ChildItem -LiteralPath $directory -Force)
    if (
        $items.Count -ne 1 -or
        (($Kind -ceq "Directory") -ne $items[0].PSIsContainer) -or
        ($items[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        -not $items[0].Name.EndsWith(
            $Suffix,
            [StringComparison]::Ordinal
        )
    ) {
        throw "$Subdirectory must contain exactly one regular $Suffix source."
    }
    return $items[0].FullName
}

function Assert-App {
    param(
        [Parameter(Mandatory)][string] $App,
        [Parameter(Mandatory)][string] $ExpectedArchitecture,
        [switch] $CheckVersion
    )

    $null = Assert-RegularPath `
        -Path $App `
        -Kind Directory `
        -Description "macOS app"
    Assert-NoForbiddenPayload -Root $App
    if ($CheckVersion) {
        $appVersion = (
            Invoke-Adapter -Operation "mac-app-version" -Source $App
        ).Trim()
        if ($appVersion -cne $Version) {
            throw "macOS app version does not match '$Version'."
        }
    }
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
}

function Get-AppFingerprint {
    param([Parameter(Mandatory)][string] $App)

    $records = [Collections.Generic.List[string]]::new()
    foreach ($item in Get-ChildItem -LiteralPath $App -Force -Recurse) {
        $relative = $item.FullName.Substring($App.Length).TrimStart(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        )
        if ($item.PSIsContainer) {
            $records.Add("D|$relative")
        } else {
            $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName).Hash
            $records.Add("F|$relative|$hash")
        }
    }
    $array = $records.ToArray()
    [Array]::Sort($array, [StringComparer]::Ordinal)
    return [string]::Join("`n", $array)
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
$rootNames = @($rootItems | ForEach-Object Name)
[Array]::Sort($rootNames, [StringComparer]::Ordinal)
if (
    [string]::Join("|", $rootNames) -cne "dmg|macos" -or
    @($rootItems | Where-Object { -not $_.PSIsContainer }).Count -ne 0
) {
    throw "macOS bundle must contain exact dmg and macos source directories."
}
$dmg = Get-ExactChild `
    -Root $bundle `
    -Subdirectory "dmg" `
    -Suffix ".dmg" `
    -Kind File
$app = Get-ExactChild `
    -Root $bundle `
    -Subdirectory "macos" `
    -Suffix ".app" `
    -Kind Directory
Assert-App `
    -App $app `
    -ExpectedArchitecture $architecture `
    -CheckVersion

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
    $mountedApps = @(
        Get-ChildItem -LiteralPath $mounted -Force |
            Where-Object {
                $_.PSIsContainer -and
                $_.Name.EndsWith(".app", [StringComparison]::Ordinal)
            }
    )
    if ($mountedApps.Count -ne 1) {
        throw "Mounted DMG must contain exactly one regular app."
    }
    Assert-App `
        -App $mountedApps[0].FullName `
        -ExpectedArchitecture $architecture
    if (
        (Get-AppFingerprint -App $app) -cne
        (Get-AppFingerprint -App $mountedApps[0].FullName)
    ) {
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
