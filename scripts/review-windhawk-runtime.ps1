#Requires -Version 7
# Isolated DLL hosts only: no Windhawk injection, real profile or task edits.
param(
    [string]$BinDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release'),
    [switch]$Consolidated
)
$ErrorActionPreference = 'Stop'
$runtime = (Resolve-Path (Join-Path $BinDir 'termielle_runtime.dll')).Path
$review = (Resolve-Path (Join-Path $BinDir 'examples\windhawk_host_review.exe')).Path
$modes = @('smoke', 'stop', 'bar', 'off', 'invalid', 'busy', 'cancel')
if ($Consolidated) { $modes += 'adapter' }
foreach ($mode in $modes) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $review
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $null = $info.ArgumentList.Add($runtime)
    $null = $info.ArgumentList.Add($mode)
    $process = [Diagnostics.Process]::Start($info)
    if (-not $process.WaitForExit(20000)) {
        throw "Isolated host timed out; PID $($process.Id) was NOT force-terminated. Diagnose before proceeding."
    }
    $code = $process.ExitCode
    $process.Dispose()
    if ($code -ne 0) { throw "Host mode $mode failed ($code)" }
}
Write-Output "PASS all $($modes.Count) isolated DLL-host checks; no review processes remain"
