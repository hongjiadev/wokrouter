[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$forbiddenFields = @{
    "request_body" = $true
    "response_body" = $true
    "prompt" = $true
    "tool_arguments" = $true
    "authorization" = $true
}
$persistentRustModels = @(
    @{
        RelativePath = "crates/wokrouter-storage/src/config/model.rs"
        Structs = @("AppConfig", "VersionedConfig", "ServerConfig", "UiConfig")
    },
    @{
        RelativePath = "crates/wokrouter-storage/src/state/store.rs"
        Structs = @("RequestMetric")
    }
)
$rustFileCount = 0
$migrationFiles = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
$findings = [System.Collections.Generic.List[object]]::new()

function Test-IdentifierStart {
    param([char]$Character)

    return [char]::IsLetter($Character) -or $Character -eq "_"
}

function Test-IdentifierContinue {
    param([char]$Character)

    return [char]::IsLetterOrDigit($Character) -or $Character -eq "_"
}

function New-Token {
    param(
        [Parameter(Mandatory)]
        [string]$Kind,

        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Value,

        [Parameter(Mandatory)]
        [int]$Line
    )

    return [pscustomobject]@{
        Kind = $Kind
        Value = $Value
        Line = $Line
    }
}

function Get-RustTokens {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Content
    )

    $tokens = [System.Collections.Generic.List[object]]::new()
    $index = 0
    $line = 1

    while ($index -lt $Content.Length) {
        $character = $Content[$index]

        if ($character -eq "`r" -or $character -eq "`n") {
            if (
                $character -eq "`r" -and
                $index + 1 -lt $Content.Length -and
                $Content[$index + 1] -eq "`n"
            ) {
                $index += 1
            }
            $line += 1
            $index += 1
            continue
        }
        if ([char]::IsWhiteSpace($character)) {
            $index += 1
            continue
        }

        if (
            $character -eq "/" -and
            $index + 1 -lt $Content.Length -and
            $Content[$index + 1] -eq "/"
        ) {
            $index += 2
            while (
                $index -lt $Content.Length -and
                $Content[$index] -ne "`r" -and
                $Content[$index] -ne "`n"
            ) {
                $index += 1
            }
            continue
        }
        if (
            $character -eq "/" -and
            $index + 1 -lt $Content.Length -and
            $Content[$index + 1] -eq "*"
        ) {
            $commentDepth = 1
            $index += 2
            while ($index -lt $Content.Length -and $commentDepth -gt 0) {
                if (
                    $Content[$index] -eq "/" -and
                    $index + 1 -lt $Content.Length -and
                    $Content[$index + 1] -eq "*"
                ) {
                    $commentDepth += 1
                    $index += 2
                    continue
                }
                if (
                    $Content[$index] -eq "*" -and
                    $index + 1 -lt $Content.Length -and
                    $Content[$index + 1] -eq "/"
                ) {
                    $commentDepth -= 1
                    $index += 2
                    continue
                }
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                $index += 1
            }
            continue
        }

        if ($character -eq "r") {
            $hashIndex = $index + 1
            while ($hashIndex -lt $Content.Length -and $Content[$hashIndex] -eq "#") {
                $hashIndex += 1
            }
            if ($hashIndex -lt $Content.Length -and $Content[$hashIndex] -eq '"') {
                $hashCount = $hashIndex - $index - 1
                $index = $hashIndex + 1
                while ($index -lt $Content.Length) {
                    if ($Content[$index] -eq "`n") {
                        $line += 1
                    }
                    if ($Content[$index] -eq '"') {
                        $closing = $true
                        for ($offset = 0; $offset -lt $hashCount; $offset += 1) {
                            if (
                                $index + 1 + $offset -ge $Content.Length -or
                                $Content[$index + 1 + $offset] -ne "#"
                            ) {
                                $closing = $false
                                break
                            }
                        }
                        if ($closing) {
                            $index += $hashCount + 1
                            break
                        }
                    }
                    $index += 1
                }
                continue
            }
            if (
                $hashIndex -eq $index + 2 -and
                $hashIndex -lt $Content.Length -and
                (Test-IdentifierStart $Content[$hashIndex])
            ) {
                $identifierLine = $line
                $identifierStart = $hashIndex
                $index = $hashIndex + 1
                while (
                    $index -lt $Content.Length -and
                    (Test-IdentifierContinue $Content[$index])
                ) {
                    $index += 1
                }
                $value = $Content.Substring($identifierStart, $index - $identifierStart)
                $tokens.Add((New-Token -Kind "Identifier" -Value $value -Line $identifierLine))
                continue
            }
        }

        if ($character -eq '"') {
            $index += 1
            while ($index -lt $Content.Length) {
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                if ($Content[$index] -eq "\") {
                    $index += 2
                    continue
                }
                if ($Content[$index] -eq '"') {
                    $index += 1
                    break
                }
                $index += 1
            }
            continue
        }
        if ($character -eq "'") {
            if (
                $index + 1 -lt $Content.Length -and
                (Test-IdentifierStart $Content[$index + 1])
            ) {
                $identifierEnd = $index + 2
                while (
                    $identifierEnd -lt $Content.Length -and
                    (Test-IdentifierContinue $Content[$identifierEnd])
                ) {
                    $identifierEnd += 1
                }
                if (
                    $identifierEnd -ge $Content.Length -or
                    $Content[$identifierEnd] -ne "'"
                ) {
                    $tokens.Add((New-Token -Kind "Symbol" -Value "'" -Line $line))
                    $index += 1
                    continue
                }
            }
            $index += 1
            while ($index -lt $Content.Length) {
                if ($Content[$index] -eq "\") {
                    $index += 2
                    continue
                }
                if ($Content[$index] -eq "'") {
                    $index += 1
                    break
                }
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                $index += 1
            }
            continue
        }

        if (Test-IdentifierStart $character) {
            $identifierLine = $line
            $identifierStart = $index
            $index += 1
            while (
                $index -lt $Content.Length -and
                (Test-IdentifierContinue $Content[$index])
            ) {
                $index += 1
            }
            $value = $Content.Substring($identifierStart, $index - $identifierStart)
            $tokens.Add((New-Token -Kind "Identifier" -Value $value -Line $identifierLine))
            continue
        }

        $tokens.Add((New-Token -Kind "Symbol" -Value ([string]$character) -Line $line))
        $index += 1
    }

    return @($tokens)
}

