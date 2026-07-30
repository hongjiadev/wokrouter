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
        [Parameter(Mandatory)][string] $Format,
        [Collections.Generic.Dictionary[string, object]] $AllowedLinks
    )

    if ($null -eq $AllowedLinks) {
        $AllowedLinks = [Collections.Generic.Dictionary[string, object]]::new(
            [StringComparer]::Ordinal
        )
    }
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $rootPrefix = $fullRoot + [IO.Path]::DirectorySeparatorChar
    $comparison = if ([IO.Path]::DirectorySeparatorChar -ceq "\") {
        [StringComparison]::OrdinalIgnoreCase
    } else {
        [StringComparison]::Ordinal
    }
    $items = [Collections.Generic.List[object]]::new()
    $seenLinks = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($fullRoot)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force) {
            $fullItem = [IO.Path]::GetFullPath($item.FullName)
            if (
                -not $fullItem.StartsWith(
                    $rootPrefix,
                    $comparison
                )
            ) {
                throw "Extracted $Format inventory escapes its temporary root."
            }
            $relative = $fullItem.Substring($rootPrefix.Length).Replace(
                [IO.Path]::DirectorySeparatorChar,
                "/"
            )
            $isReparse = (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                0
            )
            if ($isReparse) {
                if (-not $AllowedLinks.ContainsKey($relative)) {
                    throw "Extracted $Format inventory contains a reparse point."
                }
                $record = $AllowedLinks[$relative]
                [object[]] $actualTargets = @($item.Target)
                if (
                    [string]::IsNullOrWhiteSpace($ToolAdapterPath) -and
                    (
                        [string] $item.LinkType -cne
                        [string] $record.LinkType -or
                        $actualTargets.Count -ne 1 -or
                        $actualTargets[0] -isnot [string] -or
                        [string] $actualTargets[0] -cne
                        [string] $record.Target
                    )
                ) {
                    throw "Validated AppImage link changed during traversal."
                }
                $null = $seenLinks.Add($relative)
                continue
            }
            if ($AllowedLinks.ContainsKey($relative)) {
                if ([string]::IsNullOrWhiteSpace($ToolAdapterPath)) {
                    throw "Validated AppImage link changed during traversal."
                }
                if ($item.PSIsContainer) {
                    throw "Adapter AppImage link contract must be a file."
                }
                $null = $seenLinks.Add($relative)
                continue
            }
            $items.Add($item)
            if ($item.PSIsContainer) {
                $pending.Push($item.FullName)
            }
        }
    }
    if ($seenLinks.Count -ne $AllowedLinks.Count) {
        throw "Every AppImage inventory link must remain present during traversal."
    }
    return $items.ToArray()
}

function Get-CanonicalAppImageRelativeSegments {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string] $Relative
    )

    if (
        [string]::IsNullOrWhiteSpace($Relative) -or
        $Relative.Contains("\") -or
        $Relative.StartsWith("/", [StringComparison]::Ordinal) -or
        $Relative -cmatch "^[A-Za-z]:" -or
        [IO.Path]::IsPathRooted($Relative) -or
        [regex]::IsMatch($Relative, "[\x00-\x1F\x7F]")
    ) {
        throw "AppImage link inventory path must be canonical and relative."
    }
    $segments = [Collections.Generic.List[string]]::new()
    foreach ($segment in $Relative.Split("/")) {
        if (
            [string]::IsNullOrEmpty($segment) -or
            $segment -ceq "." -or
            $segment -ceq ".."
        ) {
            throw "AppImage link inventory path must be canonical and relative."
        }
        $segments.Add($segment)
    }
    return $segments.ToArray()
}

function Get-AppImageContractItem {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string[]] $Segments,
        [Parameter(Mandatory)][string] $Relative
    )

    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $current = $fullRoot
    for ($index = 0; $index -lt $Segments.Count; $index += 1) {
        $matches = @(
            Get-ChildItem -LiteralPath $current -Force |
                Where-Object Name -CEQ $Segments[$index]
        )
        if ($matches.Count -ne 1) {
            throw "AppImage inventory link '$Relative' must be present."
        }
        $item = $matches[0]
        $isLeaf = $index -eq ($Segments.Count - 1)
        if (-not $isLeaf) {
            if (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne
                0 -or
                -not $item.PSIsContainer
            ) {
                throw "AppImage inventory link '$Relative' must use regular parent directories."
            }
            $current = $item.FullName
        }
    }
    return $item
}

