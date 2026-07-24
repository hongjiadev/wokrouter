[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$scriptUnderTest = Join-Path $PSScriptRoot "check-no-body-persistence.ps1"
$shell = (Get-Process -Id $PID).Path
$forbiddenFields = @(
    "request_body",
    "response_body",
    "prompt",
    "tool_arguments",
    "authorization"
)
$fixtureRoots = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()
$scenarioCount = 0
$fixtureBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)

function New-PrivacyFixture {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("wokrouter-privacy-" + [guid]::NewGuid())
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/src/config") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/src/state") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-storage/migrations") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "crates/wokrouter-control/src") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $root "docs") -Force
    $fixtureRoots.Add($root)

    Set-Content `
        -LiteralPath (Join-Path $root "crates/wokrouter-storage/src/config/model.rs") `
        -Encoding UTF8 `
        -Value @'
pub struct AppConfig {
    pub port: u16,
}
pub struct VersionedConfig {
    pub revision: u64,
}
pub struct ServerConfig {
    pub host: String,
}
pub struct UiConfig {
    pub locale: String,
}
'@
    Set-Content `
        -LiteralPath (Join-Path $root "crates/wokrouter-storage/src/state/store.rs") `
        -Encoding UTF8 `
        -Value @'
pub struct RequestMetric {
    pub latency_ms: i64,
}
'@

    return $root
}

function Set-FixtureFile {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$RelativePath,

        [Parameter(Mandatory)]
        [string]$Content,

        [switch]$Exact
    )

    $normalizedRelativePath = $RelativePath.Replace("\", "/")
    if (
        -not $Exact -and
        $normalizedRelativePath -eq "crates/wokrouter-storage/src/config/model.rs"
    ) {
        foreach ($structDefinition in @(
                "pub struct VersionedConfig { pub revision: u64, }",
                "pub struct ServerConfig { pub host: String, }",
                "pub struct UiConfig { pub locale: String, }"
            )) {
            $structName = ($structDefinition -split "\s+")[2]
            if ($Content -notmatch "\bstruct\s+$([regex]::Escape($structName))\b") {
                $Content += "`n$structDefinition`n"
            }
        }
    }
    if (
        -not $Exact -and
        $normalizedRelativePath -eq "crates/wokrouter-storage/src/state/store.rs" -and
        $Content -notmatch "\bstruct\s+RequestMetric\b"
    ) {
        $Content += "`npub struct RequestMetric { pub latency_ms: i64, }`n"
    }

    $path = Join-Path $Root $RelativePath
    $parent = Split-Path -Parent $path
    $null = New-Item -ItemType Directory -Path $parent -Force
    Set-Content -LiteralPath $path -Value $Content -Encoding UTF8
}

function Convert-ToPlatformRelativePath {
    param(
        [Parameter(Mandatory)]
        [string]$RelativePath,

        [char]$DirectorySeparator = [System.IO.Path]::DirectorySeparatorChar
    )

    return $RelativePath.Replace("/", [string]$DirectorySeparator)
}

function Invoke-PrivacyCheck {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    $shellArguments = @("-NoProfile")
    if ($PSVersionTable.PSEdition -eq "Desktop") {
        $shellArguments += @("-ExecutionPolicy", "Bypass")
    }
    $shellArguments += @("-File", $scriptUnderTest, "-Root", $Root)

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & $shell @shellArguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    return @{
        ExitCode = $exitCode
        Output = ($output | Out-String)
    }
}

function Assert-CheckPasses {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-PrivacyCheck -Root $Root
    if ($result.ExitCode -ne 0) {
        throw "$Scenario should pass, but exited $($result.ExitCode): $($result.Output)"
    }
}

function Assert-CheckRejects {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$Field,

        [Parameter(Mandatory)]
        [string]$RelativePath,

        [Parameter(Mandatory)]
        [int]$Line,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-PrivacyCheck -Root $Root
    if ($result.ExitCode -ne 1) {
        throw "$Scenario should exit 1 for '$Field', but exited $($result.ExitCode): $($result.Output)"
    }
    if ($result.Output -notmatch [regex]::Escape($Field)) {
        throw "$Scenario rejected the fixture without identifying '$Field': $($result.Output)"
    }

    $expectedLocation = (Join-Path $Root $RelativePath) + ":$Line"
    if ($result.Output -notmatch [regex]::Escape($expectedLocation)) {
        throw "$Scenario did not report exact location '$expectedLocation': $($result.Output)"
    }
}