function Add-Finding {
    param(
        [Parameter(Mandatory)]
        [object]$Token,

        [Parameter(Mandatory)]
        [string]$Path
    )

    $findings.Add([pscustomobject]@{
            Field = $Token.Value
            Path = $Path
            Line = $Token.Line
        })
}

function Find-ForbiddenRustFields {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [string[]]$Structs,

        [Parameter(Mandatory)]
        [string]$Path
    )

    $persistentStructs = @{}
    foreach ($structName in $Structs) {
        $persistentStructs[$structName.ToLowerInvariant()] = $true
    }

    for ($index = 0; $index + 1 -lt $Tokens.Count; $index += 1) {
        if (
            $Tokens[$index].Kind -ne "Identifier" -or
            -not $Tokens[$index].Value.Equals("struct", [System.StringComparison]::Ordinal)
        ) {
            continue
        }

        $nameToken = $Tokens[$index + 1]
        if (
            $nameToken.Kind -ne "Identifier" -or
            -not $persistentStructs.ContainsKey($nameToken.Value.ToLowerInvariant())
        ) {
            continue
        }

        $openBrace = $index + 2
        while (
            $openBrace -lt $Tokens.Count -and
            $Tokens[$openBrace].Value -ne "{" -and
            $Tokens[$openBrace].Value -ne ";"
        ) {
            $openBrace += 1
        }
        if ($openBrace -ge $Tokens.Count -or $Tokens[$openBrace].Value -ne "{") {
            continue
        }

        $braceDepth = 1
        $bracketDepth = 0
        for ($fieldIndex = $openBrace + 1; $fieldIndex -lt $Tokens.Count; $fieldIndex += 1) {
            $token = $Tokens[$fieldIndex]
            if ($token.Value -eq "{") {
                $braceDepth += 1
                continue
            }
            if ($token.Value -eq "}") {
                $braceDepth -= 1
                if ($braceDepth -eq 0) {
                    $index = $fieldIndex
                    break
                }
                continue
            }
            if ($token.Value -eq "[") {
                $bracketDepth += 1
                continue
            }
            if ($token.Value -eq "]") {
                $bracketDepth -= 1
                continue
            }
            if (
                $braceDepth -eq 1 -and
                $bracketDepth -eq 0 -and
                $token.Kind -eq "Identifier" -and
                $fieldIndex + 1 -lt $Tokens.Count -and
                $Tokens[$fieldIndex + 1].Value -eq ":" -and
                (
                    $fieldIndex + 2 -ge $Tokens.Count -or
                    $Tokens[$fieldIndex + 2].Value -ne ":"
                ) -and
                $forbiddenFields.ContainsKey($token.Value.ToLowerInvariant())
            ) {
                Add-Finding -Token $token -Path $Path
            }
        }
    }
}

