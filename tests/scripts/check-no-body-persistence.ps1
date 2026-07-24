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
$inventoryErrors = [System.Collections.Generic.List[string]]::new()
$sqlParseErrors = [System.Collections.Generic.List[object]]::new()

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
        [AllowEmptyCollection()]
        [System.Collections.Generic.HashSet[string]]$FoundStructs,

        [Parameter(Mandatory)]
        [string]$Path
    )

    $persistentStructs = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($structName in $Structs) {
        $null = $persistentStructs.Add($structName)
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
            -not $persistentStructs.Contains($nameToken.Value)
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

        $null = $FoundStructs.Add($nameToken.Value)
        $braceDepth = 1
        $bracketDepth = 0
        $parenthesisDepth = 0
        $angleDepth = 0
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
                if ($bracketDepth -gt 0) {
                    $bracketDepth -= 1
                }
                continue
            }
            if ($token.Value -eq "(") {
                $parenthesisDepth += 1
                continue
            }
            if ($token.Value -eq ")") {
                if ($parenthesisDepth -gt 0) {
                    $parenthesisDepth -= 1
                }
                continue
            }
            if ($token.Value -eq "<") {
                $angleDepth += 1
                continue
            }
            if ($token.Value -eq ">") {
                if ($angleDepth -gt 0) {
                    $angleDepth -= 1
                }
                continue
            }
            if (
                $braceDepth -eq 1 -and
                $bracketDepth -eq 0 -and
                $parenthesisDepth -eq 0 -and
                $angleDepth -eq 0 -and
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

function Add-SqlParseError {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [int]$Line,

        [Parameter(Mandatory)]
        [string]$Message
    )

    $sqlParseErrors.Add([pscustomobject]@{
            Path = $Path
            Line = $Line
            Message = $Message
        })
}

function Get-SqlObjectNameEnd {
    param(
        [Parameter(Mandatory)]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [int]$Start
    )

    if ($Start -ge $Tokens.Count -or $Tokens[$Start].Kind -ne "Identifier") {
        return $Start
    }

    $cursor = $Start + 1
    while (
        $cursor + 1 -lt $Tokens.Count -and
        $Tokens[$cursor].Value -eq "." -and
        $Tokens[$cursor + 1].Kind -eq "Identifier"
    ) {
        $cursor += 2
    }
    return $cursor
}

