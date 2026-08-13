#Requires -Version 7
<#
.SYNOPSIS
Measures the overlay against the performance budgets: warm emitter latency, overlay
working set while animating the largest real asset, event-to-visible latency,
warm start to first frame, and long-run average CPU. One line per measurement
with the budget and verdict; exits 0 when every budget holds.

.EXAMPLE
pwsh -NoProfile -File scripts\benchmark.ps1
pwsh -NoProfile -File scripts\benchmark.ps1 -Samples 20 -DurationSeconds 10 -BinDir .\target\debug
#>
param(
    # Emitter timing samples after the warmups.
    [int]$Samples = 50,
    # Emitter invocations discarded before sampling.
    [int]$Warmups = 5,
    # Seconds of working-set sampling while animating.
    [int]$WorkingSetSeconds = 5,
    # Seconds of CPU measurement before the report.
    [int]$DurationSeconds = 300,
    # Directory holding termielle-app.exe and termielle-emit.exe.
    [string]$BinDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release'),
    # Skip `cargo build --release` before measuring.
    [switch]$NoBuild,
    # Print every measurement without failing on budgets; for CI, where host
    # scheduling noise would otherwise break the gate.
    [switch]$ReportOnly
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent

# The budgets, in the order they are measured.
$targets = @{
    EmitterMedianMs    = 20.0
    EmitterP95Ms       = 50.0
    EventToFrameMs     = 50.0
    WarmFirstFrameMs   = 250.0
    WorkingSetMiB      = 50.0
    IdleCpuPercentCore = 0.5
}

function Emit {
    param([string]$Pipe, [string]$Session, [string]$Event)
    $payload = '{"type":"' + $Event + '","session_id":"' + $Session + '","prompt":"x"}'
    # The emitter is a GUI-subsystem executable (so hooks never flash a
    # console), which PowerShell's `&` runs asynchronously. Drive it through
    # the .NET Process API instead: CreateProcess-based, like a real agent
    # hook, and it waits for the exit code.
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $EmitExe
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    foreach ($arg in @('--source', 'codex', '--event', $Event, '--input', 'stdin', '--pipe', $Pipe)) {
        $null = $psi.ArgumentList.Add($arg)
    }
    $proc = [System.Diagnostics.Process]::Start($psi)
    $proc.StandardInput.Write($payload)
    $proc.StandardInput.Close()
    $null = $proc.StandardOutput.ReadToEnd()
    $proc.WaitForExit()
    if ($proc.ExitCode -ne 0) {
        throw "emitter failed with exit $($proc.ExitCode)"
    }
}

function Percentile {
    param([double[]]$Values, [double]$P)
    if ($Values.Count -eq 0) { return 0.0 }
    $sorted = [double[]]($Values | Sort-Object)
    $index = [math]::Min([int][math]::Ceiling($P * $sorted.Count) - 1, $sorted.Count - 1)
    return $sorted[[math]::Max(0, $index)]
}

function Median {
    param([double[]]$Values)
    return (Percentile $Values 0.5)
}

# 1. Binaries: build on demand, then locate.
if (-not $NoBuild) {
    Push-Location $repo
    try { & cargo build --release 2>&1 | Out-Null; if ($LASTEXITCODE -ne 0) { throw 'release build failed' } }
    finally { Pop-Location }
}
$AppExe = Join-Path $BinDir 'termielle-app.exe'
$EmitExe = Join-Path $BinDir 'termielle-emit.exe'
if (-not (Test-Path -LiteralPath $AppExe)) { throw "overlay binary not found at $AppExe" }
if (-not (Test-Path -LiteralPath $EmitExe)) { throw "emitter binary not found at $EmitExe" }

# 2. Real assets next to the binaries so the overlay animates them.
$assetSource = Join-Path $repo 'assets'
$assetDir = Join-Path $BinDir 'assets'
if (Test-Path -LiteralPath $assetSource) {
    New-Item -ItemType Directory -Force -Path $assetDir | Out-Null
    Copy-Item -Force -Path (Join-Path $assetSource '*.gif') -Destination $assetDir
}

# 3. Launch the overlay on a unique pipe with an acknowledgement file.
$runId = [guid]::NewGuid().ToString('N')
$pipe = 'termielle-bench-' + $runId
$fullPipe = '\\.\pipe\' + $pipe
$ack = Join-Path $env:TEMP ("termielle-bench-{0}.jsonl" -f $runId)
Remove-Item -LiteralPath $ack -ErrorAction SilentlyContinue
$session = 'bench-' + $runId

$app = Start-Process -FilePath $AppExe -ArgumentList '--pipe', $fullPipe, '--ack-file', $ack -PassThru -WindowStyle Hidden

function Wait-AckState {
    param([string]$State, [int]$Seconds, [int]$PollMs = 50)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $ack) {
            $text = Get-Content -LiteralPath $ack -Raw
            if ($text -match ('"state":"' + $State + '"')) { return $true }
        }
        Start-Sleep -Milliseconds $PollMs
    }
    return $false
}