function Assert-InventoryFails {
    param(
        [Parameter(Mandatory)]
        [string]$Root,

        [Parameter(Mandatory)]
        [string]$ExpectedText,

        [Parameter(Mandatory)]
        [string]$Scenario
    )

    $result = Invoke-PrivacyCheck -Root $Root
    if ($result.ExitCode -ne 2) {
        throw "$Scenario should exit 2, but exited $($result.ExitCode): $($result.Output)"
    }
    if ($result.Output -notmatch "PERSISTENCE INVENTORY ERROR") {
        throw "$Scenario did not use the dedicated inventory error: $($result.Output)"
    }
    if ($result.Output -notmatch [regex]::Escape($ExpectedText)) {
        throw "$Scenario did not identify '$ExpectedText': $($result.Output)"
    }
}

function Invoke-Scenario {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [scriptblock]$Test
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
    Invoke-Scenario -Name "ordinary and transient fields stay outside persistence scope" -Test {
        $cleanRoot = New-PrivacyFixture
        Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/src/config/model.rs" -Content @'
pub struct AppConfig {
    pub port: u16,
}

pub struct EphemeralDto {
    pub request_body: String,
}
'@
        Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/src/state/store.rs" -Content @'
pub struct RequestMetric {
    pub latency_ms: i64,
}

pub struct StateHealth {
    pub prompt: String,
}

pub struct StateStore {
    pub authorization: String,
}
'@
        Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-storage/migrations/0001_initial.sql" -Content @'
CREATE TABLE request_metrics(latency_ms INTEGER NOT NULL);
'@
        Set-FixtureFile -Root $cleanRoot -RelativePath "crates/wokrouter-control/src/protocol.rs" -Content @'
pub struct RequestDto {
    pub request_body: String,
    pub response_body: String,
    pub prompt: String,
    pub tool_arguments: String,
    pub authorization: String,
}
'@
        Set-FixtureFile -Root $cleanRoot -RelativePath "docs/privacy.md" -Content @'
DTO documentation may discuss request_body, response_body, prompt, tool_arguments, and authorization.
'@
        Assert-CheckPasses -Root $cleanRoot -Scenario "ordinary DTO, transient state, and documentation fields"
    }

    foreach ($field in $forbiddenFields) {
        Invoke-Scenario -Name "persistent Rust field $field" -Test {
            $rustRoot = New-PrivacyFixture
            $relativePath = "crates/wokrouter-storage/src/config/model.rs"
            Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Content @"
pub struct AppConfig {
    pub ${field}: String,
}
"@
            Assert-CheckRejects `
                -Root $rustRoot `
                -Field $field `
                -RelativePath $relativePath `
                -Line 2 `
                -Scenario "persistent Rust struct"
        }.GetNewClosure()
    }

    Invoke-Scenario -Name "raw Rust identifier" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Content @'
pub struct AppConfig {
    pub r#prompt: String,
}
'@
        Assert-CheckRejects `
            -Root $rustRoot `
            -Field "prompt" `
            -RelativePath $relativePath `
            -Line 2 `
            -Scenario "raw Rust identifier"
    }

    Invoke-Scenario -Name "Rust lifetimes do not hide later fields" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Content @'
pub struct AppConfig<'a> {
    pub label: &'a str,
    pub prompt: String,
}
'@
        Assert-CheckRejects `
            -Root $rustRoot `
            -Field "prompt" `
            -RelativePath $relativePath `
            -Line 3 `
            -Scenario "Rust lifetime"
    }

    Invoke-Scenario -Name "Rust comments are ignored" -Test {
        $rustRoot = New-PrivacyFixture
        Set-FixtureFile -Root $rustRoot -RelativePath "crates/wokrouter-storage/src/config/model.rs" -Content @'
pub struct AppConfig {
    // pub request_body: String,
    pub port: u16,
    /* pub response_body: String, */
}
'@
        Assert-CheckPasses -Root $rustRoot -Scenario "comment-only Rust fields"
    }

    Invoke-Scenario -Name "Rust field attributes are handled" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/state/store.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Content @'
pub struct RequestMetric {
    #[serde(default)]
    pub response_body: String,
}
'@
        Assert-CheckRejects `
            -Root $rustRoot `
            -Field "response_body" `
            -RelativePath $relativePath `
            -Line 3 `
            -Scenario "attributed Rust field"
    }

    Invoke-Scenario -Name "Rust field matching is case-insensitive" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Content @'
