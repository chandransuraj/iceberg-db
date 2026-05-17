# Start Trunk with LLVM on PATH (required for zstd-sys on wasm32-unknown-unknown).
$ErrorActionPreference = "Stop"

$llvmBin = Join-Path ${env:ProgramFiles} "LLVM\bin"
if (-not (Test-Path (Join-Path $llvmBin "clang.exe"))) {
    Write-Error @"
clang.exe not found at: $llvmBin

Install LLVM, then open a NEW terminal:
  winget install --id LLVM.LLVM -e

Or add LLVM\bin to your user PATH manually.
"@
}

$env:Path = "$llvmBin;$env:Path"
# Only wasm32: global CC=clang breaks native idb-cli (stacker/liblzma need MSVC or full SDK).
$env:CC_wasm32_unknown_unknown = "clang"
$env:AR_wasm32_unknown_unknown = "llvm-ar"

Write-Host "Using clang: $(Get-Command clang | Select-Object -ExpandProperty Source)"
Set-Location $PSScriptRoot

trunk serve index.html --open @args
