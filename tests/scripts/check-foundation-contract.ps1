[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
$workflowPath = Join-Path $rootPath ".github/workflows/ci.yml"
$denyPath = Join-Path $rootPath "deny.toml"
$developmentPath = Join-Path $rootPath "docs/operations/development.md"
$runtimeSelectorPath = Join-Path $rootPath "crates/wokrouter-platform/src/wokcore_runtime.rs"
$runtimeSelectorTestsPath = Join-Path $rootPath "crates/wokrouter-platform/tests/wokcore_runtime.rs"
$commandModelPath = Join-Path $rootPath "apps/cli/src/commands/mod.rs"
$desktopControlPath = Join-Path $rootPath "apps/desktop/src-tauri/src/control.rs"
$frontendControlPath = Join-Path $rootPath "apps/desktop/src/control.ts"
$failures = [System.Collections.Generic.List[string]]::new()

function Add-ContractFailure {
    param(
        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $failures.Contains($Message)) {
        $failures.Add($Message)
    }
}

function Set-RustMaskedRange {
    param(
        [Parameter(Mandatory)]
        [char[]]$View,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [int]$End
    )

    for ($index = $Start; $index -lt $End; $index += 1) {
        if ($View[$index] -ne "`r" -and $View[$index] -ne "`n") {
            $View[$index] = " "
        }
    }
}

function Test-RustIdentifierCharacter {
    param(
        [Parameter(Mandatory)]
        [char]$Character
    )

    return $Character -eq "_" -or [char]::IsLetterOrDigit($Character)
}

function Get-RustCodeView {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $view = $Source.ToCharArray()
    $commentStrippedView = $Source.ToCharArray()
    $index = 0
    while ($index -lt $Source.Length) {
        $current = $Source[$index]
        $next = if ($index + 1 -lt $Source.Length) {
            $Source[$index + 1]
        }
        else {
            [char]0
        }

        if ($current -eq "/" -and $next -eq "/") {
            $end = $index + 2
            while (
                $end -lt $Source.Length -and
                $Source[$end] -ne "`r" -and
                $Source[$end] -ne "`n"
            ) {
                $end += 1
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            Set-RustMaskedRange `
                -View $commentStrippedView `
                -Start $index `
                -End $end
            $index = $end
            continue
        }

        if ($current -eq "/" -and $next -eq "*") {
            $depth = 1
            $end = $index + 2
            while ($end -lt $Source.Length -and $depth -gt 0) {
                if (
                    $end + 1 -lt $Source.Length -and
                    $Source[$end] -eq "/" -and
                    $Source[$end + 1] -eq "*"
                ) {
                    $depth += 1
                    $end += 2
                    continue
                }
                if (
                    $end + 1 -lt $Source.Length -and
                    $Source[$end] -eq "*" -and
                    $Source[$end + 1] -eq "/"
                ) {
                    $depth -= 1
                    $end += 2
                    continue
                }
                $end += 1
            }
            if ($depth -ne 0) {
                Add-ContractFailure `
                    -Message "$Description must be lexically valid: unterminated block comment."
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            Set-RustMaskedRange `
                -View $commentStrippedView `
                -Start $index `
                -End $end
            $index = $end
            continue
        }

        $rawPrefixLength = 0
        $rawHashCount = 0
        $rawCursor = -1
        $atTokenBoundary = $false
        if ($current -eq "r" -or $current -eq "b") {
            $atTokenBoundary = (
                $index -eq 0 -or
                -not (Test-RustIdentifierCharacter -Character $Source[$index - 1])
            )
        }
        if ($atTokenBoundary -and $current -eq "r") {
            $rawPrefixLength = 1
            $rawCursor = $index + 1
        }
        elseif (
            $atTokenBoundary -and
            $current -eq "b" -and
            $next -eq "r"
        ) {
            $rawPrefixLength = 2
            $rawCursor = $index + 2
        }
        if ($rawCursor -ge 0) {
            while (
                $rawCursor -lt $Source.Length -and
                $Source[$rawCursor] -eq "#"
            ) {
                $rawHashCount += 1
                $rawCursor += 1
            }
            if (
                $rawCursor -ge $Source.Length -or
                $Source[$rawCursor] -ne '"'
            ) {
                $rawPrefixLength = 0
                $rawCursor = -1
            }
        }
        if ($rawCursor -ge 0) {
            $end = $rawCursor + 1
            $closed = $false
            while ($end -lt $Source.Length) {
                if ($Source[$end] -eq '"') {
                    $matchesDelimiter = $true
                    for ($hash = 0; $hash -lt $rawHashCount; $hash += 1) {
                        if (
                            $end + 1 + $hash -ge $Source.Length -or
                            $Source[$end + 1 + $hash] -ne "#"
                        ) {
                            $matchesDelimiter = $false
                            break
                        }
                    }
                    if ($matchesDelimiter) {
                        $end += 1 + $rawHashCount
                        $closed = $true
                        break
                    }
                }
                $end += 1
            }
            if (-not $closed) {
                Add-ContractFailure `
                    -Message "$Description must be lexically valid: unterminated raw string."
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        $quoteIndex = -1
        if ($current -eq '"') {
            $quoteIndex = $index
        }
        elseif ($current -eq "b" -and $next -eq '"') {
            $quoteIndex = $index + 1
        }
        if ($quoteIndex -ge 0) {
            $end = $quoteIndex + 1
            $closed = $false
            while ($end -lt $Source.Length) {
                if ($Source[$end] -eq "\") {
                    $end += 2
                    continue
                }
                if ($Source[$end] -eq '"') {
                    $end += 1
                    $closed = $true
                    break
                }
                $end += 1
            }
            if (-not $closed) {
                Add-ContractFailure `
                    -Message "$Description must be lexically valid: unterminated string."
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        $characterQuote = -1
        $byteCharacter = $false
        if ($current -eq "b" -and $next -eq "'") {
            $characterQuote = $index + 1
            $byteCharacter = $true
        }
        elseif ($current -eq "'") {
            $characterQuote = $index
        }
        if ($characterQuote -ge 0) {
            $contentStart = $characterQuote + 1
            if (
                -not $byteCharacter -and
                $contentStart -lt $Source.Length -and
                (Test-RustIdentifierCharacter -Character $Source[$contentStart]) -and
                (
                    $contentStart + 1 -ge $Source.Length -or
                    $Source[$contentStart + 1] -ne "'"
                )
            ) {
                $index += 1
                continue
            }

            $end = $contentStart
            $closed = $false
            while ($end -lt $Source.Length) {
                if ($Source[$end] -eq "\") {
                    $end += 2
                    continue
                }
                if ($Source[$end] -eq "'") {
                    $end += 1
                    $closed = $true
                    break
                }
                if ($Source[$end] -eq "`r" -or $Source[$end] -eq "`n") {
                    break
                }
                $end += 1
            }
            if (-not $closed) {
                Add-ContractFailure `
                    -Message "$Description must be lexically valid: unterminated character literal."
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        $index += 1
    }

    $code = -join $view
    $braceDepth = 0
    for ($index = 0; $index -lt $code.Length; $index += 1) {
        if ($code[$index] -eq "{") {
            $braceDepth += 1
        }
        elseif ($code[$index] -eq "}") {
            $braceDepth -= 1
            if ($braceDepth -lt 0) {
                Add-ContractFailure `
                    -Message "$Description must have balanced braces: negative depth."
                return $null
            }
        }
    }
    if ($braceDepth -ne 0) {
        Add-ContractFailure `
            -Message "$Description must have balanced braces: unclosed body."
        return $null
    }

    return [pscustomobject]@{
        Code = $code
        CommentStripped = -join $commentStrippedView
    }
}

function Get-UniqueBracedItem {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$SignaturePattern,

        [Parameter(Mandatory)]
        [string]$Description,

        [switch]$TopLevel
    )

    $codeView = Get-RustCodeView -Source $Source -Description $Description
    if ($null -eq $codeView) {
        return $null
    }
    $structure = $codeView.Code
    $matches = @([regex]::Matches(
        $structure,
        $SignaturePattern,
        [System.Text.RegularExpressions.RegexOptions]::Multiline
    ))
    if ($TopLevel) {
        $topLevelMatches = @()
        foreach ($candidateMatch in $matches) {
            $depth = 0
            for ($index = 0; $index -lt $candidateMatch.Index; $index += 1) {
                if ($structure[$index] -eq "{") {
                    $depth += 1
                }
                elseif ($structure[$index] -eq "}") {
                    $depth -= 1
                }
            }
            if ($depth -eq 0) {
                $topLevelMatches += $candidateMatch
            }
        }
        $matches = @($topLevelMatches)
    }
    if ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "$Description must remain uniquely identifiable; found $($matches.Count)."
        return $null
    }

    $match = $matches[0]
    $parenthesisDepth = 0
    $bracketDepth = 0
    for (
        $index = $match.Index;
        $index -lt $match.Index + $match.Length;
        $index += 1
    ) {
        switch ($structure[$index]) {
            "(" { $parenthesisDepth += 1 }
            ")" { $parenthesisDepth -= 1 }
            "[" { $bracketDepth += 1 }
            "]" { $bracketDepth -= 1 }
        }
        if ($parenthesisDepth -lt 0 -or $bracketDepth -lt 0) {
            Add-ContractFailure `
                -Message "$Description has unbalanced signature delimiters."
            return $null
        }
    }

    $openingBrace = -1
    $scanStart = $match.Index + $match.Length
    for ($index = $scanStart; $index -lt $structure.Length; $index += 1) {
        $current = $structure[$index]
        switch ($current) {
            "(" { $parenthesisDepth += 1; continue }
            ")" {
                $parenthesisDepth -= 1
                if ($parenthesisDepth -lt 0) {
                    Add-ContractFailure `
                        -Message "$Description has unbalanced signature parentheses."
                    return $null
                }
                continue
            }
            "[" { $bracketDepth += 1; continue }
            "]" {
                $bracketDepth -= 1
                if ($bracketDepth -lt 0) {
                    Add-ContractFailure `
                        -Message "$Description has unbalanced signature brackets."
                    return $null
                }
                continue
            }
        }
        if ($parenthesisDepth -eq 0 -and $bracketDepth -eq 0) {
            if ($current -eq ";") {
                Add-ContractFailure `
                    -Message "$Description must have a braced body, not a semicolon declaration."
                return $null
            }
            if ($current -eq "{") {
                $openingBrace = $index
                break
            }
        }
    }
    if ($openingBrace -lt 0) {
        Add-ContractFailure -Message "$Description must have a braced body."
        return $null
    }
    $declarationTail = $structure.Substring(
        $scanStart,
        $openingBrace - $scanStart
    )
    if (
        $declarationTail -match
        '(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?(?:async[ \t]+)?(?:fn|mod|struct|enum|impl|trait|type|const|static)\b'
    ) {
        Add-ContractFailure `
            -Message "$Description must open its body before the next Rust item."
        return $null
    }

    $depth = 0
    $closingBrace = -1
    for ($index = $openingBrace; $index -lt $structure.Length; $index += 1) {
        if ($structure[$index] -eq "{") {
            $depth += 1
        }
        elseif ($structure[$index] -eq "}") {
            $depth -= 1
            if ($depth -lt 0) {
                Add-ContractFailure `
                    -Message "$Description has a negative braced-body depth."
                return $null
            }
            if ($depth -eq 0) {
                $closingBrace = $index
                break
            }
        }
    }
    if ($closingBrace -lt 0) {
        Add-ContractFailure -Message "$Description has an unbalanced braced body."
        return $null
    }

    return [pscustomobject]@{
        SignatureIndex = $match.Index
        OpeningBraceIndex = $openingBrace
        ClosingBraceIndex = $closingBrace
        Attributes = if ($match.Groups["attributes"].Success) {
            $Source.Substring(
                $match.Groups["attributes"].Index,
                $match.Groups["attributes"].Length
            )
        }
        else {
            ""
        }
        CodeAttributes = $match.Groups["attributes"].Value
        Body = $Source.Substring(
            $openingBrace + 1,
            $closingBrace - $openingBrace - 1
        )
        CodeBody = $structure.Substring(
            $openingBrace + 1,
            $closingBrace - $openingBrace - 1
        )
    }
}

function Get-UniquePatternIndex {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Pattern,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $matches = [regex]::Matches($Source, $Pattern)
    if ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "$Description must occur exactly once; found $($matches.Count)."
        return -1
    }
    return $matches[0].Index
}

function Get-RustOuterAttributesBeforeItem {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$ItemStart,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $codeView = Get-RustCodeView -Source $Source -Description $Description
    if ($null -eq $codeView) {
        return $null
    }
    if ($ItemStart -lt 0 -or $ItemStart -gt $Source.Length) {
        Add-ContractFailure `
            -Message "$Description has an invalid item start."
        return $null
    }

    $structure = $codeView.Code
    $cursor = $ItemStart
    $attributes = @()
    while ($cursor -gt 0) {
        while (
            $cursor -gt 0 -and
            [char]::IsWhiteSpace($structure[$cursor - 1])
        ) {
            $cursor -= 1
        }
        if ($cursor -eq 0 -or $structure[$cursor - 1] -ne "]") {
            break
        }

        $attributeEnd = $cursor
        $bracketDepth = 0
        $openingBracket = -1
        for ($index = $cursor - 1; $index -ge 0; $index -= 1) {
            if ($structure[$index] -eq "]") {
                $bracketDepth += 1
            }
            elseif ($structure[$index] -eq "[") {
                $bracketDepth -= 1
                if ($bracketDepth -eq 0) {
                    $openingBracket = $index
                    break
                }
                if ($bracketDepth -lt 0) {
                    break
                }
            }
        }
        if ($openingBracket -lt 0) {
            Add-ContractFailure `
                -Message "$Description must have balanced outer attribute brackets."
            return $null
        }

        $hashIndex = $openingBracket - 1
        while (
            $hashIndex -ge 0 -and
            [char]::IsWhiteSpace($structure[$hashIndex])
        ) {
            $hashIndex -= 1
        }
        if ($hashIndex -lt 0 -or $structure[$hashIndex] -ne "#") {
            break
        }

        $attributeStart = $hashIndex
        $attributeLength = $attributeEnd - $attributeStart
        $attributeCode = $structure.Substring(
            $attributeStart,
            $attributeLength
        )
        $delimiterStack = [System.Collections.Generic.Stack[char]]::new()
        $balanced = $true
        for ($index = 0; $index -lt $attributeCode.Length; $index += 1) {
            $current = $attributeCode[$index]
            if ($current -eq "(" -or $current -eq "[" -or $current -eq "{") {
                $delimiterStack.Push($current)
                continue
            }
            if ($current -ne ")" -and $current -ne "]" -and $current -ne "}") {
                continue
            }
            if ($delimiterStack.Count -eq 0) {
                $balanced = $false
                break
            }
            $opening = $delimiterStack.Pop()
            if (
                ($current -eq ")" -and $opening -ne "(") -or
                ($current -eq "]" -and $opening -ne "[") -or
                ($current -eq "}" -and $opening -ne "{")
            ) {
                $balanced = $false
                break
            }
        }
        if (-not $balanced -or $delimiterStack.Count -ne 0) {
            Add-ContractFailure `
                -Message "$Description must have balanced outer attribute token trees."
            return $null
        }

        $attribute = [pscustomobject]@{
            Source = $Source.Substring(
                $attributeStart,
                $attributeLength
            )
            Code = $attributeCode
        }
        $attributes = @($attribute) + $attributes
        $cursor = $attributeStart
    }

    return [pscustomobject]@{
        Items = @($attributes)
        Code = (($attributes | ForEach-Object { $_.Code }) -join "`n")
    }
}

function Get-TopLevelRustBody {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Body
    )

    $codeView = Get-RustCodeView `
        -Source $Body `
        -Description "Rust top-level body"
    if ($null -eq $codeView) {
        return ""
    }
    $structure = $codeView.Code
    $result = [System.Text.StringBuilder]::new($Body.Length)
    $depth = 0
    for ($index = 0; $index -lt $Body.Length; $index += 1) {
        $current = $structure[$index]
        if ($current -eq "{") {
            $depth += 1
            $null = $result.Append(" ")
            continue
        }
        if ($current -eq "}") {
            $depth -= 1
            if ($depth -lt 0) {
                Add-ContractFailure `
                    -Message "Rust top-level body has a negative brace depth."
                return ""
            }
            $null = $result.Append(" ")
            continue
        }
        if ($depth -eq 0) {
            $null = $result.Append($structure[$index])
        }
        elseif (
            $structure[$index] -eq "`r" -or
            $structure[$index] -eq "`n"
        ) {
            $null = $result.Append($structure[$index])
        }
        else {
            $null = $result.Append(" ")
        }
    }
    if ($depth -ne 0) {
        Add-ContractFailure `
            -Message "Rust top-level body has an unbalanced brace depth."
        return ""
    }
    return $result.ToString()
}

function Get-LineIndent {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Line
    )

    if ($Line -match "`t") {
        Add-ContractFailure -Message "Workflow YAML must not use tab indentation."
    }
    if ($Line -match "^( *)") {
        return $Matches[1].Length
    }
    return 0
}

function Get-WorkflowJobs {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$Lines
    )

    $jobsRoots = [System.Collections.Generic.List[int]]::new()
    for ($index = 0; $index -lt $Lines.Count; $index += 1) {
        if ($Lines[$index] -match "^jobs:\s*$") {
            $jobsRoots.Add($index)
        }
    }
    if ($jobsRoots.Count -ne 1) {
        Add-ContractFailure -Message "Workflow must contain exactly one root 'jobs' mapping."
        return @{}
    }

    $jobs = @{}
    $index = $jobsRoots[0] + 1
    while ($index -lt $Lines.Count) {
        $line = $Lines[$index]
        if ([string]::IsNullOrWhiteSpace($line)) {
            $index += 1
            continue
        }

        $indent = Get-LineIndent -Line $line
        if ($indent -eq 0) {
            break
        }
        if ($indent -ne 2 -or $line -notmatch "^  (?<name>[A-Za-z0-9_-]+):\s*$") {
            Add-ContractFailure `
                -Message "Workflow jobs mapping contains invalid structure at line $($index + 1)."
            $index += 1
            continue
        }

        $jobName = $Matches["name"]
        $start = $index
        $index += 1
        while ($index -lt $Lines.Count) {
            if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                $index += 1
                continue
            }
            if ((Get-LineIndent -Line $Lines[$index]) -le 2) {
                break
            }
            $index += 1
        }
        if ($jobs.ContainsKey($jobName)) {
            Add-ContractFailure -Message "Workflow job '$jobName' is defined more than once."
            continue
        }
        $jobs[$jobName] = [pscustomobject]@{
            Name = $jobName
            Lines = @($Lines[$start..($index - 1)])
        }
    }

    return $jobs
}

function Get-JobScalar {
    param(
        [Parameter(Mandatory)]
        [object]$Job,

        [Parameter(Mandatory)]
        [string]$Key
    )

    $pattern = "^    $([regex]::Escape($Key)):\s*(?<value>.*?)\s*$"
    $values = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $Job.Lines) {
        if ($line -match $pattern) {
            $values.Add($Matches["value"])
        }
    }
    if ($values.Count -gt 1) {
        Add-ContractFailure `
            -Message "Workflow job '$($Job.Name)' defines '$Key' more than once."
    }
    if ($values.Count -eq 0) {
        return $null
    }
    return $values[0]
}

function Set-StepScalar {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Fields,

        [Parameter(Mandatory)]
        [string]$Payload,

        [Parameter(Mandatory)]
        [int]$LineNumber
    )

    if ($Payload -notmatch "^(?<key>[A-Za-z0-9_-]+):\s*(?<value>.*)$") {
        Add-ContractFailure `
            -Message "Workflow step contains invalid YAML at line $LineNumber."
        return
    }
    $key = $Matches["key"]
    if ($Fields.ContainsKey($key)) {
        Add-ContractFailure `
            -Message "Workflow step defines '$key' more than once at line $LineNumber."
        return
    }
    $Fields[$key] = $Matches["value"]
}