pub struct AppConfig {
    pub Tool_Arguments: String,
}
'@
        Assert-CheckRejects `
            -Root $rustRoot `
            -Field "Tool_Arguments" `
            -RelativePath $relativePath `
            -Line 2 `
            -Scenario "case-insensitive Rust field"
    }

    Invoke-Scenario -Name "nested Rust callback parameter is not a field" -Test {
        $rustRoot = New-PrivacyFixture
        Set-FixtureFile -Root $rustRoot -RelativePath "crates/wokrouter-storage/src/config/model.rs" -Content @'
pub struct AppConfig {
    pub callback: fn(prompt: String),
}
'@
        Assert-CheckPasses -Root $rustRoot -Scenario "nested Rust callback parameter"
    }

    foreach ($field in $forbiddenFields) {
        Invoke-Scenario -Name "CREATE TABLE column $field" -Test {
            $sqlRoot = New-PrivacyFixture
            $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
            Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @"
CREATE TABLE persisted_requests(
    id TEXT PRIMARY KEY,
    ${field} TEXT
);
"@
            Assert-CheckRejects `
                -Root $sqlRoot `
                -Field $field `
                -RelativePath $relativePath `
                -Line 3 `
                -Scenario "CREATE TABLE column"
        }.GetNewClosure()
    }

    Invoke-Scenario -Name "double-quoted SQL column" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted_requests(
    "Prompt" TEXT
);
'@
        Assert-CheckRejects -Root $sqlRoot -Field "Prompt" -RelativePath $relativePath -Line 2 `
            -Scenario "double-quoted SQL column"
    }

    Invoke-Scenario -Name "bracket-quoted SQL column" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted_requests(
    [AUTHORIZATION] TEXT
);
'@
        Assert-CheckRejects -Root $sqlRoot -Field "AUTHORIZATION" -RelativePath $relativePath -Line 2 `
            -Scenario "bracket-quoted SQL column"
    }

    Invoke-Scenario -Name "backtick-quoted SQL column" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted_requests(
    `tool_arguments` TEXT
);
'@
        Assert-CheckRejects -Root $sqlRoot -Field "tool_arguments" -RelativePath $relativePath -Line 2 `
            -Scenario "backtick-quoted SQL column"
    }

    Invoke-Scenario -Name "ALTER TABLE ADD COLUMN" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
ALTER TABLE persisted_requests
ADD COLUMN "Response_Body" TEXT;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "Response_Body" -RelativePath $relativePath -Line 2 `
            -Scenario "ALTER TABLE ADD COLUMN"
    }

    Invoke-Scenario -Name "ALTER TABLE ADD without COLUMN" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
ALTER TABLE persisted_requests ADD prompt TEXT;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "prompt" -RelativePath $relativePath -Line 1 `
            -Scenario "ALTER TABLE ADD column"
    }

    Invoke-Scenario -Name "CREATE TABLE AS SELECT alias" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted AS SELECT 1 AS prompt;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "prompt" -RelativePath $relativePath -Line 1 `
            -Scenario "CTAS alias"
    }

    Invoke-Scenario -Name "CREATE TABLE AS SELECT bare output column" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted AS SELECT request_body FROM source;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "request_body" -RelativePath $relativePath -Line 1 `
            -Scenario "CTAS bare output column"
    }

    Invoke-Scenario -Name "CREATE TABLE AS SELECT qualified output column" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted AS
SELECT source.authorization
FROM source;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "authorization" -RelativePath $relativePath -Line 2 `
            -Scenario "CTAS qualified output column"
    }

    Invoke-Scenario -Name "unreliable CREATE TABLE AS SELECT fails closed" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
