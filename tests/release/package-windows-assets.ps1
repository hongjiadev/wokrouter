[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $BundleDirectory,
    [Parameter(Mandatory)][string] $DesktopExecutable,
    [Parameter(Mandatory)][string] $SidecarExecutable,
    [Parameter(Mandatory)][string] $RepositoryRoot,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $Target,
    [string] $ToolAdapterPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1") -Force
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

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
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $Description
    )

    foreach ($item in Get-ChildItem -LiteralPath $Root -Force -Recurse) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Description contains a reparse point."
        }
    }
}

function Get-PeArchitecture {
    param([Parameter(Mandatory)][string] $Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if (
        $bytes.Length -lt 64 -or
        $bytes[0] -ne [byte][char]"M" -or
        $bytes[1] -ne [byte][char]"Z"
    ) {
        throw "Windows executable is not a valid PE file."
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if (
        $peOffset -lt 0 -or
        $peOffset + 6 -gt $bytes.Length -or
        $bytes[$peOffset] -ne [byte][char]"P" -or
        $bytes[$peOffset + 1] -ne [byte][char]"E" -or
        $bytes[$peOffset + 2] -ne 0 -or
        $bytes[$peOffset + 3] -ne 0
    ) {
        throw "Windows executable has an invalid PE signature."
    }
    $machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
    switch ($machine) {
        0x8664 { return "x86_64" }
        0xaa64 { return "arm64" }
        default {
            throw ("Unsupported PE architecture 0x{0:x4}." -f $machine)
        }
    }
}

function Invoke-MsiRows {
    param(
        [Parameter(Mandatory)] $Database,
        [Parameter(Mandatory)][string] $Sql,
        [Parameter(Mandatory)][ValidateRange(1, 32)][int] $FieldCount
    )

    $view = $Database.OpenView($Sql)
    try {
        $null = $view.Execute()
        $rows = [Collections.Generic.List[object]]::new()
        while ($record = $view.Fetch()) {
            $fields = [Collections.Generic.List[string]]::new()
            for ($index = 1; $index -le $FieldCount; $index += 1) {
                $value = [string] $record.StringData($index)
                if ($value.Contains("|")) {
                    $value = $value.Substring($value.IndexOf("|") + 1)
                }
                $fields.Add($value)
            }
            $rows.Add([pscustomobject]@{ Fields = $fields.ToArray() })
        }
        return $rows.ToArray()
    }
    finally {
        $null = $view.Close()
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view) |
            Out-Null
    }
}

function Get-NativeMsiMetadata {
    param([Parameter(Mandatory)][string] $Path)

    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = $null
    try {
        $database = $installer.OpenDatabase($Path, 0)
        $propertyRows = @(
            Invoke-MsiRows `
                -Database $database `
                -Sql "SELECT ``Property``,``Value`` FROM ``Property``" `
                -FieldCount 2
        )
        $name = @(
            $propertyRows |
                Where-Object {
                    $_.Fields.Count -ge 2 -and
                    $_.Fields[0] -ceq "ProductName"
                } |
                ForEach-Object { [string] $_.Fields[1] }
        )
        $nativeVersion = @(
            $propertyRows |
                Where-Object {
                    $_.Fields.Count -ge 2 -and
                    $_.Fields[0] -ceq "ProductVersion"
                } |
                ForEach-Object { [string] $_.Fields[1] }
        )
        $fileRows = @(
            Invoke-MsiRows `
                -Database $database `
                -Sql "SELECT ``File``,``Component_``,``FileName`` FROM ``File``" `
                -FieldCount 3
        )
        $files = @(
            $fileRows |
                Where-Object { $_.Fields.Count -ge 3 } |
                ForEach-Object { [string] $_.Fields[2] }
        )
        if ($name.Count -ne 1 -or $nativeVersion.Count -ne 1) {
            throw "MSI product metadata is incomplete."
        }
        $summary = $database.SummaryInformation(0)
        try {
            $template = [string] $summary.Property(7)
        }
        finally {
            [Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary) |
                Out-Null
        }
        return @{
            Name = $name[0]
            Version = $nativeVersion[0]
            Template = $template
            Files = $files
        } | ConvertTo-Json -Compress
    }
    finally {
        if ($null -ne $database) {
            [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) |
                Out-Null
        }
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) |
            Out-Null
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
        "windows-msi-metadata" {
            return Get-NativeMsiMetadata -Path $Source
        }
        "windows-msi-extract" {
            [IO.Directory]::CreateDirectory($Destination) | Out-Null
            $log = Join-Path (Split-Path -Parent $Destination) "msiexec.log"
            $arguments = @(
                "/a",
                "`"$Source`"",
                "/qn",
                "TARGETDIR=`"$Destination`"",
                "/l*v",
                "`"$log`""
            )
            $process = Start-Process `
                -FilePath "msiexec.exe" `
                -ArgumentList $arguments `
                -Wait `
                -PassThru `
                -WindowStyle Hidden
            if ($process.ExitCode -ne 0) {
                throw "Could not extract MSI for inspection (exit $($process.ExitCode))."
            }
            return
        }
        default {
            throw "Unsupported native tool operation '$Operation'."
        }
    }
}

function Assert-NoForbiddenNames {
    param([Parameter(Mandatory)][string[]] $Names)

    $forbidden = [regex]::new(
        "(?i)(wokcore|wokrouterd|wokcore-provider-sim|wokcore-loadgen)"
    )
    if (@($Names | Where-Object { $forbidden.IsMatch($_) }).Count -gt 0) {
        throw "Windows package contains a forbidden payload."
    }
}

function Assert-SameFile {
    param(
        [Parameter(Mandatory)][string] $Expected,
        [Parameter(Mandatory)][string] $Actual,
        [Parameter(Mandatory)][string] $Description
    )

    $expectedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Expected).Hash
    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Actual).Hash
    if ($expectedHash -cne $actualHash) {
        throw "$Description bytes do not match the intended payload."
    }
}

