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
$wokcorePublicKeyPath = Join-Path $rootPath "crates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub"
$commandModelPath = Join-Path $rootPath "apps/cli/src/commands/mod.rs"
$desktopControlPath = Join-Path $rootPath "apps/desktop/src-tauri/src/control.rs"
$coreOperationPath = Join-Path $rootPath "apps/desktop/src-tauri/src/core_operation.rs"
$desktopLibPath = Join-Path $rootPath "apps/desktop/src-tauri/src/lib.rs"
$frontendControlPath = Join-Path $rootPath "apps/desktop/src/control.ts"
$coreUpdateEligibilityPath = Join-Path $rootPath "apps/desktop/src/coreUpdateEligibility.ts"
$coreLifecyclePath = Join-Path $rootPath "apps/desktop/src/components/CoreLifecycle.tsx"
$coreLifecycleTestsPath = Join-Path $rootPath "apps/desktop/src/components/CoreLifecycle.test.tsx"
$localeTestsPath = Join-Path $rootPath "apps/desktop/src/locale.test.ts"
$desktopPackagePath = Join-Path $rootPath "apps/desktop/package.json"
$eventCapabilityPath = Join-Path $rootPath "apps/desktop/src-tauri/capabilities/main.json"
$desktopBootstrapPath = Join-Path $rootPath "apps/desktop/src/main.tsx"
$frontendLocalePath = Join-Path $rootPath "apps/desktop/src/locale.ts"
$desktopI18nPath = Join-Path $rootPath "apps/desktop/src/i18n/index.ts"
$systemLocalePath = Join-Path $rootPath "crates/wokrouter-platform/src/system/locale.rs"
$packagedEventSmokePath = Join-Path $rootPath "tests/scripts/smoke-packaged-event-bridge.ps1"
$englishCatalogPath = Join-Path $rootPath "apps/desktop/src/i18n/locales/en.json"
$simplifiedChineseCatalogPath = Join-Path $rootPath "apps/desktop/src/i18n/locales/zh-CN.json"
$desktopMainPath = Join-Path $rootPath "apps/desktop/src-tauri/src/main.rs"
$windowsPackagerPath = Join-Path $rootPath "tests/release/package-windows-assets.ps1"
$coreOperationParserPath = Join-Path $rootPath "apps/desktop/src-tauri/src/core_operation/parser.rs"
$wokcoreInstallTestsPath = Join-Path $rootPath "crates/wokrouter-platform/tests/wokcore_install.rs"
$cliStartTestsPath = Join-Path $rootPath "apps/cli/src/commands/start/tests.rs"
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

function Get-PowerShellAst {
    param(
        [Parameter(Mandatory)]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Description
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

function Get-ExactPowerShellGuardAst {
    param(
        [Parameter(Mandatory)]
        [object]$Ast,

        [Parameter(Mandatory)]
        [string]$Condition,

        [Parameter(Mandatory)]
        [string]$ThrowStatement
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
    $delimiterStack = [System.Collections.Generic.Stack[char]]::new()
    for ($index = 0; $index -lt $code.Length; $index += 1) {
        $current = $code[$index]
        if ($current -eq "(" -or $current -eq "[" -or $current -eq "{") {
            $delimiterStack.Push($current)
            continue
        }
        if ($current -ne ")" -and $current -ne "]" -and $current -ne "}") {
            continue
        }
        if ($delimiterStack.Count -eq 0) {
            Add-ContractFailure `
                -Message "$Description must have balanced Rust delimiters."
            return $null
        }
        $opening = $delimiterStack.Pop()
        if (
            ($current -eq ")" -and $opening -ne "(") -or
            ($current -eq "]" -and $opening -ne "[") -or
            ($current -eq "}" -and $opening -ne "{")
        ) {
            Add-ContractFailure `
                -Message "$Description must have balanced Rust delimiters."
            return $null
        }
    }
    if ($delimiterStack.Count -ne 0) {
        Add-ContractFailure `
            -Message "$Description must have balanced Rust delimiters."
        return $null
    }

    return [pscustomobject]@{
        Code = $code
        CommentStripped = -join $commentStrippedView
    }
}

function Get-TypeScriptQuotedLiteralEnd {
    param(
        [Parameter(Mandatory)]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $quote = $Source[$Start]
    $index = $Start + 1
    while ($index -lt $Source.Length) {
        if ($Source[$index] -eq "\") {
            $index += 2
            continue
        }
        if ($Source[$index] -eq $quote) {
            return $index + 1
        }
        if ($Source[$index] -eq "`r" -or $Source[$index] -eq "`n") {
            break
        }
        $index += 1
    }

    Add-ContractFailure `
        -Message "$Description must be lexically valid: unterminated string literal."
    return -1
}

function Get-TypeScriptBlockCommentEnd {
    param(
        [Parameter(Mandatory)]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $index = $Start + 2
    while ($index + 1 -lt $Source.Length) {
        if ($Source[$index] -eq "*" -and $Source[$index + 1] -eq "/") {
            return $index + 2
        }
        $index += 1
    }

    Add-ContractFailure `
        -Message "$Description must be lexically valid: unterminated block comment."
    return -1
}

function Get-TypeScriptTemplateInterpolationEnd {
    param(
        [Parameter(Mandatory)]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $braceDepth = 1
    $index = $Start
    while ($index -lt $Source.Length) {
        $current = $Source[$index]
        $next = if ($index + 1 -lt $Source.Length) {
            $Source[$index + 1]
        }
        else {
            [char]0
        }

        if ($current -eq "/" -and $next -eq "/") {
            $index += 2
            while (
                $index -lt $Source.Length -and
                $Source[$index] -ne "`r" -and
                $Source[$index] -ne "`n"
            ) {
                $index += 1
            }
            continue
        }
        if ($current -eq "/" -and $next -eq "*") {
            $index = Get-TypeScriptBlockCommentEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($index -lt 0) {
                return -1
            }
            continue
        }
        if ($current -eq "'" -or $current -eq '"') {
            $index = Get-TypeScriptQuotedLiteralEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($index -lt 0) {
                return -1
            }
            continue
        }
        if ($current -eq "``") {
            $index = Get-TypeScriptTemplateLiteralEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($index -lt 0) {
                return -1
            }
            continue
        }
        if ($current -eq "{") {
            $braceDepth += 1
        }
        elseif ($current -eq "}") {
            $braceDepth -= 1
            if ($braceDepth -eq 0) {
                return $index + 1
            }
        }
        $index += 1
    }

    Add-ContractFailure `
        -Message "$Description must be lexically valid: unterminated template interpolation."
    return -1
}

function Get-TypeScriptTemplateLiteralEnd {
    param(
        [Parameter(Mandatory)]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$Start,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $index = $Start + 1
    while ($index -lt $Source.Length) {
        if ($Source[$index] -eq "\") {
            $index += 2
            continue
        }
        if ($Source[$index] -eq "``") {
            return $index + 1
        }
        if (
            $Source[$index] -eq '$' -and
            $index + 1 -lt $Source.Length -and
            $Source[$index + 1] -eq "{"
        ) {
            $index = Get-TypeScriptTemplateInterpolationEnd `
                -Source $Source `
                -Start ($index + 2) `
                -Description $Description
            if ($index -lt 0) {
                return -1
            }
            continue
        }
        $index += 1
    }

    Add-ContractFailure `
        -Message "$Description must be lexically valid: unterminated template literal."
    return -1
}

function Get-TypeScriptCodeView {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $view = $Source.ToCharArray()
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
            $index = $end
            continue
        }

        if ($current -eq "/" -and $next -eq "*") {
            $end = Get-TypeScriptBlockCommentEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($end -lt 0) {
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        if ($current -eq "'" -or $current -eq '"') {
            $end = Get-TypeScriptQuotedLiteralEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($end -lt 0) {
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        if ($current -eq "``") {
            $end = Get-TypeScriptTemplateLiteralEnd `
                -Source $Source `
                -Start $index `
                -Description $Description
            if ($end -lt 0) {
                return $null
            }
            Set-RustMaskedRange -View $view -Start $index -End $end
            $index = $end
            continue
        }

        $index += 1
    }

    $code = -join $view
    $delimiterStack = [System.Collections.Generic.List[char]]::new()
    for ($index = 0; $index -lt $code.Length; $index += 1) {
        $current = $code[$index]
        if ($current -eq "(" -or $current -eq "[" -or $current -eq "{") {
            $delimiterStack.Add($current)
            continue
        }
        if ($current -ne ")" -and $current -ne "]" -and $current -ne "}") {
            continue
        }
        if ($delimiterStack.Count -eq 0) {
            Add-ContractFailure `
                -Message "$Description must have balanced TypeScript delimiters."
            return $null
        }
        $opening = $delimiterStack[$delimiterStack.Count - 1]
        $delimiterStack.RemoveAt($delimiterStack.Count - 1)
        if (
            ($current -eq ")" -and $opening -ne "(") -or
            ($current -eq "]" -and $opening -ne "[") -or
            ($current -eq "}" -and $opening -ne "{")
        ) {
            Add-ContractFailure `
                -Message "$Description must have balanced TypeScript delimiters."
            return $null
        }
    }
    if ($delimiterStack.Count -ne 0) {
        Add-ContractFailure `
            -Message "$Description must have balanced TypeScript delimiters."
        return $null
    }

    return [pscustomobject]@{
        Code = $code
    }
}

function Get-TypeScriptDirectStatements {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Description,

        [AllowNull()]
        [object]$CodeView
    )

    $codeView = if ($null -ne $CodeView) {
        $CodeView
    }
    else {
        Get-TypeScriptCodeView -Source $Source -Description $Description
    }
    if ($null -eq $codeView) {
        return @()
    }
    if ($codeView.Code.Length -ne $Source.Length) {
        Add-ContractFailure `
            -Message "$Description code view must preserve source offsets."
        return @()
    }

    $statements = @()
    $statementStart = 0
    $braceDepth = 0
    $parenthesisDepth = 0
    $bracketDepth = 0
    for ($index = 0; $index -lt $codeView.Code.Length; $index += 1) {
        $current = $codeView.Code[$index]
        switch ($current) {
            "{" { $braceDepth += 1; continue }
            "(" { $parenthesisDepth += 1; continue }
            "[" { $bracketDepth += 1; continue }
            "}" { $braceDepth -= 1 }
            ")" { $parenthesisDepth -= 1 }
            "]" { $bracketDepth -= 1 }
        }
        $atStatementBoundary = (
            $braceDepth -eq 0 -and
            $parenthesisDepth -eq 0 -and
            $bracketDepth -eq 0 -and
            ($current -eq ";" -or $current -eq "}")
        )
        if (-not $atStatementBoundary) {
            continue
        }

        $trimmedStart = $statementStart
        while (
            $trimmedStart -le $index -and
            [char]::IsWhiteSpace($codeView.Code[$trimmedStart])
        ) {
            $trimmedStart += 1
        }
        if ($trimmedStart -le $index) {
            $length = $index - $trimmedStart + 1
            $statements += [pscustomobject]@{
                Index = $trimmedStart
                Length = $length
                Source = $Source.Substring($trimmedStart, $length)
                Code = $codeView.Code.Substring($trimmedStart, $length)
            }
        }
        $statementStart = $index + 1
    }

    $trimmedStart = $statementStart
    while (
        $trimmedStart -lt $codeView.Code.Length -and
        [char]::IsWhiteSpace($codeView.Code[$trimmedStart])
    ) {
        $trimmedStart += 1
    }
    if ($trimmedStart -lt $codeView.Code.Length) {
        $length = $codeView.Code.Length - $trimmedStart
        $statements += [pscustomobject]@{
            Index = $trimmedStart
            Length = $length
            Source = $Source.Substring($trimmedStart, $length)
            Code = $codeView.Code.Substring($trimmedStart, $length)
        }
    }

    return @($statements)
}