function Test-CtasSelectItem {
    param(
        [Parameter(Mandatory)]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [int]$End,

        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [int]$FallbackLine
    )

    if ($Start -gt $End) {
        Add-SqlParseError `
            -Path $Path `
            -Line $FallbackLine `
            -Message "empty CTAS select-list item"
        return
    }

    $parenthesisDepth = 0
    $aliasIndex = -1
    for ($index = $Start; $index -le $End; $index += 1) {
        if ($Tokens[$index].Value -eq "(") {
            $parenthesisDepth += 1
            continue
        }
        if ($Tokens[$index].Value -eq ")") {
            if ($parenthesisDepth -gt 0) {
                $parenthesisDepth -= 1
            }
            continue
        }
        if (
            $parenthesisDepth -eq 0 -and
            (Test-SqlKeyword -Token $Tokens[$index] -Keyword "as")
        ) {
            $aliasIndex = $index
        }
    }

    $outputToken = $null
    if ($aliasIndex -ge 0) {
        if (
            $aliasIndex + 1 -gt $End -or
            $Tokens[$aliasIndex + 1].Kind -ne "Identifier" -or
            $aliasIndex + 1 -ne $End
        ) {
            Add-SqlParseError `
                -Path $Path `
                -Line $Tokens[$aliasIndex].Line `
                -Message "unsupported CTAS alias shape"
            return
        }
        $outputToken = $Tokens[$aliasIndex + 1]
    }
    elseif ($Start -eq $End -and $Tokens[$Start].Kind -eq "Identifier") {
        $outputToken = $Tokens[$Start]
    }
    else {
        $qualifiedName = $true
        for ($index = $Start; $index -le $End; $index += 1) {
            $expectedIdentifier = (($index - $Start) % 2) -eq 0
            if (
                ($expectedIdentifier -and $Tokens[$index].Kind -ne "Identifier") -or
                (-not $expectedIdentifier -and $Tokens[$index].Value -ne ".")
            ) {
                $qualifiedName = $false
                break
            }
        }
        if (
            $qualifiedName -and
            (($End - $Start) % 2) -eq 0 -and
            $Tokens[$End].Kind -eq "Identifier"
        ) {
            $outputToken = $Tokens[$End]
        }
    }

    if ($null -eq $outputToken) {
        Add-SqlParseError `
            -Path $Path `
            -Line $Tokens[$Start].Line `
            -Message "unsupported CTAS select-list item"
        return
    }

    if ($forbiddenFields.ContainsKey($outputToken.Value.ToLowerInvariant())) {
        Add-Finding -Token $outputToken -Path $Path
    }
}

function Find-CtasOutputColumns {
    param(
        [Parameter(Mandatory)]
        [object[]]$Tokens,

        [Parameter(Mandatory)]
        [int]$SelectIndex,

        [Parameter(Mandatory)]
        [string]$Path
    )

    if (
        $SelectIndex -ge $Tokens.Count -or
        -not (Test-SqlKeyword -Token $Tokens[$SelectIndex] -Keyword "select")
    ) {
        $line = 1
        if ($SelectIndex -gt 0 -and $SelectIndex - 1 -lt $Tokens.Count) {
            $line = $Tokens[$SelectIndex - 1].Line
        }
        Add-SqlParseError -Path $Path -Line $line -Message "CTAS must contain SELECT"
        return
    }

    $cursor = $SelectIndex + 1
    if (
        $cursor -lt $Tokens.Count -and
        (
            (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "all") -or
            (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "distinct")
        )
    ) {
        $cursor += 1
    }

    $segmentStart = $cursor
    $parenthesisDepth = 0
    for ($index = $cursor; $index -lt $Tokens.Count; $index += 1) {
        if ($Tokens[$index].Value -eq "(") {
            $parenthesisDepth += 1
            continue
        }
        if ($Tokens[$index].Value -eq ")") {
            if ($parenthesisDepth -gt 0) {
                $parenthesisDepth -= 1
            }
            continue
        }
        if ($parenthesisDepth -ne 0) {
            continue
        }
        if ($Tokens[$index].Value -eq ",") {
            Test-CtasSelectItem `
                -Tokens $Tokens `
                -Start $segmentStart `
                -End ($index - 1) `
                -Path $Path `
                -FallbackLine $Tokens[$SelectIndex].Line
            $segmentStart = $index + 1
            continue
        }
        if (
            $Tokens[$index].Value -eq ";" -or
            (Test-SqlKeyword -Token $Tokens[$index] -Keyword "from")
        ) {
            Test-CtasSelectItem `
                -Tokens $Tokens `
                -Start $segmentStart `
                -End ($index - 1) `
                -Path $Path `
                -FallbackLine $Tokens[$SelectIndex].Line
            return
        }
    }

    Test-CtasSelectItem `
        -Tokens $Tokens `
        -Start $segmentStart `
        -End ($Tokens.Count - 1) `
        -Path $Path `
        -FallbackLine $Tokens[$SelectIndex].Line
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
            $nameEnd = Get-SqlObjectNameEnd -Tokens $Tokens -Start $cursor
            if ($nameEnd -eq $cursor) {
                continue
            }
            $cursor = $nameEnd

            if ($cursor -lt $Tokens.Count -and $Tokens[$cursor].Value -eq "(") {
                $depth = 1
                $segmentStart = $cursor + 1
                for (
                    $columnIndex = $cursor + 1;
                    $columnIndex -lt $Tokens.Count;
                    $columnIndex += 1
                ) {
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
                $cursor -lt $Tokens.Count -and
                (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "as")
            ) {
                Find-CtasOutputColumns `
                    -Tokens $Tokens `
                    -SelectIndex ($cursor + 1) `
                    -Path $Path
            }
            continue
        }

        if (
            (Test-SqlKeyword -Token $Tokens[$index] -Keyword "alter") -and
            $index + 1 -lt $Tokens.Count -and
            (Test-SqlKeyword -Token $Tokens[$index + 1] -Keyword "table")
        ) {
            $cursor = $index + 2
            $nameEnd = Get-SqlObjectNameEnd -Tokens $Tokens -Start $cursor
            if ($nameEnd -eq $cursor -or $nameEnd -ge $Tokens.Count) {
                continue
            }
            $cursor = $nameEnd

            if (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "add") {
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
                continue
            }

            if (
                (Test-SqlKeyword -Token $Tokens[$cursor] -Keyword "rename") -and
                $cursor + 4 -lt $Tokens.Count -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 1] -Keyword "column") -and
                $Tokens[$cursor + 2].Kind -eq "Identifier" -and
                (Test-SqlKeyword -Token $Tokens[$cursor + 3] -Keyword "to") -and
                $Tokens[$cursor + 4].Kind -eq "Identifier" -and
                $forbiddenFields.ContainsKey(
                    $Tokens[$cursor + 4].Value.ToLowerInvariant()
                )
            ) {
                Add-Finding -Token $Tokens[$cursor + 4] -Path $Path
            }
        }
    }
}

foreach ($modelSet in $persistentRustModels) {
    $path = Join-Path $rootPath $modelSet.RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        $inventoryErrors.Add(
            "Required persistence model file is missing: $path"
        )
        continue
    }

    $rustFileCount += 1
    $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
    $tokens = @(Get-RustTokens -Content $content)
    $foundStructs = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    Find-ForbiddenRustFields `
        -Tokens $tokens `
        -Structs $modelSet.Structs `
        -FoundStructs $foundStructs `
        -Path $path
    foreach ($structName in $modelSet.Structs) {
        if (-not $foundStructs.Contains($structName)) {
            $inventoryErrors.Add(
                "Required persistent struct '$structName' was not found in '$path'."
            )
        }
    }
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

if ($inventoryErrors.Count -gt 0) {
    foreach ($inventoryError in $inventoryErrors) {
        Write-Host "PERSISTENCE INVENTORY ERROR: $inventoryError"
    }
    exit 2
}

if ($sqlParseErrors.Count -gt 0) {
    foreach ($parseError in $sqlParseErrors) {
        Write-Host (
            "CTAS PARSE ERROR at {0}:{1}: {2}" -f
            $parseError.Path,
            $parseError.Line,
            $parseError.Message
        )
    }
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