CREATE TABLE persisted AS SELECT * FROM source;
'@
        $result = Invoke-PrivacyCheck -Root $sqlRoot
        if ($result.ExitCode -ne 2) {
            throw "CTAS wildcard should exit 2, but exited $($result.ExitCode): $($result.Output)"
        }
        if ($result.Output -notmatch "CTAS PARSE ERROR") {
            throw "CTAS wildcard did not report a dedicated parse error: $($result.Output)"
        }
        $expectedLocation = (Join-Path $sqlRoot $relativePath) + ":1"
        if ($result.Output -notmatch [regex]::Escape($expectedLocation)) {
            throw "CTAS wildcard did not report '$expectedLocation': $($result.Output)"
        }
    }

    Invoke-Scenario -Name "ALTER TABLE RENAME COLUMN target" -Test {
        $sqlRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/migrations/0001_initial.sql"
        Set-FixtureFile -Root $sqlRoot -RelativePath $relativePath -Content @'
ALTER TABLE persisted RENAME COLUMN safe TO authorization;
'@
        Assert-CheckRejects -Root $sqlRoot -Field "authorization" -RelativePath $relativePath -Line 1 `
            -Scenario "ALTER TABLE RENAME COLUMN target"
    }

    Invoke-Scenario -Name "CTAS and RENAME non-output contexts are ignored" -Test {
        $sqlRoot = New-PrivacyFixture
        Set-FixtureFile -Root $sqlRoot -RelativePath "crates/wokrouter-storage/migrations/0001_initial.sql" -Content @'
-- CREATE TABLE ignored AS SELECT 1 AS prompt;
CREATE TABLE request_body AS
SELECT 'authorization' AS safe_value
FROM response_body;
ALTER TABLE request_body RENAME COLUMN tool_arguments TO safe_name;
'@
        Assert-CheckPasses -Root $sqlRoot `
            -Scenario "CTAS comments, strings, table names, FROM table names, and old rename names"
    }

    Invoke-Scenario -Name "SQL non-column contexts are ignored" -Test {
        $sqlRoot = New-PrivacyFixture
        Set-FixtureFile -Root $sqlRoot -RelativePath "crates/wokrouter-storage/migrations/0001_initial.sql" -Content @'
-- CREATE TABLE ignored(request_body TEXT);
/* ALTER TABLE ignored ADD response_body TEXT; */
CREATE TABLE request_body(
    safe_value TEXT DEFAULT 'tool_arguments authorization',
    safe_prompt Prompt,
    CONSTRAINT authorization CHECK (safe_value <> 'prompt')
);
CREATE INDEX response_body ON request_body(safe_value);
ALTER TABLE request_body ADD CONSTRAINT tool_arguments UNIQUE (safe_value);
'@
        Assert-CheckPasses -Root $sqlRoot -Scenario "SQL comments, strings, table, index, constraint, and type names"
    }

    Invoke-Scenario -Name "platform relative paths support both directory separators" -Test {
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        $unixPath = Convert-ToPlatformRelativePath `
            -RelativePath $relativePath `
            -DirectorySeparator ([char]"/")
        if ($unixPath -ne $relativePath) {
            throw "Unix separator conversion returned '$unixPath'."
        }
        $windowsPath = Convert-ToPlatformRelativePath `
            -RelativePath $relativePath `
            -DirectorySeparator ([char]"\")
        $expectedWindowsPath = $relativePath.Replace("/", "\")
        if ($windowsPath -ne $expectedWindowsPath) {
            throw "Windows separator conversion returned '$windowsPath'."
        }
    }

    Invoke-Scenario -Name "missing persistent model file fails closed" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Remove-Item -LiteralPath (Join-Path $rustRoot $relativePath) -Force
        Assert-InventoryFails `
            -Root $rustRoot `
            -ExpectedText (Convert-ToPlatformRelativePath -RelativePath $relativePath) `
            -Scenario "missing persistent model file"
    }

    Invoke-Scenario -Name "missing listed persistent struct fails closed" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Exact -Content @'
pub struct AppConfig {
    pub port: u16,
}
pub struct VersionedConfig {
    pub revision: u64,
}
pub struct ServerConfig {
    pub host: String,
}
'@
        Assert-InventoryFails `
            -Root $rustRoot `
            -ExpectedText "UiConfig" `
            -Scenario "missing listed persistent struct"
    }

    Invoke-Scenario -Name "wrong-case persistent struct name fails closed" -Test {
        $rustRoot = New-PrivacyFixture
        $relativePath = "crates/wokrouter-storage/src/config/model.rs"
        Set-FixtureFile -Root $rustRoot -RelativePath $relativePath -Exact -Content @'
pub struct appconfig {
    pub port: u16,
}
pub struct VersionedConfig {
    pub revision: u64,
}
pub struct ServerConfig {
    pub host: String,
}
pub struct UiConfig {
    pub locale: String,
}
'@
        Assert-InventoryFails `
            -Root $rustRoot `
            -ExpectedText "AppConfig" `
            -Scenario "wrong-case persistent struct name"
    }

    if ($failures.Count -gt 0) {
        foreach ($failure in $failures) {
            Write-Host "SELF-TEST ERROR: $failure"
        }
        Write-Host "Privacy checker self-tests failed: $($failures.Count) of $scenarioCount scenario(s)."
        exit 1
    }

    Write-Host "Privacy checker self-tests passed: $scenarioCount scenario(s)."
}
finally {
    foreach ($root in $fixtureRoots) {
        if (Test-Path -LiteralPath $root) {
            $resolvedRoot = (Resolve-Path -LiteralPath $root).Path
            $resolvedParent = [System.IO.Path]::GetFullPath(
                (Split-Path -Parent $resolvedRoot)
            ).TrimEnd(
                [System.IO.Path]::DirectorySeparatorChar,
                [System.IO.Path]::AltDirectorySeparatorChar
            )
            $leaf = Split-Path -Leaf $resolvedRoot
            if (
                -not $resolvedParent.Equals(
                    $fixtureBase,
                    [System.StringComparison]::OrdinalIgnoreCase
                ) -or
                -not $leaf.StartsWith(
                    "wokrouter-privacy-",
                    [System.StringComparison]::Ordinal
                )
            ) {
                throw "Refusing to remove unexpected privacy fixture path '$resolvedRoot'."
            }

            Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
        }
    }
}