function ConvertTo-WorkflowStep {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string[]]$Lines,

        [Parameter(Mandatory)]
        [int]$StartLine
    )

    $fields = @{}
    $nested = @{}
    $firstPayload = $Lines[0].Substring(8)
    Set-StepScalar -Fields $fields -Payload $firstPayload -LineNumber $StartLine

    $index = 1
    while ($index -lt $Lines.Count) {
        $line = $Lines[$index]
        if ([string]::IsNullOrWhiteSpace($line)) {
            $index += 1
            continue
        }
        $indent = Get-LineIndent -Line $line
        if ($indent -ne 8 -or $line -notmatch "^        (?<payload>.+)$") {
            Add-ContractFailure `
                -Message "Workflow step contains invalid indentation at line $($StartLine + $index)."
            $index += 1
            continue
        }
        $payload = $Matches["payload"]
        if ($payload -notmatch "^(?<key>[A-Za-z0-9_-]+):\s*(?<value>.*)$") {
            Add-ContractFailure `
                -Message "Workflow step contains invalid YAML at line $($StartLine + $index)."
            $index += 1
            continue
        }

        $key = $Matches["key"]
        $value = $Matches["value"]
        if ($value -eq "|" -or $value -eq ">") {
            $blockLines = [System.Collections.Generic.List[string]]::new()
            $index += 1
            while ($index -lt $Lines.Count) {
                if (
                    -not [string]::IsNullOrWhiteSpace($Lines[$index]) -and
                    (Get-LineIndent -Line $Lines[$index]) -lt 10
                ) {
                    break
                }
                if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                    $blockLines.Add("")
                }
                else {
                    $blockLines.Add($Lines[$index].Substring(10))
                }
                $index += 1
            }
            if ($value -eq "|") {
                $fields[$key] = $blockLines -join "`n"
            }
            else {
                $fields[$key] = $blockLines -join " "
            }
            continue
        }

        if ([string]::IsNullOrEmpty($value)) {
            $childValues = @{}
            $index += 1
            while ($index -lt $Lines.Count) {
                if ([string]::IsNullOrWhiteSpace($Lines[$index])) {
                    $index += 1
                    continue
                }
                $childIndent = Get-LineIndent -Line $Lines[$index]
                if ($childIndent -le 8) {
                    break
                }
                if (
                    $childIndent -ne 10 -or
                    $Lines[$index] -notmatch
                    "^          (?<childKey>[A-Za-z0-9_-]+):\s*(?<childValue>.*)$"
                ) {
                    Add-ContractFailure `
                        -Message "Workflow step '$key' mapping is invalid at line $($StartLine + $index)."
                    $index += 1
                    continue
                }
                $childValues[$Matches["childKey"]] = $Matches["childValue"]
                $index += 1
            }
            $nested[$key] = $childValues
            continue
        }

        if ($fields.ContainsKey($key)) {
            Add-ContractFailure `
                -Message "Workflow step defines '$key' more than once at line $($StartLine + $index)."
        }
        else {
            $fields[$key] = $value
        }
        $index += 1
    }

    return [pscustomobject]@{
        Fields = $fields
        Nested = $nested
    }
}