function Get-AppImageLinkTarget {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $LinkName,
        [Parameter(Mandatory)][string[]] $LinkSegments,
        [Parameter(Mandatory)][string] $Target,
        [Parameter(Mandatory)]
        [Collections.Generic.Dictionary[string, object]] $AllLinks
    )

    if (
        [string]::IsNullOrWhiteSpace($Target) -or
        $Target.Contains("\") -or
        $Target.StartsWith("/", [StringComparison]::Ordinal) -or
        $Target -cmatch "^[A-Za-z]:" -or
        [IO.Path]::IsPathRooted($Target) -or
        [regex]::IsMatch($Target, "[\x00-\x1F\x7F]")
    ) {
        throw "AppImage link '$LinkName' target must be an unambiguous relative path."
    }
    $segments = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt ($LinkSegments.Count - 1); $index += 1) {
        $segments.Add($LinkSegments[$index])
    }
    foreach ($segment in $Target.Split("/")) {
        if ([string]::IsNullOrEmpty($segment) -or $segment -ceq ".") {
            throw "AppImage link '$LinkName' target is ambiguous."
        }
        if ($segment -ceq "..") {
            if ($segments.Count -le 1) {
                throw "AppImage link '$LinkName' target escapes AppDir."
            }
            $segments.RemoveAt($segments.Count - 1)
            continue
        }
        $segments.Add($segment)
    }
    if ($segments.Count -eq 0) {
        throw "AppImage link '$LinkName' target escapes AppDir."
    }

    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $current = $fullRoot
    for ($index = 0; $index -lt $segments.Count; $index += 1) {
        $matches = @(
            Get-ChildItem -LiteralPath $current -Force |
                Where-Object Name -CEQ $segments[$index]
        )
        if ($matches.Count -ne 1) {
            throw "AppImage link '$LinkName' target must be an existing regular file."
        }
        $item = $matches[0]
        $targetRelative = [string]::Join(
            "/",
            @($segments.ToArray())[0..$index]
        )
        if (
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
            $AllLinks.ContainsKey($targetRelative)
        ) {
            throw "AppImage link '$LinkName' target contains a reparse component."
        }
        $isLeaf = $index -eq ($segments.Count - 1)
        if (($isLeaf -and $item.PSIsContainer) -or
            (-not $isLeaf -and -not $item.PSIsContainer)) {
            throw "AppImage link '$LinkName' target must be an existing regular file."
        }
        if (-not $isLeaf) {
            $current = $item.FullName
        }
    }
    return $item.FullName
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
    $expected = [ordered]@{
        ".DirIcon" = "WokRouter.png"
        "WokRouter.desktop" = "usr/share/applications/WokRouter.desktop"
    }
    $rawRootLinks = @(
        $inventory |
            Where-Object {
                $_.Relative -is [string] -and
                -not ([string] $_.Relative).Contains("/") -and
                -not ([string] $_.Relative).Contains("\")
            }
    )
    if ($rawRootLinks.Count -ne 2) {
        throw "AppImage must contain exactly two expected root links."
    }
    $records = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    $caseInsensitive = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $segmentsByRelative = [Collections.Generic.Dictionary[string, string[]]]::new(
        [StringComparer]::Ordinal
    )
    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
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
            $records.ContainsKey($relative) -or
            -not $caseInsensitive.Add($relative)
        ) {
            throw "AppImage link inventory contains a duplicate or case-alternate path."
        }
        if ([string] $record.LinkType -cne "SymbolicLink") {
            throw "AppImage reparse points must be symbolic links."
        }
        [string[]] $segments = @(
            Get-CanonicalAppImageRelativeSegments -Relative $relative
        )
        if ($forbidden.IsMatch($segments[$segments.Count - 1])) {
            throw "AppImage link inventory contains a forbidden payload."
        }
        $item = Get-AppImageContractItem `
            -Root $Root `
            -Segments $segments `
            -Relative $relative
        if ([string]::IsNullOrWhiteSpace($ToolAdapterPath)) {
            [object[]] $actualTargets = @($item.Target)
            if (
                ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq
                0 -or
                [string] $item.LinkType -cne "SymbolicLink" -or
                $actualTargets.Count -ne 1 -or
                $actualTargets[0] -isnot [string] -or
                [string] $actualTargets[0] -cne [string] $record.Target
            ) {
                throw "Native AppImage link metadata does not match its inventory."
            }
        }
        elseif ($item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Adapter AppImage link contract must be a regular file."
        }
        $records.Add($relative, $record)
        $segmentsByRelative.Add($relative, $segments)
    }

    [string[]] $rootLinks = @(
        $records.Keys | Where-Object { -not $_.Contains("/") }
    )
    if (
        $rootLinks.Count -ne 2 -or
        -not $records.ContainsKey(".DirIcon") -or
        -not $records.ContainsKey("WokRouter.desktop")
    ) {
        throw "AppImage must contain exactly two expected root links."
    }

    $targets = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($relative in $records.Keys) {
        $record = $records[$relative]
        $targetPath = Get-AppImageLinkTarget `
            -Root $Root `
            -LinkName $relative `
            -LinkSegments $segmentsByRelative[$relative] `
            -Target ([string] $record.Target) `
            -AllLinks $records
        if (
            $expected.Contains($relative) -and
            [string] $record.Target -cne [string] $expected[$relative]
        ) {
            throw "AppImage link '$relative' does not use its expected target."
        }
        $targets.Add($relative, $targetPath)
    }
    return [pscustomobject]@{
        Records = $records
        Targets = $targets
    }
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
        [Parameter(Mandatory)][string] $ExpectedArchitecture,
        [Collections.Generic.Dictionary[string, object]] $AllowedLinks
    )

    $null = Assert-RegularPath `
        -Path $Root `
        -Kind Directory `
        -Description "Extracted $Format"
    $items = @(
        Get-SafeTreeItems `
            -Root $Root `
            -Format $Format `
            -AllowedLinks $AllowedLinks
    )
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
            $fullSource = [IO.Path]::GetFullPath($Source).TrimEnd(
                [IO.Path]::DirectorySeparatorChar,
                [IO.Path]::AltDirectorySeparatorChar
            )
            $sourcePrefix = $fullSource + [IO.Path]::DirectorySeparatorChar
            $pending = [Collections.Generic.Stack[string]]::new()
            $pending.Push($fullSource)
            while ($pending.Count -gt 0) {
                $directory = $pending.Pop()
                foreach (
                    $item in Get-ChildItem -LiteralPath $directory -Force
                ) {
                    $fullItem = [IO.Path]::GetFullPath($item.FullName)
                    if (
                        -not $fullItem.StartsWith(
                            $sourcePrefix,
                            [StringComparison]::Ordinal
                        )
                    ) {
                        throw "AppImage link inventory escapes AppDir."
                    }
                    if (
                        ($item.Attributes -band
                            [IO.FileAttributes]::ReparsePoint) -ne 0
                    ) {
                        [object[]] $targets = @($item.Target)
                        if (
                            $targets.Count -ne 1 -or
                            $targets[0] -isnot [string]
                        ) {
                            throw "Could not read AppImage symbolic-link target."
                        }
                        $records.Add([pscustomobject]@{
                            Relative = (
                                $fullItem.Substring($sourcePrefix.Length).
                                    Replace(
                                        [IO.Path]::DirectorySeparatorChar,
                                        "/"
                                    )
                            )
                            LinkType = [string] $item.LinkType
                            Target = [string] $targets[0]
                        })
                        continue
                    }
                    if ($item.PSIsContainer) {
                        $pending.Push($item.FullName)
                    }
                }
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
            $extractOutput = @(& $bash `
                -o pipefail `
                -c @'
set -euo pipefail
package=$1
destination=$2
archive="${destination}.cpio"
rpm_error="${archive}.rpm2cpio.stderr"
cpio_error="${archive}.cpio.stderr"
if rpm2cpio "$package" > "$archive" 2> "$rpm_error"; then
  :
else
  status=$?
  cat "$rpm_error" >&2 2>/dev/null || true
  printf 'rpm2cpio exited with status %s.\n' "$status" >&2
  exit "$status"
fi
if (cd -- "$destination" &&
  cpio --extract --make-directories --no-absolute-filenames --quiet < "$archive" \
    2> "$cpio_error"
); then
  :
else
  status=$?
  cat "$cpio_error" >&2 2>/dev/null || true
  printf 'cpio exited with status %s.\n' "$status" >&2
  exit "$status"
fi
rm -f -- "$archive" "$rpm_error" "$cpio_error"
'@ `
                "wokrouter-rpm-extract" `
                $Source `
                $Destination 2>&1)
            $extractExitCode = $LASTEXITCODE
            if ($extractExitCode -ne 0) {
                $details = [string]::Join("`n", [string[]] $extractOutput)
                throw "Could not extract rpm (exit $extractExitCode). $details"
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
        [Parameter(Mandatory)][string] $Extension,
        [string[]] $AllowedAuxiliaryExtensions = @(),
        [string[]] $AllowedDirectories = @()
    )

    $directory = Join-Path $Root $Subdirectory
    $null = Assert-RegularPath `
        -Path $directory `
        -Kind Directory `
        -Description "$Subdirectory source directory"
    $sources = [Collections.Generic.List[object]]::new()
    $auxiliary = [Collections.Generic.List[object]]::new()
    $directories = [Collections.Generic.List[object]]::new()
    $unknown = [Collections.Generic.List[object]]::new()
    foreach ($item in @(Get-ChildItem -LiteralPath $directory -Force)) {
        $isReparse = ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        if ($isReparse) {
            $unknown.Add($item)
            continue
        }
        if ($item.PSIsContainer) {
            $directories.Add($item)
            continue
        }
        if ($item.Name.EndsWith($Extension, [StringComparison]::OrdinalIgnoreCase)) {
            $sources.Add($item)
            continue
        }
        $isAuxiliary = $false
        foreach ($auxiliaryExtension in $AllowedAuxiliaryExtensions) {
            if ($item.Name.EndsWith(
                    $auxiliaryExtension,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                $isAuxiliary = $true
                break
            }
        }
        if ($isAuxiliary) {
            $auxiliary.Add($item)
        } else {
            $unknown.Add($item)
        }
    }
    foreach ($item in $directories) {
        $isAllowedDirectory = $false
        foreach ($allowedDirectory in $AllowedDirectories) {
            if ($item.Name.Equals($allowedDirectory, [StringComparison]::Ordinal)) {
                $isAllowedDirectory = $true
                break
            }
        }
        if (-not $isAllowedDirectory) {
            foreach ($source in $sources) {
                $sourceStem = [IO.Path]::GetFileNameWithoutExtension($source.Name)
                if ($item.Name.Equals($sourceStem, [StringComparison]::Ordinal)) {
                    $isAllowedDirectory = $true
                    break
                }
            }
        }
        if (-not $isAllowedDirectory) {
            $unknown.Add($item)
        }
    }
    if ($sources.Count -ne 1 -or $auxiliary.Count -gt 1 -or $unknown.Count -ne 0) {
        $details = @(
            foreach ($item in @(Get-ChildItem -LiteralPath $directory -Force)) {
                $kind = if ($item.PSIsContainer) { "Directory" } else { "File" }
                $reparse = if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { "Reparse" } else { "Regular" }
                "$($item.Name):${kind}:${reparse}"
            }
        )
        [Array]::Sort($details, [StringComparer]::Ordinal)
        throw "$Subdirectory must contain exactly one regular $Extension source (entries: $([string]::Join('|', $details)))."
    }
    return $sources[0].FullName
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
        $expectedPackageNames -cnotcontains [string] $metadata.Name -or
        [string] $metadata.Version -cne $Version -or
        [string] $metadata.Architecture -cne $ExpectedArchitecture
    ) {
        throw "$Kind metadata does not match the release contract (Name='$([string] $metadata.Name)', Version='$([string] $metadata.Version)', Architecture='$([string] $metadata.Architecture)', ExpectedVersion='$Version', ExpectedArchitecture='$ExpectedArchitecture')."
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
$expectedPackageNames = @("wokrouter", "wok-router")

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
    -Extension ".AppImage" `
    -AllowedDirectories @("WokRouter.AppDir") `
    -AllowedAuxiliaryExtensions @(".zsync")
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
            -ExpectedArchitecture $architecture `
            -AllowedLinks $appImageLinks.Records
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
        -LiteralPath $appImageLinks.Targets["WokRouter.desktop"]
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