function Get-RustOwnershipAtIndex {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Structure,

        [Parameter(Mandatory)]
        [int]$Index
    )

    $braceDepth = 0
    $parenthesisDepth = 0
    $bracketDepth = 0
    $statementStart = $true
    for ($cursor = 0; $cursor -lt $Index; $cursor += 1) {
        $current = $Structure[$cursor]
        if ([char]::IsWhiteSpace($current)) {
            continue
        }

        $allDepthsZero = (
            $braceDepth -eq 0 -and
            $parenthesisDepth -eq 0 -and
            $bracketDepth -eq 0
        )
        if ($current -eq "{") {
            if ($allDepthsZero) {
                $statementStart = $false
            }
            $braceDepth += 1
            continue
        }
        if ($current -eq "(") {
            if ($allDepthsZero) {
                $statementStart = $false
            }
            $parenthesisDepth += 1
            continue
        }
        if ($current -eq "[") {
            if ($allDepthsZero) {
                $statementStart = $false
            }
            $bracketDepth += 1
            continue
        }
        if ($current -eq "}") {
            $braceDepth -= 1
            if (
                $braceDepth -eq 0 -and
                $parenthesisDepth -eq 0 -and
                $bracketDepth -eq 0
            ) {
                $statementStart = $true
            }
            continue
        }
        if ($current -eq ")") {
            $parenthesisDepth -= 1
            continue
        }
        if ($current -eq "]") {
            $bracketDepth -= 1
            continue
        }
        if ($allDepthsZero) {
            if ($current -eq ";") {
                $statementStart = $true
            }
            else {
                $statementStart = $false
            }
        }
    }

    return [pscustomobject]@{
        AllDelimiterDepthsZero = (
            $braceDepth -eq 0 -and
            $parenthesisDepth -eq 0 -and
            $bracketDepth -eq 0
        )
        StatementStart = $statementStart
    }
}

function Get-RustOwnedPatternMatches {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Pattern,

        [Parameter(Mandatory)]
        [string]$Description,

        [switch]$RequireStatementStart
    )

    $codeView = Get-RustCodeView -Source $Source -Description $Description
    if ($null -eq $codeView) {
        return @()
    }
    $matches = @([regex]::Matches(
        $codeView.Code,
        $Pattern,
        [System.Text.RegularExpressions.RegexOptions]::Multiline
    ))
    $ownedMatches = @()
    foreach ($match in $matches) {
        $ownership = Get-RustOwnershipAtIndex `
            -Structure $codeView.Code `
            -Index $match.Index
        if (
            $ownership.AllDelimiterDepthsZero -and
            (-not $RequireStatementStart -or $ownership.StatementStart)
        ) {
            $ownedMatches += $match
        }
    }
    return @($ownedMatches)
}

function Get-UniqueDirectStatementIndex {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$Pattern,

        [Parameter(Mandatory)]
        [string]$Description
    )

    $matches = @(Get-RustOwnedPatternMatches `
        -Source $Source `
        -Pattern $Pattern `
        -Description $Description `
        -RequireStatementStart)
    if ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "$Description must occur as one direct statement; found $($matches.Count)."
        return -1
    }
    return $matches[0].Index
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

        [switch]$TopLevel,

        [switch]$DirectStatement,

        [int]$MatchOrdinal = -1,

        [AllowNull()]
        [object]$CodeView
    )

    $codeView = if ($null -ne $CodeView) {
        $CodeView
    }
    else {
        Get-RustCodeView -Source $Source -Description $Description
    }
    if ($null -eq $codeView) {
        return $null
    }
    $structure = $codeView.Code
    $matches = @([regex]::Matches(
        $structure,
        $SignaturePattern,
        [System.Text.RegularExpressions.RegexOptions]::Multiline
    ))
    if ($TopLevel -or $DirectStatement) {
        $topLevelMatches = @()
        foreach ($candidateMatch in $matches) {
            $ownership = Get-RustOwnershipAtIndex `
                -Structure $structure `
                -Index $candidateMatch.Index
            if (
                $ownership.AllDelimiterDepthsZero -and
                (-not $DirectStatement -or $ownership.StatementStart)
            ) {
                $topLevelMatches += $candidateMatch
            }
        }
        $matches = @($topLevelMatches)
    }
    if ($MatchOrdinal -ge 0) {
        if ($MatchOrdinal -ge $matches.Count) {
            Add-ContractFailure `
                -Message "$Description match ordinal $MatchOrdinal is unavailable; found $($matches.Count)."
            return $null
        }
        $match = $matches[$MatchOrdinal]
    }
    elseif ($matches.Count -ne 1) {
        Add-ContractFailure `
            -Message "$Description must remain uniquely identifiable; found $($matches.Count)."
        return $null
    }
    else {
        $match = $matches[0]
    }
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
        [string]$Description,

        [AllowNull()]
        [object]$CodeView
    )

    $codeView = if ($null -ne $CodeView) {
        $CodeView
    }
    else {
        Get-RustCodeView -Source $Source -Description $Description
    }
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

function Get-PreviousNonWhitespaceIndex {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$BeforeIndex
    )

    for ($index = $BeforeIndex - 1; $index -ge 0; $index -= 1) {
        if (-not [char]::IsWhiteSpace($Source[$index])) {
            return $index
        }
    }
    return -1
}

function Get-MatchingOpeningParenthesisIndex {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [int]$ClosingIndex
    )

    if (
        $ClosingIndex -lt 0 -or
        $ClosingIndex -ge $Source.Length -or
        $Source[$ClosingIndex] -ne ")"
    ) {
        return -1
    }
    $depth = 0
    for ($index = $ClosingIndex; $index -ge 0; $index -= 1) {
        if ($Source[$index] -eq ")") {
            $depth += 1
        }
        elseif ($Source[$index] -eq "(") {
            $depth -= 1
            if ($depth -eq 0) {
                return $index
            }
        }
    }
    return -1
}

function Test-TypeScriptExecutableTestDescription {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$TestDescription,

        [Parameter(Mandatory)]
        [object]$CodeView
    )

    $descriptionMatches = [regex]::Matches(
        $CodeView.CommentStripped,
        [regex]::Escape($TestDescription)
    )
    foreach ($descriptionMatch in $descriptionMatches) {
        $quoteIndex = $descriptionMatch.Index - 1
        $closingQuoteIndex = (
            $descriptionMatch.Index + $descriptionMatch.Length
        )
        if (
            $quoteIndex -lt 0 -or
            $closingQuoteIndex -ge $Source.Length
        ) {
            continue
        }
        $quote = $Source[$quoteIndex]
        if (
            ($quote -ne '"' -and $quote -ne "'") -or
            $Source[$closingQuoteIndex] -ne $quote
        ) {
            continue
        }

        $callOpeningIndex = Get-PreviousNonWhitespaceIndex `
            -Source $CodeView.Code `
            -BeforeIndex $quoteIndex
        if (
            $callOpeningIndex -lt 0 -or
            $CodeView.Code[$callOpeningIndex] -ne "("
        ) {
            continue
        }
        $calleeEndIndex = Get-PreviousNonWhitespaceIndex `
            -Source $CodeView.Code `
            -BeforeIndex $callOpeningIndex
        if ($calleeEndIndex -lt 0) {
            continue
        }
        $directPrefix = $CodeView.Code.Substring(0, $calleeEndIndex + 1)
        if (
            $directPrefix -match
            '(?<![A-Za-z0-9_$\.])(?:it|test)$'
        ) {
            return $true
        }
        if ($CodeView.Code[$calleeEndIndex] -ne ")") {
            continue
        }
        $eachOpeningIndex = Get-MatchingOpeningParenthesisIndex `
            -Source $CodeView.Code `
            -ClosingIndex $calleeEndIndex
        if ($eachOpeningIndex -lt 0) {
            continue
        }
        $eachCalleeEndIndex = Get-PreviousNonWhitespaceIndex `
            -Source $CodeView.Code `
            -BeforeIndex $eachOpeningIndex
        if ($eachCalleeEndIndex -lt 0) {
            continue
        }
        $eachPrefix = $CodeView.Code.Substring(0, $eachCalleeEndIndex + 1)
        if (
            $eachPrefix -match
            '(?<![A-Za-z0-9_$\.])(?:it|test)\.each$'
        ) {
            return $true
        }
    }
    return $false
}

function Test-RustExecutableTestFunction {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [string]$FunctionName,

        [Parameter(Mandatory)]
        [object]$CodeView,

        [Parameter(Mandatory)]
        [object]$Container
    )

    $functionPattern = (
        '(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?' +
        '(?:async[ \t]+)?fn[ \t]+' +
        [regex]::Escape($FunctionName) +
        '[ \t]*\('
    )
    $functionMatches = [regex]::Matches(
        $CodeView.Code,
        $functionPattern
    )
    if ($functionMatches.Count -ne 1) {
        return $false
    }
    $functionMatch = $functionMatches[0]
    $ownership = if ($Container.Kind -eq "TopLevel") {
        Get-RustOwnershipAtIndex `
            -Structure $CodeView.Code `
            -Index $functionMatch.Index
    }
    elseif (
        $Container.Kind -eq "CfgTestModule" -and
        $functionMatch.Index -gt $Container.OpeningBraceIndex -and
        $functionMatch.Index -lt $Container.ClosingBraceIndex
    ) {
        $modulePrefix = $CodeView.Code.Substring(
            $Container.OpeningBraceIndex + 1,
            $functionMatch.Index - $Container.OpeningBraceIndex - 1
        )
        Get-RustOwnershipAtIndex `
            -Structure $modulePrefix `
            -Index $modulePrefix.Length
    }
    else {
        $null
    }
    if (
        $null -eq $ownership -or
        -not $ownership.AllDelimiterDepthsZero
    ) {
        return $false
    }
    $attributes = Get-RustOuterAttributesBeforeItem `
        -Source $Source `
        -ItemStart $functionMatch.Index `
        -Description "Rust lifecycle acceptance test function" `
        -CodeView $CodeView
    if ($null -eq $attributes) {
        return $false
    }
    $testAttributes = [regex]::Matches(
        $attributes.Code,
        '(?m)^[ \t]*#\[(?:test|tokio::test)\][ \t]*$'
    )
    if (
        $testAttributes.Count -ne 1 -or
        $attributes.Code -match
        '(?m)^[ \t]*#\[(?:cfg|cfg_attr|ignore|should_panic)\b'
    ) {
        return $false
    }
    return $true
}

function Test-RustTestSupportFeatureInnerCfg {
    param(
        [Parameter(Mandatory)]
        [string]$CommentStrippedAttribute
    )

    return (
        $CommentStrippedAttribute -match
        '(?s)\A#!\[[ \t\r\n]*cfg[ \t\r\n]*\([ \t\r\n]*feature[ \t\r\n]*=[ \t\r\n]*"test-support"[ \t\r\n]*\)[ \t\r\n]*\]\z'
    )
}

