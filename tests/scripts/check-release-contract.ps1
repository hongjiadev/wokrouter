[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$releasePath = Join-Path $rootPath ".github/workflows/release.yml"
$ciPath = Join-Path $rootPath ".github/workflows/ci.yml"
$developmentPath = Join-Path $rootPath "docs/operations/development.md"
$releaseContractPath = Join-Path `
    $rootPath `
    "tests/release/WokRouter.ReleaseContract.psm1"
$linuxPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-linux-assets.ps1"
$macPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-macos-assets.ps1"
$windowsPackagerPath = Join-Path `
    $rootPath `
    "tests/release/package-windows-assets.ps1"
$signerPath = Join-Path $rootPath "tests/release/sign-release-bundle.ps1"
$verifierPath = Join-Path $rootPath "tests/release/verify-release-bundle.ps1"
$publicKeyPath = Join-Path $rootPath "release/minisign.pub"
$cargoManifestPath = Join-Path $rootPath "Cargo.toml"
$cargoLockPath = Join-Path $rootPath "Cargo.lock"
$packageManifestPath = Join-Path $rootPath "apps/desktop/package.json"
$desktopMainPath = Join-Path $rootPath "apps/desktop/src-tauri/src/main.rs"
$tauriConfigurationPath = Join-Path `
    $rootPath `
    "apps/desktop/src-tauri/tauri.conf.json"
$eventCapabilityPath = Join-Path `
    $rootPath `
    "apps/desktop/src-tauri/capabilities/main.json"
$failures = [System.Collections.Generic.List[string]]::new()

function Add-Failure {
    param([Parameter(Mandatory)][string]$Message)

    if (-not $failures.Contains($Message)) {
        $failures.Add($Message)
    }
}

function Get-JobBlock {
    param(
        [Parameter(Mandatory)][string]$Workflow,
        [Parameter(Mandatory)][string]$Name
    )

    $pattern = "(?ms)^  $([regex]::Escape($Name)):\s*$.*?(?=^  [A-Za-z0-9_-]+:\s*$|\z)"
    $matches = [regex]::Matches($Workflow, $pattern)
    if ($matches.Count -ne 1) {
        Add-Failure -Message "Release workflow must define job '$Name' exactly once."
        return ""
    }
    return $matches[0].Value
}

function Get-SourceMatchIndex {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Pattern
    )

    $match = [regex]::Match($Source, $Pattern)
    return $(if ($match.Success) { $match.Index } else { -1 })
}

function Test-ContainsExactBlock {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Block
    )

    return $Source.Contains($Block.Replace("`r`n", "`n"))
}

function Test-ContainsExactLine {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Line
    )

    $pattern = '(?m)^[ \t]*' + [regex]::Escape($Line) + '[ \t]*$'
    return [regex]::Matches($Source, $pattern).Count -eq 1
}

function Get-PowerShellAst {
    param(
        [Parameter(Mandatory)][string] $Source,
        [Parameter(Mandatory)][string] $Description
    )

    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseInput(
        $Source,
        [ref] $tokens,
        [ref] $parseErrors
    )
    if ($parseErrors.Count -ne 0) {
        throw "$Description contains invalid PowerShell syntax."
    }
    return $ast
}

function Get-NormalizedAstText {
    param([Parameter(Mandatory)] $Ast)

    return $Ast.Extent.Text.Replace("`r`n", "`n").Trim()
}

function Test-ExactFunctionDefinitionAst {
    param(
        [Parameter(Mandatory)] $Actual,
        [Parameter(Mandatory)] $Expected
    )

    if (
        $Actual.Name -cne $Expected.Name -or
        $Actual.IsFilter -ne $Expected.IsFilter -or
        $Actual.IsWorkflow -ne $Expected.IsWorkflow -or
        (Get-NormalizedAstText -Ast $Actual) -cne
        (Get-NormalizedAstText -Ast $Expected)
    ) {
        return $false
    }
    return $true
}

function Get-ExactDirectStatementAst {
    param(
        [Parameter(Mandatory)] $Block,
        [Parameter(Mandatory)][string] $Statement
    )

    $expected = $Statement.Replace("`r`n", "`n").Trim()
    return @(
        $Block.Statements |
            Where-Object {
                (Get-NormalizedAstText -Ast $_) -ceq $expected
            }
    )
}

function Get-VariableAssignmentAst {
    param(
        [Parameter(Mandatory)] $Ast,
        [Parameter(Mandatory)][string] $Name
    )

    return @(
        $Ast.FindAll(
            {
                param($node)

                return (
                    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                    $node.Left -is [Management.Automation.Language.VariableExpressionAst] -and
                    $node.Left.VariablePath.UserPath -ceq $Name
                )
            },
            $true
        )
    )
}

function Get-ExactGuardAst {
    param(
        [Parameter(Mandatory)] $Ast,
        [Parameter(Mandatory)][string] $Condition,
        [Parameter(Mandatory)][string] $ThrowStatement
    )

    return @(
        $Ast.FindAll(
            {
                param($node)

                if (
                    $node -isnot [Management.Automation.Language.IfStatementAst] -or
                    $node.Clauses.Count -ne 1 -or
                    $null -ne $node.ElseClause
                ) {
                    return $false
                }
                $statements = @($node.Clauses[0].Item2.Statements)
                return (
                    $node.Clauses[0].Item1.Extent.Text.Trim() -ceq $Condition -and
                    $statements.Count -eq 1 -and
                    $statements[0] -is [Management.Automation.Language.ThrowStatementAst] -and
                    $statements[0].Extent.Text.Trim() -ceq $ThrowStatement
                )
            },
            $true
        )
    )
}

function Get-RustCodeView {
    param([Parameter(Mandatory)][string] $Source)

    $result = [Text.StringBuilder]::new($Source.Length)
    $index = 0
    $lineComment = $false
    $blockDepth = 0
    [char] $quote = [char] 0
    $escaped = $false
    $rawHashCount = -1
    while ($index -lt $Source.Length) {
        $current = $Source[$index]
        $next = if ($index + 1 -lt $Source.Length) {
            $Source[$index + 1]
        } else {
            [char] 0
        }

        if ($lineComment) {
            if ($current -eq "`r" -or $current -eq "`n") {
                $lineComment = $false
                $null = $result.Append($current)
            } else {
                $null = $result.Append(" ")
            }
            $index += 1
            continue
        }

        if ($blockDepth -gt 0) {
            if ($current -eq "/" -and $next -eq "*") {
                $blockDepth += 1
                $null = $result.Append("  ")
                $index += 2
                continue
            }
            if ($current -eq "*" -and $next -eq "/") {
                $blockDepth -= 1
                $null = $result.Append("  ")
                $index += 2
                continue
            }
            if ($current -eq "`r" -or $current -eq "`n") {
                $null = $result.Append($current)
            } else {
                $null = $result.Append(" ")
            }
            $index += 1
            continue
        }

        if ($rawHashCount -ge 0) {
            if ($current -eq '"') {
                $closingMatches = (
                    $index + $rawHashCount -lt $Source.Length
                )
                for (
                    $hashIndex = 1;
                    $closingMatches -and $hashIndex -le $rawHashCount;
                    $hashIndex += 1
                ) {
                    if ($Source[$index + $hashIndex] -ne "#") {
                        $closingMatches = $false
                    }
                }
                if ($closingMatches) {
                    $closingLength = 1 + $rawHashCount
                    $null = $result.Append("".PadRight($closingLength))
                    $index += $closingLength
                    $rawHashCount = -1
                    continue
                }
            }
            if ($current -eq "`r" -or $current -eq "`n") {
                $null = $result.Append($current)
            } else {
                $null = $result.Append(" ")
            }
            $index += 1
            continue
        }

        if ($quote -ne [char] 0) {
            if ($current -eq "`r" -or $current -eq "`n") {
                $null = $result.Append($current)
            } else {
                $null = $result.Append(" ")
            }
            if ($escaped) {
                $escaped = $false
            } elseif ($current -eq "\") {
                $escaped = $true
            } elseif ($current -eq $quote) {
                $quote = [char] 0
            }
            $index += 1
            continue
        }

        $previous = if ($index -gt 0) {
            $Source[$index - 1]
        } else {
            [char] 0
        }
        $atTokenStart = (
            $index -eq 0 -or
            [string] $previous -cnotmatch '[A-Za-z0-9_]'
        )

        $rawCursor = -1
        if ($atTokenStart -and $current -eq "r") {
            $rawCursor = $index + 1
        } elseif (
            $atTokenStart -and
            $current -in @("b", "c") -and
            $next -eq "r"
        ) {
            $rawCursor = $index + 2
        }
        if ($rawCursor -ge 0) {
            $hashCount = 0
            while (
                $rawCursor -lt $Source.Length -and
                $Source[$rawCursor] -eq "#"
            ) {
                $hashCount += 1
                $rawCursor += 1
            }
            if (
                $rawCursor -lt $Source.Length -and
                $Source[$rawCursor] -eq '"'
            ) {
                $openingLength = $rawCursor - $index + 1
                $null = $result.Append("".PadRight($openingLength))
                $index += $openingLength
                $rawHashCount = $hashCount
                continue
            }
        }

        if ($current -eq "/" -and $next -eq "/") {
            $lineComment = $true
            $null = $result.Append("  ")
            $index += 2
            continue
        }
        if ($current -eq "/" -and $next -eq "*") {
            $blockDepth = 1
            $null = $result.Append("  ")
            $index += 2
            continue
        }

        $quoteIndex = -1
        if ($current -eq '"' -or $current -eq "'") {
            $quoteIndex = $index
        } elseif (
            $atTokenStart -and
            $current -in @("b", "c") -and
            $next -in @('"', "'")
        ) {
            $quoteIndex = $index + 1
        }
        if ($quoteIndex -ge 0) {
            $isCharacter = $Source[$quoteIndex] -eq "'"
            $hasCharacterEnd = -not $isCharacter
            if ($isCharacter) {
                $characterCursor = $quoteIndex + 1
                $characterEscaped = $false
                while (
                    $characterCursor -lt $Source.Length -and
                    $Source[$characterCursor] -notin @("`r", "`n")
                ) {
                    $character = $Source[$characterCursor]
                    if ($characterEscaped) {
                        $characterEscaped = $false
                    } elseif ($character -eq "\") {
                        $characterEscaped = $true
                    } elseif ($character -eq "'") {
                        $hasCharacterEnd = $true
                        break
                    }
                    $characterCursor += 1
                }
            }
            if ($hasCharacterEnd) {
                $openingLength = $quoteIndex - $index + 1
                $null = $result.Append("".PadRight($openingLength))
                $quote = $Source[$quoteIndex]
                $escaped = $false
                $index += $openingLength
                continue
            }
        }
        $null = $result.Append($current)
        $index += 1
    }
    return $result.ToString()
}

