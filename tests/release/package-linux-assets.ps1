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

function Assert-TreeSafe {
    param([Parameter(Mandatory)][string] $Root)

    foreach ($item in Get-ChildItem -LiteralPath $Root -Force -Recurse) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Extracted AppImage inventory contains a reparse point."
        }
    }
}

function Assert-NoForbiddenPayload {
    param([Parameter(Mandatory)][string] $Root)

    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    foreach ($item in Get-ChildItem -LiteralPath $Root -Force -Recurse) {
        if ($forbidden.IsMatch($item.Name)) {
            throw "Extracted AppImage contains a forbidden payload."
        }
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
        "linux-deb-metadata" {
            $name = (& dpkg-deb -f $Source Package).Trim()
            $nativeVersion = (& dpkg-deb -f $Source Version).Trim()
            $architecture = (& dpkg-deb -f $Source Architecture).Trim()
            if ($LASTEXITCODE -ne 0) {
                throw "Could not inspect deb metadata."
            }
            return @{
                Name = $name
                Version = $nativeVersion
                Architecture = $architecture
            } | ConvertTo-Json -Compress
        }
        "linux-rpm-metadata" {
            $query = & rpm -qp `
                --queryformat "%{NAME}`n%{VERSION}`n%{ARCH}`n" `
                $Source
            if ($LASTEXITCODE -ne 0) {
                throw "Could not inspect rpm metadata."
            }
            $lines = @($query -split "`r?`n" | Where-Object { $_ -cne "" })
            if ($lines.Count -ne 3) {
                throw "Could not parse rpm metadata."
            }
            return @{
                Name = $lines[0]
                Version = $lines[1]
                Architecture = $lines[2]
            } | ConvertTo-Json -Compress
        }
        "linux-appimage-extract" {
            $extractParent = Split-Path -Parent $Destination
            & $Source --appimage-extract | Out-Null
            if ($LASTEXITCODE -ne 0) {
                throw "Could not extract AppImage."
            }
            $nativeRoot = Join-Path $extractParent "squashfs-root"
            $null = Assert-RegularPath `
                -Path $nativeRoot `
                -Kind Directory `
                -Description "Extracted AppImage"
            Move-Item -LiteralPath $nativeRoot -Destination $Destination
            return
        }
        "binary-architecture" {
            $description = & file --brief $Source
            if ($LASTEXITCODE -ne 0) {
                throw "Could not inspect Linux binary architecture."
            }
            if ($description -match "(?i)(ARM aarch64|ARM64)") {
                return "arm64"
            }
            if ($description -match "(?i)(x86-64|x86_64)") {
                return "x86_64"
            }
            throw "Unsupported Linux binary architecture."
        }
        default {
            throw "Unsupported native tool operation '$Operation'."
        }
    }
}

function Get-ExactSource {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Subdirectory,
        [Parameter(Mandatory)][string] $Extension
    )

    $directory = Join-Path $Root $Subdirectory
    $null = Assert-RegularPath `
        -Path $directory `
        -Kind Directory `
        -Description "$Subdirectory source directory"
    $items = @(Get-ChildItem -LiteralPath $directory -Force)
    if (
        $items.Count -ne 1 -or
        $items[0].PSIsContainer -or
        ($items[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        -not $items[0].Name.EndsWith(
            $Extension,
            [StringComparison]::OrdinalIgnoreCase
        )
    ) {
        throw "$Subdirectory must contain exactly one regular $Extension source."
    }
    return $items[0].FullName
}

function Assert-PackageMetadata {
    param(
        [Parameter(Mandatory)][string] $Kind,
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $ExpectedArchitecture
    )

    try {
        $metadata = Invoke-Adapter `
            -Operation "linux-$Kind-metadata" `
            -Source $Path |
            ConvertFrom-Json
    }
    catch {
        throw "$Kind metadata inspection failed: $($_.Exception.Message)"
    }
    if (
        [string] $metadata.Name -cne "wokrouter" -or
        [string] $metadata.Version -cne $Version -or
        [string] $metadata.Architecture -cne $ExpectedArchitecture
    ) {
        throw "$Kind metadata does not match the release contract."
    }
}

$contract = @(
    Get-WokRouterTargetContracts -Version $Version |
        Where-Object Target -CEQ $Target
)
if ($contract.Count -ne 1 -or $contract[0].System -cne "Linux") {
    throw "Unsupported Linux release target '$Target'."
}
$architecture = [string] $contract[0].Architecture
$debArchitecture = if ($architecture -ceq "x86_64") { "amd64" } else { "arm64" }
$rpmArchitecture = if ($architecture -ceq "x86_64") { "x86_64" } else { "aarch64" }

$bundle = (Assert-RegularPath `
    -Path $BundleDirectory `
    -Kind Directory `
    -Description "Bundle directory").FullName
$rootItems = @(Get-ChildItem -LiteralPath $bundle -Force)
$rootNames = @($rootItems | ForEach-Object Name)
[Array]::Sort($rootNames, [StringComparer]::Ordinal)
if (
    [string]::Join("|", $rootNames) -cne "appimage|deb|rpm" -or
    @($rootItems | Where-Object { -not $_.PSIsContainer }).Count -ne 0
) {
    throw "Linux bundle must contain exactly one regular source directory per format."
}

$appImage = Get-ExactSource `
    -Root $bundle `
    -Subdirectory "appimage" `
    -Extension ".AppImage"
$deb = Get-ExactSource -Root $bundle -Subdirectory "deb" -Extension ".deb"
$rpm = Get-ExactSource -Root $bundle -Subdirectory "rpm" -Extension ".rpm"
Assert-PackageMetadata `
    -Kind "deb" `
    -Path $deb `
    -ExpectedArchitecture $debArchitecture
Assert-PackageMetadata `
    -Kind "rpm" `
    -Path $rpm `
    -ExpectedArchitecture $rpmArchitecture

$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporary = Join-Path $temporaryParent (
    "wokrouter-linux-package-" + [Guid]::NewGuid().ToString("N")
)
[IO.Directory]::CreateDirectory($temporary) | Out-Null
try {
    $appDir = Join-Path $temporary "AppDir"
    Push-Location -LiteralPath $temporary
    try {
        $null = Invoke-Adapter `
            -Operation "linux-appimage-extract" `
            -Source $appImage `
            -Destination $appDir
    }
    finally {
        Pop-Location
    }
    $null = Assert-RegularPath `
        -Path $appDir `
        -Kind Directory `
        -Description "Extracted AppImage"
    Assert-TreeSafe -Root $appDir
    Assert-NoForbiddenPayload -Root $appDir

    $desktop = Join-Path $appDir "usr/bin/wokrouter-desktop"
    $sidecar = Join-Path $appDir "usr/bin/wokrouter"
    foreach ($binary in @($desktop, $sidecar)) {
        $null = Assert-RegularPath `
            -Path $binary `
            -Kind File `
            -Description "AppImage binary"
        $actualArchitecture = (
            Invoke-Adapter -Operation "binary-architecture" -Source $binary
        ).Trim()
        if ($actualArchitecture -cne $architecture) {
            throw "AppImage binary architecture does not match '$architecture'."
        }
    }
    $desktopEntry = Get-Content `
        -Raw `
        -Encoding UTF8 `
        -LiteralPath (Join-Path $appDir "WokRouter.desktop")
    if ($desktopEntry -notmatch (
            "(?m)^X-AppImage-Version=" + [regex]::Escape($Version) + "$"
        )) {
        throw "AppImage version metadata does not match '$Version'."
    }
}
finally {
    $fullTemporary = [IO.Path]::GetFullPath($temporary)
    if (
        $fullTemporary.StartsWith(
            $temporaryParent,
            [StringComparison]::OrdinalIgnoreCase
        ) -and
        [IO.Path]::GetFileName($fullTemporary) -cmatch (
            "^wokrouter-linux-package-[0-9a-f]{32}$"
        ) -and
        [IO.Directory]::Exists($fullTemporary)
    ) {
        [IO.Directory]::Delete($fullTemporary, $true)
    }
}

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

$prefix = "WokRouter-v$Version-Linux-$architecture"
$outputs = @(
    @{ Source = $appImage; Name = "$prefix.AppImage" },
    @{ Source = $deb; Name = "$prefix.deb" },
    @{ Source = $rpm; Name = "$prefix.rpm" }
)
foreach ($asset in $outputs) {
    $destination = Join-Path $output $asset.Name
    [IO.File]::Copy($asset.Source, $destination, $false)
    Write-Output $destination
}
