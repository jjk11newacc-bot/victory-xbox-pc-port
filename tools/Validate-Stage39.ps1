$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed ($LASTEXITCODE)" }
}
Invoke-Checked 'cargo' @('check','--locked','--target','x86_64-pc-windows-msvc','-p','victory','--features','diagnostics,blake3/pure','--bins','--tests')
Invoke-Checked 'python' @('tools/check_nvidia_profile.py')
Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','volume_axes_match_every_cpu_address')
Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','graphics_binding_cache_retains_only_exact_values_until_invalidation')
Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','draws_promote_early_pipelines_without_claiming_them_twice')
# Run once on WARP, then on hardware in a separate process.
$old = $env:VICTORY_D3D12_WARP
try {
    $env:VICTORY_D3D12_WARP = '1'
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','--features','diagnostics','gpu_detile_readback_matches_cpu_rows','--','--nocapture')
    Remove-Item Env:VICTORY_D3D12_WARP -ErrorAction SilentlyContinue
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','--features','diagnostics','gpu_detile_readback_matches_cpu_rows','--','--nocapture')
} finally {
    if ($null -eq $old) { Remove-Item Env:VICTORY_D3D12_WARP -ErrorAction SilentlyContinue } else { $env:VICTORY_D3D12_WARP = $old }
}
