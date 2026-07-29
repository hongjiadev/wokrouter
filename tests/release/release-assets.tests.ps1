[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1"
$linuxScript = Join-Path $PSScriptRoot "package-linux-assets.ps1"
$macScript = Join-Path $PSScriptRoot "package-macos-assets.ps1"
$windowsScript = Join-Path $PSScriptRoot "package-windows-assets.ps1"
$fixtureRoots = [Collections.Generic.List[string]]::new()
$failures = [Collections.Generic.List[string]]::new()
$scenarioCount = 0
$version = "0.1.1"
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).Path
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
    [IO.Path]::DirectorySeparatorChar,
    [IO.Path]::AltDirectorySeparatorChar
)

function New-FixtureRoot {
    $root = Join-Path $temporaryRoot (
        "wokrouter-release-assets-" + [Guid]::NewGuid().ToString("N")
    )
    [IO.Directory]::CreateDirectory($root) | Out-Null
    $script:fixtureRoots.Add($root)
    return $root
}

function Write-Utf8File {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][AllowEmptyString()][string] $Content
    )

    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrEmpty($parent)) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    [IO.File]::WriteAllText(
        $Path,
        $Content,
        [Text.UTF8Encoding]::new($false)
    )
}

function Write-MinimalPe {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][ValidateSet("x86_64", "arm64")]
        [string] $Architecture,
        [Parameter(Mandatory)][string] $Marker
    )

    $bytes = [byte[]]::new(160)
    $bytes[0] = [byte][char]"M"
    $bytes[1] = [byte][char]"Z"
    [BitConverter]::GetBytes([int] 128).CopyTo($bytes, 0x3c)
    $bytes[128] = [byte][char]"P"
    $bytes[129] = [byte][char]"E"
    $machine = if ($Architecture -ceq "x86_64") {
        [uint16] 0x8664
    } else {
        [uint16] 0xaa64
    }
    [BitConverter]::GetBytes($machine).CopyTo($bytes, 132)
    [Text.Encoding]::ASCII.GetBytes($Marker).CopyTo($bytes, 140)
    $parent = Split-Path -Parent $Path
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function New-ToolAdapter {
    param([Parameter(Mandatory)][string] $Root)

    $path = Join-Path $Root "native-tool-adapter.ps1"
    Write-Utf8File -Path $path -Content @'
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $Operation,
    [string] $Source,
    [string] $Destination
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$root = $env:WOKROUTER_RELEASE_FIXTURE_ROOT
if ([string]::IsNullOrWhiteSpace($root)) {
    throw "Fixture root is unavailable."
}
[IO.File]::AppendAllText(
    (Join-Path $root "adapter.log"),
    "$Operation|$Source|$Destination|$((Get-Location).Path)`n",
    [Text.UTF8Encoding]::new($false)
)
switch ($Operation) {
    "linux-deb-metadata" { Get-Content -Raw -Encoding UTF8 $Source }
    "linux-rpm-metadata" { Get-Content -Raw -Encoding UTF8 $Source }
    "linux-appimage-extract" {
        Copy-Item -LiteralPath (Join-Path $root "appimage-tree") `
            -Destination $Destination -Recurse
        if (Test-Path -LiteralPath (Join-Path $root "appimage-junction")) {
            $null = New-Item `
                -ItemType Junction `
                -Path (Join-Path $Destination ".DirIcon") `
                -Target (Join-Path $root "link-target")
        }
        if (Test-Path -LiteralPath (Join-Path $root "appimage-extra-reparse")) {
            $null = New-Item `
                -ItemType Junction `
                -Path (Join-Path $Destination "usr/lib/unlisted-link") `
                -Target (Join-Path $root "link-target")
        }
    }
    "linux-appimage-link-inventory" {
        Get-Content `
            -Raw `
            -Encoding UTF8 `
            (Join-Path $root "appimage-link-inventory.json")
    }
    "linux-deb-extract" {
        Copy-Item -LiteralPath (Join-Path $root "deb-tree") `
            -Destination $Destination -Recurse
        if (Test-Path -LiteralPath (Join-Path $root "deb-reparse")) {
            $null = New-Item `
                -ItemType Junction `
                -Path (Join-Path $Destination "unsafe-link") `
                -Target (Join-Path $root "link-target")
        }
    }
    "linux-rpm-extract" {
        Copy-Item -LiteralPath (Join-Path $root "rpm-tree") `
            -Destination $Destination -Recurse
        if (Test-Path -LiteralPath (Join-Path $root "rpm-reparse")) {
            $null = New-Item `
                -ItemType Junction `
                -Path (Join-Path $Destination "unsafe-link") `
                -Target (Join-Path $root "link-target")
        }
    }
    "binary-architecture" {
        $text = Get-Content -Raw -Encoding UTF8 $Source
        if ($text.Contains("arm64")) { "arm64" } else { "x86_64" }
    }
    "mac-app-metadata" {
        Get-Content -Raw -Encoding UTF8 (
            Join-Path $Source "Contents/Info.metadata.json"
        )
    }
    "mac-app-inventory" {
        $inventory = if (
            $Source.StartsWith(
                (Join-Path $root "dmg-mount"),
                [StringComparison]::OrdinalIgnoreCase
            )
        ) {
            "mounted-app-inventory.json"
        } else {
            "source-app-inventory.json"
        }
        Get-Content -Raw -Encoding UTF8 (Join-Path $root $inventory)
    }
    "mac-dmg-root-inventory" {
        $override = Join-Path $root "dmg-root-inventory.json"
        if (Test-Path -LiteralPath $override -PathType Leaf) {
            Get-Content -Raw -Encoding UTF8 $override
            break
        }
        @(
            Get-ChildItem -LiteralPath $Source -Force |
                ForEach-Object {
                    [pscustomobject]@{
                        Kind = if ($_.PSIsContainer) { "Directory" } else { "File" }
                        Name = $_.Name
                        Target = $null
                    }
                }
        ) | ConvertTo-Json -Compress
    }
    "mac-attach" { Join-Path $root "dmg-mount" }
    "mac-detach" { }
    "mac-create-tar" {
        Write-Utf8File -Path $Destination -Content "tar:$Source"
    }
    "mac-create-zip" {
        Write-Utf8File -Path $Destination -Content "zip:$Source"
    }
    "mac-lipo-architecture" {
        $text = Get-Content -Raw -Encoding UTF8 $Source
        if ($text.Contains("arm64")) { "arm64" } else { "x86_64" }
    }
    "windows-msi-metadata" { Get-Content -Raw -Encoding UTF8 $Source }
    "windows-msi-extract" {
        Copy-Item -Path (Join-Path $root "msi-payload/*") `
            -Destination $Destination -Recurse
    }
    default { throw "Unexpected native tool operation '$Operation'." }
}
'@
    return $path
}

function Copy-ReleaseDocuments {
    param(
        [Parameter(Mandatory)][string] $Destination,
        [string] $SourceRoot = $repositoryRoot
    )

    [IO.Directory]::CreateDirectory($Destination) | Out-Null
    foreach ($name in @(
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "NOTICE.md",
            "README.md"
        )) {
        [IO.File]::Copy(
            (Join-Path $SourceRoot $name),
            (Join-Path $Destination $name)
        )
    }
}

function Write-AppImageLinkInventory {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][object[]] $Inventory
    )

    Write-Utf8File `
        -Path (Join-Path $Root "appimage-link-inventory.json") `
        -Content ($Inventory | ConvertTo-Json -Compress)
}

function Read-AppImageLinkInventory {
    param([Parameter(Mandatory)][string] $Root)

    return @(
        Get-Content `
            -Raw `
            -Encoding UTF8 `
            -LiteralPath (Join-Path $Root "appimage-link-inventory.json") |
            ConvertFrom-Json |
            ForEach-Object { $_ }
    )
}