function Get-ActiveWindowsSubsystemAttributes {
    param([Parameter(Mandatory)][string] $Source)

    $activeSource = Get-RustCodeView -Source $Source
    return @(
        [regex]::Matches(
            $activeSource,
            '(?ms)^[ \t]*#![ \t]*\[[^\]]*\bwindows_subsystem\b[^\]]*\]'
        )
    )
}

function Read-BoundedUtf8Text {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][long]$MaximumBytes
    )

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -eq 0 -or $bytes.Length -gt $MaximumBytes) {
        throw "Release source file has an invalid size: $Path"
    }
    return [Text.UTF8Encoding]::new($false, $true).GetString($bytes).Replace(
        "`r`n",
        "`n"
    )
}

function Get-CargoWorkspaceVersion {
    param([Parameter(Mandatory)][string]$Text)

    $sections = @(
        [regex]::Matches(
            $Text,
            '(?ms)^\[workspace\.package\][ \t]*\n(?<body>.*?)(?=^\[[^\r\n]+\][ \t]*$|\z)'
        )
    )
    if ($sections.Count -ne 1) {
        throw "Cargo.toml must contain exactly one [workspace.package] section."
    }
    $versions = @(
        [regex]::Matches(
            $sections[0].Groups["body"].Value,
            '(?m)^version[ \t]*=[ \t]*"(?<value>[^"]+)"[ \t]*$'
        )
    )
    if ($versions.Count -ne 1) {
        throw "Cargo.toml workspace package must contain exactly one version."
    }
    return $versions[0].Groups["value"].Value
}

function Get-JsonReleaseVersion {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][string]$Name
    )

    $members = @([regex]::Matches($Text, '(?m)"version"[ \t\r\n]*:'))
    if ($members.Count -ne 1) {
        throw "$Name must contain exactly one version member."
    }
    $document = $Text | ConvertFrom-Json
    $properties = @(
        $document.PSObject.Properties |
            Where-Object { $_.Name -ieq "version" }
    )
    if (
        $properties.Count -ne 1 -or
        $properties[0].Name -cne "version" -or
        $properties[0].Value -isnot [string]
    ) {
        throw "$Name must contain one exact string version member."
    }
    return [string]$properties[0].Value
}

function Get-WokRouterLockVersions {
    param([Parameter(Mandatory)][string]$Text)

    $wanted = @(
        "wokrouter-cli",
        "wokrouter-desktop",
        "wokrouter-platform",
        "wokrouter-storage",
        "wokrouter-wokcore-client"
    )
    $blocks = @(
        [regex]::Matches(
            $Text,
            '(?ms)^\[\[package\]\][ \t]*\n.*?(?=^\[\[package\]\][ \t]*$|\z)'
        )
    )
    $versions = [System.Collections.Generic.List[string]]::new()
    foreach ($packageName in $wanted) {
        $matching = @(
            $blocks |
                Where-Object {
                    $_.Value -match (
                        '(?m)^name[ \t]*=[ \t]*"' +
                        [regex]::Escape($packageName) +
                        '"[ \t]*$'
                    )
                }
        )
        if ($matching.Count -ne 1) {
            throw "Cargo.lock must contain exactly one '$packageName' package."
        }
        $version = @(
            [regex]::Matches(
                $matching[0].Value,
                '(?m)^version[ \t]*=[ \t]*"(?<value>[^"]+)"[ \t]*$'
            )
        )
        if ($version.Count -ne 1) {
            throw "Cargo.lock package '$packageName' must contain exactly one version."
        }
        $versions.Add($version[0].Groups["value"].Value)
    }
    return $versions.ToArray()
}

foreach ($path in @(
        $releasePath,
        $ciPath,
        $developmentPath,
        $releaseContractPath,
        $linuxPackagerPath,
        $macPackagerPath,
        $windowsPackagerPath,
        $signerPath,
        $verifierPath,
        $publicKeyPath,
        $cargoManifestPath,
        $cargoLockPath,
        $packageManifestPath,
        $desktopMainPath,
        $tauriConfigurationPath,
        $eventCapabilityPath
    )) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Add-Failure -Message "Required release contract file is missing: $path"
    }
}