function Get-JobSteps {
    param(
        [Parameter(Mandatory)]
        [object]$Job
    )

    $stepsLine = -1
    for ($index = 0; $index -lt $Job.Lines.Count; $index += 1) {
        if ($Job.Lines[$index] -match "^    steps:\s*$") {
            if ($stepsLine -ge 0) {
                Add-ContractFailure `
                    -Message "Workflow job '$($Job.Name)' defines steps more than once."
            }
            $stepsLine = $index
        }
    }
    if ($stepsLine -lt 0) {
        Add-ContractFailure -Message "Workflow job '$($Job.Name)' is missing steps."
        return @()
    }

    $steps = [System.Collections.Generic.List[object]]::new()
    $index = $stepsLine + 1
    while ($index -lt $Job.Lines.Count) {
        if ([string]::IsNullOrWhiteSpace($Job.Lines[$index])) {
            $index += 1
            continue
        }
        $indent = Get-LineIndent -Line $Job.Lines[$index]
        if ($indent -le 4) {
            break
        }
        if ($Job.Lines[$index] -notmatch "^      - ") {
            Add-ContractFailure `
                -Message "Workflow job '$($Job.Name)' has an invalid step at line $($index + 1)."
            $index += 1
            continue
        }

        $start = $index
        $index += 1
        while ($index -lt $Job.Lines.Count) {
            if (
                -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
                (
                    (Get-LineIndent -Line $Job.Lines[$index]) -le 4 -or
                    $Job.Lines[$index] -match "^      - "
                )
            ) {
                break
            }
            $index += 1
        }
        $stepLines = @($Job.Lines[$start..($index - 1)])
        $steps.Add((
                ConvertTo-WorkflowStep `
                    -Lines $stepLines `
                    -StartLine ($start + 1)
            ))
    }

    return @($steps)
}

function Get-PlatformMatrixRunners {
    param(
        [Parameter(Mandatory)]
        [object]$Job
    )

    $strategyIndex = -1
    for ($index = 0; $index -lt $Job.Lines.Count; $index += 1) {
        if ($Job.Lines[$index] -match "^    strategy:\s*$") {
            $strategyIndex = $index
            break
        }
    }
    if ($strategyIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy."
        return @()
    }

    $matrixIndex = -1
    for ($index = $strategyIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if (
            -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
            (Get-LineIndent -Line $Job.Lines[$index]) -le 4
        ) {
            break
        }
        if ($Job.Lines[$index] -match "^      matrix:\s*$") {
            $matrixIndex = $index
            break
        }
    }
    if ($matrixIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy.matrix."
        return @()
    }

    $osIndex = -1
    for ($index = $matrixIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if (
            -not [string]::IsNullOrWhiteSpace($Job.Lines[$index]) -and
            (Get-LineIndent -Line $Job.Lines[$index]) -le 6
        ) {
            break
        }
        if ($Job.Lines[$index] -match "^        os:\s*$") {
            $osIndex = $index
            break
        }
    }
    if ($osIndex -lt 0) {
        Add-ContractFailure -Message "Platform matrix job is missing strategy.matrix.os."
        return @()
    }

    $runners = [System.Collections.Generic.List[string]]::new()
    for ($index = $osIndex + 1; $index -lt $Job.Lines.Count; $index += 1) {
        if ([string]::IsNullOrWhiteSpace($Job.Lines[$index])) {
            continue
        }
        if ((Get-LineIndent -Line $Job.Lines[$index]) -le 8) {
            break
        }
        if ($Job.Lines[$index] -match "^          - (?<runner>[A-Za-z0-9_.-]+)\s*$") {
            $runners.Add($Matches["runner"])
        }
        else {
            Add-ContractFailure `
                -Message "Platform matrix contains invalid runner structure."
        }
    }
    return @($runners)
}

function Assert-JobRunStep {
    param(
        [Parameter(Mandatory)]
        [string]$JobName,

        [Parameter(Mandatory)]
        [object[]]$Steps,

        [Parameter(Mandatory)]
        [string]$Command
    )

    $matches = @(
        $Steps | Where-Object {
            $_.Fields.ContainsKey("run") -and $_.Fields["run"] -eq $Command
        }
    )
    if ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must contain one independent '$Command' step."
        return
    }
    if ($matches[0].Fields.ContainsKey("continue-on-error")) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must propagate failure from '$Command'."
    }
}

function Get-ActionStep {
    param(
        [Parameter(Mandatory)]
        [object[]]$Steps,

        [Parameter(Mandatory)]
        [string]$Action
    )

    $matches = @(
        $Steps | Where-Object {
            $_.Fields.ContainsKey("uses") -and $_.Fields["uses"] -eq $Action
        }
    )
    if ($matches.Count -eq 1) {
        return $matches[0]
    }
    return $null
}

function Assert-NestedValue {
    param(
        [Parameter(Mandatory)]
        [object]$Step,

        [Parameter(Mandatory)]
        [string]$Mapping,

        [Parameter(Mandatory)]
        [string]$Key,

        [Parameter(Mandatory)]
        [string]$Value,

        [Parameter(Mandatory)]
        [string]$Message
    )

    if (
        -not $Step.Nested.ContainsKey($Mapping) -or
        -not $Step.Nested[$Mapping].ContainsKey($Key) -or
        $Step.Nested[$Mapping][$Key] -ne $Value
    ) {
        Add-ContractFailure -Message $Message
    }
}

$workflowLines = @(Get-Content -LiteralPath $workflowPath -Encoding UTF8)
$workflow = $workflowLines -join "`n"
$deny = Get-Content -LiteralPath $denyPath -Raw -Encoding UTF8
$development = Get-Content -LiteralPath $developmentPath -Raw -Encoding UTF8
$runtimeSelector = Get-Content -LiteralPath $runtimeSelectorPath -Raw -Encoding UTF8
$runtimeSelectorTests = Get-Content -LiteralPath $runtimeSelectorTestsPath -Raw -Encoding UTF8
$commandModel = Get-Content -LiteralPath $commandModelPath -Raw -Encoding UTF8
$desktopControl = Get-Content -LiteralPath $desktopControlPath -Raw -Encoding UTF8
$frontendControl = Get-Content -LiteralPath $frontendControlPath -Raw -Encoding UTF8
$jobs = Get-WorkflowJobs -Lines $workflowLines

$requiredJobs = @(
    "rust",
    "frontend",
    "native-test-matrix",
    "target-check-matrix",
    "compatibility",
    "platform-check"
)
foreach ($jobName in $requiredJobs) {
    if (-not $jobs.ContainsKey($jobName)) {
        Add-ContractFailure -Message "Workflow jobs mapping is missing '$jobName'."
    }
}

foreach ($providerVariable in @(
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY"
    )) {
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
        Add-ContractFailure `
            -Message "Workflow must define provider environment variable '$providerVariable' exactly once as empty."
    }
}

if ($jobs.ContainsKey("rust")) {
    $rustSteps = @(Get-JobSteps -Job $jobs["rust"])
    foreach ($command in @(
            "node apps/desktop/scripts/stage-sidecars.mjs",
            "pwsh tests/scripts/check-public-repo-hygiene.tests.ps1",
            "pwsh tests/scripts/check-public-repo-hygiene.ps1",
            "pwsh tests/scripts/check-core-boundary.tests.ps1",
            "pwsh tests/scripts/check-core-boundary.ps1",
            "pwsh tests/scripts/check-no-body-persistence.tests.ps1",
            "pwsh tests/scripts/check-no-body-persistence.ps1",
            "pwsh tests/scripts/check-foundation-contract.tests.ps1",
            "pwsh tests/scripts/check-foundation-contract.ps1",
            "pwsh tests/scripts/check-release-contract.tests.ps1",
            "pwsh tests/scripts/check-release-contract.ps1",
            "cargo fmt --all -- --check",
            "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings"
        )) {
        Assert-JobRunStep -JobName "rust" -Steps $rustSteps -Command $command
    }

    $rustToolchain = Get-ActionStep `
        -Steps $rustSteps `
        -Action "actions-rust-lang/setup-rust-toolchain@v1"
    if ($null -eq $rustToolchain) {
        Add-ContractFailure -Message "Rust job must set up the pinned Rust toolchain."
    }
    else {
        Assert-NestedValue `
            -Step $rustToolchain `
            -Mapping "with" `
            -Key "toolchain" `
            -Value "1.97.1" `
            -Message "Rust job must pin Rust 1.97.1."
    }

    $denyStep = Get-ActionStep `
        -Steps $rustSteps `
        -Action "EmbarkStudios/cargo-deny-action@v2.1.1"
    if ($null -eq $denyStep) {
        Add-ContractFailure `
            -Message "Rust job must run cargo-deny-action v2.1.1."
    }
    else {
        Assert-NestedValue `
            -Step $denyStep `
            -Mapping "with" `
            -Key "command" `
            -Value "check" `
            -Message "Rust cargo-deny step must run the check command."
        Assert-NestedValue `
            -Step $denyStep `
            -Mapping "with" `
            -Key "arguments" `
            -Value "--all-features" `
            -Message "Rust cargo-deny step must check all features."
    }
}

if ($jobs.ContainsKey("frontend")) {
    $frontendSteps = @(Get-JobSteps -Job $jobs["frontend"])
    foreach ($command in @(
            "pnpm --dir apps/desktop install --frozen-lockfile",
            "pnpm --dir apps/desktop typecheck",
            "pnpm --dir apps/desktop test:unit",
            "pnpm --dir apps/desktop build"
        )) {
        Assert-JobRunStep -JobName "frontend" -Steps $frontendSteps -Command $command
    }

    $pnpmStep = Get-ActionStep -Steps $frontendSteps -Action "pnpm/action-setup@v6"
    if ($null -eq $pnpmStep) {
        Add-ContractFailure -Message "Frontend job must set up pnpm."
    }
    else {
        Assert-NestedValue `
            -Step $pnpmStep `
            -Mapping "with" `
            -Key "version" `
            -Value "11.17.0" `
            -Message "Frontend job must pin pnpm 11.17.0."
    }
}

function Assert-TargetMatrix {
    param(
        [Parameter(Mandatory)]
        [object]$Job,

        [Parameter(Mandatory)]
        [string]$JobName
    )

    if ((Get-JobScalar -Job $Job -Key "runs-on") -ne '${{ matrix.os }}') {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' runs-on must be '`${{ matrix.os }}'."
    }
    $jobText = $Job.Lines -join "`n"
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
        if ($jobText -notmatch $pattern) {
            Add-ContractFailure `
                -Message "Workflow job '$JobName' is missing native runner '$($pair[0])' for '$($pair[1])'."
        }
    }
    $targetCount = @(
        $Job.Lines | Where-Object { $_ -match "^            target: " }
    ).Count
    if ($targetCount -ne 6) {
        Add-ContractFailure `
            -Message "Workflow job '$JobName' must contain exactly 6 target entries."
    }
}

if ($jobs.ContainsKey("native-test-matrix")) {
    $nativeJob = $jobs["native-test-matrix"]
    Assert-TargetMatrix -Job $nativeJob -JobName "native-test-matrix"
    $nativeSteps = @(Get-JobSteps -Job $nativeJob)
    Assert-JobRunStep `
        -JobName "native-test-matrix" `
        -Steps $nativeSteps `
        -Command "./tests/scripts/run-fixed-test-host.tests.ps1"
    $fixedHostSelfTests = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq "./tests/scripts/run-fixed-test-host.tests.ps1"
        }
    )
    $windowsX64Condition = (
        "runner.os == 'Windows' && " +
        "matrix.target == 'x86_64-pc-windows-msvc'"
    )
    if (
        $fixedHostSelfTests.Count -ne 1 -or
        -not $fixedHostSelfTests[0].Fields.ContainsKey("if") -or
        $fixedHostSelfTests[0].Fields["if"] -ne $windowsX64Condition
    ) {
        Add-ContractFailure `
            -Message "The fixed test host self-test must run only for the Windows x64 target."
    }
    $fixedHostSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "run-fixed-test-host\.ps1" -and
            $_.Fields["run"] -match "HarnessArguments @\(`"--nocapture`"\)"
        }
    )
    if ($fixedHostSteps.Count -ne 1) {
        Add-ContractFailure `
            -Message "Windows native tests must execute the workspace through the fixed test host."
    }
    else {
        if (
            -not $fixedHostSteps[0].Fields.ContainsKey("if") -or
            $fixedHostSteps[0].Fields["if"] -ne $windowsX64Condition
        ) {
            Add-ContractFailure `
                -Message "The fixed test host step must run only for the Windows x64 target."
        }
        $providerClearPattern = (
            '(?ms)\$env:OPENAI_API_KEY = ""\n' +
            '\$env:ANTHROPIC_API_KEY = ""\n' +
            '\$env:GEMINI_API_KEY = ""\n' +
            '\$env:GOOGLE_API_KEY = ""\n' +
            '\s*& \./tests/scripts/run-fixed-test-host\.ps1'
        )
        if ($fixedHostSteps[0].Fields["run"] -notmatch $providerClearPattern) {
            Add-ContractFailure `
                -Message "Windows Rust tests must clear all four Provider keys immediately before the fixed host."
        }
    }
    $arm64ToolSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "Microsoft\.VisualStudio\.Component\.VC\.Tools\.ARM64"
        }
    )
    $windowsArm64Condition = (
        "runner.os == 'Windows' && " +
        "matrix.target == 'aarch64-pc-windows-msvc'"
    )
    if (
        $arm64ToolSteps.Count -ne 1 -or
        -not $arm64ToolSteps[0].Fields.ContainsKey("if") -or
        $arm64ToolSteps[0].Fields["if"] -ne $windowsArm64Condition -or
        $arm64ToolSteps[0].Fields["run"] -notmatch "-WindowStyle Hidden"
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 native checks must install the Visual C++ ARM64 tools."
    }
    $arm64CompileSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq (
                'cargo test --workspace --all-features --locked --no-run ' +
                '--target ${{ matrix.target }}'
            )
        }
    )
    if (
        $arm64CompileSteps.Count -ne 1 -or
        -not $arm64CompileSteps[0].Fields.ContainsKey("if") -or
        $arm64CompileSteps[0].Fields["if"] -ne $windowsArm64Condition
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 tests must compile without running."
    }
    $allowedCargoRuns = @(
        'cargo test --workspace --all-features --locked',
        (
            'cargo test --workspace --all-features --locked --no-run ' +
            '--target ${{ matrix.target }}'
        )
    )
    $unexpectedCargoTests = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "(?m)(^|\n)\s*cargo test " -and
            $_.Fields["run"] -notin $allowedCargoRuns
        }
    )
    if ($unexpectedCargoTests.Count -ne 0) {
        Add-ContractFailure `
            -Message "Direct Windows Cargo tests are forbidden outside the two exact native matrix paths."
    }
    $hashedExecutableSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match (
                "(?im)(^|\n).*target.*[\\/]deps[\\/].*\.exe(?:\s|$)"
            )
        }
    )
    if ($hashedExecutableSteps.Count -ne 0) {
        Add-ContractFailure `
            -Message "Cargo hash test executables must never run directly on Windows."
    }
    $nativeCargoSteps = @(
        $nativeSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -eq "cargo test --workspace --all-features --locked"
        }
    )
    if (
        $nativeCargoSteps.Count -ne 1 -or
        -not $nativeCargoSteps[0].Fields.ContainsKey("if") -or
        $nativeCargoSteps[0].Fields["if"] -ne "runner.os != 'Windows'"
    ) {
        Add-ContractFailure `
            -Message "Direct Cargo workspace tests must be restricted to non-Windows native runners."
    }
}

if ($jobs.ContainsKey("target-check-matrix")) {
    $targetJob = $jobs["target-check-matrix"]
    Assert-TargetMatrix -Job $targetJob -JobName "target-check-matrix"
    $targetSteps = @(Get-JobSteps -Job $targetJob)
    Assert-JobRunStep `
        -JobName "target-check-matrix" `
        -Steps $targetSteps `
        -Command 'cargo check --workspace --all-features --locked --target ${{ matrix.target }}'
    $arm64ToolSteps = @(
        $targetSteps | Where-Object {
            $_.Fields.ContainsKey("run") -and
            $_.Fields["run"] -match "Microsoft\.VisualStudio\.Component\.VC\.Tools\.ARM64"
        }
    )
    if (
        $arm64ToolSteps.Count -ne 1 -or
        -not $arm64ToolSteps[0].Fields.ContainsKey("if") -or
        $arm64ToolSteps[0].Fields["if"] -ne $windowsArm64Condition -or
        $arm64ToolSteps[0].Fields["run"] -notmatch "-WindowStyle Hidden"
    ) {
        Add-ContractFailure `
            -Message "Windows ARM64 target checks must install the Visual C++ ARM64 tools."
    }
}

if ($jobs.ContainsKey("compatibility")) {
    $compatibilitySteps = @(Get-JobSteps -Job $jobs["compatibility"])
    foreach ($command in @(
            "cargo test -p wokrouter-wokcore-client --test handshake current_wokrouter_accepts_current_wokcore --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake compatible_handshake_accepts_unknown_same_major_fields --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake legacy_same_major_runtime_without_installation_id_remains_running --locked",
            "cargo test -p wokrouter-wokcore-client --test handshake non_overlapping_api_major_is_incompatible_without_http_fallback --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install an_existing_compatible_install_is_never_overwritten --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install installing_wokcore_does_not_modify_wokrouter_binary_or_version --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1 --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_missing_v2_manifest_falls_back_to_the_signed_v1_release --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_present_invalid_v2_manifest_never_downgrades_to_v1 --locked",
            "cargo test -p wokrouter-platform --features test-support --test wokcore_install wokcore_install_rejects_a_signed_v1_schema_at_the_v2_endpoint_without_downgrading --locked"
        )) {
        Assert-JobRunStep `
            -JobName "compatibility" `
            -Steps $compatibilitySteps `
            -Command $command
    }
}

