# run_bench.ps1 — end-to-end Python vs Rust benchmark.
#
#   1. transpile the Dipulse protocol modules with py2rs
#   2. build the generated Rust project
#   3. generate the shared fixtures
#   4. run both sides and compare timings and result digests
#
# Usage:
#   .\run_bench.ps1 [--repeat 7]

param([int]$Repeat = 9, [int]$Count = 4096)

$ErrorActionPreference = "Continue"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

$root    = Split-Path -Parent $PSScriptRoot   # Rustify
$project = Split-Path -Parent $root            # project
$py2rs   = $root
$dipulse = Join-Path $project "Dipulse"
$bench   = $PSScriptRoot
$rustOut = Join-Path $root "rust_out"

Write-Host "== 1. transpile ==" -ForegroundColor Cyan
& (Join-Path $py2rs "run.ps1") build `
    --root $dipulse `
    --entry "gateway/app/drivers/bms/jk/parser.py" `
    --out $rustOut
if ($LASTEXITCODE -ne 0) { throw "py2rs build failed" }

Copy-Item (Join-Path $bench "bench_main.rs") (Join-Path $rustOut "src\main.rs") -Force

Write-Host "== 2. build generated Rust ==" -ForegroundColor Cyan
Push-Location $rustOut
cargo build --release 2>&1 | Select-String -Pattern "^error" -Context 0,6 | Write-Host
cargo build --release 2>&1 | Select-Object -Last 1 | Write-Host
Pop-Location

Write-Host "== 3. fixtures ==" -ForegroundColor Cyan
Push-Location $bench
python make_fixtures.py --count $Count
Pop-Location

Write-Host "== 4. benchmark ==" -ForegroundColor Cyan
Push-Location $bench
$py = python bench_python.py --repeat $Repeat
$rs = & (Join-Path $rustOut "target\release\dipulse_protocol.exe") (Join-Path $bench "fixtures.bin") $Repeat
Pop-Location

Write-Host ""
Write-Host "Python (CPython 3.13):" -ForegroundColor Yellow
$py | Write-Host
Write-Host ""
Write-Host "Rust (stable 1.99.1, --release):" -ForegroundColor Yellow
$rs | Write-Host

# Parse both tables and print the speed-up.
function Get-Table($rows) {
    $map = @{}
    foreach ($line in $rows) {
        if ($line -match '^(modbus|tlv|pb)\s+(\d+)\s+([\d.]+)\s+([\d.]+)\s+([0-9a-f]+)') {
            $map[$Matches[1]] = @{ frames = [int]$Matches[2]; ms = [double]$Matches[3]; us = [double]$Matches[4]; digest = $Matches[5] }
        }
    }
    return $map
}
$pyT = Get-Table $py
$rsT = Get-Table $rs

Write-Host ""
Write-Host ("{0,-9} {1,10} {2,12} {3,12} {4,10}  {5}" -f "workload", "frames", "python us", "rust us", "speedup", "parity")
Write-Host ("-" * 72)
foreach ($k in @("modbus", "tlv", "pb")) {
    if ($pyT.ContainsKey($k) -and $rsT.ContainsKey($k)) {
        $speed = $pyT[$k].us / $rsT[$k].us
        $parity = if ($pyT[$k].digest -eq $rsT[$k].digest) { "MATCH" } else { "DIFFER" }
        Write-Host ("{0,-9} {1,10} {2,12:N3} {3,12:N3} {4,9:N1}x  {5}" -f $k, $pyT[$k].frames, $pyT[$k].us, $rsT[$k].us, $speed, $parity)
    }
}
