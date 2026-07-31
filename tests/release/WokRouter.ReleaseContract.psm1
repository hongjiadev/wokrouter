function Assert-WokRouterReleaseVersion {
    param([Parameter(Mandatory)][string] $Version)

    if (
        $Version -cnotmatch (
            "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\." +
            "(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?" +
            "(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
        )
    ) {
        throw "WokRouter release version must be canonical SemVer."
    }
}

function Get-WokRouterTargetContracts {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $Version)

    Assert-WokRouterReleaseVersion -Version $Version
    $contracts = @(
        [pscustomobject]@{
            Target = "x86_64-pc-windows-msvc"
            System = "Windows"
            Architecture = "x86_64"
            Version = $Version
        },
        [pscustomobject]@{
            Target = "aarch64-pc-windows-msvc"
            System = "Windows"
            Architecture = "arm64"
            Version = $Version
        },
        [pscustomobject]@{
            Target = "x86_64-apple-darwin"
            System = "macOS"
            Architecture = "x86_64"
            Version = $Version
        },
        [pscustomobject]@{
            Target = "aarch64-apple-darwin"
            System = "macOS"
            Architecture = "arm64"
            Version = $Version
        },
        [pscustomobject]@{
            Target = "x86_64-unknown-linux-gnu"
            System = "Linux"
            Architecture = "x86_64"
            Version = $Version
        },
        [pscustomobject]@{
            Target = "aarch64-unknown-linux-gnu"
            System = "Linux"
            Architecture = "arm64"
            Version = $Version
        }
    )
    [Array]::Sort(
        $contracts,
        [System.Collections.Generic.Comparer[object]]::Create(
            [System.Comparison[object]] {
                param($left, $right)
                return [StringComparer]::Ordinal.Compare(
                    [string] $left.Target,
                    [string] $right.Target
                )
            }
        )
    )
    return $contracts
}

function Get-WokRouterPayloadNames {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $Version)

    $names = [Collections.Generic.List[string]]::new()
    foreach ($contract in Get-WokRouterTargetContracts -Version $Version) {
        $formats = switch ($contract.System) {
            "Linux" { @("AppImage", "deb", "rpm") }
            "macOS" { @("dmg", "tar.gz", "zip") }
            "Windows" { @("msi", "Portable.zip") }
            default { throw "Unsupported release system '$($contract.System)'." }
        }
        $prefix = (
            "WokRouter-v$Version-$($contract.System)-" +
            $contract.Architecture
        )
        foreach ($format in $formats) {
            $names.Add($(if ($format -ceq "Portable.zip") {
                "$prefix-Portable.zip"
            } else {
                "$prefix.$format"
            }))
        }
    }
    [string[]] $orderedNames = $names.ToArray()
    [Array]::Sort($orderedNames, [StringComparer]::Ordinal)
    return $orderedNames
}

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

Export-ModuleMember `
    -Function Get-WokRouterTargetContracts, Get-WokRouterPayloadNames, Get-PeSubsystem