if ($failures.Count -eq 0) {
    try {
        $eventCapability = Read-BoundedUtf8Text `
            -Path $eventCapabilityPath `
            -MaximumBytes 16384 |
            ConvertFrom-Json
        $properties = @($eventCapability.PSObject.Properties.Name | Sort-Object)
        $expectedProperties = @(
            '$schema',
            'description',
            'identifier',
            'permissions',
            'windows'
        )
        $windows = @($eventCapability.windows)
        $permissions = @($eventCapability.permissions)
        if (
            (Compare-Object $properties $expectedProperties) -or
            $eventCapability.'$schema' -cne '../gen/schemas/desktop-schema.json' -or
            $eventCapability.identifier -cne 'main-event-listener' -or
            $windows.Count -ne 1 -or
            $windows[0] -cne 'main' -or
            $permissions.Count -ne 2 -or
            $permissions[0] -cne 'core:event:allow-listen' -or
            $permissions[1] -cne 'core:event:allow-unlisten'
        ) {
            throw "unexpected capability shape"
        }
    }
    catch {
        Add-Failure `
            -Message "Release desktop must package the main window's exact event listen/unlisten capability."
    }

    $desktopMain = Read-BoundedUtf8Text `
        -Path $desktopMainPath `
        -MaximumBytes 131072
    $desktopSubsystemAttribute = (
        '#![cfg_attr(all(windows, not(debug_assertions)), ' +
        'windows_subsystem = "windows")]'
    )
    if (-not $desktopMain.StartsWith(
            "$desktopSubsystemAttribute`n",
            [StringComparison]::Ordinal
        )) {
        Add-Failure `
            -Message "Desktop main must begin with the exact release-only GUI subsystem attribute."
    }

    $desktopSubsystemAttributes = @(
        Get-ActiveWindowsSubsystemAttributes -Source $desktopMain
    )
    if (
        $desktopSubsystemAttributes.Count -ne 1 -or
        $desktopMain.Substring(
            $desktopSubsystemAttributes[0].Index,
            $desktopSubsystemAttributes[0].Length
        ).Trim() -cne
        $desktopSubsystemAttribute
    ) {
        Add-Failure `
            -Message "The release-only attribute must be the desktop's only active Windows subsystem declaration."
    }

    $otherSubsystemMains = @(
        foreach ($sourceRootName in @("apps", "crates")) {
            $sourceRoot = Join-Path $rootPath $sourceRootName
            if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
                continue
            }
            foreach ($main in Get-ChildItem `
                    -LiteralPath $sourceRoot `
                    -Filter "main.rs" `
                    -File `
                    -Recurse) {
                if ($main.FullName -ceq $desktopMainPath) {
                    continue
                }
                $source = Read-BoundedUtf8Text `
                    -Path $main.FullName `
                    -MaximumBytes 1048576
                if (@(
                        Get-ActiveWindowsSubsystemAttributes -Source $source
                    ).Count -ne 0) {
                    $main.FullName
                }
            }
        }
    )
    if ($otherSubsystemMains.Count -ne 0) {
        Add-Failure `
            -Message "A Windows subsystem declaration may appear only in desktop main.rs."
    }

    $releaseContractSource = Read-BoundedUtf8Text `
        -Path $releaseContractPath `
        -MaximumBytes 262144
    $releaseContractAst = $null
    try {
        $releaseContractAst = Get-PowerShellAst `
            -Source $releaseContractSource `
            -Description "Release contract module"
    }
    catch {
        Add-Failure `
            -Message "Release contract must define the exact script-scope PE subsystem helper: $($_.Exception.Message)"
    }
    if ($null -ne $releaseContractAst) {
        $peSubsystemFunctions = @(
            $releaseContractAst.FindAll(
                {
                    param($node)

                    return (
                        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                        $node.Name -ceq "Get-PeSubsystem"
                    )
                },
                $true
            )
        )
        $scriptScopePeSubsystemFunctions = @(
            $peSubsystemFunctions |
                Where-Object {
                    [object]::ReferenceEquals(
                        $_.Parent,
                        $releaseContractAst.EndBlock
                    )
                }
        )
        if (
            $peSubsystemFunctions.Count -ne 1 -or
            $scriptScopePeSubsystemFunctions.Count -ne 1
        ) {
            Add-Failure `
                -Message "Release contract must define and export the exact script-scope PE subsystem helper."
        }
    }
    $expectedPeSubsystemSource = @'
function Get-PeSubsystem {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $Path)

    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 0x40 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        throw "Windows executable has no valid DOS header."
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
    if (
        $peOffset -lt 0 -or
        $peOffset + 24 + 70 -gt $bytes.Length -or
        [Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4) -cne "PE`0`0"
    ) {
        throw "Windows executable has no valid PE header."
    }
    $optionalHeader = $peOffset + 24
    $magic = [BitConverter]::ToUInt16($bytes, $optionalHeader)
    if ($magic -notin @([UInt16] 0x10B, [UInt16] 0x20B)) {
        throw "Windows executable has an unsupported optional header."
    }
    return [BitConverter]::ToUInt16($bytes, $optionalHeader + 68)
}
'@
    $expectedPeSubsystemAst = Get-PowerShellAst `
        -Source $expectedPeSubsystemSource `
        -Description "Expected PE subsystem helper"
    $expectedPeSubsystemFunction = @(
        $expectedPeSubsystemAst.EndBlock.Statements |
            Where-Object {
                $_ -is [Management.Automation.Language.FunctionDefinitionAst]
            }
    )[0]
    if (
        $scriptScopePeSubsystemFunctions.Count -ne 1 -or
        -not (Test-ExactFunctionDefinitionAst `
            -Actual $scriptScopePeSubsystemFunctions[0] `
            -Expected $expectedPeSubsystemFunction) -or
        -not (Test-ContainsExactLine `
            -Source $releaseContractSource `
            -Line '-Function Get-WokRouterTargetContracts, Get-WokRouterPayloadNames, Get-PeSubsystem')
    ) {
        Add-Failure `
            -Message "Release contract must define and export the exact script-scope PE subsystem helper."
    }

    $windowsPackagerSource = Read-BoundedUtf8Text `
        -Path $windowsPackagerPath `
        -MaximumBytes 1048576
    $windowsPackagerAst = $null
    try {
        $windowsPackagerAst = Get-PowerShellAst `
            -Source $windowsPackagerSource `
            -Description "Windows packager"
    }
    catch {
        Add-Failure `
            -Message "Windows packager GUI subsystem checks are invalid: $($_.Exception.Message)"
    }

    $sourceDesktopGuards = @()
    $sourceSidecarGuards = @()
    $msiDesktopGuards = @()
    $msiSidecarGuards = @()
    $portableDesktopCountGuards = @()
    $portableDesktopGuards = @()
    $portableSidecarCountGuards = @()
    $portableSidecarGuards = @()
    if ($null -ne $windowsPackagerAst) {
        $sourceDesktopGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $desktop) -ne 2' `
                -ThrowStatement 'throw "Windows desktop executable must use the GUI subsystem."'
        )
        $sourceSidecarGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $sidecar) -ne 3' `
                -ThrowStatement 'throw "Windows sidecar executable must use the console subsystem."'
        )
        $msiDesktopGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $byName["wokrouter-desktop.exe"]) -ne 2' `
                -ThrowStatement 'throw "MSI desktop executable must use the GUI subsystem."'
        )
        $msiSidecarGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $byName["wokrouter.exe"]) -ne 3' `
                -ThrowStatement 'throw "MSI sidecar executable must use the console subsystem."'
        )
        $portableDesktopGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $portableDesktop) -ne 2' `
                -ThrowStatement 'throw "Portable desktop executable must use the GUI subsystem."'
        )
        $portableSidecarGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '(& $peSubsystemCommand -Path $portableSidecar) -ne 3' `
                -ThrowStatement 'throw "Portable sidecar executable must use the console subsystem."'
        )
        $portableDesktopCountGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '$portableDesktopFiles.Count -ne 1' `
                -ThrowStatement 'throw "Portable archive must contain one desktop executable."'
        )
        $portableSidecarCountGuards = @(
            Get-ExactGuardAst `
                -Ast $windowsPackagerAst `
                -Condition '$portableSidecarFiles.Count -ne 1' `
                -ThrowStatement 'throw "Portable archive must contain one sidecar executable."'
        )

        $releaseContractModulePathStatements = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement '$releaseContractModulePath = Join-Path $PSScriptRoot "WokRouter.ReleaseContract.psm1"'
        )
        $releaseContractModuleStatements = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement '$releaseContractModule = Import-Module $releaseContractModulePath -Force -PassThru'
        )
        $peSubsystemFunctionStatements = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement @'
$ExecutionContext.SessionState.PSVariable.Set(
    [Management.Automation.PSVariable]::new(
        "peSubsystemFunction",
        $releaseContractModule.ExportedFunctions["Get-PeSubsystem"],
        [Management.Automation.ScopedItemOptions]::Constant
    )
)
'@
        )
        $releaseContractIdentityGuards = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement @'
if (
    $releaseContractModule.Name -cne "WokRouter.ReleaseContract" -or
    -not [StringComparer]::OrdinalIgnoreCase.Equals(
        [IO.Path]::GetFullPath($releaseContractModule.Path),
        [IO.Path]::GetFullPath($releaseContractModulePath)
    ) -or
    $peSubsystemFunction -isnot [Management.Automation.FunctionInfo] -or
    $peSubsystemFunction.Name -cne "Get-PeSubsystem" -or
    $peSubsystemFunction.ModuleName -cne "WokRouter.ReleaseContract"
) {
    throw "Windows release contract PE subsystem helper is unavailable."
}
'@
        )
        $peSubsystemCommandStatements = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement @'
$ExecutionContext.SessionState.PSVariable.Set(
    [Management.Automation.PSVariable]::new(
        "peSubsystemCommand",
        $peSubsystemFunction.ScriptBlock,
        [Management.Automation.ScopedItemOptions]::Constant
    )
)
'@
        )
        $peSubsystemSnapshotGuards = @(
            Get-ExactDirectStatementAst `
                -Block $windowsPackagerAst.EndBlock `
                -Statement @'
if ($peSubsystemCommand -isnot [Management.Automation.ScriptBlock]) {
    throw "Windows release contract PE subsystem helper is unavailable."
}
'@
        )
        $releaseContractModulePathAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $windowsPackagerAst `
                -Name "releaseContractModulePath"
        )
        $releaseContractModuleAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $windowsPackagerAst `
                -Name "releaseContractModule"
        )
        $peSubsystemCommandAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $windowsPackagerAst `
                -Name "peSubsystemCommand"
        )
        $peSubsystemFunctionAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $windowsPackagerAst `
                -Name "peSubsystemFunction"
        )
        $peSubsystemCommandNameLiterals = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    return (
                        $node -is [Management.Automation.Language.StringConstantExpressionAst] -and
                        $node.Value -ieq "peSubsystemCommand"
                    )
                },
                $true
            )
        )
        $peSubsystemFunctionNameLiterals = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    return (
                        $node -is [Management.Automation.Language.StringConstantExpressionAst] -and
                        $node.Value -ieq "peSubsystemFunction"
                    )
                },
                $true
            )
        )
        $peSubsystemVariableWriteCommands = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    if ($node -isnot [Management.Automation.Language.CommandAst]) {
                        return $false
                    }
                    $name = $node.GetCommandName()
                    if ($null -eq $name) {
                        return $false
                    }
                    $leaf = $name.Split("\")[-1]
                    return (
                        $leaf -iin @(
                            "Set-Variable",
                            "New-Variable",
                            "Remove-Variable",
                            "Clear-Variable",
                            "Set-Item",
                            "Remove-Item",
                            "Clear-Item",
                            "Rename-Item",
                            "Move-Item"
                        ) -and
                        $node.Extent.Text -imatch (
                            '(?:Variable:\s*)?peSubsystem(?:Command|Function)'
                        )
                    )
                },
                $true
            )
        )
        $peSubsystemFunctionWriteCommands = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    if ($node -isnot [Management.Automation.Language.CommandAst]) {
                        return $false
                    }
                    $name = $node.GetCommandName()
                    if ($null -eq $name) {
                        return $false
                    }
                    $leaf = $name.Split("\")[-1]
                    return (
                        $leaf -iin @(
                            "Set-Item",
                            "Remove-Item",
                            "Clear-Item",
                            "Rename-Item",
                            "Move-Item"
                        ) -and
                        $node.Extent.Text -imatch (
                            'Function:\s*Get-PeSubsystem'
                        )
                    )
                },
                $true
            )
        )
        $directPeSubsystemCommands = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    if ($node -isnot [Management.Automation.Language.CommandAst]) {
                        return $false
                    }
                    $name = $node.GetCommandName()
                    return (
                        $null -ne $name -and
                        $name -cmatch '(^|\\)Get-PeSubsystem$'
                    )
                },
                $true
            )
        )
        $localPeSubsystemFunctions = @(
            $windowsPackagerAst.FindAll(
                {
                    param($node)

                    return (
                        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                        $node.Name -cmatch '(^|\\)Get-PeSubsystem$'
                    )
                },
                $true
            )
        )
        $modulePathIndex = -1
        $moduleIndex = -1
        $functionIndex = -1
        $identityIndex = -1
        $commandIndex = -1
        $snapshotGuardIndex = -1
        if (
            $releaseContractModulePathStatements.Count -eq 1 -and
            $releaseContractModuleStatements.Count -eq 1 -and
            $peSubsystemFunctionStatements.Count -eq 1 -and
            $peSubsystemCommandStatements.Count -eq 1 -and
            $releaseContractIdentityGuards.Count -eq 1 -and
            $peSubsystemSnapshotGuards.Count -eq 1
        ) {
            $modulePathIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $releaseContractModulePathStatements[0]
            )
            $moduleIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $releaseContractModuleStatements[0]
            )
            $functionIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $peSubsystemFunctionStatements[0]
            )
            $identityIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $releaseContractIdentityGuards[0]
            )
            $commandIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $peSubsystemCommandStatements[0]
            )
            $snapshotGuardIndex = $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $peSubsystemSnapshotGuards[0]
            )
        }
        $moduleImportIsOwned = (
            $releaseContractModulePathAssignments.Count -eq 1 -and
            $releaseContractModuleAssignments.Count -eq 1 -and
            $peSubsystemCommandAssignments.Count -eq 0 -and
            $peSubsystemFunctionAssignments.Count -eq 0 -and
            $peSubsystemCommandNameLiterals.Count -eq 1 -and
            $peSubsystemFunctionNameLiterals.Count -eq 1 -and
            $peSubsystemVariableWriteCommands.Count -eq 0 -and
            $peSubsystemFunctionWriteCommands.Count -eq 0 -and
            $directPeSubsystemCommands.Count -eq 0 -and
            $localPeSubsystemFunctions.Count -eq 0 -and
            $modulePathIndex -ge 0 -and
            $moduleIndex -eq ($modulePathIndex + 1) -and
            $functionIndex -eq ($moduleIndex + 1) -and
            $identityIndex -eq ($functionIndex + 1) -and
            $commandIndex -eq ($identityIndex + 1) -and
            $snapshotGuardIndex -eq ($commandIndex + 1) -and
            $sourceDesktopGuards.Count -eq 1 -and
            $snapshotGuardIndex -lt $windowsPackagerAst.EndBlock.Statements.IndexOf(
                $sourceDesktopGuards[0]
            )
        )
        if (-not $moduleImportIsOwned) {
            Add-Failure `
                -Message "Windows packager must retain the owned PE subsystem FunctionInfo snapshot binding."
        }
    }

    $sourceDesktopGuardIsOwned = (
        $sourceDesktopGuards.Count -eq 1 -and
        [object]::ReferenceEquals(
            $sourceDesktopGuards[0].Parent,
            $windowsPackagerAst.EndBlock
        )
    )
    if (-not $sourceDesktopGuardIsOwned) {
        Add-Failure `
            -Message "Windows packager must retain the active script-scope source desktop GUI subsystem check."
    }
    $sourceSidecarGuardIsOwned = (
        $sourceSidecarGuards.Count -eq 1 -and
        [object]::ReferenceEquals(
            $sourceSidecarGuards[0].Parent,
            $windowsPackagerAst.EndBlock
        ) -and
        $sourceDesktopGuards.Count -eq 1 -and
        $sourceDesktopGuards[0].Extent.StartOffset -lt
        $sourceSidecarGuards[0].Extent.StartOffset
    )
    if (-not $sourceSidecarGuardIsOwned) {
        Add-Failure `
            -Message "Windows packager must retain the active script-scope source sidecar console subsystem check."
    }

    $packageTry = $null
    if (
        $msiDesktopGuards.Count -eq 1 -and
        $msiSidecarGuards.Count -eq 1 -and
        $portableDesktopGuards.Count -eq 1 -and
        $portableSidecarGuards.Count -eq 1
    ) {
        $msiOwner = $msiDesktopGuards[0].Parent.Parent
        $msiSidecarOwner = $msiSidecarGuards[0].Parent.Parent
        $portableOwner = $portableDesktopGuards[0].Parent.Parent
        $portableSidecarOwner = $portableSidecarGuards[0].Parent.Parent
        if (
            $msiOwner -is [Management.Automation.Language.TryStatementAst] -and
            [object]::ReferenceEquals($msiOwner, $msiSidecarOwner) -and
            [object]::ReferenceEquals($msiOwner, $portableOwner) -and
            [object]::ReferenceEquals($msiOwner, $portableSidecarOwner) -and
            [object]::ReferenceEquals(
                $msiDesktopGuards[0].Parent,
                $msiOwner.Body
            ) -and
            [object]::ReferenceEquals(
                $msiSidecarGuards[0].Parent,
                $msiOwner.Body
            ) -and
            [object]::ReferenceEquals(
                $portableDesktopGuards[0].Parent,
                $msiOwner.Body
            ) -and
            [object]::ReferenceEquals(
                $portableSidecarGuards[0].Parent,
                $msiOwner.Body
            ) -and
            [object]::ReferenceEquals(
                $msiOwner.Parent,
                $windowsPackagerAst.EndBlock
            ) -and
            $msiDesktopGuards[0].Extent.StartOffset -lt
            $portableDesktopGuards[0].Extent.StartOffset -and
            $msiSidecarGuards[0].Extent.StartOffset -lt
            $portableDesktopGuards[0].Extent.StartOffset -and
            $portableDesktopGuards[0].Extent.StartOffset -lt
            $portableSidecarGuards[0].Extent.StartOffset
        ) {
            $packageTry = $msiOwner
        }
    }
    if ($null -eq $packageTry) {
        Add-Failure `
            -Message "Windows packager must retain the active MSI desktop GUI subsystem check in the package try block."
        Add-Failure `
            -Message "Windows packager must retain the active MSI sidecar console subsystem check in the package try block."
        Add-Failure `
            -Message "Windows packager must retain the active Portable desktop GUI subsystem check in the package try block."
        Add-Failure `
            -Message "Windows packager must retain the active Portable sidecar console subsystem check in the package try block."
    }
    $msiSidecarIdentityIsOwned = $false
    if ($null -ne $packageTry) {
        $msiSidecarIdentityStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
Assert-SameFile `
        -Expected $sidecar `
        -Actual $byName["wokrouter.exe"] `
        -Description "MSI sidecar executable"
'@
        )
        if ($msiSidecarIdentityStatements.Count -eq 1) {
            $msiSidecarGuardIndex = $packageTry.Body.Statements.IndexOf(
                $msiSidecarGuards[0]
            )
            $msiSidecarIdentityIndex = $packageTry.Body.Statements.IndexOf(
                $msiSidecarIdentityStatements[0]
            )
            $msiSidecarIdentityIsOwned = (
                $msiSidecarGuardIndex -ge 0 -and
                $msiSidecarIdentityIndex -gt $msiSidecarGuardIndex
            )
        }
    }
    if (-not $msiSidecarIdentityIsOwned) {
        Add-Failure `
            -Message "Windows packager must validate the extracted MSI sidecar console subsystem before retaining exact byte identity."
    }

    $portableProvenanceIsOwned = $false
    $portableSidecarProvenanceIsOwned = $false
    if ($null -ne $packageTry) {
        $zipOutputStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement '$zipOutput = Join-Path $output "$prefix-Portable.zip"'
        )
        $archiveOpenStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