if ($jobs.ContainsKey("platform-check")) {
    $aggregator = $jobs["platform-check"]
    if ((Get-JobScalar -Job $aggregator -Key "if") -ne "always()") {
        Add-ContractFailure -Message "Platform aggregator if must be 'always()'."
    }
    $aggregatorText = $aggregator.Lines -join "`n"
    foreach ($dependency in @(
            "rust",
            "frontend",
            "native-test-matrix",
            "target-check-matrix",
            "compatibility"
        )) {
        if ($aggregatorText -notmatch "(?m)^      - $([regex]::Escape($dependency))$") {
            Add-ContractFailure `
                -Message "Platform aggregator must require '$dependency'."
        }
        if ($aggregatorText -notmatch [regex]::Escape("needs.$dependency.result")) {
            Add-ContractFailure `
                -Message "Platform aggregator must verify the result of '$dependency'."
        }
    }
}

if ($deny -notmatch "(?m)^yanked\s*=\s*`"deny`"\s*$") {
    Add-ContractFailure -Message "deny.toml must deny yanked dependencies."
}
if ($deny -notmatch '(?m)^\s*"aarch64-apple-darwin",\s*$') {
    Add-ContractFailure -Message "deny.toml must include the Apple Silicon target."
}
if ($development -notmatch "cargo-deny 0\.20\.2") {
    Add-ContractFailure -Message "Development docs must pin cargo-deny 0.20.2."
}
if (
    $development -notmatch
    "cargo install --locked cargo-deny --version 0\.20\.2"
) {
    Add-ContractFailure `
        -Message "Development docs must give the CI-matching cargo-deny install command."
}
if ($development -notmatch "cargo deny --version") {
    Add-ContractFailure `
        -Message "Development docs must require cargo-deny version verification."
}
$runtimeCodeView = Get-RustCodeView `
    -Source $runtimeSelector `
    -Description "WokCore runtime selector source"
if ($null -ne $runtimeCodeView) {
    $environmentLiteralMatches = [regex]::Matches(
        $runtimeCodeView.CommentStripped,
        '"WOKROUTER_DEV_WOKCORE_EXECUTABLE"'
    )
    if ($environmentLiteralMatches.Count -ne 1) {
        Add-ContractFailure `
            -Message "WokCore runtime selector environment literal must occur exactly once outside comments."
    }
}
$developmentModule = Get-UniqueBracedItem `
    -Source $runtimeSelector `
    -SignaturePattern '(?ms)(?<attributes>(?:(?:^[ \t]*#\[[^\r\n]*\][ \t]*\r?\n)|(?:^[ \t]*\r?\n))*)^[ \t]*mod[ \t]+development[ \t]*' `
    -Description "Development module" `
    -TopLevel
