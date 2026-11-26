$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed ($LASTEXITCODE)" }
}
# Prerequisite: initialize verified format tables using the repository's
# Initialize-FormatTables.ps1; no proprietary assets in this archive.
Invoke-Checked 'cargo' @('check','--locked','--target','x86_64-pc-windows-msvc','-p','victory','--features','diagnostics,blake3/pure','--bins','--tests')
Invoke-Checked 'python' @('tools/check_nvidia_profile.py')
Invoke-Checked 'python' @('tools/check_stage42_model.py')
# Test the actual Rust code, not just Python reference models. Running all
# tests makes an obsolete/misspelled filter unable to report zero tests passed.
$names = @('VICTORY_D3D12_WARP','VICTORY_NVIDIA','VICTORY_REQUIRE_GPU_TEST','VICTORY_TESTS_ALLOW_SOFTWARE_SKIP','VICTORY_INDEXED_PENDING','VICTORY_INDEXED_WATCH','VICTORY_BATCH_HAZARD_QUERY','VICTORY_RECLAIM_RAW_SLOTS','VICTORY_REUSE_IDLE_DIRECTORY')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process'); [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
try {
    $env:VICTORY_NVIDIA = '1'
    $env:VICTORY_REQUIRE_GPU_TEST = '1'
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','--features','diagnostics','--','--test-threads=1')
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-shader','--features','host','--','--test-threads=1')
    $env:VICTORY_INDEXED_PENDING = '1'
    $env:VICTORY_INDEXED_WATCH = '1'
    $env:VICTORY_BATCH_HAZARD_QUERY = '1'
    $env:VICTORY_RECLAIM_RAW_SLOTS = '1'
    $env:VICTORY_REUSE_IDLE_DIRECTORY = '1'
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','--features','diagnostics','--','--test-threads=1')
    Remove-Item Env:VICTORY_REQUIRE_GPU_TEST
    Remove-Item Env:VICTORY_NVIDIA
    $env:VICTORY_D3D12_WARP = '1'
    $result = & cargo test --locked -p victory-gpu --features diagnostics 'texture::gpu_detile::tests::gpu_detile_readback_matches_cpu_rows' -- --exact --nocapture 2>&1
    $code = $LASTEXITCODE
    $result | ForEach-Object { Write-Host $_ }
    if ($code -ne 0 -or -not ($result | Select-String -SimpleMatch 'STAGE42_GPU_DETILE_PARITY_OK')) { throw 'WARP parity test failed or did not execute' }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
