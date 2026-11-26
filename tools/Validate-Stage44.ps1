$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    & (Join-Path $PSScriptRoot 'Validate-Stage43.ps1')
    & python (Join-Path $PSScriptRoot 'check_stage44.py') --hardware
    if ($LASTEXITCODE -ne 0) { throw "Stage 44 checks failed ($LASTEXITCODE)" }
    Write-Host 'Stage 44 actual test suite and NVIDIA detile parity completed. Gameplay/FPS still needs measurement.'
} finally { Pop-Location }
