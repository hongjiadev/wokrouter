[CmdletBinding()]
param(
    [string]$Root
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot "../.."
}

$forbiddenFieldPattern = "request_body|response_body|prompt|tool_arguments|authorization"
$persistentRustFiles = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
$migrationFiles = [System.Collections.Generic.List[System.IO.FileInfo]]::new()
$findings = [System.Collections.Generic.List[object]]::new()
$rootPath = (Resolve-Path -LiteralPath $Root).Path

$configModel = Join-Path $rootPath "crates/wokrouter-storage/src/config/model.rs"
if (Test-Path -LiteralPath $configModel -PathType Leaf) {
    $persistentRustFiles.Add((Get-Item -LiteralPath $configModel))
}

$stateRoot = Join-Path $rootPath "crates/wokrouter-storage/src/state"
if (Test-Path -LiteralPath $stateRoot -PathType Container) {
    foreach ($file in Get-ChildItem -LiteralPath $stateRoot -Recurse -File -Filter "*.rs") {
        $persistentRustFiles.Add($file)
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

if (($persistentRustFiles.Count + $migrationFiles.Count) -eq 0) {
    Write-Error "No persistence models or SQL migrations were found below '$rootPath'."
    exit 2
}

$rustStructPattern = [regex]::new(
    "^\s*(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+[A-Za-z_][A-Za-z0-9_]*(?:\s*<[^>{}]*>)?\s*(?:where[^{]+)?\{(?<body>.*?)^\s*\}",
    [System.Text.RegularExpressions.RegexOptions]::Multiline -bor
        [System.Text.RegularExpressions.RegexOptions]::Singleline
)
$rustFieldPattern = [regex]::new(
    "^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?<field>$forbiddenFieldPattern)\s*:",
    [System.Text.RegularExpressions.RegexOptions]::IgnoreCase -bor
        [System.Text.RegularExpressions.RegexOptions]::Multiline
)

foreach ($file in $persistentRustFiles) {
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    foreach ($structMatch in $rustStructPattern.Matches($content)) {
        $body = $structMatch.Groups["body"]
        foreach ($fieldMatch in $rustFieldPattern.Matches($body.Value)) {
            $absoluteIndex = $body.Index + $fieldMatch.Groups["field"].Index
            $line = [regex]::Matches($content.Substring(0, $absoluteIndex), "\r\n|\r|\n").Count + 1
            $findings.Add([pscustomobject]@{
                    Field = $fieldMatch.Groups["field"].Value
                    Path = $file.FullName
                    Line = $line
                })
        }
    }
}

$sqlIgnoredTextPattern = [regex]::new(
    "/\*.*?\*/|--[^\r\n]*|'(?:''|[^'])*'",
    [System.Text.RegularExpressions.RegexOptions]::Multiline -bor
        [System.Text.RegularExpressions.RegexOptions]::Singleline
)
$sqlFieldPattern = [regex]::new(
    "(?<![A-Za-z0-9_])(?<field>$forbiddenFieldPattern)(?![A-Za-z0-9_])",
    [System.Text.RegularExpressions.RegexOptions]::IgnoreCase
)

foreach ($file in $migrationFiles) {
    $content = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    $searchableContent = $sqlIgnoredTextPattern.Replace(
        $content,
        {
            param($match)
            return [regex]::Replace($match.Value, "[^\r\n]", " ")
        }
    )

    foreach ($fieldMatch in $sqlFieldPattern.Matches($searchableContent)) {
        $line = [regex]::Matches(
            $searchableContent.Substring(0, $fieldMatch.Groups["field"].Index),
            "\r\n|\r|\n"
        ).Count + 1
        $findings.Add([pscustomobject]@{
                Field = $fieldMatch.Groups["field"].Value
                Path = $file.FullName
                Line = $line
            })
    }
}

if ($findings.Count -gt 0) {
    foreach ($finding in $findings) {
        Write-Error (
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
    $persistentRustFiles.Count,
    $migrationFiles.Count
)