$archive = [IO.Compression.ZipFile]::Open(
        $zipOutput,
        [IO.Compression.ZipArchiveMode]::Create
    )
'@
        )
        $portableRootStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement '$portableExtracted = Join-Path $temporary "portable"'
        )
        $portableDirectoryStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement '[IO.Directory]::CreateDirectory($portableExtracted) | Out-Null'
        )
        $portableExtractionStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
[IO.Compression.ZipFile]::ExtractToDirectory(
        $zipOutput,
        $portableExtracted
    )
'@
        )
        $portableTreeSafetyStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
Assert-TreeSafe `
        -Root $portableExtracted `
        -Description "Extracted Portable archive"
'@
        )
        $portableQueryStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
$portableDesktopFiles = @(
        Get-ChildItem `
            -LiteralPath $portableExtracted `
            -Force `
            -Recurse `
            -File |
            Where-Object Name -CEQ "wokrouter-desktop.exe"
    )
'@
        )
        $portableAssignmentStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement '$portableDesktop = $portableDesktopFiles[0].FullName'
        )
        $portableSidecarQueryStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement @'
$portableSidecarFiles = @(
        Get-ChildItem `
            -LiteralPath $portableExtracted `
            -Force `
            -Recurse `
            -File |
            Where-Object Name -CEQ "wokrouter.exe"
    )
