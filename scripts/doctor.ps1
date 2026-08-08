#Requires -Version 7
<#
.SYNOPSIS
Validates a Termielle installation end to end: host, binaries, the overlay
smoke test, and a unique-pipe sink through smoke mode driven by the emitter.

.OUTPUTS
One PASS/FAIL line per check. Exits 0 only when every check passes.

.EXAMPLE
pwsh -NoProfile -File scripts\doctor.ps1
pwsh -NoProfile -File scripts\doctor.ps1 -BinDir .\target\debug
#>
param(
    # Directory holding termielle-app.exe and termielle-emit.exe.
    [string]$BinDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release')
)

$ErrorActionPreference = 'Stop'
$failures = [System.Collections.Generic.List[string]]::new()
$checks = 0

function Assert-Check {
    param([bool]$Condition, [string]$Name)
    $script:checks++
    if ($Condition) {
        Write-Host "PASS $Name"
    } else {
        Write-Host "FAIL $Name" -ForegroundColor Red
        $script:failures.Add($Name)
    }
}

# 1. Host: Windows and a supported architecture, running PowerShell 7.
Assert-Check $IsWindows 'Windows host'
Assert-Check ($PSVersionTable.PSVersion.Major -ge 7) 'PowerShell 7'
$arch = $env:PROCESSOR_ARCHITECTURE
Assert-Check ($arch -in @('AMD64', 'ARM64')) "Supported architecture ($arch)"

# 2. Binaries are present and loadable (VerifySignature-free: a self-built
#    copy has no certificate, so existence and a clean smoke are the checks).
$app = Join-Path $BinDir 'termielle-app.exe'
$emit = Join-Path $BinDir 'termielle-emit.exe'
Assert-Check (Test-Path -LiteralPath $app -PathType Leaf) "Overlay binary at $app"
Assert-Check (Test-Path -LiteralPath $emit -PathType Leaf) "Emitter binary at $emit"

if ($failures.Count -eq 0) {
    # 3. The overlay smoke test on the default pipe. Exit 3 means a live
    #    overlay holds the instance mutex: the check must not pass silently.
    $smoke = Start-Process -FilePath $app -ArgumentList '--smoke-test' -PassThru -Wait -WindowStyle Hidden
    Assert-Check ($smoke.ExitCode -eq 0) 'Overlay smoke test passes (default pipe)'

    # 4. A unique pipe sink through smoke mode: the emitter alone must
    #    deliver the event that clears the smoke test.
    $pipe = 'termielle-doctor-' + [guid]::NewGuid().ToString('N')
    $fullPipe = '\\.\pipe\' + $pipe
    $smoke2 = Start-Process -FilePath $app -ArgumentList '--smoke-test', '--pipe', $fullPipe -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 400
    $payload = '{"type":"prompt_submitted","session_id":"doctor","prompt":"x"}'
    $null = $payload | & $emit --source codex --event prompt_submitted --input stdin --pipe $fullPipe
    $emitExit = $LASTEXITCODE
    $settled = $smoke2.WaitForExit(15000)
    Assert-Check ($emitExit -eq 0) 'Emitter delivers over the unique pipe'
    Assert-Check ($settled -and $smoke2.ExitCode -eq 0) 'Smoke passes on the external event'
}

if ($failures.Count -gt 0) {
    Write-Host ''
    Write-Host ("FAILED {0} of {1} checks: {2}" -f $failures.Count, $checks, ($failures -join '; ')) -ForegroundColor Red
    exit 1
}
Write-Host ''
Write-Host "ALL $checks CHECKS PASSED" -ForegroundColor Green
exit 0