if ($null -ne $developmentModule) {
    if (
        $developmentModule.CodeAttributes -notmatch
        '(?m)^[ \t]*#\[cfg\([ \t]*debug_assertions[ \t]*\)\][ \t]*$'
    ) {
        Add-ContractFailure `
            -Message "Development module must remain behind debug_assertions."
    }

    $developmentBody = Get-TopLevelRustBody -Body $developmentModule.Body
    $environmentConstants = [regex]::Matches(
        $developmentBody,
        '(?m)^[ \t]*pub\(super\)[ \t]+const[ \t]+EXECUTABLE_ENV[ \t]*:[ \t]*&str[ \t]*=[ \t]+;'
    )
    if ($environmentConstants.Count -ne 1) {
        Add-ContractFailure `
            -Message "Development module must define its executable environment constant exactly once."
    }
    else {
        $environmentConstant = $developmentModule.Body.Substring(
            $environmentConstants[0].Index,
            $environmentConstants[0].Length
        )
        if (
            $environmentConstant -notmatch
            '(?m)^[ \t]*pub\(super\)[ \t]+const[ \t]+EXECUTABLE_ENV[ \t]*:[ \t]*&str[ \t]*=[ \t]*"WOKROUTER_DEV_WOKCORE_EXECUTABLE"[ \t]*;[ \t]*$'
        ) {
            Add-ContractFailure `
                -Message "Development module executable environment constant must retain its exact value."
        }
    }

    $candidateFromEnvironment = Get-UniqueBracedItem `
        -Source $developmentModule.Body `
        -SignaturePattern '(?m)^[ \t]*pub\(super\)[ \t]+fn[ \t]+candidate_from_environment[ \t]*\([^)]*\)[ \t]*->[^{]+' `
        -Description "Development environment candidate function" `
        -TopLevel
    if ($null -ne $candidateFromEnvironment) {
        $candidateTopLevel = Get-TopLevelRustBody `
            -Body $candidateFromEnvironment.Body
        if (
            ($candidateTopLevel -replace '\s', '') -ne
            'candidate_from_value(std::env::var_os(EXECUTABLE_ENV))'
        ) {
            Add-ContractFailure `
                -Message "Development module environment lookup must remain the active top-level candidate expression."
        }
    }
}

$debugSelectOnce = Get-UniqueBracedItem `
    -Source $runtimeSelector `
    -SignaturePattern '(?ms)(?<attributes>^[ \t]*#\[cfg\([ \t]*debug_assertions[ \t]*\)\][ \t]*\r?\n(?:(?:^[ \t]*#\[[^\r\n]*\][ \t]*\r?\n)|(?:^[ \t]*\r?\n))*)^[ \t]*async[ \t]+fn[ \t]+select_once[ \t]*\([^)]*\)[ \t]*->[^{]+' `
    -Description "Debug select_once" `
    -TopLevel
if ($null -ne $debugSelectOnce) {
    $debugSelectOnceTopLevel = Get-TopLevelRustBody `
        -Body $debugSelectOnce.Body
    $null = Get-UniquePatternIndex `
        -Source $debugSelectOnceTopLevel `
        -Pattern '(?m)^[ \t]*let[ \t]+candidate[ \t]*=[ \t]*development::candidate_from_environment\(\)[ \t]*;' `
        -Description "Debug select_once development candidate call"
}

$releaseSelectOnce = Get-UniqueBracedItem `
    -Source $runtimeSelector `
    -SignaturePattern '(?ms)(?<attributes>^[ \t]*#\[cfg\([ \t]*not\([ \t]*debug_assertions[ \t]*\)[ \t]*\)\][ \t]*\r?\n(?:(?:^[ \t]*#\[[^\r\n]*\][ \t]*\r?\n)|(?:^[ \t]*\r?\n))*)^[ \t]*async[ \t]+fn[ \t]+select_once[ \t]*\([^)]*\)[ \t]*->[^{]+' `
    -Description "Release select_once" `
    -TopLevel
if ($null -ne $releaseSelectOnce) {
    $releaseSelectOnceTopLevel = Get-TopLevelRustBody `
        -Body $releaseSelectOnce.Body
    if (
        ($releaseSelectOnceTopLevel -replace '\s', '') -ne
        'select_production(paths,&discover_wokcore_executable)'
    ) {
        Add-ContractFailure `
            -Message "Release select_once must directly call select_production with production discovery."
    }
    if (
        $releaseSelectOnce.CodeBody -match
        '(?:\bstd::env\b|\benv[ \t\r\n]*::|\bvar_os[ \t\r\n]*\(|\bdevelopment[ \t\r\n]*::|\bcandidate_from_environment[ \t\r\n]*\()'
    ) {
        Add-ContractFailure `
            -Message "Release select_once must not read environment variables or access development candidates."
    }
}

$dependencySelector = Get-UniqueBracedItem `
    -Source $runtimeSelector `
    -SignaturePattern '(?m)^[ \t]*async[ \t]+fn[ \t]+select_with_dependencies[ \t]*\(' `
    -Description "select_with_dependencies" `
    -TopLevel
if ($null -ne $dependencySelector) {
    $selectorTopLevel = Get-TopLevelRustBody -Body $dependencySelector.Body
    $timeoutConstants = [regex]::Matches(
        $selectorTopLevel,
        '(?m)^[ \t]*const[ \t]+DEVELOPMENT_TIMEOUT[ \t]*:[ \t]*Duration[ \t]*=[ \t]*Duration::from_secs\([ \t]*5[ \t]*\)[ \t]*;'
    )
    if ($timeoutConstants.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies named constants must define the five-second deadline exactly once."
    }
    $retryConstants = [regex]::Matches(
        $selectorTopLevel,
        '(?m)^[ \t]*const[ \t]+DEVELOPMENT_RETRY_DELAY[ \t]*:[ \t]*Duration[ \t]*=[ \t]*Duration::from_millis\([ \t]*50[ \t]*\)[ \t]*;'
    )
    if ($retryConstants.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies named constants must define the 50-ms retry interval exactly once."
    }
    $deadlineReferences = [regex]::Matches(
        $selectorTopLevel,
        '(?m)^[ \t]*let[ \t]+deadline[ \t]*=[ \t]*Instant::now\(\)[ \t]*\+[ \t]*DEVELOPMENT_TIMEOUT[ \t]*;'
    )
    if ($deadlineReferences.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies constant references must use DEVELOPMENT_TIMEOUT for the deadline."
    }
    $candidateBindings = [regex]::Matches(
        $selectorTopLevel,
        '(?m)^[ \t]*let[ \t]+Some\(candidate\)[ \t]*=[ \t]*candidate[ \t]+else[ \t]*'
    )
    if ($candidateBindings.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies candidate must be bound on the active top-level path."
    }

    $selectionLoop = Get-UniqueBracedItem `
        -Source $dependencySelector.Body `
        -SignaturePattern '(?m)^[ \t]*loop[ \t]*' `
        -Description "select_with_dependencies selection loop" `
        -TopLevel
    if ($null -ne $selectionLoop) {
        $loopTopLevel = Get-TopLevelRustBody -Body $selectionLoop.Body
        $retryReferences = [regex]::Matches(
            $loopTopLevel,
            '(?m)^[ \t]*tokio::time::sleep\([ \t]*DEVELOPMENT_RETRY_DELAY\.min\([ \t]*deadline[ \t]*-[ \t]*now[ \t]*\)[ \t]*\)\.await[ \t]*;'
        )
        if ($retryReferences.Count -ne 1) {
            Add-ContractFailure `
                -Message "select_with_dependencies constant references must use DEVELOPMENT_RETRY_DELAY for sleep."
        }

        $selectionIf = Get-UniqueBracedItem `
            -Source $selectionLoop.Body `
            -SignaturePattern '(?ms)^[ \t]*if[ \t]+let[ \t]+Some\(process_id\)[ \t]*=[ \t]*client\.discovered_process_id\(\)[ \t\r\n]*&&[ \t\r\n]*process_matches\([ \t]*process_id[ \t]*,[ \t]*&candidate[ \t]*\)[ \t]*' `
            -Description "select_with_dependencies selection loop initial process identity check" `
            -TopLevel
        if ($null -ne $selectionIf) {
            $selectionBody = Get-TopLevelRustBody -Body $selectionIf.Body
            $boundClientIndex = Get-UniquePatternIndex `
                -Source $selectionBody `
                -Pattern 'let[ \t]+bound[ \t]*=[ \t]*client\.bound_to_process\([ \t]*process_id[ \t]*\)[ \t]*;' `
                -Description "select_with_dependencies selection loop in order PID-bound client"
            $connectionIndex = Get-UniquePatternIndex `
                -Source $selectionBody `
                -Pattern '(?s)let[ \t]+Ok\(connection\)[ \t\r\n]*=[ \t\r\n]*tokio::time::timeout_at\([ \t\r\n]*deadline[ \t\r\n]*,[ \t\r\n]*connection_probe\([ \t\r\n]*bound\.clone\(\)[ \t\r\n]*\)[ \t\r\n]*\)\.await' `
                -Description "select_with_dependencies selection loop in order deadline-bound connection probe"
            $secondIdentityIndex = Get-UniquePatternIndex `
                -Source $selectionBody `
                -Pattern 'let[ \t]+still_matches[ \t]*=[ \t]*process_matches\([ \t]*process_id[ \t]*,[ \t]*&candidate[ \t]*\)[ \t]*;' `
                -Description "select_with_dependencies selection loop in order post-connection process identity check"

            if (
                @(
                    $boundClientIndex,
                    $connectionIndex,
                    $secondIdentityIndex
                ) -notcontains -1 -and
                -not (
                    $boundClientIndex -lt $connectionIndex -and
                    $connectionIndex -lt $secondIdentityIndex
                )
            ) {
                Add-ContractFailure `
                    -Message "select_with_dependencies selection loop must bind the PID, probe before the deadline, then recheck identity in order."
            }
        }
    }
}

$desktopErrorEnum = Get-UniqueBracedItem `
    -Source $desktopControl `
    -SignaturePattern '(?m)^[ \t]*pub\(crate\)[ \t]+enum[ \t]+DesktopControlError[ \t]*' `
    -Description "DesktopControlError enum"
if ($null -ne $desktopErrorEnum) {
    $desktopErrorMatches = [regex]::Matches(
        $desktopErrorEnum.CodeBody,
        '(?m)^[ \t]*#\[error\([ \t]+\)\][ \t]*\r?\n[ \t]*DevelopmentRuntimeManagedByIde[ \t]*,[ \t]*$'
    )
    if ($desktopErrorMatches.Count -ne 1) {
        Add-ContractFailure `
            -Message "DesktopControlError IDE-managed variant must occur exactly once."
    }
    else {
        $desktopErrorSource = $desktopErrorEnum.Body.Substring(
            $desktopErrorMatches[0].Index,
            $desktopErrorMatches[0].Length
        )
        if (
            $desktopErrorSource -notmatch
            '(?m)^[ \t]*#\[error\("development_runtime_managed_by_ide"\)\][ \t]*\r?\n[ \t]*DevelopmentRuntimeManagedByIde[ \t]*,[ \t]*$'
        ) {
            Add-ContractFailure `
                -Message "DesktopControlError IDE-managed variant must retain its exact error value."
        }
    }
}

$noSwitchTest = Get-UniqueBracedItem `
    -Source $runtimeSelectorTests `
    -SignaturePattern '(?m)^[ \t]*async[ \t]+fn[ \t]+a_selected_development_session_never_switches_to_production[ \t]*\([^)]*\)[ \t]*' `
    -Description "Development no-switch regression test" `
    -TopLevel
if ($null -ne $noSwitchTest) {
    $noSwitchAttributes = Get-RustOuterAttributesBeforeItem `
        -Source $runtimeSelectorTests `
        -ItemStart $noSwitchTest.SignatureIndex `
        -Description "Development no-switch regression test attributes"
    if ($null -ne $noSwitchAttributes) {
        $tokioTestAttributeCount = 0
        $executionChangingAttributeCount = 0
        foreach ($attribute in $noSwitchAttributes.Items) {
            if (
                $attribute.Code -match
                '(?s)^[ \t\r\n]*#[ \t\r\n]*\[[ \t\r\n]*tokio[ \t\r\n]*::[ \t\r\n]*test\b.*\][ \t\r\n]*$'
            ) {
                $tokioTestAttributeCount += 1
            }
            if (
                $attribute.Code -match
                '(?<![A-Za-z0-9_])(?:cfg|cfg_attr|ignore|should_panic)(?![A-Za-z0-9_])'
            ) {
                $executionChangingAttributeCount += 1
            }
        }
        if ($tokioTestAttributeCount -ne 1) {
            Add-ContractFailure `
                -Message "Development no-switch regression test must contain exactly one Tokio test attribute."
        }
        if ($executionChangingAttributeCount -gt 0) {
            Add-ContractFailure `
                -Message "Development no-switch regression test must not be ignored or use execution-changing attributes."
        }
    }

    $noSwitchBody = Get-TopLevelRustBody -Body $noSwitchTest.Body
    $null = Get-UniquePatternIndex `
        -Source $noSwitchBody `
        -Pattern 'assert_eq!\([ \t\r\n]*selected\.channel\(\)[ \t\r\n]*,[ \t\r\n]*WokCoreRuntimeChannel::Development[ \t\r\n]*\)[ \t]*;' `
        -Description "Development no-switch test development channel assertion"
    $null = Get-UniquePatternIndex `
        -Source $noSwitchBody `
        -Pattern 'assert_eq!\([ \t\r\n]*selected\.executable\(\)[ \t\r\n]*,[ \t\r\n]*Some\(development\.as_path\(\)\)[ \t\r\n]*\)[ \t]*;' `
        -Description "Development no-switch test selected executable assertion"
    $null = Get-UniquePatternIndex `
        -Source $noSwitchBody `
        -Pattern 'assert_eq!\([ \t\r\n]*selected\.connection\(\)\.await[ \t\r\n]*,[ \t\r\n]*CoreConnection::Stopped[ \t\r\n]*\)[ \t]*;' `
        -Description "Development no-switch test stopped retained connection assertion"
    $null = Get-UniquePatternIndex `
        -Source $noSwitchBody `
        -Pattern 'assert!\([ \t\r\n]*replacement\.received_requests\(\)\.await\.unwrap\(\)\.is_empty\(\)[ \t\r\n]*\)[ \t]*;' `
        -Description "Development no-switch test replacement zero requests assertion"
    $null = Get-UniquePatternIndex `
        -Source $noSwitchBody `
        -Pattern 'assert_eq!\([ \t\r\n]*discoveries\.load\(Ordering::SeqCst\)[ \t\r\n]*,[ \t\r\n]*0[ \t\r\n]*\)[ \t]*;' `
        -Description "Development no-switch test production discovery zero calls assertion"
}

$rustStatusMatch = [regex]::Match(
    $commandModel,
    '(?s)pub struct CoreStatus\s*\{(?<body>.*?)\r?\n\}'
)
if (-not $rustStatusMatch.Success) {
    Add-ContractFailure -Message "Rust runtime status model must remain identifiable."
}
else {
    $rustStatus = $rustStatusMatch.Groups["body"].Value
    if ($rustStatus -notmatch '(?m)^\s*pub runtime_channel\s*:') {
        Add-ContractFailure -Message "Rust runtime status must expose runtime_channel."
    }
    if ($rustStatus -match '(?m)^\s*pub (pid|path|executable)\s*:') {
        Add-ContractFailure `
            -Message "Rust runtime status must not expose a private runtime field."
    }
}

$frontendStatusMatch = [regex]::Match(
    $frontendControl,
    '(?s)const coreStatusSchema\s*=\s*z\s*\.object\(\{(?<body>.*?)\}\)\s*\.strict\(\);'
)
if (-not $frontendStatusMatch.Success) {
    Add-ContractFailure -Message "Frontend runtime status model must remain identifiable."
}
else {
    $frontendStatus = $frontendStatusMatch.Groups["body"].Value
    if ($frontendStatus -notmatch '(?m)^\s*runtime_channel\s*:') {
        Add-ContractFailure -Message "Frontend runtime status must expose runtime_channel."
    }
    if ($frontendStatus -match '(?m)^\s*(pid|path|executable)\s*:') {
        Add-ContractFailure `
            -Message "Frontend runtime status must not expose a private runtime field."
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Host "CONTRACT ERROR: $failure"
    }
    exit 1
}

Write-Host "Foundation CI/configuration contract passed."