'@
        )
        $portableSidecarAssignmentStatements = @(
            Get-ExactDirectStatementAst `
                -Block $packageTry.Body `
                -Statement '$portableSidecar = $portableSidecarFiles[0].FullName'
        )
        $zipOutputAssignments = @(
            Get-VariableAssignmentAst -Ast $packageTry.Body -Name "zipOutput"
        )
        $portableRootAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $packageTry.Body `
                -Name "portableExtracted"
        )
        $portableFileAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $packageTry.Body `
                -Name "portableDesktopFiles"
        )
        $portableDesktopAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $packageTry.Body `
                -Name "portableDesktop"
        )
        $portableSidecarFileAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $packageTry.Body `
                -Name "portableSidecarFiles"
        )
        $portableSidecarAssignments = @(
            Get-VariableAssignmentAst `
                -Ast $packageTry.Body `
                -Name "portableSidecar"
        )
        $zipOutputIndex = -1
        $archiveOpenIndex = -1
        $portableRootIndex = -1
        $portableDirectoryIndex = -1
        $portableExtractionIndex = -1
        $portableTreeSafetyIndex = -1
        $portableQueryIndex = -1
        $portableCountGuardIndex = -1
        $portableAssignmentIndex = -1
        $portableSubsystemGuardIndex = -1
        $portableSidecarQueryIndex = -1
        $portableSidecarCountGuardIndex = -1
        $portableSidecarAssignmentIndex = -1
        $portableSidecarSubsystemGuardIndex = -1
        $archiveTryIsOwned = $false
        if (
            $zipOutputStatements.Count -eq 1 -and
            $archiveOpenStatements.Count -eq 1 -and
            $portableRootStatements.Count -eq 1 -and
            $portableDirectoryStatements.Count -eq 1 -and
            $portableExtractionStatements.Count -eq 1 -and
            $portableTreeSafetyStatements.Count -eq 1 -and
            $portableQueryStatements.Count -eq 1 -and
            $portableDesktopCountGuards.Count -eq 1 -and
            $portableAssignmentStatements.Count -eq 1 -and
            $portableDesktopGuards.Count -eq 1 -and
            $portableSidecarQueryStatements.Count -eq 1 -and
            $portableSidecarCountGuards.Count -eq 1 -and
            $portableSidecarAssignmentStatements.Count -eq 1 -and
            $portableSidecarGuards.Count -eq 1
        ) {
            $zipOutputIndex = $packageTry.Body.Statements.IndexOf(
                $zipOutputStatements[0]
            )
            $archiveOpenIndex = $packageTry.Body.Statements.IndexOf(
                $archiveOpenStatements[0]
            )
            $portableRootIndex = $packageTry.Body.Statements.IndexOf(
                $portableRootStatements[0]
            )
            $portableDirectoryIndex = $packageTry.Body.Statements.IndexOf(
                $portableDirectoryStatements[0]
            )
            $portableExtractionIndex = $packageTry.Body.Statements.IndexOf(
                $portableExtractionStatements[0]
            )
            $portableTreeSafetyIndex = $packageTry.Body.Statements.IndexOf(
                $portableTreeSafetyStatements[0]
            )
            $portableQueryIndex = $packageTry.Body.Statements.IndexOf(
                $portableQueryStatements[0]
            )
            $portableCountGuardIndex = $packageTry.Body.Statements.IndexOf(
                $portableDesktopCountGuards[0]
            )
            $portableAssignmentIndex = $packageTry.Body.Statements.IndexOf(
                $portableAssignmentStatements[0]
            )
            $portableSubsystemGuardIndex = $packageTry.Body.Statements.IndexOf(
                $portableDesktopGuards[0]
            )
            $portableSidecarQueryIndex = $packageTry.Body.Statements.IndexOf(
                $portableSidecarQueryStatements[0]
            )
            $portableSidecarCountGuardIndex = $packageTry.Body.Statements.IndexOf(
                $portableSidecarCountGuards[0]
            )
            $portableSidecarAssignmentIndex = $packageTry.Body.Statements.IndexOf(
                $portableSidecarAssignmentStatements[0]
            )
            $portableSidecarSubsystemGuardIndex = $packageTry.Body.Statements.IndexOf(
                $portableSidecarGuards[0]
            )
            if (
                $archiveOpenIndex + 1 -lt $packageTry.Body.Statements.Count
            ) {
                $archiveTry = $packageTry.Body.Statements[$archiveOpenIndex + 1]
                if (
                    $archiveTry -is [Management.Automation.Language.TryStatementAst] -and
                    $archiveTry.CatchClauses.Count -eq 0 -and
                    $null -ne $archiveTry.Finally
                ) {
                    $archiveDisposeStatements = @(
                        Get-ExactDirectStatementAst `
                            -Block $archiveTry.Finally `
                            -Statement '$archive.Dispose()'
                    )
                    $archiveTryIsOwned = (
                        $archiveDisposeStatements.Count -eq 1 -and
                        $archiveTry.Finally.Statements.Count -eq 1
                    )
                }
            }
        }
        if (
            $zipOutputStatements.Count -eq 1 -and
            $archiveOpenStatements.Count -eq 1 -and
            $archiveTryIsOwned -and
            $portableRootStatements.Count -eq 1 -and
            $portableDirectoryStatements.Count -eq 1 -and
            $portableExtractionStatements.Count -eq 1 -and
            $portableTreeSafetyStatements.Count -eq 1 -and
            $portableQueryStatements.Count -eq 1 -and
            $portableDesktopCountGuards.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableDesktopCountGuards[0].Parent,
                $packageTry.Body
            ) -and
            $portableAssignmentStatements.Count -eq 1 -and
            $portableDesktopGuards.Count -eq 1 -and
            $zipOutputAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $zipOutputAssignments[0],
                $zipOutputStatements[0]
            ) -and
            $portableRootAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableRootAssignments[0],
                $portableRootStatements[0]
            ) -and
            $portableFileAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableFileAssignments[0],
                $portableQueryStatements[0]
            ) -and
            $portableDesktopAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableDesktopAssignments[0],
                $portableAssignmentStatements[0]
            ) -and
            $zipOutputIndex -lt $archiveOpenIndex -and
            $portableRootIndex -eq $archiveOpenIndex + 2 -and
            $portableDirectoryIndex -eq $archiveOpenIndex + 3 -and
            $portableExtractionIndex -eq $archiveOpenIndex + 4 -and
            $portableTreeSafetyIndex -eq $archiveOpenIndex + 5 -and
            $portableQueryIndex -eq $archiveOpenIndex + 6 -and
            $portableCountGuardIndex -eq $archiveOpenIndex + 7 -and
            $portableAssignmentIndex -eq $archiveOpenIndex + 8 -and
            $portableSubsystemGuardIndex -eq $archiveOpenIndex + 9
        ) {
            $portableProvenanceIsOwned = $true
        }
        if (
            $portableProvenanceIsOwned -and
            $portableSidecarQueryStatements.Count -eq 1 -and
            $portableSidecarCountGuards.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableSidecarCountGuards[0].Parent,
                $packageTry.Body
            ) -and
            $portableSidecarAssignmentStatements.Count -eq 1 -and
            $portableSidecarGuards.Count -eq 1 -and
            $portableSidecarFileAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableSidecarFileAssignments[0],
                $portableSidecarQueryStatements[0]
            ) -and
            $portableSidecarAssignments.Count -eq 1 -and
            [object]::ReferenceEquals(
                $portableSidecarAssignments[0],
                $portableSidecarAssignmentStatements[0]
            ) -and
            $portableSidecarQueryIndex -eq $archiveOpenIndex + 10 -and
            $portableSidecarCountGuardIndex -eq $archiveOpenIndex + 11 -and
            $portableSidecarAssignmentIndex -eq $archiveOpenIndex + 12 -and
            $portableSidecarSubsystemGuardIndex -eq $archiveOpenIndex + 13
        ) {
            $portableSidecarProvenanceIsOwned = $true
        }
    }
    if (-not $portableProvenanceIsOwned) {
        Add-Failure `
            -Message "Windows packager must retain the active Portable desktop extraction provenance and GUI subsystem check."
    }
    if (-not $portableSidecarProvenanceIsOwned) {
        Add-Failure `
            -Message "Windows packager must retain the active Portable sidecar extraction provenance and console subsystem check."
    }

    try {
        $workspaceVersion = Get-CargoWorkspaceVersion -Text (
            Read-BoundedUtf8Text -Path $cargoManifestPath -MaximumBytes 131072
        )
        [string[]]$sourceVersions = @(
            $workspaceVersion
            Get-JsonReleaseVersion `
                -Text (
                    Read-BoundedUtf8Text `
                        -Path $packageManifestPath `
                        -MaximumBytes 262144
                ) `
                -Name "apps/desktop/package.json"
            Get-JsonReleaseVersion `
                -Text (
                    Read-BoundedUtf8Text `
                        -Path $tauriConfigurationPath `
                        -MaximumBytes 262144
                ) `
                -Name "apps/desktop/src-tauri/tauri.conf.json"
            Get-WokRouterLockVersions -Text (
                Read-BoundedUtf8Text -Path $cargoLockPath -MaximumBytes 8388608
            )
        )
        foreach ($sourceVersion in $sourceVersions) {
            if ($sourceVersion -cne $workspaceVersion) {
                throw "WokRouter product source versions must match exactly."
            }
        }
    }
    catch {
        Add-Failure -Message "WokRouter source versions are invalid: $($_.Exception.Message)"
    }

    Import-Module $releaseContractPath -Force
    try {
        [string[]] $expectedTargets = @(
            "aarch64-apple-darwin",
            "aarch64-pc-windows-msvc",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu"
        )
        [string[]] $expectedPayloads = @(
            "WokRouter-v0.1.22-Linux-arm64.AppImage",
            "WokRouter-v0.1.22-Linux-arm64.deb",
            "WokRouter-v0.1.22-Linux-arm64.rpm",
            "WokRouter-v0.1.22-Linux-x86_64.AppImage",
            "WokRouter-v0.1.22-Linux-x86_64.deb",
            "WokRouter-v0.1.22-Linux-x86_64.rpm",
            "WokRouter-v0.1.22-Windows-arm64-Portable.zip",
            "WokRouter-v0.1.22-Windows-arm64.msi",
            "WokRouter-v0.1.22-Windows-x86_64-Portable.zip",
            "WokRouter-v0.1.22-Windows-x86_64.msi",
            "WokRouter-v0.1.22-macOS-arm64.dmg",
            "WokRouter-v0.1.22-macOS-arm64.tar.gz",
            "WokRouter-v0.1.22-macOS-arm64.zip",
            "WokRouter-v0.1.22-macOS-x86_64.dmg",
            "WokRouter-v0.1.22-macOS-x86_64.tar.gz",
            "WokRouter-v0.1.22-macOS-x86_64.zip"
        )
        [string[]] $actualTargets = @(
            Get-WokRouterTargetContracts -Version "0.1.22" |
                ForEach-Object Target
        )
        [string[]] $actualPayloads = @(
            Get-WokRouterPayloadNames -Version "0.1.22"
        )
        if (
            [string]::Join("`n", $actualTargets) -cne
            [string]::Join("`n", $expectedTargets)
        ) {
            Add-Failure `
                -Message "Release contract must return the exact 6 ordinal target names."
        }
        if (
            [string]::Join("`n", $actualPayloads) -cne
            [string]::Join("`n", $expectedPayloads) -or
            $actualPayloads -match "unknown|pc-windows|apple-darwin"
        ) {
            Add-Failure `
                -Message "Release contract must return the exact 16 friendly payload names."
        }
    }
    catch {
        Add-Failure `
            -Message "Release asset contract could not be evaluated: $($_.Exception.Message)"
    }
    finally {
        Remove-Module WokRouter.ReleaseContract -ErrorAction SilentlyContinue
    }

    foreach ($packagerPath in @(
            $linuxPackagerPath,
            $macPackagerPath,
            $windowsPackagerPath
        )) {
        $packagerSource = Get-Content `
            -LiteralPath $packagerPath `
            -Raw `
            -Encoding UTF8
        if ($packagerSource -match "(?i)\b(?:skip|bypass)\b") {
            Add-Failure `
                -Message "Release packagers must not contain Skip or Bypass production paths."
        }
    }

    $release = (Get-Content -LiteralPath $releasePath -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $ci = (Get-Content -LiteralPath $ciPath -Raw -Encoding UTF8).Replace("`r`n", "`n")
    $development = Get-Content -LiteralPath $developmentPath -Raw -Encoding UTF8

    if ($release -notmatch '(?m)^      - "v\*"$') {
        Add-Failure -Message "Release workflow must verify WokRouter v* tag pushes."
    }
    if (
        $release -notmatch '(?m)^  workflow_dispatch:\s*$' -or
        $release -notmatch '(?m)^      release_tag:\s*$' -or
        $release -notmatch '(?m)^        required: true\s*$'
    ) {
        Add-Failure -Message "Release workflow must require a release_tag for manual verification."
    }
    if ($release -notmatch '(?m)^permissions:\n  contents: read\s*$') {
        Add-Failure -Message "Release workflow root permissions must be contents: read."
    }
    $concurrencyBlock = @'
concurrency:
  group: wokrouter-release-${{ github.event_name == 'workflow_dispatch' && inputs.release_tag || github.ref_name }}
  cancel-in-progress: false
'@
    if (-not (Test-ContainsExactBlock -Source $release -Block $concurrencyBlock)) {
        Add-Failure `
            -Message "Release workflow must serialize the same release tag without cancellation."
    }

    foreach ($providerVariable in @(
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY"
        )) {
        foreach ($workflow in @($release, $ci)) {
            $providerDefinitions = @(
                [regex]::Matches(
                    $workflow,
                    "(?m)^\s*$providerVariable`:\s*.*$"
                )
            )
            if (
                $workflow -notmatch "(?m)^  $providerVariable`: `"`"`$" -or
                $providerDefinitions.Count -ne 1
            ) {
                Add-Failure `
                    -Message "CI and release workflows must define '$providerVariable' exactly once as empty."
            }
        }
    }

    $versionJob = Get-JobBlock -Workflow $release -Name "release-version"
    $tagCheckout = @'
      - uses: actions/checkout@v6
        with:
          fetch-depth: 1
          ref: ${{ github.event_name == 'workflow_dispatch' && format('refs/tags/{0}', inputs.release_tag) || github.ref }}
'@
    if (
        $versionJob -notmatch [regex]::Escape('${{ inputs.release_tag }}') -or
        $versionJob -notmatch [regex]::Escape('${{ github.ref_name }}') -or
        $versionJob -notmatch "canonical WokRouter semver tag" -or
        $versionJob -notmatch [regex]::Escape('$tag.Substring(1)') -or
        -not (Test-ContainsExactBlock -Source $versionJob -Block $tagCheckout) -or
        $versionJob -notmatch '(?m)^          "source_sha=\$sourceSha" \|$' -or
        -not $versionJob.Contains("Read-ExactUtf8File") -or
        -not $versionJob.Contains("Get-CargoWorkspaceVersion") -or
        -not $versionJob.Contains("Get-JsonVersion") -or
        -not $versionJob.Contains("Get-LockPackageVersions") -or
        -not $versionJob.Contains(
            "WokRouter source version does not match release tag."
        )
    ) {
        Add-Failure `
            -Message "Release source and version must be resolved from the requested WokRouter tag commit and match every product source."
    }
    if ($release -match '(?m)^\s*WOKCORE_[A-Z_]*VERSION:') {
        Add-Failure -Message "WokRouter release version must not depend on a WokCore version."
    }

    $buildJob = Get-JobBlock -Workflow $release -Name "release-build"
    $sourceCheckout = @'
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
          ref: ${{ needs.release-version.outputs.source_sha }}
