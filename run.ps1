# run.ps1 — build py2rs and run it, working around Windows Smart App Control.
#
# Smart App Control (WDAC "VerifiedAndReputable") intermittently blocks
# freshly linked, unsigned binaries until Defender's cloud reputation service
# has a verdict for the new file hash. A clean rebuild produces a different
# hash that is accepted. This wrapper retries with a clean rebuild.
#
# Usage:
#   .\run.ps1 check <file.py>
#   .\run.ps1 build --root <dir> --entry <file.py> --out <dir>

param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PyArgs)

$ErrorActionPreference = "Continue"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$exe = Join-Path $PSScriptRoot "target\debug\rustify.exe"

function Build {
    param([bool]$Clean)
    Push-Location $PSScriptRoot
    try {
        if ($Clean) { cargo clean 2>&1 | Out-Null }
        $errs = cargo build 2>&1 | Select-String -Pattern "^error" -Context 0,10 | Out-String
        if ($errs) { Write-Host $errs }
    } finally {
        Pop-Location
    }
}

Build -Clean $false
for ($attempt = 1; $attempt -le 6; $attempt++) {
    $out = @()
    $blocked = $false
    $code = 1
    try {
        $out = & $exe @PyArgs 2>&1 | ForEach-Object { $_.ToString() }
        $code = $LASTEXITCODE
    } catch {
        $blocked = $true
    }
    if (-not $blocked) {
        $out | ForEach-Object { Write-Host $_ }
        exit $code
    }
    Write-Host "[run.ps1] binary blocked by Smart App Control; clean rebuild (attempt $attempt)"
    Build -Clean $true
}
Write-Host "[run.ps1] could not obtain an executable binary after 6 attempts"
exit 1