function Get-SqlTokens {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Content
    )

    $tokens = [System.Collections.Generic.List[object]]::new()
    $index = 0
    $line = 1

    while ($index -lt $Content.Length) {
        $character = $Content[$index]

        if ($character -eq "`r" -or $character -eq "`n") {
            if (
                $character -eq "`r" -and
                $index + 1 -lt $Content.Length -and
                $Content[$index + 1] -eq "`n"
            ) {
                $index += 1
            }
            $line += 1
            $index += 1
            continue
        }
        if ([char]::IsWhiteSpace($character)) {
            $index += 1
            continue
        }

        if (
            $character -eq "-" -and
            $index + 1 -lt $Content.Length -and
            $Content[$index + 1] -eq "-"
        ) {
            $index += 2
            while (
                $index -lt $Content.Length -and
                $Content[$index] -ne "`r" -and
                $Content[$index] -ne "`n"
            ) {
                $index += 1
            }
            continue
        }
        if (
            $character -eq "/" -and
            $index + 1 -lt $Content.Length -and
            $Content[$index + 1] -eq "*"
        ) {
            $index += 2
            while ($index + 1 -lt $Content.Length) {
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                if ($Content[$index] -eq "*" -and $Content[$index + 1] -eq "/") {
                    $index += 2
                    break
                }
                $index += 1
            }
            continue
        }

        if ($character -eq "'") {
            $index += 1
            while ($index -lt $Content.Length) {
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                if ($Content[$index] -eq "'") {
                    if ($index + 1 -lt $Content.Length -and $Content[$index + 1] -eq "'") {
                        $index += 2
                        continue
                    }
                    $index += 1
                    break
                }
                $index += 1
            }
            continue
        }

        if ($character -eq '"' -or $character -eq "[" -or $character -eq "``") {
            $identifierLine = $line
            $closingCharacter = $character
            if ($character -eq "[") {
                $closingCharacter = "]"
            }
            $index += 1
            $valueBuilder = [System.Text.StringBuilder]::new()
            while ($index -lt $Content.Length) {
                if ($Content[$index] -eq "`n") {
                    $line += 1
                }
                if ($Content[$index] -eq $closingCharacter) {
                    if (
                        $index + 1 -lt $Content.Length -and
                        $Content[$index + 1] -eq $closingCharacter
                    ) {
                        $null = $valueBuilder.Append($closingCharacter)
                        $index += 2
                        continue
                    }
                    $index += 1
                    break
                }
                $null = $valueBuilder.Append($Content[$index])
                $index += 1
            }
            $tokens.Add((
                    New-Token `
                        -Kind "Identifier" `
                        -Value $valueBuilder.ToString() `
                        -Line $identifierLine
                ))
            continue
        }

        if (Test-IdentifierStart $character) {
            $identifierLine = $line
            $identifierStart = $index
            $index += 1
            while (
                $index -lt $Content.Length -and
                (Test-IdentifierContinue $Content[$index])
            ) {
                $index += 1
            }
            $value = $Content.Substring($identifierStart, $index - $identifierStart)
            $tokens.Add((New-Token -Kind "Identifier" -Value $value -Line $identifierLine))
            continue
        }

        $tokens.Add((New-Token -Kind "Symbol" -Value ([string]$character) -Line $line))
        $index += 1
    }

    return @($tokens)
}

function Test-SqlKeyword {
    param(
        [Parameter(Mandatory)]
        [object]$Token,

        [Parameter(Mandatory)]
        [string]$Keyword
    )

    return (
        $Token.Kind -eq "Identifier" -and
        $Token.Value.Equals($Keyword, [System.StringComparison]::OrdinalIgnoreCase)
    )
}

function Test-SqlConstraintKeyword {
    param(
        [Parameter(Mandatory)]
        [object]$Token
    )

    return @(
        "constraint",
        "primary",
        "unique",
        "check",
        "foreign",
        "exclude"
    ) -contains $Token.Value.ToLowerInvariant()
}

function Test-CreateTableColumn {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [int]$End,

        [Parameter(Mandatory)]
        [string]$Path
    )

    if ($Start -gt $End) {
        return
    }
    $columnToken = $Tokens[$Start]
    if (
        $columnToken.Kind -eq "Identifier" -and
        -not (Test-SqlConstraintKeyword $columnToken) -and
        $forbiddenFields.ContainsKey($columnToken.Value.ToLowerInvariant())
    ) {
        Add-Finding -Token $columnToken -Path $Path
    }
}

function Find-ForbiddenSqlColumns {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [string]$Path
    )

    for ($index = 0; $index -lt $Tokens.Count; $index += 1) {
        if (Test-SqlKeyword -Token $Tokens[$index] -Keyword "create") {
            $cursor = $index + 1
            if (
                $cursor -lt $Tokens.Count -and
                (
                    (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "temp") -or
                    (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "temporary")
                )
            ) {
                $cursor += 1
            }
            if (
                $cursor -ge $Tokens.Count -or
                -not (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "table")
            ) {
                continue
            }
            $cursor += 1
            if (
                $cursor + 2 -lt $Tokens.Count -and
                (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "if") -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 1] -Keyword "not") -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 2] -Keyword "exists")
            ) {
                $cursor += 3
            }
            while ($cursor -lt $Tokens.Count -and $Tokens[$cursor].Value -ne "(") {
                if ($Tokens[$cursor].Value -eq ";") {
                    break
                }
                $cursor += 1
            }
            if ($cursor -ge $Tokens.Count -or $Tokens[$cursor].Value -ne "(") {
                continue
            }

            $depth = 1
            $segmentStart = $cursor + 1
            for ($columnIndex = $cursor + 1; $columnIndex -lt $Tokens.Count; $columnIndex += 1) {
                if ($Tokens[$columnIndex].Value -eq "(") {
                    $depth += 1
                    continue
                }
                if ($Tokens[$columnIndex].Value -eq ")") {
                    $depth -= 1
                    if ($depth -eq 0) {
                        Test-CreateTableColumn `
                            -Tokens $Tokens `
                            -Start $segmentStart `
                            -End ($columnIndex - 1) `
                            -Path $Path
                        $index = $columnIndex
                        break
                    }
                    continue
                }
                if ($depth -eq 1 -and $Tokens[$columnIndex].Value -eq ",") {
                    Test-CreateTableColumn `
                        -Tokens $Tokens `
                        -Start $segmentStart `
                        -End ($columnIndex - 1) `
                        -Path $Path
                    $segmentStart = $columnIndex + 1
                }
            }
            continue
        }

        if (
            (Test-SqlKeyword -Token $Tokens[$index] -Keyword "alter") -and
            $index + 1 -lt $Tokens.Count -and
            (Test-SqlKeyword -Token $Tokens[$index + 1] -Keyword "table")
        ) {
            $cursor = $index + 2
            while (
                $cursor -lt $Tokens.Count -and
                -not (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "add") -and
                $Tokens[$cursor].Value -ne ";"
            ) {
                $cursor += 1
            }
            if (
                $cursor -ge $Tokens.Count -or
                -not (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "add")
            ) {
                continue
            }
            $cursor += 1
            if (
                $cursor -lt $Tokens.Count -and
                (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "column")
            ) {
                $cursor += 1
            }
            if (
                $cursor + 2 -lt $Tokens.Count -and
                (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "if") -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 1] -Keyword "not") -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 2] -Keyword "exists")
            ) {
                $cursor += 3
            }
            if (
                $cursor -lt $Tokens.Count -and
                $Tokens[$cursor].Kind -eq "Identifier" -and
                -not (Test-SqlConstraintKeyword $Tokens[$cursor]) -and
                $forbiddenFields.ContainsKey($Tokens[$cursor].Value.ToLowerInvariant())
            ) {
                Add-Finding -Token $Tokens[$cursor] -Path $Path
            }
        }
    }
}

foreach ($modelSet in $persistentRustModels) {
    $path = Join-Path $rootPath $modelSet.RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        continue
    }

    $rustFileCount += 1
    $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
    $tokens = @(Get-RustTokens -Content $content)
    Find-ForbiddenRustFields -Tokens $tokens -Structs $modelSet.Structs -Path $path
}

$cratesRoot = Join-Path $rootPath "crates"
if (Test-Path -LiteralPath $cratesRoot -PathType Container) {
    foreach ($file in Get-ChildItem -LiteralPath $cratesRoot -Recurse -File -Filter "*.sql") {
        if ($file.FullName -match "[\\/]migrations[\\/]") {
            $migrationFiles.Add($file)
        }
    }
}

foreach ($file in $migrationFiles) {
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    $tokens = @(Get-SqlTokens -Content $content)
    Find-ForbiddenSqlColumns -Tokens $tokens -Path $file.FullName
}

if (($rustFileCount + $migrationFiles.Count) -eq 0) {
    Write-Host "No persistence models or SQL migrations were found below '$rootPath'."
    exit 2
}

if ($findings.Count -gt 0) {
    foreach ($finding in $findings) {
        Write-Host (
            "Forbidden persisted field '{0}' at {1}:{2}" -f
            $finding.Field,
            $finding.Path,
            $finding.Line
        )
    }
    exit 1
}

Write-Host (
    "Persistence privacy check passed ({0} Rust model file(s), {1} migration file(s))." -f
    $rustFileCount,
    $migrationFiles.Count
)
