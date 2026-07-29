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

function Get-SafeTreeItems {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Format
    )

    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $rootPrefix = $fullRoot + [IO.Path]::DirectorySeparatorChar
    $items = [Collections.Generic.List[object]]::new()
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($fullRoot)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force) {
            $fullItem = [IO.Path]::GetFullPath($item.FullName)
            if (
                -not $fullItem.StartsWith(
                    $rootPrefix,
                    [StringComparison]::Ordinal
                )
            ) {
                throw "Extracted $Format inventory escapes its temporary root."
            }
            if (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                0
            ) {
                if (
                    $Format -ceq "AppImage" -and
                    $directory.Equals(
                        $fullRoot,
                        [StringComparison]::Ordinal
                    ) -and
                    @(".DirIcon", "WokRouter.desktop") -ccontains $item.Name
                ) {
                    continue
                }
                throw "Extracted $Format inventory contains a reparse point."
            }
            $items.Add($item)
            if ($item.PSIsContainer) {
                $pending.Push($item.FullName)
            }
        }
    }
    return $items.ToArray()
}

function Get-AppImageLinkTarget {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $LinkName,
        [Parameter(Mandatory)][string] $Target,
        [Parameter(Mandatory)][string] $ExpectedTarget
    )

    if (
        [string]::IsNullOrWhiteSpace($Target) -or
        $Target.Contains("\") -or
        $Target -cmatch "^[A-Za-z]:" -or
        [IO.Path]::IsPathRooted($Target)
    ) {
        throw "AppImage link '$LinkName' target must be an unambiguous relative path."
    }
    $segments = [Collections.Generic.List[string]]::new()
    foreach ($segment in $Target.Split("/")) {
        if ([string]::IsNullOrEmpty($segment) -or $segment -ceq ".") {
            throw "AppImage link '$LinkName' target is ambiguous."
        }
        if ($segment -ceq "..") {
            if ($segments.Count -eq 0) {
                throw "AppImage link '$LinkName' target escapes AppDir."
            }
            $segments.RemoveAt($segments.Count - 1)
            continue
        }
        if ($segment.IndexOf([char] 0) -ge 0) {
            throw "AppImage link '$LinkName' target is malformed."
        }
        $segments.Add($segment)
    }
    $normalizedTarget = [string]::Join("/", $segments)
    if ($normalizedTarget -cne $ExpectedTarget -or $Target -cne $ExpectedTarget) {
        throw "AppImage link '$LinkName' does not use its expected target."
    }

    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $fullTarget = $fullRoot
    foreach ($segment in $segments) {
        $fullTarget = Join-Path $fullTarget $segment
    }
    $fullTarget = [IO.Path]::GetFullPath($fullTarget)
    $rootPrefix = $fullRoot + [IO.Path]::DirectorySeparatorChar
    $comparison = if ([IO.Path]::DirectorySeparatorChar -ceq "\") {
        [StringComparison]::OrdinalIgnoreCase
    } else {
        [StringComparison]::Ordinal
    }
    if (-not $fullTarget.StartsWith($rootPrefix, $comparison)) {
        throw "AppImage link '$LinkName' target escapes AppDir."
    }

    $current = $fullRoot
    for ($index = 0; $index -lt $segments.Count; $index += 1) {
        $current = Join-Path $current $segments[$index]
        try {
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
        }
        catch {
            throw "AppImage link '$LinkName' target must be an existing regular file."
        }
        if (
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        ) {
            throw "AppImage link '$LinkName' target contains a reparse component."
        }
        $isLeaf = $index -eq ($segments.Count - 1)
        if (($isLeaf -and $item.PSIsContainer) -or
            (-not $isLeaf -and -not $item.PSIsContainer)) {
            throw "AppImage link '$LinkName' target must be an existing regular file."
        }
    }
    return $fullTarget
}