function New-LinuxFixture {
    param(
        [Parameter(Mandatory)][string] $Root,
        [ValidateSet("x86_64", "arm64")][string] $Architecture = "x86_64"
    )

    $bundle = Join-Path $Root "bundle"
    foreach ($kind in @("appimage", "deb", "rpm")) {
        [IO.Directory]::CreateDirectory((Join-Path $bundle $kind)) | Out-Null
    }
    Write-Utf8File `
        -Path (Join-Path $bundle "appimage/WokRouter.AppImage") `
        -Content "appimage-$Architecture"
    $debArchitecture = if ($Architecture -ceq "x86_64") { "amd64" } else { "arm64" }
    $rpmArchitecture = if ($Architecture -ceq "x86_64") { "x86_64" } else { "aarch64" }
    Write-Utf8File `
        -Path (Join-Path $bundle "deb/WokRouter.deb") `
        -Content (@{
            Name = "wokrouter"
            Version = $version
            Architecture = $debArchitecture
        } | ConvertTo-Json -Compress)
    Write-Utf8File `
        -Path (Join-Path $bundle "rpm/WokRouter.rpm") `
        -Content (@{
            Name = "wokrouter"
            Version = $version
            Architecture = $rpmArchitecture
        } | ConvertTo-Json -Compress)
    $appRoot = Join-Path $Root "appimage-tree"
    Write-Utf8File `
        -Path (Join-Path $appRoot "usr/bin/wokrouter-desktop") `
        -Content "desktop-$Architecture"
    Write-Utf8File `
        -Path (Join-Path $appRoot "usr/bin/wokrouter") `
        -Content "sidecar-$Architecture"
    Copy-ReleaseDocuments -Destination (Join-Path $appRoot "usr/share/wokrouter")
    Write-Utf8File `
        -Path (Join-Path $appRoot "WokRouter.png") `
        -Content "icon"
    Write-Utf8File `
        -Path (
            Join-Path `
                $appRoot `
                "usr/share/applications/WokRouter.desktop"
        ) `
        -Content "X-AppImage-Version=$version"
    $linuxTriplet = if ($Architecture -ceq "x86_64") {
        "x86_64-linux-gnu"
    } else {
        "aarch64-linux-gnu"
    }
    $gtkLink = "usr/lib/gtk-3.0/3.0.0/immodules/im-ibus.so"
    Write-Utf8File `
        -Path (
            Join-Path `
                $appRoot `
                "usr/lib/$linuxTriplet/gtk-3.0/3.0.0/immodules/im-ibus.so"
        ) `
        -Content "gtk-im-module-$Architecture"
    foreach ($relative in @(".DirIcon", "WokRouter.desktop", $gtkLink)) {
        Write-Utf8File `
            -Path (Join-Path $appRoot $relative) `
            -Content "adapter-link-$relative"
    }
    Write-AppImageLinkInventory -Root $Root -Inventory @(
        [pscustomobject]@{
            Relative = ".DirIcon"
            LinkType = "SymbolicLink"
            Target = "WokRouter.png"
        },
        [pscustomobject]@{
            Relative = "WokRouter.desktop"
            LinkType = "SymbolicLink"
            Target = "usr/share/applications/WokRouter.desktop"
        },
        [pscustomobject]@{
            Relative = $gtkLink
            LinkType = "SymbolicLink"
            Target = "../../../$linuxTriplet/gtk-3.0/3.0.0/immodules/im-ibus.so"
        }
    )
    foreach ($kind in @("deb", "rpm")) {
        $payloadRoot = Join-Path $Root "$kind-tree"
        Write-Utf8File `
            -Path (Join-Path $payloadRoot "usr/bin/wokrouter-desktop") `
            -Content "desktop-$Architecture"
        Write-Utf8File `
            -Path (Join-Path $payloadRoot "usr/bin/wokrouter") `
            -Content "sidecar-$Architecture"
        Copy-ReleaseDocuments `
            -Destination (Join-Path $payloadRoot "usr/share/wokrouter")
    }
    return $bundle
}

function Get-MacFixtureInventory {
    param([Parameter(Mandatory)][string] $App)

    $records = [Collections.Generic.List[object]]::new()
    foreach ($item in Get-ChildItem -LiteralPath $App -Force -Recurse) {
        $relative = $item.FullName.Substring($App.Length).TrimStart(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        ).Replace([IO.Path]::DirectorySeparatorChar, "/")
        if ($item.PSIsContainer) {
            $records.Add([pscustomobject]@{
                Kind = "Directory"
                Relative = $relative
                Target = $null
                Sha256 = $null
            })
        } else {
            $records.Add([pscustomobject]@{
                Kind = "File"
                Relative = $relative
                Target = $null
                Sha256 = (
                    Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName
                ).Hash
            })
        }
    }
    return $records.ToArray()
}

function Write-MacInventoryOverride {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][ValidateSet("source", "mounted")]
        [string] $Kind,
        [Parameter(Mandatory)][object[]] $Inventory
    )

    Write-Utf8File `
        -Path (Join-Path $Root "$Kind-app-inventory.json") `
        -Content ($Inventory | ConvertTo-Json -Compress -Depth 4)
}

function New-MacFixture {
    param(
        [Parameter(Mandatory)][string] $Root,
        [ValidateSet("x86_64", "arm64")][string] $Architecture = "x86_64"
    )

    $bundle = Join-Path $Root "bundle"
    [IO.Directory]::CreateDirectory((Join-Path $bundle "dmg")) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $bundle "macos")) | Out-Null
    Write-Utf8File -Path (Join-Path $bundle "dmg/WokRouter.dmg") -Content "dmg"
    $app = Join-Path $bundle "macos/WokRouter.app"
    Write-Utf8File `
        -Path (Join-Path $app "Contents/MacOS/wokrouter-desktop") `
        -Content "desktop-$Architecture"
    Write-Utf8File `
        -Path (Join-Path $app "Contents/MacOS/wokrouter") `
        -Content "sidecar-$Architecture"
    Write-Utf8File `
        -Path (Join-Path $app "Contents/Info.metadata.json") `
        -Content (@{
            CFBundleIdentifier = "dev.wokrouter.desktop"
            CFBundleExecutable = "wokrouter-desktop"
            CFBundleShortVersionString = $version
            CFBundleName = "WokRouter"
        } | ConvertTo-Json -Compress)
    Copy-ReleaseDocuments -Destination (Join-Path $app "Contents/Resources")

    $mountedApp = Join-Path $Root "dmg-mount/WokRouter.app"
    Copy-Item -LiteralPath $app -Destination $mountedApp -Recurse
    Write-MacInventoryOverride `
        -Root $Root `
        -Kind source `
        -Inventory @(Get-MacFixtureInventory -App $app)
    Write-MacInventoryOverride `
        -Root $Root `
        -Kind mounted `
        -Inventory @(Get-MacFixtureInventory -App $mountedApp)
    return $bundle
}

function New-WindowsFixture {
    param(
        [Parameter(Mandatory)][string] $Root,
        [ValidateSet("x86_64", "arm64")][string] $Architecture = "x86_64"
    )

    $bundle = Join-Path $Root "bundle"
    [IO.Directory]::CreateDirectory((Join-Path $bundle "msi")) | Out-Null
    $template = if ($Architecture -ceq "x86_64") { "x64;1033" } else { "Arm64;1033" }
    $msi = Join-Path $bundle "msi/WokRouter.msi"
    Write-Utf8File -Path $msi -Content (@{
        Name = "WokRouter"
        Version = $version
        Template = $template
        Files = @(
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "NOTICE.md",
            "README.md",
            "wokrouter-desktop.exe",
            "wokrouter.exe"
        )
    } | ConvertTo-Json -Compress)
    $desktop = Join-Path $Root "wokrouter-desktop.exe"
    $sidecar = Join-Path $Root "wokrouter.exe"
    Write-MinimalPe -Path $desktop -Architecture $Architecture -Marker "desktop"
    Write-MinimalPe -Path $sidecar -Architecture $Architecture -Marker "sidecar"
    $payload = Join-Path $Root "msi-payload"
    Copy-ReleaseDocuments -Destination $payload
    [IO.File]::Copy($desktop, (Join-Path $payload "wokrouter-desktop.exe"))
    [IO.File]::Copy($sidecar, (Join-Path $payload "wokrouter.exe"))
    return [pscustomobject]@{
        Bundle = $bundle
        Desktop = $desktop
        Sidecar = $sidecar
    }
}

function Invoke-Packager {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][hashtable] $Arguments,
        [Parameter(Mandatory)][string] $FixtureRoot
    )

    $previousFixture = $env:WOKROUTER_RELEASE_FIXTURE_ROOT
    try {
        $env:WOKROUTER_RELEASE_FIXTURE_ROOT = $FixtureRoot
        return @(& $Path @Arguments)
    }
    finally {
        $env:WOKROUTER_RELEASE_FIXTURE_ROOT = $previousFixture
    }
}

function Assert-Rejects {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][hashtable] $Arguments,
        [Parameter(Mandatory)][string] $FixtureRoot,
        [Parameter(Mandatory)][string] $ExpectedText
    )

    try {
        $null = Invoke-Packager `
            -Path $Path `
            -Arguments $Arguments `
            -FixtureRoot $FixtureRoot
    }
    catch {
        if ($_.Exception.Message -notmatch [regex]::Escape($ExpectedText)) {
            throw "Expected '$ExpectedText', got '$($_.Exception.Message)'."
        }
        return
    }
    throw "Expected packaging to reject '$ExpectedText'."
}

