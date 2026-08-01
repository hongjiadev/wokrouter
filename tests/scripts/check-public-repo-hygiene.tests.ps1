$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$checker = Join-Path $PSScriptRoot "check-public-repo-hygiene.ps1"
if (-not (Test-Path -LiteralPath $checker)) {
    throw "Missing hygiene checker: $checker"
}

$clean = @(
    "100644 0000000000000000000000000000000000000000 0`tREADME.md",
    "100644 0000000000000000000000000000000000000000 0`tdocs/architecture.md",
    "100644 0000000000000000000000000000000000000000 0`tdocs/api-spec.md",
    "100644 0000000000000000000000000000000000000000 0`t.github/workflows/ci.yml",
    "100644 0000000000000000000000000000000000000000 0`tnotes/daily-progress.md",
    "100644 0000000000000000000000000000000000000000 0`trelease/minisign.pub",
    "100644 0000000000000000000000000000000000000000 0`tcrates/wokrouter-platform/src/wokcore_install/wokcore-minisign.pub",
    "100644 0000000000000000000000000000000000000000 0`tcrates/wokrouter-platform/tests/fixtures/wokcore-install/minisign.pub",
    "100644 0000000000000000000000000000000000000000 0`tcrates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v1.json.minisig",
    "100644 0000000000000000000000000000000000000000 0`tcrates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v2.json.minisig"
)
& $checker -IndexLines $clean

foreach ($forbidden in @(
    "100644 0000000000000000000000000000000000000000 0`tdocs/superpowers/plan.md",
    "100644 0000000000000000000000000000000000000000 0`t.superpowers/review.md",
    "100644 0000000000000000000000000000000000000000 0`t.subpowers/sessions/active.md",
    "120000 0000000000000000000000000000000000000000 0`tdocs/superpowers",
    "120000 0000000000000000000000000000000000000000 0`tinternal-docs",
    "100644 0000000000000000000000000000000000000000 0`tnotes/ai-review.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/ai-progress.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/codex-handoff.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/plan-claude.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/ai-private-review.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/review-generated-by-codex.md",
    "100644 0000000000000000000000000000000000000000 0`tnotes/CLAUDE_internal_PROGRESS.txt",
    "100644 0000000000000000000000000000000000000000 0`trelease/other.pub",
    "100644 0000000000000000000000000000000000000000 0`trelease/payload.minisig",
    "100644 0000000000000000000000000000000000000000 0`trelease/SHA256SUMS",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-Windows-x86_64.msi",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-Linux-x86_64.deb",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-Linux-x86_64.rpm",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-Linux-x86_64.AppImage",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-macOS-x86_64.dmg",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-Windows-x86_64-Portable.zip",
    "100644 0000000000000000000000000000000000000000 0`trelease/WokRouter-v1.2.3-macOS-x86_64.tar.gz",
    "100644 0000000000000000000000000000000000000000 0`tsecrets/wokrouter-minisign.key"
)) {
    $failed = $false
    try {
        & $checker -IndexLines @($forbidden)
    } catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Expected forbidden index entry to fail: $forbidden"
    }
}

foreach ($privateHeader in @(
        "untrusted comment: minisign encrypted " + "secret key",
        "untrusted comment: minisign " + "secret key"
    )) {
    $failed = $false
    try {
        & $checker `
            -IndexLines @(
                "100644 0000000000000000000000000000000000000000 0`tsrc/embedded.txt"
            ) `
            -TrackedTextByPath @{
                "src/embedded.txt" = $privateHeader
            }
    } catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Expected tracked Minisign private key header to fail."
    }
}

Write-Output "public repository hygiene tests passed"