$contract = @(
    Get-WokRouterTargetContracts -Version $Version |
        Where-Object Target -CEQ $Target
)
if ($contract.Count -ne 1 -or $contract[0].System -cne "Windows") {
    throw "Unsupported Windows release target '$Target'."
}
$architecture = [string] $contract[0].Architecture
$template = if ($architecture -ceq "x86_64") { "x64;0" } else { "Arm64;0" }

$bundle = (Assert-RegularPath `
    -Path $BundleDirectory `
    -Kind Directory `
    -Description "Bundle directory").FullName
$rootItems = @(Get-ChildItem -LiteralPath $bundle -Force)
if (
    $rootItems.Count -ne 1 -or
    -not $rootItems[0].PSIsContainer -or
    $rootItems[0].Name -cne "msi" -or
    ($rootItems[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
) {
    throw "Windows bundle must contain exactly one regular MSI source directory."
}
$msiItems = @(Get-ChildItem -LiteralPath $rootItems[0].FullName -Force)
if (
    $msiItems.Count -ne 1 -or
    $msiItems[0].PSIsContainer -or
    ($msiItems[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
    -not $msiItems[0].Name.EndsWith(
        ".msi",
        [StringComparison]::OrdinalIgnoreCase
    )
) {
    throw "MSI directory must contain exactly one regular .msi source."
}
$msi = $msiItems[0].FullName

$desktop = (Assert-RegularPath `
    -Path $DesktopExecutable `
    -Kind File `
    -Description "Desktop executable").FullName
$sidecar = (Assert-RegularPath `
    -Path $SidecarExecutable `
    -Kind File `
    -Description "Sidecar executable").FullName
foreach ($executable in @($desktop, $sidecar)) {
    if ((Get-PeArchitecture -Path $executable) -cne $architecture) {
        throw "Windows executable architecture does not match '$architecture'."
    }
}

$repository = (Assert-RegularPath `
    -Path $RepositoryRoot `
    -Kind Directory `
    -Description "Repository root").FullName