function Invoke-Scenario {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][scriptblock] $Test
    )

    $script:scenarioCount += 1
    try {
        & $Test
        Write-Host "PASS: $Name"
    }
    catch {
        $script:failures.Add("${Name}: $($_.Exception.Message)")
        Write-Host "FAIL: $Name"
    }
}

try {
    Import-Module $modulePath -Force

    Invoke-Scenario -Name "contract returns exact ordinal 6/16 sequences" -Test {
        $expectedTargets = @(
            "aarch64-apple-darwin",
            "aarch64-pc-windows-msvc",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu"
        )
        $expectedPayloads = @(
            "WokRouter-v0.1.1-Linux-arm64.AppImage",
            "WokRouter-v0.1.1-Linux-arm64.deb",
            "WokRouter-v0.1.1-Linux-arm64.rpm",
            "WokRouter-v0.1.1-Linux-x86_64.AppImage",
            "WokRouter-v0.1.1-Linux-x86_64.deb",
            "WokRouter-v0.1.1-Linux-x86_64.rpm",
            "WokRouter-v0.1.1-Windows-arm64-Portable.zip",
            "WokRouter-v0.1.1-Windows-arm64.msi",
            "WokRouter-v0.1.1-Windows-x86_64-Portable.zip",
            "WokRouter-v0.1.1-Windows-x86_64.msi",
            "WokRouter-v0.1.1-macOS-arm64.dmg",
            "WokRouter-v0.1.1-macOS-arm64.tar.gz",
            "WokRouter-v0.1.1-macOS-arm64.zip",
            "WokRouter-v0.1.1-macOS-x86_64.dmg",
            "WokRouter-v0.1.1-macOS-x86_64.tar.gz",
            "WokRouter-v0.1.1-macOS-x86_64.zip"
        )
        $actualTargets = @(
            Get-WokRouterTargetContracts -Version $version |
                ForEach-Object Target
        )
        $actualPayloads = @(Get-WokRouterPayloadNames -Version $version)
        if ([string]::Join("`n", $actualTargets) -cne [string]::Join("`n", $expectedTargets)) {
            throw "Target ordering differs from the exact ordinal contract."
        }
        if ([string]::Join("`n", $actualPayloads) -cne [string]::Join("`n", $expectedPayloads)) {
            throw "Payload ordering differs from the exact ordinal contract."
        }
        if ($actualPayloads -match "unknown|pc-windows|apple-darwin") {
            throw "Public WokRouter names expose an internal target segment."
        }
    }

    Invoke-Scenario -Name "Tauri bundles the four public release documents" -Test {
        $configuration = Get-Content `
            -Raw `
            -Encoding UTF8 `
            -LiteralPath (
                Join-Path $repositoryRoot "apps/desktop/src-tauri/tauri.conf.json"
            ) |
            ConvertFrom-Json
        foreach ($name in @(
                "LICENSE-APACHE",
                "LICENSE-MIT",
                "NOTICE.md",
                "README.md"
            )) {
            $source = "../../../$name"
            $property = $configuration.bundle.resources.PSObject.Properties[
                $source
            ]
            if ($null -eq $property -or [string] $property.Value -cne $name) {
                throw "Tauri bundle resource mapping is missing '$name'."
            }
        }
    }

    Invoke-Scenario -Name "Linux production recursively inventories AppImage links without following them" -Test {
        $source = Get-Content -Raw -Encoding UTF8 -LiteralPath $linuxScript
        $inventoryBlock = [regex]::Match(
            $source,
            '(?s)"linux-appimage-link-inventory"\s*\{(?<Body>.*?)' +
            '(?=\r?\n\s*}\r?\n\s*"linux-deb-extract")'
        )
        if (-not $inventoryBlock.Success) {
            throw "Linux production link inventory block is unavailable."
        }
        $body = $inventoryBlock.Groups["Body"].Value
        foreach ($required in @(
                '$item.LinkType',
                '$item.Target',
                '[Collections.Generic.Stack[string]]',
                'Get-ChildItem -LiteralPath $directory -Force'
            )) {
            if (-not $body.Contains($required)) {
                throw "Linux production link inventory is missing '$required'."
            }
        }
        if ($body.Contains("-Recurse")) {
            throw "Linux production link inventory must not use recursive traversal."
        }
    }

    Invoke-Scenario -Name "Linux accepts exact x86_64 Tauri AppImage links" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $output = Join-Path $root "output"
        $actual = Invoke-Packager -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = $output
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        }
        $expected = @(
            "WokRouter-v0.1.1-Linux-x86_64.AppImage",
            "WokRouter-v0.1.1-Linux-x86_64.deb",
            "WokRouter-v0.1.1-Linux-x86_64.rpm"
        )
        [string[]] $names = @(
            Get-ChildItem -LiteralPath $output -File | ForEach-Object Name
        )
        [Array]::Sort($names, [StringComparer]::Ordinal)
        if ([string]::Join("|", $names) -cne [string]::Join("|", $expected)) {
            throw "Linux output inventory is not exact."
        }
        if ($actual.Count -ne 3) { throw "Linux packager returned the wrong output count." }
        $extractRecord = @(
            Get-Content -Encoding UTF8 (Join-Path $root "adapter.log") |
                Where-Object {
                    $_.StartsWith(
                        "linux-appimage-extract|",
                        [StringComparison]::Ordinal
                    )
                }
        )
        if ($extractRecord.Count -ne 1) {
            throw "Linux extraction adapter was not called exactly once."
        }
        $extractFields = $extractRecord[0].Split("|")
        if ($extractFields.Count -ne 4) {
            throw "Linux extraction adapter did not record its working directory."
        }
        $extractWorkingDirectory = [IO.Path]::GetFullPath($extractFields[3])
        $extractParent = [IO.Directory]::GetParent(
            $extractWorkingDirectory
        ).FullName.TrimEnd(
            [IO.Path]::DirectorySeparatorChar,
            [IO.Path]::AltDirectorySeparatorChar
        )
        if (
            -not $extractParent.Equals(
                $temporaryRoot,
                [StringComparison]::OrdinalIgnoreCase
            ) -or
            [IO.Path]::GetFileName($extractWorkingDirectory) -cnotmatch (
                "^wokrouter-linux-package-[0-9a-f]{32}$"
            )
        ) {
            throw "Linux extraction did not run in its validated temporary root."
        }
        foreach ($operation in @("linux-deb-extract", "linux-rpm-extract")) {
            if (
                @(
                    Get-Content -Encoding UTF8 (Join-Path $root "adapter.log") |
                        Where-Object {
                            $_.StartsWith(
                                "$operation|",
                                [StringComparison]::Ordinal
                            )
                        }
                ).Count -ne 1
            ) {
                throw "$operation was not called exactly once."
            }
        }
        if (Test-Path -LiteralPath $extractWorkingDirectory) {
            throw "Linux temporary extraction root was not cleaned after success."
        }
    }

    Invoke-Scenario -Name "Linux rejects missing duplicate extra and directory sources" -Test {
        foreach ($mutation in @("missing", "duplicate", "extra", "directory")) {
            $root = New-FixtureRoot
            $bundle = New-LinuxFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            switch ($mutation) {
                "missing" {
                    Remove-Item -LiteralPath (Join-Path $bundle "rpm/WokRouter.rpm")
                }
                "duplicate" {
                    Copy-Item `
                        -LiteralPath (Join-Path $bundle "deb/WokRouter.deb") `
                        -Destination (Join-Path $bundle "deb/duplicate.deb")
                }
                "extra" {
                    Write-Utf8File `
                        -Path (Join-Path $bundle "appimage/unexpected.txt") `
                        -Content "extra"
                }
                "directory" {
                    Remove-Item -LiteralPath (Join-Path $bundle "appimage/WokRouter.AppImage")
                    [IO.Directory]::CreateDirectory(
                        (Join-Path $bundle "appimage/WokRouter.AppImage")
                    ) | Out-Null
                }
            }
            Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-unknown-linux-gnu"
                ToolAdapterPath = $adapter
            } -ExpectedText "exactly one regular"
        }
    }

    Invoke-Scenario -Name "Linux rejects wrong metadata architecture and forbidden AppDir" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $deb = Join-Path $bundle "deb/WokRouter.deb"
        $metadata = Get-Content -Raw -Encoding UTF8 $deb | ConvertFrom-Json
        $metadata.Version = "0.1.0"
        Write-Utf8File -Path $deb -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "metadata"

        $metadata.Version = $version
        $metadata.Architecture = "arm64"
        Write-Utf8File -Path $deb -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "metadata"

        $metadata.Architecture = "amd64"
        Write-Utf8File -Path $deb -Content ($metadata | ConvertTo-Json -Compress)
        Write-Utf8File `
            -Path (Join-Path $root "appimage-tree/usr/bin/wokcore") `
            -Content "forbidden"
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "forbidden"
    }

    foreach ($kind in @("deb", "rpm")) {
        foreach ($mutation in @(
                "missing-sidecar",
                "wrong-architecture",
                "forbidden",
                "missing-document",
                "wrong-document",
                "case-alternate",
                "reparse"
            )) {
            Invoke-Scenario -Name "Linux rejects $kind $mutation content" -Test {
                $root = New-FixtureRoot
                $bundle = New-LinuxFixture -Root $root
                $adapter = New-ToolAdapter -Root $root
                $tree = Join-Path $root "$kind-tree"
                $expected = switch ($mutation) {
                    "missing-sidecar" {
                        Remove-Item -LiteralPath (Join-Path $tree "usr/bin/wokrouter")
                        "inventory"
                    }
                    "wrong-architecture" {
                        Write-Utf8File `
                            -Path (Join-Path $tree "usr/bin/wokrouter") `
                            -Content "sidecar-arm64"
                        "architecture"
                    }
                    "forbidden" {
                        Write-Utf8File `
                            -Path (Join-Path $tree "usr/bin/wokcore") `
                            -Content "forbidden"
                        "forbidden"
                    }
                    "missing-document" {
                        Remove-Item -LiteralPath (
                            Join-Path $tree "usr/share/wokrouter/README.md"
                        )
                        "inventory"
                    }
                    "wrong-document" {
                        Write-Utf8File `
                            -Path (Join-Path $tree "usr/share/wokrouter/NOTICE.md") `
                            -Content "wrong document"
                        "byte-identical"
                    }
                    "case-alternate" {
                        Write-Utf8File `
                            -Path (Join-Path $tree "alternate/README.MD") `
                            -Content "alternate"
                        "inventory"
                    }
                    "reparse" {
                        [IO.Directory]::CreateDirectory(
                            (Join-Path $root "link-target")
                        ) | Out-Null
                        Write-Utf8File `
                            -Path (Join-Path $root "$kind-reparse") `
                            -Content "create reparse fixture"
                        "reparse"
                    }
                }
                Assert-Rejects `
                    -Path $linuxScript `
                    -FixtureRoot $root `
                    -Arguments @{
                        BundleDirectory = $bundle
                        OutputDirectory = (Join-Path $root "output")
                        Version = $version
                        Target = "x86_64-unknown-linux-gnu"
                        ToolAdapterPath = $adapter
                    } `
                    -ExpectedText $expected
            }
        }
    }

    foreach ($mutation in @(
            "missing-diricon",
            "missing-desktop",
            "absolute-target",
            "escape-target",
            "broken-target",
            "wrong-in-root-target",
            "extra-root-link",
            "nested-link",
            "case-alternate",
            "duplicate",
            "junction"
        )) {
        Invoke-Scenario -Name "Linux rejects AppImage link $mutation" -Test {
            $root = New-FixtureRoot
            $bundle = New-LinuxFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
            $expected = switch ($mutation) {
                "missing-diricon" {
                    $inventory = @(
                        $inventory |
                            Where-Object Relative -CNE ".DirIcon"
                    )
                    "exactly"
                }
                "missing-desktop" {
                    $inventory = @(
                        $inventory |
                            Where-Object Relative -CNE "WokRouter.desktop"
                    )
                    "exactly"
                }
                "absolute-target" {
                    $inventory[0].Target = "/tmp/WokRouter.png"
                    "relative"
                }
                "escape-target" {
                    $inventory[0].Target = "../WokRouter.png"
                    "escapes"
                }
                "broken-target" {
                    Remove-Item -LiteralPath (
                        Join-Path $root "appimage-tree/WokRouter.png"
                    )
                    "regular file"
                }
                "wrong-in-root-target" {
                    Write-Utf8File `
                        -Path (
                            Join-Path $root "appimage-tree/Other.png"
                        ) `
                        -Content "wrong icon"
                    $inventory[0].Target = "Other.png"
                    "expected target"
                }
                "extra-root-link" {
                    $inventory += [pscustomobject]@{
                        Relative = "Unexpected"
                        LinkType = "SymbolicLink"
                        Target = "WokRouter.png"
                    }
                    "exactly"
                }
                "nested-link" {
                    $inventory[0].Relative = "usr/share/.DirIcon"
                    "root"
                }
                "case-alternate" {
                    $inventory[0].Relative = ".diricon"
                    "case-sensitive"
                }
                "duplicate" {
                    $inventory += [pscustomobject]@{
                        Relative = ".DirIcon"
                        LinkType = "SymbolicLink"
                        Target = "WokRouter.png"
                    }
                    "exactly"
                }
                "junction" {
                    [IO.Directory]::CreateDirectory(
                        (Join-Path $root "link-target")
                    ) | Out-Null
                    Remove-Item -LiteralPath (
                        Join-Path $root "appimage-tree/.DirIcon"
                    )
                    Write-Utf8File `
                        -Path (Join-Path $root "appimage-junction") `
                        -Content "create junction fixture"
                    $inventory[0].LinkType = "Junction"
                    "symbolic link"
                }
            }
            Write-AppImageLinkInventory `
                -Root $root `
                -Inventory $inventory
            Assert-Rejects `
                -Path $linuxScript `
                -FixtureRoot $root `
                -Arguments @{
                    BundleDirectory = $bundle
                    OutputDirectory = (Join-Path $root "output")
                    Version = $version
                    Target = "x86_64-unknown-linux-gnu"
                    ToolAdapterPath = $adapter
                } `
                    -ExpectedText $expected
        }
    }

    foreach ($case in @(
            @{
                Name = "absolute target"
                Target = "/tmp/im-ibus.so"
                Expected = "relative"
            },
            @{
                Name = "rooted target"
                Target = "\tmp\im-ibus.so"
                Expected = "relative"
            },
            @{
                Name = "drive target"
                Target = "C:/tmp/im-ibus.so"
                Expected = "relative"
            },
            @{
                Name = "UNC target"
                Target = "//server/share/im-ibus.so"
                Expected = "relative"
            },
            @{
                Name = "lexical escape"
                Target = "../../../../../outside/im-ibus.so"
                Expected = "escapes"
            },
            @{
                Name = "broken target"
                Target = "../../../x86_64-linux-gnu/gtk-3.0/3.0.0/immodules/missing.so"
                Expected = "regular file"
            },
            @{
                Name = "directory leaf"
                Target = "../../../x86_64-linux-gnu/gtk-3.0/3.0.0/immodules"
                Expected = "regular file"
            }
        )) {
        Invoke-Scenario -Name "Linux rejects nested AppImage $($case.Name)" -Test {
            $root = New-FixtureRoot
            $bundle = New-LinuxFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
            $nested = @(
                $inventory |
                    Where-Object Relative -CEQ (
                        "usr/lib/gtk-3.0/3.0.0/immodules/im-ibus.so"
                    )
            )
            if ($nested.Count -ne 1) {
                throw "Nested GTK link fixture is not exact."
            }
            $nested[0].Target = [string] $case.Target
            Write-AppImageLinkInventory -Root $root -Inventory $inventory
            Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-unknown-linux-gnu"
                ToolAdapterPath = $adapter
            } -ExpectedText ([string] $case.Expected)
        }
    }

    Invoke-Scenario -Name "Linux rejects nested AppImage link target chains" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
        $targetRelative = (
            "usr/lib/x86_64-linux-gnu/" +
            "gtk-3.0/3.0.0/immodules/im-ibus.so"
        )
        Write-Utf8File `
            -Path (
                Join-Path `
                    $root `
                    (
                        "appimage-tree/usr/lib/x86_64-linux-gnu/" +
                        "gtk-3.0/3.0.0/immodules/im-ibus-real.so"
                    )
            ) `
            -Content "real-gtk-module"
        $inventory += [pscustomobject]@{
            Relative = $targetRelative
            LinkType = "SymbolicLink"
            Target = "im-ibus-real.so"
        }
        Write-AppImageLinkInventory -Root $root -Inventory $inventory
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "reparse component"
    }

    Invoke-Scenario -Name "Linux rejects non-symbolic nested AppImage reparse records" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
        $inventory[2].LinkType = "Junction"
        Write-AppImageLinkInventory -Root $root -Inventory $inventory
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "symbolic link"
    }

    foreach ($case in @(
            @{
                Name = "rooted path"
                Relative = "/usr/lib/im-ibus.so"
            },
            @{
                Name = "backslash path"
                Relative = "usr\lib\im-ibus.so"
            },
            @{
                Name = "drive path"
                Relative = "C:/usr/lib/im-ibus.so"
            },
            @{
                Name = "empty segment"
                Relative = "usr//lib/im-ibus.so"
            },
            @{
                Name = "dot segment"
                Relative = "usr/./lib/im-ibus.so"
            },
            @{
                Name = "dot-dot segment"
                Relative = "usr/lib/../lib/im-ibus.so"
            },
            @{
                Name = "control character"
                Relative = "usr/lib/im`nbus.so"
            }
        )) {
        Invoke-Scenario -Name "Linux rejects AppImage inventory $($case.Name)" -Test {
            $root = New-FixtureRoot
            $bundle = New-LinuxFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
            $inventory[2].Relative = [string] $case.Relative
            Write-AppImageLinkInventory -Root $root -Inventory $inventory
            Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-unknown-linux-gnu"
                ToolAdapterPath = $adapter
            } -ExpectedText "path"
        }
    }

    foreach ($mutation in @("duplicate", "case-alternate")) {
        Invoke-Scenario -Name "Linux rejects AppImage nested $mutation inventory" -Test {
            $root = New-FixtureRoot
            $bundle = New-LinuxFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
            $duplicate = [pscustomobject]@{
                Relative = [string] $inventory[2].Relative
                LinkType = "SymbolicLink"
                Target = [string] $inventory[2].Target
            }
            if ($mutation -ceq "case-alternate") {
                $duplicate.Relative = $duplicate.Relative.Replace(
                    "usr/",
                    "USR/"
                )
            }
            $inventory += $duplicate
            Write-AppImageLinkInventory -Root $root -Inventory $inventory
            Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-unknown-linux-gnu"
                ToolAdapterPath = $adapter
            } -ExpectedText "duplicate"
        }
    }

    Invoke-Scenario -Name "Linux rejects forbidden AppImage link names" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        [object[]] $inventory = @(Read-AppImageLinkInventory -Root $root)
        $forbiddenRelative = "usr/lib/wokcore-provider.so"
        Write-Utf8File `
            -Path (Join-Path $root "appimage-tree/$forbiddenRelative") `
            -Content "adapter-link-forbidden"
        $inventory += [pscustomobject]@{
            Relative = $forbiddenRelative
            LinkType = "SymbolicLink"
            Target = (
                "x86_64-linux-gnu/" +
                "gtk-3.0/3.0.0/immodules/im-ibus.so"
            )
        }
        Write-AppImageLinkInventory -Root $root -Inventory $inventory
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "forbidden"
    }

    Invoke-Scenario -Name "Linux rejects AppImage reparse points omitted from adapter inventory" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        [IO.Directory]::CreateDirectory((Join-Path $root "link-target")) |
            Out-Null
        Write-Utf8File `
            -Path (Join-Path $root "appimage-extra-reparse") `
            -Content "create unlisted reparse fixture"
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "reparse"
    }

    Invoke-Scenario -Name "Linux rejects AppImage inventory records absent from the extracted tree" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Remove-Item -LiteralPath (
            Join-Path `
                $root `
                "appimage-tree/usr/lib/gtk-3.0/3.0.0/immodules/im-ibus.so"
        )
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "present"
    }

    Invoke-Scenario -Name "Linux accepts exact arm64 Tauri AppImage links" -Test {
        $root = New-FixtureRoot
        $bundle = New-LinuxFixture -Root $root -Architecture arm64
        $adapter = New-ToolAdapter -Root $root
        $actual = Invoke-Packager `
            -Path $linuxScript `
            -FixtureRoot $root `
            -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "aarch64-unknown-linux-gnu"
                ToolAdapterPath = $adapter
            }
        if ($actual.Count -ne 3) {
            throw "Linux arm64 packager returned the wrong output count."
        }
    }

    Invoke-Scenario -Name "macOS packages one app into exact three formats" -Test {
        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $output = Join-Path $root "output"
        $actual = Invoke-Packager -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = $output
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        }
        $expected = @(
            "WokRouter-v0.1.1-macOS-x86_64.dmg",
            "WokRouter-v0.1.1-macOS-x86_64.tar.gz",
            "WokRouter-v0.1.1-macOS-x86_64.zip"
        )
        [string[]] $names = @(
            Get-ChildItem -LiteralPath $output -File | ForEach-Object Name
        )
        [Array]::Sort($names, [StringComparer]::Ordinal)
        if ([string]::Join("|", $names) -cne [string]::Join("|", $expected)) {
            throw "macOS output inventory is not exact."
        }
        if ($actual.Count -ne 3) { throw "macOS packager returned the wrong output count." }
        $log = Get-Content -Raw -Encoding UTF8 (Join-Path $root "adapter.log")
        foreach ($operation in @(
                "mac-attach",
                "mac-lipo-architecture",
                "mac-create-tar",
                "mac-create-zip",
                "mac-detach"
            )) {
            if (-not $log.Contains($operation)) {
                throw "macOS adapter did not receive '$operation'."
            }
        }
    }

    Invoke-Scenario -Name "macOS rejects wrong version architecture and forbidden payload" -Test {
        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $metadataPath = Join-Path `
            $bundle `
            "macos/WokRouter.app/Contents/Info.metadata.json"
        $metadata = Get-Content `
            -Raw `
            -Encoding UTF8 `
            -LiteralPath $metadataPath |
            ConvertFrom-Json
        $metadata.CFBundleShortVersionString = "0.1.0"
        Write-Utf8File `
            -Path $metadataPath `
            -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "version"

        $metadata.CFBundleShortVersionString = $version
        Write-Utf8File `
            -Path $metadataPath `
            -Content ($metadata | ConvertTo-Json -Compress)
        Write-Utf8File `
            -Path (Join-Path $root "dmg-mount/WokRouter.app/Contents/MacOS/wokrouter") `
            -Content "sidecar-arm64"
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "architecture"
        $log = Get-Content -Raw -Encoding UTF8 (Join-Path $root "adapter.log")
        if (-not $log.Contains("mac-detach")) {
            throw "macOS failure did not detach the mounted DMG."
        }

        Write-Utf8File `
            -Path (Join-Path $root "dmg-mount/WokRouter.app/Contents/MacOS/wokrouter") `
            -Content "sidecar-x86_64"
        Write-Utf8File `
            -Path (Join-Path $bundle "macos/WokRouter.app/Contents/MacOS/wokrouterd") `
            -Content "forbidden"
        Write-MacInventoryOverride `
            -Root $root `
            -Kind source `
            -Inventory @(
                Get-MacFixtureInventory `
                    -App (Join-Path $bundle "macos/WokRouter.app")
            )
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "forbidden"
    }

    Invoke-Scenario -Name "macOS requires exact app leaf and bundle identity" -Test {
        foreach ($mutation in @(
                "source-leaf",
                "mounted-leaf",
                "identifier",
                "mounted-identifier",
                "executable",
                "name"
            )) {
            $root = New-FixtureRoot
            $bundle = New-MacFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            $expected = switch ($mutation) {
                "source-leaf" {
                    Move-Item `
                        -LiteralPath (Join-Path $bundle "macos/WokRouter.app") `
                        -Destination (Join-Path $bundle "macos/Other.app")
                    "WokRouter.app"
                }
                "mounted-leaf" {
                    Move-Item `
                        -LiteralPath (Join-Path $root "dmg-mount/WokRouter.app") `
                        -Destination (Join-Path $root "dmg-mount/Other.app")
                    "WokRouter.app"
                }
                default {
                    $metadataPath = if ($mutation -ceq "mounted-identifier") {
                        Join-Path `
                            $root `
                            "dmg-mount/WokRouter.app/Contents/Info.metadata.json"
                    } else {
                        Join-Path `
                            $bundle `
                            "macos/WokRouter.app/Contents/Info.metadata.json"
                    }
                    $metadata = Get-Content `
                        -Raw `
                        -Encoding UTF8 `
                        -LiteralPath $metadataPath |
                        ConvertFrom-Json
                    switch ($mutation) {
                        { $_ -in @("identifier", "mounted-identifier") } {
                            $metadata.CFBundleIdentifier = "dev.other.desktop"
                        }
                        "executable" {
                            $metadata.CFBundleExecutable = "Other"
                        }
                        "name" {
                            $metadata.CFBundleName = "Other"
                        }
                    }
                    Write-Utf8File `
                        -Path $metadataPath `
                        -Content ($metadata | ConvertTo-Json -Compress)
                    $mutation.Replace("mounted-", "")
                }
            }
            Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-apple-darwin"
                ToolAdapterPath = $adapter
            } -ExpectedText $expected
        }
    }

    Invoke-Scenario -Name "macOS enforces exact DMG root inventory" -Test {
        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Write-Utf8File `
            -Path (Join-Path $root "dmg-mount/unexpected.txt") `
            -Content "unexpected"
        $output = Join-Path $root "output"
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = $output
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "inventory"
        if (
            (Test-Path -LiteralPath $output) -and
            @(Get-ChildItem -LiteralPath $output -Force).Count -ne 0
        ) {
            throw "Rejected DMG root emitted release assets."
        }
        $log = Get-Content -Raw -Encoding UTF8 (Join-Path $root "adapter.log")
        if (-not $log.Contains("mac-detach")) {
            throw "DMG root failure did not detach the mounted image."
        }

        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Write-Utf8File `
            -Path (Join-Path $root "dmg-root-inventory.json") `
            -Content (@(
                @{
                    Kind = "Directory"
                    Name = "WokRouter.app"
                    Target = $null
                },
                @{
                    Kind = "Link"
                    Name = "Applications"
                    Target = "/tmp/Applications"
                }
            ) | ConvertTo-Json -Compress)
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "Applications"

        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Write-Utf8File `
            -Path (Join-Path $root "dmg-root-inventory.json") `
            -Content (@(
                @{
                    Kind = "Directory"
                    Name = ".DS_Store"
                    Target = $null
                },
                @{
                    Kind = "Directory"
                    Name = "WokRouter.app"
                    Target = $null
                }
            ) | ConvertTo-Json -Compress)
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "wrong type"

        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Write-Utf8File `
            -Path (Join-Path $root "dmg-mount/wokcore.txt") `
            -Content "forbidden"
        Assert-Rejects -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        } -ExpectedText "forbidden"

        $root = New-FixtureRoot
        $bundle = New-MacFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        Write-Utf8File `
            -Path (Join-Path $root "dmg-root-inventory.json") `
            -Content (@(
                @{
                    Kind = "File"
                    Name = ".DS_Store"
                    Target = $null
                },
                @{
                    Kind = "Directory"
                    Name = ".background"
                    Target = $null
                },
                @{
                    Kind = "File"
                    Name = ".VolumeIcon.icns"
                    Target = $null
                },
                @{
                    Kind = "Link"
                    Name = "Applications"
                    Target = "/Applications"
                },
                @{
                    Kind = "Directory"
                    Name = "WokRouter.app"
                    Target = $null
                }
            ) | ConvertTo-Json -Compress)
        $actual = Invoke-Packager -Path $macScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $bundle
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-apple-darwin"
            ToolAdapterPath = $adapter
        }
        if ($actual.Count -ne 3) {
            throw "Known Tauri DMG metadata was not accepted."
        }
    }

    Invoke-Scenario -Name "macOS fingerprints symlinks without following them" -Test {
        foreach ($case in @(
                "internal",
                "darwin-system",
                "darwin-usr-lib",
                "mismatch",
                "escape",
                "absolute-system-escape",
                "absolute-usr-escape",
                "absolute-denied"
            )) {
            $root = New-FixtureRoot
            $bundle = New-MacFixture -Root $root
            $adapter = New-ToolAdapter -Root $root
            $sourceDocument = Get-Content `
                -Raw `
                -Encoding UTF8 `
                -LiteralPath (Join-Path $root "source-app-inventory.json") |
                ConvertFrom-Json
            [object[]] $source = @(
                $sourceDocument | ForEach-Object { $_ }
            )
            $mountedDocument = Get-Content `
                -Raw `
                -Encoding UTF8 `
                -LiteralPath (Join-Path $root "mounted-app-inventory.json") |
                ConvertFrom-Json
            [object[]] $mounted = @(
                $mountedDocument | ForEach-Object { $_ }
            )
            $sourceTarget = switch ($case) {
                "internal" { "." }
                "darwin-system" { "/System/Library/Frameworks/AppKit.framework" }
                "darwin-usr-lib" { "/usr/lib/libobjc.A.dylib" }
                "mismatch" { "." }
                "escape" { "../../../../outside" }
                "absolute-system-escape" {
                    "/System/Library/../../tmp/evil"
                }
                "absolute-usr-escape" {
                    "/usr/lib/../../../tmp/evil"
                }
                "absolute-denied" { "/tmp/evil" }
            }
            $mountedTarget = if ($case -ceq "mismatch") {
                "../MacOS"
            } else {
                $sourceTarget
            }
            $source += [pscustomobject]@{
                Kind = "Link"
                Relative = "Contents/Resources/current"
                Target = $sourceTarget
                Sha256 = $null
            }
            $mounted += [pscustomobject]@{
                Kind = "Link"
                Relative = "Contents/Resources/current"
                Target = $mountedTarget
                Sha256 = $null
            }
            Write-MacInventoryOverride `
                -Root $root `
                -Kind source `
                -Inventory $source
            Write-MacInventoryOverride `
                -Root $root `
                -Kind mounted `
                -Inventory $mounted
            $arguments = @{
                BundleDirectory = $bundle
                OutputDirectory = (Join-Path $root "output")
                Version = $version
                Target = "x86_64-apple-darwin"
                ToolAdapterPath = $adapter
            }
            if ($case -in @("internal", "darwin-system", "darwin-usr-lib")) {
                $actual = Invoke-Packager `
                    -Path $macScript `
                    -FixtureRoot $root `
                    -Arguments $arguments
                if ($actual.Count -ne 3) {
                    throw "Allowed '$case' symlink was rejected."
                }
            } else {
                $expected = switch ($case) {
                    "mismatch" { "does not match" }
                    "escape" { "escapes" }
                    "absolute-system-escape" { "absolute" }
                    "absolute-usr-escape" { "absolute" }
                    "absolute-denied" { "absolute" }
                }
                Assert-Rejects `
                    -Path $macScript `
                    -FixtureRoot $root `
                    -Arguments $arguments `
                    -ExpectedText $expected
            }
        }
    }

    Invoke-Scenario -Name "Windows packages exact MSI and flat Portable zip" -Test {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $root = New-FixtureRoot
        $fixture = New-WindowsFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $output = Join-Path $root "output"
        $actual = Invoke-Packager -Path $windowsScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $fixture.Bundle
            DesktopExecutable = $fixture.Desktop
            SidecarExecutable = $fixture.Sidecar
            RepositoryRoot = $repositoryRoot
            OutputDirectory = $output
            Version = $version
            Target = "x86_64-pc-windows-msvc"
            ToolAdapterPath = $adapter
        }
        $expected = @(
            "WokRouter-v0.1.1-Windows-x86_64-Portable.zip",
            "WokRouter-v0.1.1-Windows-x86_64.msi"
        )
        [string[]] $names = @(
            Get-ChildItem -LiteralPath $output -File | ForEach-Object Name
        )
        [Array]::Sort($names, [StringComparer]::Ordinal)
        if ([string]::Join("|", $names) -cne [string]::Join("|", $expected)) {
            throw "Windows output inventory is not exact."
        }
        if ($actual.Count -ne 2) { throw "Windows packager returned the wrong output count." }
        $archive = [IO.Compression.ZipFile]::OpenRead(
            (Join-Path $output $expected[0])
        )
        try {
            [string[]] $entries = @(
                $archive.Entries | ForEach-Object FullName
            )
            [Array]::Sort($entries, [StringComparer]::Ordinal)
            $wanted = @(
                "LICENSE-APACHE",
                "LICENSE-MIT",
                "NOTICE.md",
                "README.md",
                "wokrouter-desktop.exe",
                "wokrouter.exe"
            )
            if ([string]::Join("|", $entries) -cne [string]::Join("|", $wanted)) {
                throw "Portable ZIP is not the exact flat inventory."
            }
        }
        finally {
            $archive.Dispose()
        }
    }

    Invoke-Scenario -Name "Windows rejects wrong MSI metadata and PE architecture" -Test {
        $root = New-FixtureRoot
        $fixture = New-WindowsFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $msi = Join-Path $fixture.Bundle "msi/WokRouter.msi"
        $metadata = Get-Content -Raw -Encoding UTF8 $msi | ConvertFrom-Json
        $metadata.Version = "0.1.0"
        Write-Utf8File -Path $msi -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $windowsScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $fixture.Bundle
            DesktopExecutable = $fixture.Desktop
            SidecarExecutable = $fixture.Sidecar
            RepositoryRoot = $repositoryRoot
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-pc-windows-msvc"
            ToolAdapterPath = $adapter
        } -ExpectedText "metadata"

        $metadata.Version = $version
        Write-Utf8File -Path $msi -Content ($metadata | ConvertTo-Json -Compress)
        Write-MinimalPe -Path $fixture.Sidecar -Architecture "arm64" -Marker "sidecar"
        Assert-Rejects -Path $windowsScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $fixture.Bundle
            DesktopExecutable = $fixture.Desktop
            SidecarExecutable = $fixture.Sidecar
            RepositoryRoot = $repositoryRoot
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-pc-windows-msvc"
            ToolAdapterPath = $adapter
        } -ExpectedText "architecture"
    }

    Invoke-Scenario -Name "Windows rejects extra MSI payload and forbidden names" -Test {
        $root = New-FixtureRoot
        $fixture = New-WindowsFixture -Root $root
        $adapter = New-ToolAdapter -Root $root
        $msi = Join-Path $fixture.Bundle "msi/WokRouter.msi"
        $metadata = Get-Content -Raw -Encoding UTF8 $msi | ConvertFrom-Json
        $metadata.Files += "unexpected.dll"
        Write-Utf8File -Path $msi -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $windowsScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $fixture.Bundle
            DesktopExecutable = $fixture.Desktop
            SidecarExecutable = $fixture.Sidecar
            RepositoryRoot = $repositoryRoot
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-pc-windows-msvc"
            ToolAdapterPath = $adapter
        } -ExpectedText "inventory"

        $metadata.Files = @(
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "NOTICE.md",
            "README.md",
            "wokrouter-desktop.exe",
            "wokcore.exe"
        )
        Write-Utf8File -Path $msi -Content ($metadata | ConvertTo-Json -Compress)
        Assert-Rejects -Path $windowsScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $fixture.Bundle
            DesktopExecutable = $fixture.Desktop
            SidecarExecutable = $fixture.Sidecar
            RepositoryRoot = $repositoryRoot
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-pc-windows-msvc"
            ToolAdapterPath = $adapter
        } -ExpectedText "forbidden"
    }

    Invoke-Scenario -Name "packagers reject reparse input roots" -Test {
        $root = New-FixtureRoot
        $real = Join-Path $root "real"
        $bundle = New-LinuxFixture -Root $real
        $adapter = New-ToolAdapter -Root $root
        $junction = Join-Path $root "bundle-junction"
        $null = New-Item -ItemType Junction -Path $junction -Target $bundle
        Assert-Rejects -Path $linuxScript -FixtureRoot $root -Arguments @{
            BundleDirectory = $junction
            OutputDirectory = (Join-Path $root "output")
            Version = $version
            Target = "x86_64-unknown-linux-gnu"
            ToolAdapterPath = $adapter
        } -ExpectedText "reparse"
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "RELEASE ASSET TEST ERROR: $failure"
        }
        throw "Release asset tests failed: $($failures.Count) of $scenarioCount."
    }
    Write-Host "Release asset tests passed: $scenarioCount scenario(s)."
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
            $leaf -cnotmatch "^wokrouter-release-assets-[0-9a-f]{32}$"
        ) {
            throw "Refusing to remove unexpected fixture root '$full'."
        }
        foreach ($reparse in @(
                Get-ChildItem -LiteralPath $full -Force -Recurse |
                    Where-Object {
                        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
                    }
            )) {
            [IO.Directory]::Delete($reparse.FullName, $false)
        }
        [IO.Directory]::Delete($full, $true)
    }
}