function Get-RustExecutableTestContainer {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [ValidateSet("TopLevel", "CfgTestModule")]
        [string]$Kind,

        [Parameter(Mandatory)]
        [object]$CodeView,

        [bool]$RequireTestSupportFeatureCfg = $false
    )

    $fileInnerCfgAttributes = @(
        Get-RustDirectConditionalInnerAttributes `
            -Source $Source `
            -CodeView $CodeView
    )
    if ($Kind -eq "TopLevel") {
        if ($RequireTestSupportFeatureCfg) {
            if (
                $fileInnerCfgAttributes.Count -ne 1 -or
                -not (
                    Test-RustTestSupportFeatureInnerCfg `
                        -CommentStrippedAttribute (
                            $fileInnerCfgAttributes[0].CommentStripped
                        )
                )
            ) {
                return $null
            }
        }
        elseif ($fileInnerCfgAttributes.Count -ne 0) {
            return $null
        }
        return [pscustomobject]@{
            Kind = "TopLevel"
        }
    }
    if ($fileInnerCfgAttributes.Count -ne 0) {
        return $null
    }

    $testModule = Get-UniqueBracedItem `
        -Source $Source `
        -SignaturePattern '(?ms)(?<attributes>(?:(?:^[ \t]*#\[[^\r\n]*\][ \t]*\r?\n)|(?:^[ \t]*\r?\n))*)^[ \t]*mod[ \t]+tests[ \t]*' `
        -Description "Lifecycle acceptance test module" `
        -TopLevel `
        -CodeView $CodeView
    if (
        $null -eq $testModule -or
        $testModule.CodeAttributes -notmatch
        '(?s)^\s*#\[cfg\([ \t]*test[ \t]*\)\]\s*$'
    ) {
        return $null
    }
    $moduleBodyLength = (
        $testModule.ClosingBraceIndex -
        $testModule.OpeningBraceIndex -
        1
    )
    $moduleCodeView = [pscustomobject]@{
        Code = $testModule.CodeBody
        CommentStripped = $CodeView.CommentStripped.Substring(
            $testModule.OpeningBraceIndex + 1,
            $moduleBodyLength
        )
    }
    $moduleInnerCfgAttributes = @(
        Get-RustDirectConditionalInnerAttributes `
            -Source $testModule.Body `
            -CodeView $moduleCodeView
    )
    if ($moduleInnerCfgAttributes.Count -ne 0) {
        return $null
    }

    return [pscustomobject]@{
        Kind = "CfgTestModule"
        OpeningBraceIndex = $testModule.OpeningBraceIndex
        ClosingBraceIndex = $testModule.ClosingBraceIndex
    }
}

function Get-RustDirectConditionalInnerAttributes {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string]$Source,

        [Parameter(Mandatory)]
        [object]$CodeView
    )

    $attributes = @()
    $attributeStarts = [regex]::Matches(
        $CodeView.Code,
        '(?m)^[ \t]*#!\['
    )
    foreach ($attributeStart in $attributeStarts) {
        $hashIndex = $CodeView.Code.IndexOf(
            "#",
            $attributeStart.Index,
            $attributeStart.Length
        )
        $ownership = Get-RustOwnershipAtIndex `
            -Structure $CodeView.Code `
            -Index $hashIndex
        if (-not $ownership.AllDelimiterDepthsZero) {
            continue
        }

        $openingBracketIndex = $CodeView.Code.IndexOf(
            "[",
            $hashIndex,
            $attributeStart.Index + $attributeStart.Length - $hashIndex
        )
        $bracketDepth = 0
        $closingBracketIndex = -1
        for (
            $index = $openingBracketIndex;
            $index -lt $CodeView.Code.Length;
            $index += 1
        ) {
            if ($CodeView.Code[$index] -eq "[") {
                $bracketDepth += 1
            }
            elseif ($CodeView.Code[$index] -eq "]") {
                $bracketDepth -= 1
                if ($bracketDepth -eq 0) {
                    $closingBracketIndex = $index
                    break
                }
            }
        }
        if ($closingBracketIndex -lt 0) {
            continue
        }
        $attributeLength = $closingBracketIndex - $hashIndex + 1
        $commentStripped = $CodeView.CommentStripped.Substring(
            $hashIndex,
            $attributeLength
        )
        if (
            $commentStripped -match
            '(?s)^#!\[[ \t\r\n]*(?:cfg|cfg_attr)\b'
        ) {
            $attributes += [pscustomobject]@{
                CommentStripped = $commentStripped
            }
        }
    }
    return @($attributes)
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
$wokcorePublicKey = Get-Content -LiteralPath $wokcorePublicKeyPath -Raw -Encoding UTF8
$commandModel = Get-Content -LiteralPath $commandModelPath -Raw -Encoding UTF8
$desktopControl = Get-Content -LiteralPath $desktopControlPath -Raw -Encoding UTF8
$coreOperation = Get-Content -LiteralPath $coreOperationPath -Raw -Encoding UTF8
$desktopLib = Get-Content -LiteralPath $desktopLibPath -Raw -Encoding UTF8
$frontendControl = Get-Content -LiteralPath $frontendControlPath -Raw -Encoding UTF8
$coreUpdateEligibility = Get-Content -LiteralPath $coreUpdateEligibilityPath -Raw -Encoding UTF8
$coreLifecycle = Get-Content -LiteralPath $coreLifecyclePath -Raw -Encoding UTF8
$coreLifecycleTests = Get-Content -LiteralPath $coreLifecycleTestsPath -Raw -Encoding UTF8
$localeTests = Get-Content -LiteralPath $localeTestsPath -Raw -Encoding UTF8
$desktopPackageSource = Get-Content -LiteralPath $desktopPackagePath -Raw -Encoding UTF8
$eventCapabilitySource = Get-Content -LiteralPath $eventCapabilityPath -Raw -Encoding UTF8
$desktopBootstrap = Get-Content -LiteralPath $desktopBootstrapPath -Raw -Encoding UTF8
$frontendLocale = Get-Content -LiteralPath $frontendLocalePath -Raw -Encoding UTF8
$desktopI18n = Get-Content -LiteralPath $desktopI18nPath -Raw -Encoding UTF8
$systemLocale = Get-Content -LiteralPath $systemLocalePath -Raw -Encoding UTF8
$packagedEventSmoke = Get-Content -LiteralPath $packagedEventSmokePath -Raw -Encoding UTF8
$desktopMain = Get-Content -LiteralPath $desktopMainPath -Raw -Encoding UTF8
$windowsPackager = Get-Content -LiteralPath $windowsPackagerPath -Raw -Encoding UTF8
$coreOperationParser = Get-Content -LiteralPath $coreOperationParserPath -Raw -Encoding UTF8
$wokcoreInstallTests = Get-Content -LiteralPath $wokcoreInstallTestsPath -Raw -Encoding UTF8
$cliStartTests = Get-Content -LiteralPath $cliStartTestsPath -Raw -Encoding UTF8
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

$desktopPackage = $null
try {
    $desktopPackage = $desktopPackageSource | ConvertFrom-Json
}
catch {
    Add-ContractFailure -Message "Desktop package manifest must contain valid JSON."
}
if (
    $null -ne $desktopPackage -and
    $desktopPackage.scripts.'i18n:check' -cne
    "node scripts/check-i18n-catalogs.mjs"
) {
    Add-ContractFailure `
        -Message "Desktop package must expose the standalone i18n:check catalog command."
}

$eventCapability = $null
try {
    $eventCapability = $eventCapabilitySource | ConvertFrom-Json
}
catch {
    Add-ContractFailure -Message "Desktop event capability must contain valid JSON."
}
if ($null -ne $eventCapability) {
    $capabilityProperties = @($eventCapability.PSObject.Properties.Name | Sort-Object)
    $expectedCapabilityProperties = @(
        '$schema',
        'description',
        'identifier',
        'permissions',
        'windows'
    )
    $capabilityWindows = @($eventCapability.windows)
    $capabilityPermissions = @($eventCapability.permissions)
    if (
        (Compare-Object $capabilityProperties $expectedCapabilityProperties) -or
        $eventCapability.'$schema' -cne '../gen/schemas/desktop-schema.json' -or
        $eventCapability.identifier -cne 'main-event-listener' -or
        $eventCapability.description -cne
        'Allows the main window to monitor WokCore operations.' -or
        $capabilityWindows.Count -ne 1 -or
        $capabilityWindows[0] -cne 'main' -or
        $capabilityPermissions.Count -ne 2 -or
        $capabilityPermissions[0] -cne 'core:event:allow-listen' -or
        $capabilityPermissions[1] -cne 'core:event:allow-unlisten'
    ) {
        Add-ContractFailure `
            -Message "Desktop main window must receive only core event listen and unlisten permissions."
    }
}

$systemLocaleCodeView = Get-RustCodeView `
    -Source $systemLocale `
    -Description "System locale module"
$detectSystemLocale = Get-UniqueBracedItem `
    -Source $systemLocale `
    -SignaturePattern '(?m)^pub\s+fn\s+detect_system_locale\s*\(\s*\)\s*->\s*Option\s*<\s*String\s*>' `
    -Description "System locale detector" `
    -TopLevel `
    -CodeView $systemLocaleCodeView
$localeFromCandidate = Get-UniqueBracedItem `
    -Source $systemLocale `
    -SignaturePattern '(?m)^fn\s+locale_from_candidate\s*\(\s*candidate\s*:\s*Option\s*<\s*&str\s*>\s*\)\s*->\s*Option\s*<\s*String\s*>' `
    -Description "System locale candidate normalizer" `
    -TopLevel `
    -CodeView $systemLocaleCodeView
if (
    $null -eq $detectSystemLocale -or
    ($detectSystemLocale.CodeBody -replace '\s', '') -cne
    'locale_from_candidate(sys_locale::get_locale().as_deref())' -or
    $null -eq $localeFromCandidate -or
    ($localeFromCandidate.CodeBody -replace '\s', '') -cne
    'candidate.and_then(normalize_locale)'
) {
    Add-ContractFailure `
        -Message "System locale detection must preserve a missing or invalid OS candidate as Option::None."
}

$desktopLibCodeViewForLocale = Get-RustCodeView `
    -Source $desktopLib `
    -Description "Desktop Tauri library"
$systemLocaleCommand = Get-UniqueBracedItem `
    -Source $desktopLib `
    -SignaturePattern '(?m)^fn\s+system_locale\s*\(\s*\)\s*->\s*Option\s*<\s*String\s*>' `
    -Description "Desktop system locale command" `
    -TopLevel `
    -CodeView $desktopLibCodeViewForLocale
if (
    $null -eq $systemLocaleCommand -or
    ($systemLocaleCommand.CodeBody -replace '\s', '') -cne
    'wokrouter_platform::detect_system_locale()'
) {
    Add-ContractFailure `
        -Message "Desktop system_locale command must preserve the optional OS locale candidate."
}

$frontendLocaleCodeView = Get-TypeScriptCodeView `
    -Source $frontendLocale `
    -Description "Desktop locale resolver"
$supportedLocaleResolver = Get-UniqueBracedItem `
    -Source $frontendLocale `
    -SignaturePattern '(?m)^export\s+function\s+resolveSupportedLocale\s*\(\s*systemLocale\s*:\s*string\s*\|\s*null\s*\|\s*undefined\s*,' `
    -Description "Desktop supported locale resolver" `
    -TopLevel `
    -CodeView $frontendLocaleCodeView
if ($null -eq $supportedLocaleResolver) {
    Add-ContractFailure `
        -Message "Desktop locale resolver must accept a null OS candidate for navigator fallback."
}

$packagedEventSmokeAst = Get-PowerShellAst `
    -Source $packagedEventSmoke `
    -Description "Packaged desktop event bridge smoke"
$smokeCommands = @($packagedEventSmokeAst.FindAll({
            param($node)
            $node -is [Management.Automation.Language.CommandAst]
        }, $true) | ForEach-Object { $_.GetCommandName() })
if (
    $smokeCommands -notcontains 'Start-Process' -or
    $smokeCommands -notcontains 'Invoke-RestMethod' -or
    $packagedEventSmoke -notmatch 'ClientWebSocket' -or
    $packagedEventSmoke -notmatch 'Runtime\.evaluate' -or
    $packagedEventSmoke -notmatch 'document\.querySelector\([^\r\n]*role="progressbar"' -or
    $packagedEventSmoke -notmatch
    '-not\s*\(\s*Test-Path\s+-LiteralPath\s+\$sidecarMarker\s+-PathType\s+Leaf\s*\)' -or
    $packagedEventSmoke -notmatch 'WOKROUTER_EVENT_SMOKE_MARKER'
) {
    Add-ContractFailure `
        -Message "Packaged desktop smoke must launch the real EXE and observe a started sidecar plus WebView progress."
}

foreach ($catalog in @(
        @{
            Path = $englishCatalogPath
            Description = "English catalog"
        },
        @{
            Path = $simplifiedChineseCatalogPath
            Description = "Simplified Chinese catalog"
        }
    )) {
    if (-not (Test-Path -LiteralPath $catalog.Path -PathType Leaf)) {
        Add-ContractFailure `
            -Message "Desktop i18n must retain the $($catalog.Description)."
    }
}

$desktopBootstrapCodeView = Get-TypeScriptCodeView `
    -Source $desktopBootstrap `
    -Description "Desktop bootstrap module"
$bootstrap = Get-UniqueBracedItem `
    -Source $desktopBootstrap `
    -SignaturePattern '(?m)^export[ \t]+async[ \t]+function[ \t]+bootstrap[ \t]*\(' `
    -Description "Desktop bootstrap" `
    -TopLevel `
    -CodeView $desktopBootstrapCodeView
if ($null -ne $bootstrap) {
    $bootstrapCodeView = [pscustomobject]@{ Code = $bootstrap.CodeBody }
    $bootstrapStatements = @(Get-TypeScriptDirectStatements `
        -Source $bootstrap.Body `
        -Description "Desktop bootstrap body" `
        -CodeView $bootstrapCodeView)
    $systemLocaleCalls = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch(
                $_.Source,
                '(?s)^const\s+systemLocale\s*=\s*await\s+invoke\s*<\s*string\s*\|\s*null\s*>\s*\(\s*"system_locale"\s*\)\s*\.\s*catch\s*\(\s*\(\s*\)\s*=>\s*undefined\s*,?\s*\)\s*;$'
            )
        })
    $localeResolutionCalls = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch(
                $_.Source,
                '(?s)^const\s+locale\s*=\s*resolveSupportedLocale\s*\(\s*systemLocale\s*,\s*browserLocaleCandidates\s*\(\s*window\s*\.\s*navigator\s*\)\s*,?\s*\)\s*;$'
            )
        })
    $initializeCalls = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch(
                $_.Source,
                '^await\s+initializeI18n\s*\(\s*locale\s*\)\s*;$'
            )
        })
    $documentLocaleCalls = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch(
                $_.Source,
                '^initializeDocumentLocale\s*\(\s*document\s*\.\s*documentElement\s*,\s*locale\s*\)\s*;$'
            )
        })
    $renderCalls = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch(
                $_.Source,
                '(?s)^createRoot\s*\(\s*root\s*\)\s*\.\s*render\s*\(.*\)\s*;$'
            )
        })
    $unconditionalTerminators = @($bootstrapStatements | Where-Object {
            [regex]::IsMatch($_.Code, '^(?:return|throw)\b') -or
            [regex]::IsMatch(
                $_.Code,
                '(?s)^if\s*\(\s*true\s*\)\s*\{\s*(?:return(?:\s+[^;]*)?|throw\s+[^;]+)\s*;\s*\}\s*$'
            )
        })
    $requiredBootstrapStatementsPresent = (
        $systemLocaleCalls.Count -eq 1 -and
        $localeResolutionCalls.Count -eq 1 -and
        $initializeCalls.Count -eq 1 -and
        $documentLocaleCalls.Count -eq 1 -and
        $renderCalls.Count -eq 1
    )
    $bootstrapHasEarlyTerminator = $false
    if ($renderCalls.Count -eq 1) {
        $bootstrapHasEarlyTerminator = @(
            $unconditionalTerminators | Where-Object {
                $_.Index -lt $renderCalls[0].Index
            }
        ).Count -gt 0
    }
    if (-not $requiredBootstrapStatementsPresent -or $bootstrapHasEarlyTerminator) {
        Add-ContractFailure `
            -Message "Desktop bootstrap must retain reachable direct bootstrap statements for system locale resolution, i18n initialization, document locale initialization, and rendering."
    }
    if (
        $requiredBootstrapStatementsPresent -and
        (
            $systemLocaleCalls[0].Index -ge $localeResolutionCalls[0].Index -or
            $localeResolutionCalls[0].Index -ge $initializeCalls[0].Index -or
            $initializeCalls[0].Index -ge $documentLocaleCalls[0].Index -or
            $documentLocaleCalls[0].Index -ge $renderCalls[0].Index
        )
    ) {
        Add-ContractFailure `
            -Message "Desktop bootstrap must resolve the system locale and await i18n before desktop rendering."
    }
}

if ($null -ne $desktopBootstrapCodeView) {
    $desktopModuleStatements = @(Get-TypeScriptDirectStatements `
        -Source $desktopBootstrap `
        -Description "Desktop bootstrap module" `
        -CodeView $desktopBootstrapCodeView)
    $bootstrapInvocations = @($desktopModuleStatements | Where-Object {
            [regex]::IsMatch($_.Source, '^void\s+bootstrap\s*\(\s*\)\s*;$')
        })
    if ($bootstrapInvocations.Count -ne 1) {
        Add-ContractFailure `
            -Message "Desktop bootstrap module must invoke bootstrap at module scope exactly once."
    }
}

$desktopI18nCodeView = Get-TypeScriptCodeView `
    -Source $desktopI18n `
    -Description "Desktop i18n module"
$i18nInitializer = Get-UniqueBracedItem `
    -Source $desktopI18n `
    -SignaturePattern '(?m)^export[ \t]+async[ \t]+function[ \t]+initializeI18n[ \t]*\(' `
    -Description "Desktop i18n initializer" `
    -TopLevel `
    -CodeView $desktopI18nCodeView
if ($null -ne $i18nInitializer) {
    $i18nInitializerCodeView = [pscustomobject]@{
        Code = $i18nInitializer.CodeBody
    }
    $i18nStatements = @(Get-TypeScriptDirectStatements `
        -Source $i18nInitializer.Body `
        -Description "Desktop i18n initializer body" `
        -CodeView $i18nInitializerCodeView)
    $initStatements = @($i18nStatements | Where-Object {
            $_.Code -match '^await\s+i18n\s*\.\s*use\s*\(\s*initReactI18next\s*\)\s*\.\s*init\s*\('
        })
    $supportedLanguagesAreExact = $false
    if ($initStatements.Count -eq 1) {
        $initStatement = $initStatements[0]
        $initCall = [regex]::Match(
            $initStatement.Code,
            '^await\s+i18n\s*\.\s*use\s*\(\s*initReactI18next\s*\)\s*\.\s*init\s*\('
        )
        $optionsOpeningBrace = $initCall.Index + $initCall.Length
        while (
            $optionsOpeningBrace -lt $initStatement.Code.Length -and
            [char]::IsWhiteSpace($initStatement.Code[$optionsOpeningBrace])
        ) {
            $optionsOpeningBrace += 1
        }
        if (
            $optionsOpeningBrace -lt $initStatement.Code.Length -and
            $initStatement.Code[$optionsOpeningBrace] -eq "{"
        ) {
            $depth = 0
            $optionsClosingBrace = -1
            for (
                $index = $optionsOpeningBrace;
                $index -lt $initStatement.Code.Length;
                $index += 1
            ) {
                if ($initStatement.Code[$index] -eq "{") {
                    $depth += 1
                }
                elseif ($initStatement.Code[$index] -eq "}") {
                    $depth -= 1
                    if ($depth -eq 0) {
                        $optionsClosingBrace = $index
                        break
                    }
                }
            }
            if ($optionsClosingBrace -gt $optionsOpeningBrace) {
                $optionsBodyStart = $optionsOpeningBrace + 1
                $optionsBodyLength = $optionsClosingBrace - $optionsBodyStart
                $optionsCodeBody = $initStatement.Code.Substring(
                    $optionsBodyStart,
                    $optionsBodyLength
                )
                $optionsSourceBody = $initStatement.Source.Substring(
                    $optionsBodyStart,
                    $optionsBodyLength
                )
                $supportedLanguageProperties = @()
                foreach ($candidate in @([regex]::Matches(
                            $optionsCodeBody,
                            '(?<![A-Za-z0-9_$])supportedLngs\s*:'
                        ))) {
                    $ownership = Get-RustOwnershipAtIndex `
                        -Structure $optionsCodeBody `
                        -Index $candidate.Index
                    if ($ownership.AllDelimiterDepthsZero) {
                        $supportedLanguageProperties += $candidate
                    }
                }
                $exactSupportedLanguageProperties = @(
                    $supportedLanguageProperties | Where-Object {
                        $sourceTail = $optionsSourceBody.Substring($_.Index)
                        [regex]::IsMatch(
                            $sourceTail,
                            '^supportedLngs\s*:\s*\[\s*"en"\s*,\s*"zh-CN"\s*\]\s*(?=,|$)'
                        )
                    }
                )
                $topLevelSpreadProperties = @()
                foreach ($candidate in @([regex]::Matches(
                            $optionsCodeBody,
                            '\.\.\.'
                        ))) {
                    $ownership = Get-RustOwnershipAtIndex `
                        -Structure $optionsCodeBody `
                        -Index $candidate.Index
                    if ($ownership.AllDelimiterDepthsZero) {
                        $topLevelSpreadProperties += $candidate
                    }
                }
                $supportedLanguagesAreExact = (
                    $supportedLanguageProperties.Count -eq 1 -and
                    $exactSupportedLanguageProperties.Count -eq 1 -and
                    $topLevelSpreadProperties.Count -eq 0
                )
            }
        }
    }
    if (-not $supportedLanguagesAreExact) {
        Add-ContractFailure `
            -Message 'Desktop i18n must bind supportedLngs: ["en", "zh-CN"] exactly to the awaited i18n.init options.'
    }
}

$desktopMainCodeView = Get-RustCodeView `
    -Source $desktopMain `
    -Description "Desktop Rust entry point"
if ($null -ne $desktopMainCodeView) {
    $subsystemAttributes = @([regex]::Matches(
            $desktopMainCodeView.Code,
            '(?m)^#!\[cfg_attr\([ \t]*all\([ \t]*windows[ \t]*,[ \t]*not\([ \t]*debug_assertions[ \t]*\)[ \t]*\)[ \t]*,[ \t]*windows_subsystem[ \t]*=[ \t]+\)[ \t]*\][ \t]*$'
        ))
    $exactSubsystemAttributes = @(
        $subsystemAttributes | Where-Object {
            ($desktopMain.Substring($_.Index, $_.Length) -replace '\s', '') -ceq
            '#![cfg_attr(all(windows,not(debug_assertions)),windows_subsystem="windows")]'
        }
    )
    if ($exactSubsystemAttributes.Count -ne 1) {
        Add-ContractFailure `
            -Message 'Desktop Rust entry point must retain windows_subsystem = "windows" for non-debug Windows builds.'
    }
}

$windowsPackagerAst = $null
try {
    $windowsPackagerAst = Get-PowerShellAst `
        -Source $windowsPackager `
        -Description "Windows packager"
}
catch {
    Add-ContractFailure `
        -Message "Windows packager GUI subsystem checks are invalid: $($_.Exception.Message)"
}
if ($null -ne $windowsPackagerAst) {
    $sourceDesktopGuards = @(
        Get-ExactPowerShellGuardAst `
            -Ast $windowsPackagerAst `
            -Condition '(Get-PeSubsystem -Path $desktop) -ne 2' `
            -ThrowStatement 'throw "Windows desktop executable must use the GUI subsystem."'
    )
    $sourceDesktopGuardIsInvalid = (
        $sourceDesktopGuards.Count -ne 1 -or
        -not [object]::ReferenceEquals(
            $sourceDesktopGuards[0].Parent,
            $windowsPackagerAst.EndBlock
        )
    )
    if ($sourceDesktopGuardIsInvalid) {
        Add-ContractFailure `
            -Message "Windows packager must retain the active script-scope source desktop GUI subsystem check."
    }
    else {
        $guardStatementIndex = [array]::IndexOf(
            @($windowsPackagerAst.EndBlock.Statements),
            $sourceDesktopGuards[0]
        )
        $terminalStatementsBeforeGuard = @(
            $windowsPackagerAst.EndBlock.Statements |
                Select-Object -First $guardStatementIndex |
                Where-Object {
                    $_ -is [System.Management.Automation.Language.ReturnStatementAst] -or
                    $_ -is [System.Management.Automation.Language.ExitStatementAst] -or
                    $_ -is [System.Management.Automation.Language.ThrowStatementAst]
                }
        )
        if (
            $guardStatementIndex -lt 0 -or
            $terminalStatementsBeforeGuard.Count -gt 0
        ) {
            Add-ContractFailure `
                -Message "Windows packager must retain a reachable script-scope source desktop GUI subsystem check."
        }
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
            "pnpm --dir apps/desktop i18n:check",
            "pnpm --dir apps/desktop typecheck",
            "pnpm --dir apps/desktop test:unit",
            "pnpm --dir apps/desktop build"
        )) {
        Assert-JobRunStep -JobName "frontend" -Steps $frontendSteps -Command $command
    }

    $installIndex = -1
    $catalogIndex = -1
    $testIndex = -1
    for ($index = 0; $index -lt $frontendSteps.Count; $index += 1) {
        if (-not $frontendSteps[$index].Fields.ContainsKey("run")) {
            continue
        }
        switch ($frontendSteps[$index].Fields["run"]) {
            "pnpm --dir apps/desktop install --frozen-lockfile" {
                $installIndex = $index
            }
            "pnpm --dir apps/desktop i18n:check" {
                $catalogIndex = $index
                if (
                    -not $frontendSteps[$index].Fields.ContainsKey("name") -or
                    $frontendSteps[$index].Fields["name"] -cne
                    "Check desktop translation catalogs"
                ) {
                    Add-ContractFailure `
                        -Message "Frontend catalog check step must use the documented name."
                }
            }
            "pnpm --dir apps/desktop test:unit" {
                $testIndex = $index
            }
        }
    }
    if (
        $installIndex -ge 0 -and
        $catalogIndex -ge 0 -and
        $testIndex -ge 0 -and
        (
            $installIndex -ge $catalogIndex -or
            $catalogIndex -ge $testIndex
        )
    ) {
        Add-ContractFailure `
            -Message "Frontend catalog check must run after frozen install and before tests."
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

$lifecycleEvidenceFragments = @(
    "## WokCore lifecycle acceptance evidence",
    "does not currently provide a command that drives a live signed",
    "pnpm.cmd --dir apps/desktop exec vitest run src/components/CoreLifecycle.test.tsx",
    "tests/scripts/run-fixed-test-host.ps1",
    "1. **Missing to running without a click.**",
    "missing_production_runtime_installs_starts_authorizes_and_reports_structured_progress",
    "signed_release_reports_monotonic_download_and_authoritative_install_phases",
    "2. **Signed update cancel and confirm.**",
    "accessible confirmation and invokes the expected version once",
    "system_runner_uses_only_the_three_fixed_child_commands",
    "3. **Active requests remain.**",
    "returns management after",
    "versions_bytes_and_active_requests_are_strictly_validated",
    "4. **Verification failure and rollback.**",
    "artifact_hash_mismatch_leaves_no_install_or_record",
    "invalid_manifest_signature_is_rejected_before_artifact_download",
    "5. **Close and reopen during an operation.**",
    "duplicate_installs_coalesce_conflicts_fail_and_terminal_allows_retry",
    "subscribes before recovering a running snapshot and",
    "6. **IDE Development performs zero update work.**",
    "development_suppresses_every_install_and_update_path_before_authority_or_runner",
    "a_selected_development_session_never_switches_to_production",
    "7. **Chinese and English UI.**",
    "pnpm.cmd --dir apps/desktop exec vitest run src/locale.test.ts",
    "pwsh tests/scripts/check-foundation-contract.tests.ps1"
)
$lifecycleEvidenceComplete = $true
foreach ($lifecycleEvidenceFragment in $lifecycleEvidenceFragments) {
    if (-not $development.Contains($lifecycleEvidenceFragment)) {
        $lifecycleEvidenceComplete = $false
        break
    }
}
if (-not $lifecycleEvidenceComplete) {
    Add-ContractFailure `
        -Message "Development docs must retain reproducible lifecycle acceptance evidence for all seven paths and disclose missing live GUI harnesses."
}
$lifecycleAcceptanceFixtures = @(
    @{
        Kind = "TypeScript"
        Source = $coreLifecycleTests
        Names = @(
            "starts one production install in StrictMode and restores normal content after success",
            "requires an accessible confirmation and invokes the expected version once",
            "returns management after active requests defer the update and reconfirms retry",
            "subscribes before recovering a running snapshot and unmounts only the listener",
            "treats install_in_progress as another process and polls trusted status without retrying",
            "never checks or installs updates for a development %s runtime",
            "never starts production installation for a development %s status"
        )
    },
    @{
        Kind = "Rust"
        Container = "CfgTestModule"
        Source = $coreOperation
        Names = @(
            "system_runner_uses_only_the_three_fixed_child_commands",
            "duplicate_installs_coalesce_conflicts_fail_and_terminal_allows_retry",
            "development_suppresses_every_install_and_update_path_before_authority_or_runner"
        )
    },
    @{
        Kind = "Rust"
        Container = "CfgTestModule"
        Source = $coreOperationParser
        Names = @(
            "versions_bytes_and_active_requests_are_strictly_validated",
            "update_active_requests_are_valid_during_rolling_back"
        )
    },
    @{
        Kind = "Rust"
        Container = "TopLevel"
        RequireTestSupportFeatureCfg = $true
        Source = $wokcoreInstallTests
        Names = @(
            "signed_release_reports_monotonic_download_and_authoritative_install_phases",
            "artifact_hash_mismatch_leaves_no_install_or_record",
            "invalid_manifest_signature_is_rejected_before_artifact_download"
        )
    },
    @{
        Kind = "Rust"
        Container = "TopLevel"
        Source = $cliStartTests
        Names = @(
            "missing_production_runtime_installs_starts_authorizes_and_reports_structured_progress"
        )
    },
    @{
        Kind = "Rust"
        Container = "TopLevel"
        Source = $runtimeSelectorTests
        Names = @(
            "a_selected_development_session_never_switches_to_production"
        )
    },
    @{
        Kind = "TypeScript"
        Source = $localeTests
        Names = @(
            "initializes the document from the selected %s catalog"
        )
    }
)
$lifecycleAcceptanceFixturesExist = $true
foreach ($fixtureGroup in $lifecycleAcceptanceFixtures) {
    $fixtureCodeView = Get-RustCodeView `
        -Source $fixtureGroup.Source `
        -Description "$($fixtureGroup.Kind) lifecycle acceptance test source"
    if ($null -eq $fixtureCodeView) {
        $lifecycleAcceptanceFixturesExist = $false
        break
    }
    $fixtureContainer = if ($fixtureGroup.Kind -eq "Rust") {
        Get-RustExecutableTestContainer `
            -Source $fixtureGroup.Source `
            -Kind $fixtureGroup.Container `
            -CodeView $fixtureCodeView `
            -RequireTestSupportFeatureCfg (
                [bool]$fixtureGroup.RequireTestSupportFeatureCfg
            )
    }
    else {
        $null
    }
    if (
        $fixtureGroup.Kind -eq "Rust" -and
        $null -eq $fixtureContainer
    ) {
        $lifecycleAcceptanceFixturesExist = $false
        break
    }
    foreach ($fixtureName in $fixtureGroup.Names) {
        $fixtureExists = if ($fixtureGroup.Kind -eq "TypeScript") {
            Test-TypeScriptExecutableTestDescription `
                -Source $fixtureGroup.Source `
                -TestDescription $fixtureName `
                -CodeView $fixtureCodeView
        }
        else {
            Test-RustExecutableTestFunction `
                -Source $fixtureGroup.Source `
                -FunctionName $fixtureName `
                -CodeView $fixtureCodeView `
                -Container $fixtureContainer
        }
        if (-not $fixtureExists) {
            $lifecycleAcceptanceFixturesExist = $false
            break
        }
    }
    if (-not $lifecycleAcceptanceFixturesExist) {
        break
    }
}
if (-not $lifecycleAcceptanceFixturesExist) {
    Add-ContractFailure `
        -Message "Every documented lifecycle acceptance fixture must remain present in an executable test source."
}

$expectedWokCorePublicKey = @"
untrusted comment: minisign public key 7EF262CD8E9FE136
RWQ24Z+OzWLyfjz0X7JFepiizNYEsUBt/cJisQWQ9o9EAK8TURVs9hts
"@
if (
    $wokcorePublicKey.TrimEnd("`r", "`n") -ne
    $expectedWokCorePublicKey.TrimEnd("`r", "`n")
) {
    Add-ContractFailure `
        -Message "The production Minisign public key must retain key id 7EF262CD8E9FE136 and its exact validated payload."
}

$secretHeaderPattern = (
    '(?im)^[ \t]*untrusted comment:[ \t]*' +
    'minisign[ \t]+(?:encrypted[ \t]+)?secret[ \t]+key\b'
)
$generatedDirectoryNames = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)
foreach (
    $generatedDirectoryName in @(
        ".git",
        ".next",
        "build",
        "coverage",
        "dist",
        "gen",
        "node_modules",
        "target"
    )
) {
    $null = $generatedDirectoryNames.Add($generatedDirectoryName)
}
$secretHeaderFound = $false
foreach ($sourceRootName in @("apps", "crates")) {
    $sourceRoot = Join-Path $rootPath $sourceRootName
    if (-not (Test-Path -LiteralPath $sourceRoot)) {
        continue
    }
    $sourceRootPrefix = $sourceRoot.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    foreach ($sourceFile in Get-ChildItem -LiteralPath $sourceRoot -Recurse -File -Force) {
        $relativePath = $sourceFile.FullName.Substring($sourceRootPrefix.Length)
        $relativeSegments = @($relativePath -split '[\\/]')
        $generated = $false
        for (
            $segmentIndex = 0;
            $segmentIndex -lt $relativeSegments.Count - 1;
            $segmentIndex += 1
        ) {
            if ($generatedDirectoryNames.Contains($relativeSegments[$segmentIndex])) {
                $generated = $true
                break
            }
        }
        if ($generated) {
            continue
        }

        $buffer = New-Object byte[] (64 * 1024)
        $stream = [System.IO.File]::Open(
            $sourceFile.FullName,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::ReadWrite
        )
        try {
            $bytesRead = $stream.Read($buffer, 0, $buffer.Length)
        }
        finally {
            $stream.Dispose()
        }
        $sourceText = [System.Text.Encoding]::UTF8.GetString(
            $buffer,
            0,
            $bytesRead
        ).TrimStart([char]0xFEFF)
        if ($sourceText -match $secretHeaderPattern) {
            $secretHeaderFound = $true
            break
        }
    }
    if ($secretHeaderFound) {
        break
    }
}
if ($secretHeaderFound) {
    Add-ContractFailure `
        -Message "Product source must not contain a Minisign private or encrypted secret key header."
}

$coreOperationCodeView = Get-RustCodeView `
    -Source $coreOperation `
    -Description "Core operation coordinator source"
$desktopLibCodeView = Get-RustCodeView `
    -Source $desktopLib `
    -Description "Desktop Tauri library source"

$installSpec = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*fn[ \t]+install[ \t]*\([ \t]*program[ \t]*:[ \t]*PathBuf[ \t]*\)[ \t]*->[^{]+' `
    -Description "Core operation structured install command" `
    -CodeView $coreOperationCodeView
if (
    $null -ne $installSpec -and
    ($installSpec.Body -replace '\s', '') -ne
    'Self::raw(program,["start","--json","--progress-jsonl"])'
) {
    Add-ContractFailure `
        -Message "Core operation structured WokRouter start arguments must remain exactly start --json --progress-jsonl."
}

$updateInstallSpec = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*fn[ \t]+update_install[ \t]*\([ \t]*program[ \t]*:[ \t]*PathBuf[ \t]*\)[ \t]*->[^{]+' `
    -Description "Core operation structured update-install command" `
    -CodeView $coreOperationCodeView
if (
    $null -ne $updateInstallSpec -and
    ($updateInstallSpec.Body -replace '\s', '') -ne
    'Self::raw(program,["update","--install","--json","--progress-jsonl"],)'
) {
    Add-ContractFailure `
        -Message "Core operation structured WokCore update-install arguments must remain exactly update --install --json --progress-jsonl."
}

$systemOperationRunner = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^impl[ \t]+OperationRunner[ \t]+for[ \t]+SystemOperationRunner[ \t]*' `
    -Description "System operation runner implementation" `
    -TopLevel `
    -CodeView $coreOperationCodeView
$systemOperationRun = if ($null -ne $systemOperationRunner) {
    Get-UniqueBracedItem `
        -Source $systemOperationRunner.Body `
        -SignaturePattern '(?m)^[ \t]*fn[ \t]+run[ \t]*\(' `
        -Description "System operation runner run method" `
        -TopLevel
}
else {
    $null
}
$systemOperationRequestMatch = if ($null -ne $systemOperationRun) {
    Get-UniqueBracedItem `
        -Source $systemOperationRun.Body `
        -SignaturePattern '(?m)^[ \t]*let[ \t]+\([ \t]*operation[ \t]*,[ \t]*spec[ \t]*\)[ \t]*=[ \t]*match[ \t]+request[ \t]*' `
        -Description "System operation runner request dispatch"
}
else {
    $null
}
if ($null -ne $systemOperationRequestMatch) {
    $systemOperationRequestCode = (
        Get-RustCodeView `
            -Source $systemOperationRequestMatch.Body `
            -Description "System operation runner request dispatch body"
    ).Code
    $systemInstallWiring = [regex]::Matches(
        $systemOperationRequestCode,
        '(?ms)OperationRequest::Install[ \t\r\n]*=>[ \t\r\n]*\([ \t\r\n]*CoreOperationKind::Install[ \t]*,[ \t\r\n]*CommandSpec::install\([ \t\r\n]*bundled_wokrouter_executable\(\)\?[ \t\r\n]*\)[ \t]*,[ \t\r\n]*\)[ \t]*,'
    )
    if ($systemInstallWiring.Count -ne 1) {
        Add-ContractFailure `
            -Message "System operation runner install wiring must dispatch Install through CommandSpec::install."
    }
    $systemUpdateWiring = [regex]::Matches(
        $systemOperationRequestCode,
        '(?ms)OperationRequest::Update[ \t\r\n]*\{[ \t\r\n]*executable[ \t\r\n]*\}[ \t\r\n]*=>[ \t\r\n]*\([ \t\r\n]*CoreOperationKind::Update[ \t]*,[ \t\r\n]*CommandSpec::update_install\([ \t\r\n]*executable[ \t\r\n]*\)[ \t]*,[ \t\r\n]*\)[ \t]*,'
    )
    if ($systemUpdateWiring.Count -ne 1) {
        Add-ContractFailure `
            -Message "System operation runner update wiring must dispatch Update through CommandSpec::update_install."
    }
}

$spawnChild = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^fn[ \t]+spawn_child[ \t]*\(' `
    -Description "Core operation child spawn function" `
    -TopLevel `
    -CodeView $coreOperationCodeView
if ($null -ne $spawnChild) {
    $null = Get-UniqueDirectStatementIndex `
        -Source $spawnChild.Body `
        -Pattern '(?m)^[ \t]*#\[cfg\(windows\)\][ \t]*\r?\n[ \t]*command\.creation_flags\(policy\.creation_flags\)[ \t]*;' `
        -Description "Core operation child spawn must directly apply CREATE_NO_WINDOW through the Windows policy"
    if (
        $spawnChild.CodeBody -notmatch
        '\.kill_on_drop\([ \t\r\n]*policy\.kill_on_drop[ \t\r\n]*\)'
    ) {
        Add-ContractFailure `
            -Message "Core operation child spawn must apply the detached kill-on-drop policy."
    }
}

$childPolicy = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^fn[ \t]+child_process_policy[ \t]*\(' `
    -Description "Core operation long-child policy" `
    -TopLevel `
    -CodeView $coreOperationCodeView
$childPolicyBody = if ($null -ne $childPolicy) {
    $coreOperationCodeView.CommentStripped.Substring(
        $childPolicy.OpeningBraceIndex + 1,
        $childPolicy.ClosingBraceIndex - $childPolicy.OpeningBraceIndex - 1
    )
}
else {
    ""
}
if (
    $null -ne $childPolicy -and
    $childPolicyBody -notmatch
    '(?ms)ChildProcessPolicy[ \t\r\n]*\{[ \t\r\n]*kill_on_drop:[ \t]*false,[ \t\r\n]*#\[cfg\(windows\)\][ \t\r\n]*creation_flags:[ \t]*0x0800_0000,[ \t\r\n]*\}'
) {
    Add-ContractFailure `
        -Message "Core operation long-child CREATE_NO_WINDOW policy must remain 0x08000000 with kill_on_drop false."
}

if (
    $null -ne $coreOperationCodeView -and
    $coreOperationCodeView.Code -match
    '\.kill_on_drop\([ \t\r\n]*true[ \t\r\n]*\)'
) {
    Add-ContractFailure `
        -Message "Core operation coordinator must reject kill_on_drop(true) for transactional children."
}

$operationEventSink = Get-UniqueBracedItem `
    -Source $desktopLib `
    -SignaturePattern '(?m)^impl[ \t]+OperationEventSink[ \t]+for[ \t]+TauriOperationEventSink[ \t]*' `
    -Description "Desktop operation event sink implementation" `
    -TopLevel `
    -CodeView $desktopLibCodeView
$operationEventEmit = if ($null -ne $operationEventSink) {
    Get-UniqueBracedItem `
        -Source $operationEventSink.Body `
        -SignaturePattern "(?m)^[ \t]*fn[ \t]+emit[ \t]*<'a>[ \t]*\(" `
        -Description "Desktop operation event sink emit method" `
        -TopLevel
}
else {
    $null
}
$operationEventEmitBody = if ($null -ne $operationEventEmit) {
    (
        Get-RustCodeView `
            -Source $operationEventEmit.Body `
            -Description "Desktop operation event sink emit body"
    ).CommentStripped -replace '\s', ''
}
else {
    ""
}
if (
    $null -ne $operationEventEmit -and
    $operationEventEmitBody -cne
    'Box::pin(asyncmove{let_=self.app.emit("core-operation-progress",snapshot);})'
) {
    Add-ContractFailure `
        -Message "Desktop operation sink must emit exactly one core-operation-progress event."
}

$installAndStartCoreFor = Get-UniqueBracedItem `
    -Source $desktopLib `
    -SignaturePattern '(?m)^async[ \t]+fn[ \t]+install_and_start_core_for[ \t]*\(' `
    -Description "Desktop install command event wiring" `
    -TopLevel `
    -CodeView $desktopLibCodeView
$installAndStartCoreForBody = if ($null -ne $installAndStartCoreFor) {
    (
        Get-RustCodeView `
            -Source $installAndStartCoreFor.Body `
            -Description "Desktop install command event wiring body"
    ).CommentStripped -replace '\s', ''
}
else {
    ""
}
if (
    $null -ne $installAndStartCoreFor -and
    $installAndStartCoreForBody -cne
    'state.core_operations.install_and_start(Arc::new(TauriOperationEventSink{app})).await.map_err(|error|error.to_string())'
) {
    Add-ContractFailure `
        -Message "Desktop install command Tauri operation event sink wiring must pass TauriOperationEventSink { app }."
}

$installCoreUpdateFor = Get-UniqueBracedItem `
    -Source $desktopLib `
    -SignaturePattern '(?m)^async[ \t]+fn[ \t]+install_core_update_for[ \t]*\(' `
    -Description "Desktop update command event wiring" `
    -TopLevel `
    -CodeView $desktopLibCodeView
$installCoreUpdateForBody = if ($null -ne $installCoreUpdateFor) {
    (
        Get-RustCodeView `
            -Source $installCoreUpdateFor.Body `
            -Description "Desktop update command event wiring body"
    ).CommentStripped -replace '\s', ''
}
else {
    ""
}
if (
    $null -ne $installCoreUpdateFor -and
    $installCoreUpdateForBody -cne
    'state.core_operations.install_update(&expected_version,Arc::new(TauriOperationEventSink{app})).await.map_err(|error|error.to_string())'
) {
    Add-ContractFailure `
        -Message "Desktop update command Tauri operation event sink wiring must pass TauriOperationEventSink { app }."
}

$coreOperationError = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^pub\(crate\)[ \t]+enum[ \t]+CoreOperationError[ \t]*' `
    -Description "CoreOperationError enum" `
    -TopLevel `
    -CodeView $coreOperationCodeView
$coreOperationErrorBody = if ($null -ne $coreOperationError) {
    $coreOperationCodeView.CommentStripped.Substring(
        $coreOperationError.OpeningBraceIndex + 1,
        $coreOperationError.ClosingBraceIndex -
            $coreOperationError.OpeningBraceIndex -
            1
    )
}
else {
    ""
}
if ($null -ne $coreOperationError) {
    $operationConflictMatches = [regex]::Matches(
        $coreOperationErrorBody,
        '(?m)^[ \t]*#\[error\("operation_in_progress"\)\][ \t]*\r?\n[ \t]*OperationInProgress[ \t]*,[ \t]*$'
    )
    if ($operationConflictMatches.Count -ne 1) {
        Add-ContractFailure `
            -Message "CoreOperationError must retain the exact operation_in_progress conflict code."
    }
    $developmentManagedMatches = [regex]::Matches(
        $coreOperationErrorBody,
        '(?m)^[ \t]*#\[error\("development_runtime_managed_by_ide"\)\][ \t]*\r?\n[ \t]*DevelopmentRuntimeManagedByIde[ \t]*,[ \t]*$'
    )
    if ($developmentManagedMatches.Count -ne 1) {
        Add-ContractFailure `
            -Message "CoreOperationError must retain development_runtime_managed_by_ide for the backend update gate."
    }
}

$trustedProductionExecutable = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*async[ \t]+fn[ \t]+trusted_production_executable[ \t]*\(' `
    -Description "Core operation trusted production executable gate" `
    -CodeView $coreOperationCodeView
if (
    $null -ne $trustedProductionExecutable -and
    $trustedProductionExecutable.CodeBody -notmatch
    '(?ms)if[ \t\r\n]+runtime\.channel\(\)[ \t\r\n]*==[ \t\r\n]*WokCoreRuntimeChannel::Development[ \t\r\n]*\{[ \t\r\n]*return[ \t\r\n]+Err\(CoreOperationError::DevelopmentRuntimeManagedByIde\)[ \t]*;[ \t\r\n]*\}'
) {
    Add-ContractFailure `
        -Message "Backend development update gate must reject Development before trusted executable reuse or discovery."
}
if ($null -ne $trustedProductionExecutable) {
    $trustedDevelopmentGate = Get-UniqueBracedItem `
        -Source $trustedProductionExecutable.Body `
        -SignaturePattern '(?m)^[ \t]*if[ \t]+runtime\.channel\(\)[ \t]*==[ \t]*WokCoreRuntimeChannel::Development[ \t]*' `
        -Description "Backend development update gate" `
        -DirectStatement
    $trustedExecutableReuse = Get-UniqueBracedItem `
        -Source $trustedProductionExecutable.Body `
        -SignaturePattern '(?m)^[ \t]*if[ \t]+let[ \t]+Some\(executable\)[ \t]*=[ \t]*runtime\.executable\(\)[ \t]*' `
        -Description "Backend trusted executable reuse" `
        -DirectStatement
    $trustedAuthorityDiscoveryIndex = Get-UniqueDirectStatementIndex `
        -Source $trustedProductionExecutable.Body `
        -Pattern '(?m)^[ \t]*self\.authority[ \t\r\n]*\.[ \t\r\n]*discover\(\)\?' `
        -Description "Backend trusted authority discovery"
    if ($null -ne $trustedDevelopmentGate) {
        $null = Get-UniqueDirectStatementIndex `
            -Source $trustedDevelopmentGate.Body `
            -Pattern '(?m)^[ \t]*return[ \t]+Err\(CoreOperationError::DevelopmentRuntimeManagedByIde\)[ \t]*;' `
            -Description "Backend development update gate return"
    }
    if (
        $null -ne $trustedDevelopmentGate -and
        $null -ne $trustedExecutableReuse -and
        (
            $trustedDevelopmentGate.ClosingBraceIndex -ge
            $trustedExecutableReuse.SignatureIndex -or
            (
                $trustedAuthorityDiscoveryIndex -ge 0 -and
                $trustedDevelopmentGate.ClosingBraceIndex -ge
                $trustedAuthorityDiscoveryIndex
            )
        )
    ) {
        Add-ContractFailure `
            -Message "Backend development update gate must dominate executable reuse and trusted discovery."
    }
}

$checkUpdate = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*pub\(crate\)[ \t]+async[ \t]+fn[ \t]+check_update[ \t]*\(' `
    -Description "Core operation update check" `
    -CodeView $coreOperationCodeView
if ($null -ne $checkUpdate) {
    $checkUpdateConflict = Get-UniqueBracedItem `
        -Source $checkUpdate.Body `
        -SignaturePattern '(?m)^[ \t]*if[ \t]+self\.state\.lock\(\)\.await\.active\.is_some\(\)[ \t]*' `
        -Description "check_update active-operation conflict branch" `
        -DirectStatement
    if ($null -ne $checkUpdateConflict) {
        $null = Get-UniqueDirectStatementIndex `
            -Source $checkUpdateConflict.Body `
            -Pattern '(?m)^[ \t]*return[ \t]+Err\(CoreOperationError::OperationInProgress\)[ \t]*;' `
            -Description "check_update operation_in_progress return"
    }
    $null = Get-UniqueDirectStatementIndex `
        -Source $checkUpdate.Body `
        -Pattern '(?m)^[ \t]*let[ \t]+executable[ \t]*=[ \t]*self\.trusted_production_executable\(\)\.await\?[ \t]*;' `
        -Description "check_update must obtain a production-gated trusted executable"
}

$installUpdate = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*pub\(crate\)[ \t]+async[ \t]+fn[ \t]+install_update[ \t]*\(' `
    -Description "Core operation update install" `
    -CodeView $coreOperationCodeView
if ($null -ne $installUpdate) {
    $developmentGateIndex = Get-UniqueDirectStatementIndex `
        -Source $installUpdate.Body `
        -Pattern '(?m)^[ \t]*self\.require_production_channel\(\)\.await\?[ \t]*;' `
        -Description "install_update must reject Development before validation or child work"
    $versionValidationIndex = Get-UniqueDirectStatementIndex `
        -Source $installUpdate.Body `
        -Pattern '(?m)^[ \t]*Version::parse\(expected_version\)\.map_err\(\|_\|[ \t]*CoreOperationError::InvalidProgress\)\?[ \t]*;' `
        -Description "install_update expected-version validation"
    if (
        $developmentGateIndex -ge 0 -and
        $versionValidationIndex -ge 0 -and
        $developmentGateIndex -ge $versionValidationIndex
    ) {
        Add-ContractFailure `
            -Message "install_update must reject Development before validation or child work."
    }
    $installUpdateConflict = Get-UniqueBracedItem `
        -Source $installUpdate.Body `
        -SignaturePattern '(?m)^[ \t]*if[ \t]+self\.state\.lock\(\)\.await\.active\.is_some\(\)[ \t]*' `
        -Description "install_update active-operation conflict branch" `
        -DirectStatement
    if ($null -ne $installUpdateConflict) {
        $null = Get-UniqueDirectStatementIndex `
            -Source $installUpdateConflict.Body `
            -Pattern '(?m)^[ \t]*return[ \t]+Err\(CoreOperationError::OperationInProgress\)[ \t]*;' `
            -Description "install_update operation_in_progress return"
    }
}

$startOperation = Get-UniqueBracedItem `
    -Source $coreOperation `
    -SignaturePattern '(?m)^[ \t]*async[ \t]+fn[ \t]+start_operation[ \t]*\(' `
    -Description "Core operation start arbitration" `
    -CodeView $coreOperationCodeView
if ($null -ne $startOperation) {
    $startOperationConflict = Get-UniqueBracedItem `
        -Source $startOperation.Body `
        -SignaturePattern '(?m)^[ \t]*if[ \t]+let[ \t]+Some\(active\)[ \t]*=[ \t]*&state\.active[ \t]*' `
        -Description "start_operation active-operation conflict branch"
    if ($null -ne $startOperationConflict) {
        $null = Get-UniqueDirectStatementIndex `
            -Source $startOperationConflict.Body `
            -Pattern '(?m)^[ \t]*return[ \t]+Err\(CoreOperationError::OperationInProgress\)[ \t]*;' `
            -Description "start_operation operation_in_progress return"
    }
}

$frontendEligibility = Get-UniqueBracedItem `
    -Source $coreUpdateEligibility `
    -SignaturePattern '(?m)^export[ \t]+function[ \t]+isCoreUpdateEligible[ \t]*\(' `
    -Description "Frontend core update eligibility" `
    -TopLevel
if (
    $null -ne $frontendEligibility -and
    ($frontendEligibility.Body -replace '\s', '') -ne
    'return(status?.runtime_channel==="production"&&eligibleUpdateStates.has(status.state));'
) {
    Add-ContractFailure `
        -Message "Frontend update eligibility must require the production runtime channel and an eligible state."
}

$coreLifecycleCodeView = Get-RustCodeView `
    -Source $coreLifecycle `
    -Description "Core lifecycle frontend source"
if ($null -ne $coreLifecycleCodeView) {
    $eligibilityCalls = [regex]::Matches(
        $coreLifecycleCodeView.Code,
        '(?<![A-Za-z0-9_])isCoreUpdateEligible[ \t\r\n]*\('
    )
    if ($eligibilityCalls.Count -ne 9) {
        Add-ContractFailure `
            -Message "Core lifecycle must retain all nine active shared update eligibility checks."
    }
    foreach ($requiredGate in @(
            @{
                Pattern = '(?s)operation[ \t]*!==[ \t]*undefined[ \t\r\n]*\|\|[ \t\r\n]*!isCoreUpdateEligible\([ \t]*status\.data[ \t]*\)'
                Message = "Automatic update check must use the shared eligibility gate."
            },
            @{
                Pattern = '(?s)activeUpdateCheckRequestId\.current[ \t]*!==[ \t]*undefined[ \t\r\n]*\|\|[ \t\r\n]*!latestBridgeReady\.current[ \t\r\n]*\|\|[ \t\r\n]*blocksUpdateInteraction\(latestOperation\.current\)[ \t\r\n]*\|\|[ \t\r\n]*!isCoreUpdateEligible\(latestStatus\.current\)'
                Message = "Manual update check must use the shared eligibility gate."
            },
            @{
                Pattern = '(?s)const[ \t]+requestUpdate[ \t]*=[ \t]*useCallback\([ \t\r\n]*\(trigger\?:[ \t]*HTMLButtonElement\)[ \t]*=>[ \t]*\{[ \t\r\n]*if[ \t]*\([ \t\r\n]*!latestBridgeReady\.current[ \t\r\n]*\|\|[ \t\r\n]*blocksUpdateInteraction\(latestOperation\.current\)[ \t\r\n]*\|\|[ \t\r\n]*!isCoreUpdateEligible\(latestStatus\.current\)[ \t\r\n]*\|\|[ \t\r\n]*updateCheck\?\.code[ \t]*!==[ \t]*'
                Message = "Update prompt must use the shared eligibility gate."
            },
            @{
                Pattern = '(?s)const[ \t]+confirmUpdate[ \t]*=[ \t]*useCallback\(\(\)[ \t]*=>[ \t]*\{[ \t\r\n]*const[ \t]+targetVersion[ \t]*=[ \t]*updateCheck\?\.targetVersion[ \t]*;[ \t\r\n]*if[ \t]*\([ \t\r\n]*updateRequested\.current[ \t\r\n]*\|\|[ \t\r\n]*!latestBridgeReady\.current[ \t\r\n]*\|\|[ \t\r\n]*blocksUpdateInteraction\(latestOperation\.current\)[ \t\r\n]*\|\|[ \t\r\n]*!isCoreUpdateEligible\(latestStatus\.current\)[ \t\r\n]*\|\|[ \t\r\n]*updateCheck\?\.code[ \t]*!==[ \t]*'
                Message = "Update confirmation must use the shared eligibility gate."
            }
        )) {
        if ($coreLifecycleCodeView.Code -notmatch $requiredGate.Pattern) {
            Add-ContractFailure -Message $requiredGate.Message
        }
    }

    $manualUpdateCheck = Get-UniqueBracedItem `
        -Source $coreLifecycle `
        -SignaturePattern '(?m)^[ \t]*\(openConfirmation:[ \t]*boolean,[ \t]*trigger\?:[ \t]*HTMLButtonElement\)[ \t]*=>[ \t]*' `
        -Description "Manual update check callback" `
        -CodeView $coreLifecycleCodeView
    if ($null -ne $manualUpdateCheck) {
        $manualUpdateGate = Get-UniqueBracedItem `
            -Source $manualUpdateCheck.Body `
            -SignaturePattern '(?m)^[ \t]*if[ \t]*\(' `
            -Description "Manual update eligibility gate" `
            -DirectStatement
        $manualUpdateSideEffectIndex = Get-UniqueDirectStatementIndex `
            -Source $manualUpdateCheck.Body `
            -Pattern '(?m)^[ \t]*void[ \t]+retryCoreUpdateCheck\(\)' `
            -Description "Manual retryCoreUpdateCheck side effect"
        $manualRetryCalls = [regex]::Matches(
            $manualUpdateCheck.CodeBody,
            '(?<![A-Za-z0-9_])retryCoreUpdateCheck[ \t\r\n]*\('
        )
        if (
            $null -eq $manualUpdateGate -or
            $manualUpdateSideEffectIndex -lt 0 -or
            $manualRetryCalls.Count -ne 1 -or
            $manualUpdateGate.ClosingBraceIndex -ge
            $manualUpdateSideEffectIndex
        ) {
            Add-ContractFailure `
                -Message "Manual update gate must dominate retryCoreUpdateCheck and every update-check side effect."
        }
    }

    if (
        $coreLifecycleCodeView.Code -notmatch
        '(?ms)^[ \t]*useEffect\(\(\)[ \t]*=>[ \t]*\{[ \t\r\n]*if[ \t]*\([ \t\r\n]*!bridgeReady[ \t\r\n]*\|\|[ \t\r\n]*startupCheckConsumed\.current'
    ) {
        Add-ContractFailure `
            -Message "Automatic update check must require bridgeReady before every side effect."
    }
    if (
        $coreLifecycleCodeView.Code -notmatch
        '(?ms)^[ \t]*useEffect\(\(\)[ \t]*=>[ \t]*\{[ \t\r\n]*if[ \t]*\([ \t\r\n]*!bridgeReady[ \t\r\n]*\|\|[ \t\r\n]*installRequested\.current'
    ) {
        Add-ContractFailure `
            -Message "Automatic install must require bridgeReady before every side effect."
    }

    $confirmUpdateStart = Get-UniquePatternIndex `
        -Source $coreLifecycleCodeView.Code `
        -Pattern '(?m)^[ \t]*const[ \t]+confirmUpdate[ \t]*=[ \t]*useCallback' `
        -Description "Update confirmation callback"
    $confirmationDialogStart = Get-UniquePatternIndex `
        -Source $coreLifecycleCodeView.Code `
        -Pattern '(?m)^[ \t]*const[ \t]+updateConfirmationDialog[ \t]*=' `
        -Description "Update confirmation dialog"
    if (
        $confirmUpdateStart -ge 0 -and
        $confirmationDialogStart -gt $confirmUpdateStart
    ) {
        $confirmUpdateLength = $confirmationDialogStart - $confirmUpdateStart
        $confirmUpdateCode = $coreLifecycleCodeView.Code.Substring(
            $confirmUpdateStart,
            $confirmUpdateLength
        )
        $confirmUpdateCommentStripped = (
            $coreLifecycleCodeView.CommentStripped.Substring(
                $confirmUpdateStart,
                $confirmUpdateLength
            )
        )
        $allInstallCoreUpdateCalls = [regex]::Matches(
            $coreLifecycleCodeView.Code,
            '(?<![A-Za-z0-9_])installCoreUpdate[ \t\r\n]*\('
        )
        $confirmationInstallCoreUpdateCalls = [regex]::Matches(
            $confirmUpdateCode,
            '(?<![A-Za-z0-9_])installCoreUpdate[ \t\r\n]*\('
        )
        if (
            $allInstallCoreUpdateCalls.Count -ne 1 -or
            $confirmationInstallCoreUpdateCalls.Count -ne 1
        ) {
            Add-ContractFailure `
                -Message "Core lifecycle must retain confirmation-only installCoreUpdate ownership."
        }
        $completeConfirmationGuard = (
            '(?s)const[ \t]+targetVersion[ \t]*=[ \t]*' +
            'updateCheck\?\.targetVersion[ \t]*;[ \t\r\n]*' +
            'if[ \t]*\([ \t\r\n]*' +
            'updateRequested\.current[ \t\r\n]*\|\|[ \t\r\n]*' +
            '!latestBridgeReady\.current[ \t\r\n]*\|\|[ \t\r\n]*' +
            'blocksUpdateInteraction\(latestOperation\.current\)[ \t\r\n]*\|\|[ \t\r\n]*' +
            '!isCoreUpdateEligible\(latestStatus\.current\)[ \t\r\n]*\|\|[ \t\r\n]*' +
            'updateCheck\?\.code[ \t]*!==[ \t]*"update_available"[ \t\r\n]*\|\|[ \t\r\n]*' +
            'targetVersion[ \t]*===[ \t]*undefined[ \t\r\n]*' +
            '\)[ \t\r\n]*\{[ \t\r\n]*return[ \t]*;[ \t\r\n]*\}' +
            '(?:(?!installCoreUpdate).)*' +
            'void[ \t]+installCoreUpdate\([ \t]*targetVersion[ \t]*\)'
        )
        if ($confirmUpdateCommentStripped -notmatch $completeConfirmationGuard) {
            Add-ContractFailure `
                -Message "installCoreUpdate must run only after the complete confirmation guard."
        }
    }
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

$selectOnceSignaturePattern = (
    '(?m)^[ \t]*async[ \t]+fn[ \t]+select_once\b'
)
$selectOnceMatches = @(Get-RustOwnedPatternMatches `
    -Source $runtimeSelector `
    -Pattern $selectOnceSignaturePattern `
    -Description "Top-level select_once functions")
$debugSelectOnce = $null
$releaseSelectOnce = $null
if ($selectOnceMatches.Count -ne 2) {
    Add-ContractFailure `
        -Message "WokCore runtime selector must contain exactly two top-level select_once functions; found $($selectOnceMatches.Count)."
}
else {
    for ($ordinal = 0; $ordinal -lt $selectOnceMatches.Count; $ordinal += 1) {
        $selectOnce = Get-UniqueBracedItem `
            -Source $runtimeSelector `
            -SignaturePattern $selectOnceSignaturePattern `
            -Description "Top-level select_once function $ordinal" `
            -TopLevel `
            -MatchOrdinal $ordinal
        if ($null -eq $selectOnce) {
            continue
        }
        $attributes = Get-RustOuterAttributesBeforeItem `
            -Source $runtimeSelector `
            -ItemStart $selectOnce.SignatureIndex `
            -Description "Top-level select_once selector attributes"
        if ($null -eq $attributes) {
            continue
        }

        $selectorCfgAttributes = @()
        foreach ($attribute in $attributes.Items) {
            if (
                $attribute.Code -match
                '(?<![A-Za-z0-9_])(?:cfg|cfg_attr)(?![A-Za-z0-9_])'
            ) {
                $selectorCfgAttributes += $attribute
            }
        }
        if ($selectorCfgAttributes.Count -ne 1) {
            Add-ContractFailure `
                -Message "Each select_once selector attributes set must contain exactly one cfg and no cfg_attr."
            continue
        }
        $normalizedCfg = $selectorCfgAttributes[0].Code -replace '\s', ''
        if ($normalizedCfg -eq '#[cfg(debug_assertions)]') {
            if ($null -ne $debugSelectOnce) {
                Add-ContractFailure `
                    -Message "Debug select_once selector attributes must identify exactly one function."
            }
            else {
                $debugSelectOnce = $selectOnce
            }
        }
        elseif ($normalizedCfg -eq '#[cfg(not(debug_assertions))]') {
            if ($null -ne $releaseSelectOnce) {
                Add-ContractFailure `
                    -Message "Release select_once selector attributes must identify exactly one function."
            }
            else {
                $releaseSelectOnce = $selectOnce
            }
        }
        else {
            Add-ContractFailure `
                -Message "select_once selector attributes must be exactly cfg(debug_assertions) or cfg(not(debug_assertions))."
        }
    }
}
if ($null -eq $debugSelectOnce) {
    Add-ContractFailure `
        -Message "Debug select_once selector attributes must identify exactly one function."
}
else {
    $null = Get-UniqueDirectStatementIndex `
        -Source $debugSelectOnce.Body `
        -Pattern '(?m)^[ \t]*let[ \t]+candidate[ \t]*=[ \t]*development::candidate_from_environment\(\)[ \t]*;' `
        -Description "Debug select_once development candidate call"
    $debugSelectOnceCandidateFlow = $debugSelectOnce.CodeBody -replace '\s', ''
    if (
        $debugSelectOnceCandidateFlow -ne
        'letcandidate=development::candidate_from_environment();select_with_dependencies(paths,candidate,&crate::system::process_executable_matches,&probe_connection,&discover_wokcore_executable,).await'
    ) {
        Add-ContractFailure `
            -Message "Debug select_once candidate flow must pass the environment candidate unchanged into select_with_dependencies."
    }
}
if ($null -eq $releaseSelectOnce) {
    Add-ContractFailure `
        -Message "Release select_once selector attributes must identify exactly one function."
}
else {
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
    $timeoutConstants = @(Get-RustOwnedPatternMatches `
        -Source $dependencySelector.Body `
        -Pattern '(?m)^[ \t]*const[ \t]+DEVELOPMENT_TIMEOUT[ \t]*:[ \t]*Duration[ \t]*=[ \t]*Duration::from_secs\([ \t]*5[ \t]*\)[ \t]*;' `
        -Description "select_with_dependencies five-second deadline constant" `
        -RequireStatementStart)
    if ($timeoutConstants.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies named constants must define the five-second deadline exactly once."
    }
    $retryConstants = @(Get-RustOwnedPatternMatches `
        -Source $dependencySelector.Body `
        -Pattern '(?m)^[ \t]*const[ \t]+DEVELOPMENT_RETRY_DELAY[ \t]*:[ \t]*Duration[ \t]*=[ \t]*Duration::from_millis\([ \t]*50[ \t]*\)[ \t]*;' `
        -Description "select_with_dependencies retry interval constant" `
        -RequireStatementStart)
    if ($retryConstants.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies named constants must define the 50-ms retry interval exactly once."
    }
    $deadlineReferences = @(Get-RustOwnedPatternMatches `
        -Source $dependencySelector.Body `
        -Pattern '(?m)^[ \t]*let[ \t]+deadline[ \t]*=[ \t]*Instant::now\(\)[ \t]*\+[ \t]*DEVELOPMENT_TIMEOUT[ \t]*;' `
        -Description "select_with_dependencies deadline constant use" `
        -RequireStatementStart)
    if ($deadlineReferences.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies constant references must use DEVELOPMENT_TIMEOUT for the deadline."
    }
    $candidateBindings = @(Get-RustOwnedPatternMatches `
        -Source $dependencySelector.Body `
        -Pattern '(?m)^[ \t]*let[ \t]+Some\(candidate\)[ \t]*=[ \t]*candidate[ \t]+else[ \t]*' `
        -Description "select_with_dependencies candidate binding" `
        -RequireStatementStart)
    if ($candidateBindings.Count -ne 1) {
        Add-ContractFailure `
            -Message "select_with_dependencies candidate must be bound on the active top-level path."
    }

    $selectionLoop = Get-UniqueBracedItem `
        -Source $dependencySelector.Body `
        -SignaturePattern '(?m)^[ \t]*loop[ \t]*' `
        -Description "select_with_dependencies selection loop" `
        -DirectStatement
    if ($null -ne $selectionLoop) {
        $null = Get-UniqueDirectStatementIndex `
            -Source $selectionLoop.Body `
            -Pattern '(?m)^[ \t]*tokio::time::sleep\([ \t]*DEVELOPMENT_RETRY_DELAY\.min\([ \t]*deadline[ \t]*-[ \t]*now[ \t]*\)[ \t]*\)\.await[ \t]*;' `
            -Description "select_with_dependencies constant references must use DEVELOPMENT_RETRY_DELAY for sleep"

        $selectionIf = Get-UniqueBracedItem `
            -Source $selectionLoop.Body `
            -SignaturePattern '(?ms)^[ \t]*if[ \t]+let[ \t]+Some\(process_id\)[ \t]*=[ \t]*client\.discovered_process_id\(\)[ \t\r\n]*&&[ \t\r\n]*process_matches\([ \t]*process_id[ \t]*,[ \t]*&candidate[ \t]*\)[ \t]*' `
            -Description "select_with_dependencies selection loop initial process identity check" `
            -DirectStatement
        if ($null -ne $selectionIf) {
            $boundClientIndex = Get-UniqueDirectStatementIndex `
                -Source $selectionIf.Body `
                -Pattern 'let[ \t]+bound[ \t]*=[ \t]*client\.bound_to_process\([ \t]*process_id[ \t]*\)[ \t]*;' `
                -Description "select_with_dependencies selection loop in order PID-bound client"
            $connectionIndex = Get-UniqueDirectStatementIndex `
                -Source $selectionIf.Body `
                -Pattern '(?s)let[ \t]+Ok\(connection\)[ \t\r\n]*=[ \t\r\n]*tokio::time::timeout_at\([ \t\r\n]*deadline[ \t\r\n]*,[ \t\r\n]*connection_probe\([ \t\r\n]*bound\.clone\(\)[ \t\r\n]*\)[ \t\r\n]*\)\.await' `
                -Description "select_with_dependencies selection loop in order deadline-bound connection probe"
            $secondIdentityIndex = Get-UniqueDirectStatementIndex `
                -Source $selectionIf.Body `
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
