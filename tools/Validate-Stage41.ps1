$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed ($LASTEXITCODE)" }
}
& (Join-Path $PSScriptRoot 'Validate-Stage40.ps1')
Invoke-Checked 'python' @('tools/check_stage41_model.py')
foreach ($test in @('pending_range_index_matches_ordered_declarations','hash_overlap_orders_only_potentially_conflicting_runs','title_copy_fallback_zeros_only_through_the_kernel','linear_and_one_dimensional_axes_match_cpu_addresses')) {
    Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu',$test)
}
$old = $env:VICTORY_INDEXED_PENDING
try {
    foreach ($mode in @('0','1')) {
        $env:VICTORY_INDEXED_PENDING = $mode
        Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','a_range_stays_closed_until_every_flight_that_declared_it_is_done')
    }
} finally {
    if ($null -eq $old) { Remove-Item Env:VICTORY_INDEXED_PENDING -ErrorAction SilentlyContinue } else { $env:VICTORY_INDEXED_PENDING = $old }
}