try {
    # 4. Warm start to first frame: the first ack after launch.
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    if (-not (Wait-AckState 'idle' 15)) { throw 'overlay never presented its first frame' }
    $sw.Stop()
    $warmFirstFrame = $sw.Elapsed.TotalMilliseconds

    # 5. Event-to-visible latency: send needs_input, time the state on screen.
    #    The 5 ms poll keeps stopwatch quantization a small fraction of the
    #    50 ms budget.
    $sw.Restart()
    Emit -Pipe $fullPipe -Session $session -Event 'needs_input'
    if (-not (Wait-AckState 'needs_input' 10 5)) { throw 'overlay never reached needs_input' }
    $sw.Stop()
    $eventToFrame = $sw.Elapsed.TotalMilliseconds

    # 6. Overlay working set while the largest real asset animates.
    $peak = 0.0
    $wsSamples = [System.Collections.Generic.List[double]]::new()
    $deadline = (Get-Date).AddSeconds($WorkingSetSeconds)
    while ((Get-Date) -lt $deadline) {
        $app.Refresh()
        $mib = $app.WorkingSet64 / 1MB
        $wsSamples.Add($mib)
        if ($mib -gt $peak) { $peak = $mib }
        Start-Sleep -Milliseconds 100
    }
    $workingSetMean = (($wsSamples | Measure-Object -Average).Average)

    # 7. Emitter latency: warmups, then timed sends of the same event.
    for ($i = 0; $i -lt $Warmups; $i++) { Emit -Pipe $fullPipe -Session $session -Event 'prompt_submitted' }
    Start-Sleep -Milliseconds 500
    $emitSamples = [System.Collections.Generic.List[double]]::new()
    for ($i = 0; $i -lt $Samples; $i++) {
        $sw.Restart()
        Emit -Pipe $fullPipe -Session $session -Event 'prompt_submitted'
        $emitSamples.Add($sw.Elapsed.TotalMilliseconds)
    }

    # 8. Long-run average CPU while idle (Ready hold has elapsed; the overlay
    #    sits in Idle).
    $cpuStart = $app.TotalProcessorTime.TotalMilliseconds
    $wallStart = (Get-Date)
    Start-Sleep -Seconds $DurationSeconds
    $cpuDelta = $app.TotalProcessorTime.TotalMilliseconds - $cpuStart
    $wallMs = ((Get-Date) - $wallStart).TotalMilliseconds
    $cores = [Environment]::ProcessorCount
    $cpuPctCore = ($cpuDelta / $wallMs) * 100.0 / $cores
}
finally {
    if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
    Remove-Item -LiteralPath $ack -ErrorAction SilentlyContinue
}

# 9. Verdicts.
$results = [ordered]@{
    'Emitter median (ms)'        = (Median $emitSamples.ToArray())
    'Emitter p95 (ms)'           = (Percentile $emitSamples.ToArray() 0.95)
    'Event-to-frame (ms)'        = $eventToFrame
    'Warm first frame (ms)'      = $warmFirstFrame
    'Working set mean (MiB)'     = $workingSetMean
    'Working set peak (MiB)'     = $peak
    'Idle CPU (% of one core)'   = $cpuPctCore
}

$fails = 0
foreach ($key in $results.Keys) {
    $budget = switch -Regex ($key) {
        'Emitter median' { $targets.EmitterMedianMs }
        'Emitter p95' { $targets.EmitterP95Ms }
        'Event-to-frame' { $targets.EventToFrameMs }
        'Warm first frame' { $targets.WarmFirstFrameMs }
        'Working set' { $targets.WorkingSetMiB }
        'Idle CPU' { $targets.IdleCpuPercentCore }
        default { [double]::MaxValue }
    }
    $value = [double]$results[$key]
    $ok = $value -le $budget
    if (-not $ok) { $fails++ }
    $line = '{0,-24} {1,10:F2} (budget {2:F2}) {3}' -f $key, $value, $budget, $(if ($ok) { 'PASS' } else { 'FAIL' })
    if ($ok) { Write-Host $line } else { Write-Host $line -ForegroundColor Red }
}

if ($ReportOnly) {
    Write-Host 'REPORT ONLY: no budget gate applied'
    exit 0
}
if ($fails -gt 0) { Write-Host "FAILED $fails budget(s)" -ForegroundColor Red; exit 1 }
Write-Host 'ALL BUDGETS HELD' -ForegroundColor Green
exit 0