$documentNames = @(
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "NOTICE.md",
    "README.md"
)
$documents = @{}
foreach ($name in $documentNames) {
    $documents[$name] = (Assert-RegularPath `
        -Path (Join-Path $repository $name) `
        -Kind File `
        -Description "Release document '$name'").FullName
}

try {
    $metadata = Invoke-Adapter `
        -Operation "windows-msi-metadata" `
        -Source $msi |
        ConvertFrom-Json
}
catch {
    throw "MSI metadata inspection failed: $($_.Exception.Message)"
}
[string[]] $metadataFiles = @(
    $metadata.Files | ForEach-Object { [string] $_ }
)
Assert-NoForbiddenNames -Names $metadataFiles
[string[]] $wanted = @(
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "NOTICE.md",
    "README.md",
    "wokrouter-desktop.exe",
    "wokrouter.exe"
)
[string[]] $orderedMetadata = @($metadataFiles)
[Array]::Sort($orderedMetadata, [StringComparer]::Ordinal)
if (
    [string] $metadata.Name -cne "WokRouter" -or
    [string] $metadata.Version -cne $Version -or
    [string] $metadata.Template -cne $template
) {
    throw "MSI metadata does not match the release contract (Name='$([string] $metadata.Name)', Version='$([string] $metadata.Version)', Template='$([string] $metadata.Template)', ExpectedVersion='$Version', ExpectedTemplate='$template')."
}
if (
    [string]::Join("|", $orderedMetadata) -cne
    [string]::Join("|", $wanted)
) {
    throw "MSI payload inventory does not match the release contract."
}

$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporary = Join-Path $temporaryParent (
    "wokrouter-windows-package-" + [Guid]::NewGuid().ToString("N")
)
[IO.Directory]::CreateDirectory($temporary) | Out-Null
try {
    $extracted = Join-Path $temporary "msi"
    [IO.Directory]::CreateDirectory($extracted) | Out-Null
    $null = Invoke-Adapter `
        -Operation "windows-msi-extract" `
        -Source $msi `
        -Destination $extracted
    Assert-TreeSafe -Root $extracted -Description "Extracted MSI"
    $extractedFiles = @(Get-ChildItem -LiteralPath $extracted -Force -Recurse -File)
    [string[]] $extractedNames = @($extractedFiles | ForEach-Object Name)
    Assert-NoForbiddenNames -Names $extractedNames
    [string[]] $orderedExtracted = @($extractedNames)
    [Array]::Sort($orderedExtracted, [StringComparer]::Ordinal)
    [string[]] $expectedExtracted = @(
        $wanted + [IO.Path]::GetFileName($msi)
    )
    [Array]::Sort($expectedExtracted, [StringComparer]::Ordinal)
    if (
        [string]::Join("|", $orderedExtracted) -cne
        [string]::Join("|", $expectedExtracted)
    ) {
        throw (
            "Extracted MSI payload inventory does not match the contract: " +
            [string]::Join("|", $orderedExtracted)
        )
    }
    $byName = @{}
    foreach ($file in $extractedFiles) {
        if ($byName.ContainsKey($file.Name)) {
            throw "Extracted MSI payload inventory contains duplicate names."
        }
        $byName[$file.Name] = $file.FullName
    }
    Assert-SameFile `
        -Expected $desktop `
        -Actual $byName["wokrouter-desktop.exe"] `
        -Description "MSI desktop executable"
    Assert-SameFile `
        -Expected $sidecar `
        -Actual $byName["wokrouter.exe"] `
        -Description "MSI sidecar executable"
    foreach ($name in $documentNames) {
        Assert-SameFile `
            -Expected $documents[$name] `
            -Actual $byName[$name] `
            -Description "MSI document '$name'"
    }
    foreach ($name in @("wokrouter-desktop.exe", "wokrouter.exe")) {
        if ((Get-PeArchitecture -Path $byName[$name]) -cne $architecture) {
            throw "Extracted MSI executable architecture does not match."
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

    $prefix = "WokRouter-v$Version-Windows-$architecture"
    $msiOutput = Join-Path $output "$prefix.msi"
    $zipOutput = Join-Path $output "$prefix-Portable.zip"
    [IO.File]::Copy($msi, $msiOutput, $false)

    $archive = [IO.Compression.ZipFile]::Open(
        $zipOutput,
        [IO.Compression.ZipArchiveMode]::Create
    )
    try {
        foreach ($name in $documentNames) {
            $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                $documents[$name],
                $name,
                [IO.Compression.CompressionLevel]::Optimal
            )
        }
        $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive,
            $desktop,
            "wokrouter-desktop.exe",
            [IO.Compression.CompressionLevel]::Optimal
        )
        $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive,
            $sidecar,
            "wokrouter.exe",
            [IO.Compression.CompressionLevel]::Optimal
        )
    }
    finally {
        $archive.Dispose()
    }

    Write-Output $zipOutput
    Write-Output $msiOutput
}
finally {
    $fullTemporary = [IO.Path]::GetFullPath($temporary)
    if (
        $fullTemporary.StartsWith(
            $temporaryParent,
            [StringComparison]::OrdinalIgnoreCase
        ) -and
        [IO.Path]::GetFileName($fullTemporary) -cmatch (
            "^wokrouter-windows-package-[0-9a-f]{32}$"
        ) -and
        [IO.Directory]::Exists($fullTemporary)
    ) {
        [IO.Directory]::Delete($fullTemporary, $true)
    }
}
