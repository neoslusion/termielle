#Requires -Version 7
param(
    [string]$BinDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release'),
    [string]$ConfigPath = (Join-Path $HOME '.termielle\config.json'),
    [ValidateRange(1, 20)][int]$Samples = 5,
    [ValidateRange(5, 300)][int]$DurationSeconds = 20,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$executable = (Resolve-Path (Join-Path $BinDir 'termielle-app.exe')).Path
$config = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json -AsHashtable
$config.island.layout = 'bar'
$config.island.bar.reserve_space = $false
$config.island.bar.replace_taskbar = $false
$config.island.forward_toasts = $false
$runRoot = Join-Path (Join-Path $repo 'target') ('bar-performance-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path (Join-Path $runRoot '.termielle') -Force
$isolatedConfig = Join-Path $runRoot '.termielle\config.json'
$config | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $isolatedConfig -Encoding utf8
$startups = [System.Collections.Generic.List[double]]::new()
$process = $null

function Read-PresentCount([string]$Path) {
    if (!(Test-Path -LiteralPath $Path)) { return 0 }
    return @(Get-Content -LiteralPath $Path).Count
}

try {
    for ($sample = 0; $sample -lt $Samples; $sample++) {
        $ack = Join-Path $runRoot "frames-$sample.jsonl"
        $info = [System.Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $executable
        $info.WorkingDirectory = $repo
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.Environment['USERPROFILE'] = $runRoot
        foreach ($argument in @('--pipe', ('\\.\pipe\termielle-perf-' + [guid]::NewGuid().ToString('N')), '--config', $isolatedConfig, '--ack-file', $ack)) {
            $info.ArgumentList.Add($argument)
        }
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        $process = [System.Diagnostics.Process]::Start($info)
        while ((Read-PresentCount $ack) -eq 0) {
            if ($process.HasExited) { throw "Performance instance exited with code $($process.ExitCode)" }
            if ($watch.Elapsed.TotalSeconds -ge 20) { throw 'No first-frame acknowledgement within 20 seconds' }
            Start-Sleep -Milliseconds 5
        }
        $startups.Add($watch.Elapsed.TotalMilliseconds)
        if ($sample -lt $Samples - 1) {
            $process.Kill()
            $process.WaitForExit()
            $process.Dispose()
            $process = $null
        }
    }
    Start-Sleep -Seconds 6
    $process.Refresh()
    $cpuStart = $process.TotalProcessorTime.TotalMilliseconds
    $presentStart = Read-PresentCount $ack
    $workingSets = [System.Collections.Generic.List[double]]::new()
    $privateBytes = [System.Collections.Generic.List[double]]::new()
    $watch.Restart()
    while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $process.Refresh()
        if ($process.HasExited) { throw 'Performance instance exited during resource sampling' }
        $workingSets.Add($process.WorkingSet64 / 1MB)
        $privateBytes.Add($process.PrivateMemorySize64 / 1MB)
        Start-Sleep -Milliseconds 200
    }
    $process.Refresh()
    $cpuMs = $process.TotalProcessorTime.TotalMilliseconds - $cpuStart
    $wallMs = $watch.Elapsed.TotalMilliseconds
    $presentCount = (Read-PresentCount $ack) - $presentStart
    $orderedStartups = @($startups | Sort-Object)
    $middle = [int][math]::Floor($orderedStartups.Count / 2)
    $median = $orderedStartups[$middle]
    if ($orderedStartups.Count % 2 -eq 0) {
        $median = ($orderedStartups[$middle - 1] + $median) / 2
    }
    $report = [ordered]@{
        timestamp = (Get-Date).ToString('o')
        executable = $executable
        sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
        configuration = $config
        startup_samples_ms = $startups.ToArray()
        startup_median_ms = $median
        startup_max_ms = $orderedStartups[-1]
        steady_cpu_percent_one_core = $cpuMs / $wallMs * 100
        working_set_mean_mib = ($workingSets | Measure-Object -Average).Average
        working_set_peak_mib = ($workingSets | Measure-Object -Maximum).Maximum
        private_bytes_mean_mib = ($privateBytes | Measure-Object -Average).Average
        present_rate_hz = $presentCount / ($wallMs / 1000)
        measured_seconds = $wallMs / 1000
        artifacts = $runRoot
    }
    $json = $report | ConvertTo-Json -Depth 20
    if ($OutputPath) { $json | Set-Content -LiteralPath $OutputPath -Encoding utf8 }
    $json
} finally {
    if ($process) {
        if (!$process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        $process.Dispose()
    }
}