function Get-ValidatedAppImageLinks {
    param([Parameter(Mandatory)][string] $Root)

    try {
        [object[]] $inventory = @(
            Invoke-Adapter `
                -Operation "linux-appimage-link-inventory" `
                -Source $Root |
                ConvertFrom-Json |
                ForEach-Object { $_ }
        )
    }
    catch {
        throw "AppImage link inventory failed: $($_.Exception.Message)"
    }
    if ($inventory.Count -ne 2) {
        throw "AppImage must contain exactly two expected root links."
    }

    $expected = [ordered]@{
        ".DirIcon" = "WokRouter.png"
        "WokRouter.desktop" = "usr/share/applications/WokRouter.desktop"
    }
    $validated = [ordered]@{}
    foreach ($record in $inventory) {
        $properties = @($record.PSObject.Properties | ForEach-Object Name)
        [Array]::Sort($properties, [StringComparer]::Ordinal)
        if (
            [string]::Join("|", $properties) -cne
            "LinkType|Relative|Target" -or
            $record.Relative -isnot [string] -or
            $record.LinkType -isnot [string] -or
            $record.Target -isnot [string]
        ) {
            throw "AppImage link inventory is malformed."
        }
        $relative = [string] $record.Relative
        if ($relative.Contains("/") -or $relative.Contains("\")) {
            throw "AppImage links are allowed only at the AppDir root."
        }
        $caseMatches = @(
            $expected.Keys |
                Where-Object {
                    $_.Equals(
                        $relative,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }
        )
        if ($caseMatches.Count -eq 1 -and $caseMatches[0] -cne $relative) {
            throw "AppImage expected link names are case-sensitive."
        }
        if (
            $caseMatches.Count -ne 1 -or
            $validated.Contains($relative)
        ) {
            throw "AppImage must contain exactly the two expected root links."
        }
        if ([string] $record.LinkType -cne "SymbolicLink") {
            throw "AppImage expected reparse points must be symbolic links."
        }
        $validated[$relative] = Get-AppImageLinkTarget `
            -Root $Root `
            -LinkName $relative `
            -Target ([string] $record.Target) `
            -ExpectedTarget ([string] $expected[$relative])
    }
    if ($validated.Count -ne 2) {
        throw "AppImage must contain exactly the two expected root links."
    }
    return $validated
}

function Remove-TemporaryTree {
    param([Parameter(Mandatory)][string] $Root)

    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force) {
            if (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                0
            ) {
                if ($item.PSIsContainer) {
                    [IO.Directory]::Delete($item.FullName, $false)
                }
                else {
                    [IO.File]::Delete($item.FullName)
                }
            }
            elseif ($item.PSIsContainer) {
                $pending.Push($item.FullName)
            }
        }
    }
    [IO.Directory]::Delete($Root, $true)
}

function Assert-NoForbiddenPayload {
    param(
        [Parameter(Mandatory)][object[]] $Items,
        [Parameter(Mandatory)][string] $Format
    )

    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    foreach ($item in $Items) {
        if ($forbidden.IsMatch($item.Name)) {
            throw "Extracted $Format contains a forbidden payload."
        }
    }
}