'@
    if (-not (Test-ContainsExactBlock -Source $buildJob -Block $sourceCheckout)) {
        Add-Failure `
            -Message "Release builds must checkout the commit resolved from the requested WokRouter tag."
    }
    $expectedPairs = @(
        @("windows-latest", "x86_64-pc-windows-msvc"),
        @("windows-latest", "aarch64-pc-windows-msvc"),
        @("macos-15-intel", "x86_64-apple-darwin"),
        @("macos-14", "aarch64-apple-darwin"),
        @("ubuntu-24.04", "x86_64-unknown-linux-gnu"),
        @("ubuntu-24.04-arm", "aarch64-unknown-linux-gnu")
    )
    foreach ($pair in $expectedPairs) {
        $pattern = "(?m)^          - os: $([regex]::Escape($pair[0]))\n            target: $([regex]::Escape($pair[1]))$"
        if ($buildJob -notmatch $pattern) {
            Add-Failure `
                -Message "Release matrix is missing '$($pair[1])' on '$($pair[0])'."
        }
    }
    if (
        @([regex]::Matches($buildJob, '(?m)^            target: ')).Count -ne
        6
    ) {
        Add-Failure `
            -Message "Release build matrix must contain exactly 6 targets."
    }
    foreach ($requiredText in @(
            "WOKROUTER_BUNDLE_KIND: online",
            'WOKROUTER_RELEASE_VERSION: ${{ needs.release-version.outputs.version }}',
            'WOKROUTER_TARGET_TRIPLE: ${{ matrix.target }}',
            "sudo apt-get install --yes --no-install-recommends",
            'name: wokrouter-payload-${{ matrix.target }}',
            'path: target/wokrouter-public-${{ matrix.target }}/*'
        )) {
        if (-not $buildJob.Contains($requiredText)) {
            Add-Failure -Message "Release build is missing required boundary text '$requiredText'."
        }
    }
    $arm64ToolCondition = (
        "if: runner.os == 'Windows' && " +
        "matrix.target == 'aarch64-pc-windows-msvc'"
    )
    if (
        @(
            $buildJob -split "`n" |
                Where-Object {
                    $_.Trim() -ceq 'pnpm --dir apps/desktop tauri build `'
                }
        ).Count -ne 3 -or
        -not $buildJob.Contains($arm64ToolCondition) -or
        -not $buildJob.Contains(
            "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
        ) -or
        -not $buildJob.Contains("-WindowStyle Hidden") -or
        $buildJob.Contains("ToolAdapterPath")
    ) {
        Add-Failure `
            -Message "Release builds must run one explicit native packager path per platform and install Windows ARM64 tools."
    }
    foreach ($platformBuild in @(
            @("runner.os == 'Linux'", "--bundles appimage,deb,rpm"),
            @("runner.os == 'macOS'", "--bundles app,dmg"),
            @("runner.os == 'Windows'", "--bundles msi")
        )) {
        $pattern = (
            "(?ms)^      - name: Build .*?`n" +
            "        if: $([regex]::Escape($platformBuild[0]))`n" +
            ".*?$([regex]::Escape($platformBuild[1]))"
        )
        if ($buildJob -notmatch $pattern) {
            Add-Failure `
                -Message "Release build is missing the scoped '$($platformBuild[1])' command."
        }
        if (
            @(
                $buildJob -split "`n" |
                    Where-Object {
                        $_.Trim() -ceq "$($platformBuild[1]) ``"
                    }
            ).Count -ne 1
        ) {
            Add-Failure `
                -Message "Release build must contain one executable '$($platformBuild[1])' line."
        }
    }
    foreach ($packager in @(
            "package-linux-assets.ps1",
            "package-macos-assets.ps1",
            "package-windows-assets.ps1"
        )) {
        if (
            @(
                $buildJob -split "`n" |
                    Where-Object {
                        $_.Trim() -ceq "& tests/release/$packager ``"
                    }
            ).Count -ne 1
        ) {
            Add-Failure `
                -Message "Release build must execute '$packager' exactly once."
        }
    }

    $compatibilityJob = Get-JobBlock -Workflow $release -Name "release-compatibility"
    if (-not (Test-ContainsExactBlock -Source $compatibilityJob -Block $sourceCheckout)) {
        Add-Failure `
            -Message "Release compatibility tests must checkout the requested WokRouter tag commit."
    }
    foreach ($testName in @(
            "current_wokrouter_accepts_current_wokcore",
            "compatible_handshake_accepts_unknown_same_major_fields",
            "legacy_same_major_runtime_without_installation_id_remains_running",
            "non_overlapping_api_major_is_incompatible_without_http_fallback",
            "an_existing_compatible_install_is_never_overwritten",
            "installing_wokcore_does_not_modify_wokrouter_binary_or_version",
            "wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1",
            "wokcore_install_missing_v2_manifest_falls_back_to_the_signed_v1_release",
            "wokcore_install_present_invalid_v2_manifest_never_downgrades_to_v1",
            "wokcore_install_rejects_a_signed_v1_schema_at_the_v2_endpoint_without_downgrading"
        )) {
        $testPattern = "(?m)^        run: cargo test .* $([regex]::Escape($testName)) --locked$"
        if ($compatibilityJob -notmatch $testPattern) {
            Add-Failure `
                -Message "Release compatibility matrix must execute '$testName' as a Cargo test."
        }
    }

    $verifyJob = Get-JobBlock -Workflow $release -Name "release-assemble"
    foreach ($requiredText in @(
            "release-build",
            "release-compatibility",
            "merge-multiple: true",
            "Get-WokRouterPayloadNames",
            "WOKROUTER_MINISIGN_SECRET_KEY",
            "sign-release-bundle.ps1",
            "verify-release-bundle.ps1"
        )) {
        if (-not $verifyJob.Contains($requiredText)) {
            Add-Failure -Message "Release verification is missing '$requiredText'."
        }
    }
    $assembleCheckout = @'
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
          ref: ${{ needs.release-version.outputs.source_sha }}
'@
    if (
        -not (Test-ContainsExactBlock -Source $verifyJob -Block $assembleCheckout) -or
        -not $verifyJob.Contains("sudo apt-get install --yes --no-install-recommends minisign") -or
        -not $verifyJob.Contains("pattern: wokrouter-payload-*") -or
        -not $verifyJob.Contains(
            'name: wokrouter-${{ needs.release-version.outputs.tag }}-signed'
        )
    ) {
        Add-Failure `
            -Message "Release assembly must checkout the verified source and produce one exact signed bundle artifact."
    }
    $payloadIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            Get-WokRouterPayloadNames -Version '
    $secretIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^          WOKROUTER_MINISIGN_SECRET_KEY: '
    $signIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            & tests/release/sign-release-bundle\.ps1 `\s*$'
    $localVerifyIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^            & tests/release/verify-release-bundle\.ps1 `\s*$'
    $signedUploadIndex = Get-SourceMatchIndex `
        -Source $verifyJob `
        -Pattern '(?m)^          name: wokrouter-\$\{\{ needs\.release-version\.outputs\.tag \}\}-signed$'
    if (
        -not $verifyJob.Contains('$items.Count -ne 16') -or
        $payloadIndex -lt 0 -or
        $secretIndex -le $payloadIndex -or
        $signIndex -le $secretIndex -or
        $localVerifyIndex -le $signIndex -or
        $signedUploadIndex -le $localVerifyIndex -or
        -not $verifyJob.Contains("-PublicKeyPath release/minisign.pub")
    ) {
        Add-Failure `
            -Message "Release assembly must require 16 payloads before reading the secret, then sign and locally verify before upload."
    }

    $publishJob = Get-JobBlock -Workflow $release -Name "publish"
    $draftCreateBlock = @'
            gh release create "$RELEASE_TAG" \
              --repo "$GITHUB_REPOSITORY" \
              --verify-tag \
              --draft \
'@
    $publishEditBlock = @'
          gh release edit "$RELEASE_TAG" \
            --repo "$GITHUB_REPOSITORY" \
            --draft=false
'@
    $preMutationIdentityBlock = @'
          begin_release_mutation() {
            if [[ "$release_mutation_started" == "false" ]]; then
              require_remote_tag_commit
              release_mutation_started=true
            fi
          }
'@
    $preCreateIdentityBlock = @'
            begin_release_mutation
            gh release create "$RELEASE_TAG" \
'@
    $preDeleteIdentityBlock = @'
            begin_release_mutation
            gh release delete-asset "$RELEASE_TAG" "$asset" \
'@
    $preUploadIdentityBlock = @'
          begin_release_mutation
          gh release upload "$RELEASE_TAG" dist/* \
'@
    $prePublicationIdentityBlock = @'
          require_remote_tag_commit
          gh release edit "$RELEASE_TAG" \
'@
    if (
        $publishJob -notmatch [regex]::Escape("startsWith(github.ref, 'refs/tags/')") -or
        $publishJob -notmatch '(?m)^    permissions:\n      contents: write\s*$' -or
        $publishJob -notmatch 'gh release create "\$RELEASE_TAG"' -or
        $publishJob -notmatch '--verify-tag' -or
        $publishJob -notmatch 'isDraft' -or
        $publishJob -notmatch 'gh release delete-asset' -or
        $publishJob -notmatch 'gh release download' -or
        $publishJob -notmatch 'verify-release-bundle\.ps1' -or
        $publishJob -notmatch 'gh release edit "\$RELEASE_TAG"' -or
        $publishJob -notmatch '--draft=false' -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $draftCreateBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $publishEditBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $preMutationIdentityBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $preCreateIdentityBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $preDeleteIdentityBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $preUploadIdentityBlock) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $prePublicationIdentityBlock) -or
        -not $publishJob.Contains("gh api") -or
        -not $publishJob.Contains("SOURCE_SHA") -or
        -not $publishJob.Contains("Remote WokRouter tag commit does not match source SHA.") -or
        $publishJob -notmatch [regex]::Escape('--repo "$GITHUB_REPOSITORY"') -or
        @([regex]::Matches($publishJob, '\bgh release (?:view|create|delete-asset|upload|download|edit)\b')).Count -ne
        @([regex]::Matches(
                $publishJob,
                [regex]::Escape('--repo "$GITHUB_REPOSITORY"')
            )).Count
    ) {
        Add-Failure -Message "Publishing must be tag-only, verified, scoped to contents: write, and use an explicit GitHub repository."
    }
    if (
        @([regex]::Matches($release, '(?m)^\s+contents: write\s*$')).Count -ne
        1 -or
        $publishJob -notmatch (
            "(?m)^    if: github\.event_name == 'push' && " +
            "startsWith\(github\.ref, 'refs/tags/'\)$"
        ) -or
        -not (Test-ContainsExactBlock -Source $publishJob -Block $assembleCheckout) -or
        -not $publishJob.Contains(
            'name: wokrouter-${{ needs.release-version.outputs.tag }}-signed'
        ) -or
        -not $publishJob.Contains("-PublicKeyPath release/minisign.pub") -or
        -not $publishJob.Contains("Expected exactly 35 signed WokRouter assets") -or
        -not $publishJob.Contains('"${#local_assets[@]}" -ne 35') -or
        -not $publishJob.Contains(
            "sudo apt-get install --yes --no-install-recommends minisign"
        ) -or
        -not $publishJob.Contains(
            "The WokRouter draft became public before asset cleanup."
        ) -or
        $publishJob -notmatch '(?m)^              --draft \\\s*$'
    ) {
        Add-Failure `
            -Message "Only a tag push may publish the exact externally verified 35-file bundle."
    }
    $preMutationVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^      - name: Verify the signed bundle before release mutation$'
    $firstPublishVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          & tests/release/verify-release-bundle\.ps1 `\s*$'
    $releaseView = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release view "\$RELEASE_TAG" \\$'
    $draftGuard = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            if \[\[ "\$\(jq -r ''\.isDraft'''
    $draftCreate = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release create "\$RELEASE_TAG" \\$'
    $assetCleanup = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^            gh release delete-asset "\$RELEASE_TAG" "\$asset" \\$'
    $preUploadGuard = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern (
            '(?ms)^          if \[\[ "\$\(\n' +
            '            gh release view "\$RELEASE_TAG" \\\n' +
            '              --repo "\$GITHUB_REPOSITORY" \\\n' +
            '              --json isDraft \\\n' +
            '              --jq ''\.isDraft''\n' +
            '          \)" != "true" \]\]; then\n' +
            '            echo "The WokRouter draft became public before upload\." >&2\n' +
            '            exit 1\n' +
            '          fi\n' +
            '          begin_release_mutation\n' +
            '          gh release upload "\$RELEASE_TAG" dist/\* \\$'
        )
    $assetUpload = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release upload "\$RELEASE_TAG" dist/\* \\$'
    $remoteDownload = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release download "\$RELEASE_TAG" \\$'
    $remoteVerify = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          pwsh tests/release/verify-release-bundle\.ps1 \\$'
    $publishRelease = Get-SourceMatchIndex `
        -Source $publishJob `
        -Pattern '(?m)^          gh release edit "\$RELEASE_TAG" \\$'
    if (
        $preMutationVerify -lt 0 -or
        $firstPublishVerify -le $preMutationVerify -or
        $releaseView -le $firstPublishVerify -or
        $draftGuard -le $releaseView -or
        $draftCreate -le $draftGuard -or
        $assetCleanup -le $draftGuard -or
        $preUploadGuard -le $assetCleanup -or
        $assetUpload -le $preUploadGuard -or
        $remoteDownload -le $assetUpload -or
        $remoteVerify -le $remoteDownload -or
        $publishRelease -le $remoteVerify
    ) {
        Add-Failure `
            -Message "Publishing must guard a draft, clear stale draft assets, upload, re-download, verify, and only then publish."
    }

    $signingSteps = @(
        [regex]::Matches(
            $verifyJob,
            "(?ms)^      - name: Sign and locally verify the release bundle\s*$.*?(?=^      - |\z)"
        )
    )
    $signingStep = if ($signingSteps.Count -eq 1) {
        $signingSteps[0].Value
    }
    else {
        ""
    }
    $releaseWithoutSigningStep = if ($signingStep -eq "") {
        $release
    }
    else {
        $release.Replace($signingStep, "")
    }
    if (
        $signingStep -eq "" -or
        -not $signingStep.Contains("WOKROUTER_MINISIGN_SECRET_KEY") -or
        $releaseWithoutSigningStep.Contains("WOKROUTER_MINISIGN_SECRET_KEY")
    ) {
        Add-Failure `
            -Message "The WOKROUTER_MINISIGN_SECRET_KEY secret must appear only in the release-assemble signing step."
    }
    if (
        $signingStep -notmatch (
            '(?ms)try \{\n\s+\[IO\.File\]::WriteAllText\(.*?' +
            'sign-release-bundle\.ps1.*?finally \{.*?' +
            '\[IO\.File\]::WriteAllBytes\(.*?' +
            'Remove-Item -LiteralPath \$secretPath -Force'
        )
    ) {
        Add-Failure `
            -Message "The plaintext Minisign key write must be covered by secure finally cleanup."
    }
    if ($release.Contains("Expected five release archives")) {
        Add-Failure `
            -Message "The old five-archive release verification path must be removed."
    }

    foreach ($requiredFact in @(
            "wokrouter-test-host.exe",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "online WokRouter",
            "WokRouter tag",
            "independent",
            "exactly 16",
            "exactly 35",
            "release/minisign.pub",
            "immutable"
        )) {
        if ($development -notmatch [regex]::Escape($requiredFact)) {
            Add-Failure `
                -Message "Development docs must describe release fact '$requiredFact'."
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "RELEASE CONTRACT ERROR: $failure"
    }
    exit 1
}

Write-Host "Release contract passed."
