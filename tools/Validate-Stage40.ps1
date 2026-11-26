$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
& (Join-Path $PSScriptRoot 'Validate-Stage39.ps1')
function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed ($LASTEXITCODE)" }
}
Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','generation_index_preserves_exact_filtered_journal')
Invoke-Checked 'cargo' @('test','--locked','-p','victory-gpu','retired_buffers_cannot_refill_a_pressure_limited_pool')
Invoke-Checked 'python' @('tools/check_stage40_model.py')