function Get-RequiredPayloadFiles {
    param(
        [Parameter(Mandatory)][object[]] $Items,
        [Parameter(Mandatory)][string] $Format
    )

    $payloads = [ordered]@{}
    foreach ($name in @(
            "wokrouter-desktop",
            "wokrouter",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "NOTICE.md",
            "README.md"
        )) {
        $matching = @(
            $Items |
                Where-Object {
                    -not $_.PSIsContainer -and
                    $_.Name.Equals(
                        $name,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }
        )
        if (
            $matching.Count -ne 1 -or
            ($matching[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
            0 -or
            $matching[0].Name -cne $name
        ) {
            throw "Extracted $Format required payload inventory is invalid for '$name'."
        }
        $payloads[$name] = $matching[0].FullName
    }
    return $payloads
}

function Get-ValidatedPayloadFiles {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Format,
        [Parameter(Mandatory)][string] $ExpectedArchitecture
    )

    $null = Assert-RegularPath `
        -Path $Root `
        -Kind Directory `
        -Description "Extracted $Format"
    $items = @(Get-SafeTreeItems -Root $Root -Format $Format)
    Assert-NoForbiddenPayload -Items $items -Format $Format
    $payloads = Get-RequiredPayloadFiles -Items $items -Format $Format
    foreach ($name in @("wokrouter-desktop", "wokrouter")) {
        $actualArchitecture = (
            Invoke-Adapter `
                -Operation "binary-architecture" `
                -Source $payloads[$name]
        ).Trim()
        if ($actualArchitecture -cne $ExpectedArchitecture) {
            throw "$Format binary architecture does not match '$ExpectedArchitecture'."
        }
    }
    return $payloads
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
        "linux-appimage-link-inventory" {
            $records = [Collections.Generic.List[object]]::new()
            foreach (
                $item in Get-ChildItem -LiteralPath $Source -Force |
                    Where-Object {
                        ($_.Attributes -band
                            [IO.FileAttributes]::ReparsePoint) -ne 0
                    }
            ) {
                [object[]] $targets = @($item.Target)
                if ($targets.Count -ne 1 -or $targets[0] -isnot [string]) {
                    throw "Could not read AppImage symbolic-link target."
                }
                $records.Add([pscustomobject]@{
                    Relative = $item.Name
                    LinkType = [string] $item.LinkType
                    Target = [string] $targets[0]
                })
            }
            return ConvertTo-Json `
                -Compress `
                -InputObject @($records.ToArray())
        }
        "linux-deb-extract" {
            & dpkg-deb --extract $Source $Destination
            if ($LASTEXITCODE -ne 0) {
                throw "Could not extract deb."
            }
            return
        }
        "linux-rpm-extract" {
            [IO.Directory]::CreateDirectory($Destination) | Out-Null
            $bash = (Get-Command bash -ErrorAction Stop).Source
            & $bash `
                -o pipefail `
                -c @'
set -euo pipefail
package=$1
destination=$2
cd -- "$destination"
rpm2cpio "$package" |
  cpio --extract --make-directories --no-absolute-filenames --quiet
'@ `
                "wokrouter-rpm-extract" `
                $Source `
                $Destination
            if ($LASTEXITCODE -ne 0) {
                throw "Could not extract rpm."
            }
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
    $debDir = Join-Path $temporary "deb-root"
    $rpmDir = Join-Path $temporary "rpm-root"
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
    $null = Invoke-Adapter `
        -Operation "linux-deb-extract" `
        -Source $deb `
        -Destination $debDir
    $null = Invoke-Adapter `
        -Operation "linux-rpm-extract" `
        -Source $rpm `
        -Destination $rpmDir

    $appImageLinks = Get-ValidatedAppImageLinks -Root $appDir
    $payloads = [ordered]@{
        AppImage = Get-ValidatedPayloadFiles `
            -Root $appDir `
            -Format "AppImage" `
            -ExpectedArchitecture $architecture
        deb = Get-ValidatedPayloadFiles `
            -Root $debDir `
            -Format "deb" `
            -ExpectedArchitecture $architecture
        rpm = Get-ValidatedPayloadFiles `
            -Root $rpmDir `
            -Format "rpm" `
            -ExpectedArchitecture $architecture
    }
    foreach ($name in @(
            "wokrouter-desktop",
            "wokrouter",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "NOTICE.md",
            "README.md"
        )) {
        $expectedHash = (
            Get-FileHash `
                -Algorithm SHA256 `
                -LiteralPath $payloads.AppImage[$name]
        ).Hash
        foreach ($format in @("deb", "rpm")) {
            $actualHash = (
                Get-FileHash `
                    -Algorithm SHA256 `
                    -LiteralPath $payloads[$format][$name]
            ).Hash
            if ($actualHash -cne $expectedHash) {
                throw "Linux payload '$name' must be byte-identical across formats."
            }
        }
    }
    $desktopEntry = Get-Content `
        -Raw `
        -Encoding UTF8 `
        -LiteralPath $appImageLinks["WokRouter.desktop"]
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
        Remove-TemporaryTree -Root $fullTemporary
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
