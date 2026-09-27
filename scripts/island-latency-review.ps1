#Requires -Version 7
# Direct pipe-to-present diagnostic, excluding emitter process startup.
param([int]$Samples = 30, [string]$BinDir = (Join-Path $PSScriptRoot '..\target\release'))
$ErrorActionPreference = 'Stop'
$runId = [guid]::NewGuid().ToString('N')
$pipeName = 'termielle-latency-' + $runId
$reviewDirectory = Join-Path ([IO.Path]::GetTempPath()) ('termielle-latency-' + $runId)
$null = New-Item -ItemType Directory -Path $reviewDirectory
$config = Join-Path $reviewDirectory 'config.json'
$ack = Join-Path $reviewDirectory 'ack.jsonl'
'{"island":{"widgets":[],"face_animated":false,"forward_toasts":false,"bar":{"reserve_space":false,"replace_taskbar":false,"modules_left":[],"modules_right":[]}}}' | Set-Content $config
$process = $null
try {
    $process = Start-Process -FilePath (Join-Path $BinDir 'termielle-app.exe') -WindowStyle Hidden -PassThru `
        -ArgumentList '--pipe', ('\\.\pipe\' + $pipeName), '--config', $config, '--ack-file', $ack
    $startupDeadline = [DateTime]::UtcNow.AddSeconds(15)
    while (!(Test-Path $ack) -or (Get-Item $ack).Length -eq 0) {
        if ($process.HasExited -or [DateTime]::UtcNow -gt $startupDeadline) { throw 'startup timed out' }
        Start-Sleep -Milliseconds 10
    }
    $observations = [Collections.Generic.List[double]]::new()
    for ($index = 0; $index -lt $Samples; $index++) {
        $kind = if ($index % 2 -eq 0) { 'needs_input' } else { 'turn_completed' }
        $state = if ($index % 2 -eq 0) { 'needs_input' } else { 'ready' }
        $before = @(Get-Content $ack).Count
        $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', $pipeName, [IO.Pipes.PipeDirection]::Out)
        try {
            $pipe.Connect(2000)
            $sent = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            $payload = @{ version = 1; source = 'codex'; session_id = 'latency'; event = $kind; timestamp_ms = $sent } | ConvertTo-Json -Compress
            $bytes = [Text.Encoding]::UTF8.GetBytes($payload + "`n")
            $pipe.Write($bytes, 0, $bytes.Length)
        } finally { $pipe.Dispose() }
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        $found = $null
        while (!$found) {
            $lines = @(Get-Content $ack)
            foreach ($line in ($lines | Select-Object -Skip $before)) {
                try { $record = $line | ConvertFrom-Json } catch { continue }
                if ($record.state -eq $state -and $record.ts -ge $sent) { $found = $record; break }
            }
            if ([DateTime]::UtcNow -gt $deadline) { throw "no present for event $index" }
            if (!$found) { Start-Sleep -Milliseconds 1 }
        }
        $observations.Add($found.ts - $sent)
    }
    $sorted = @($observations | Sort-Object)
    $process.Refresh()
    'Direct pipe-to-present: {0} events, median {1:F2} ms, p95 {2:F2} ms, max {3:F2} ms' -f `
        $Samples, $sorted[[int][Math]::Floor($Samples / 2)], $sorted[[int][Math]::Ceiling($Samples * .95) - 1], $sorted[-1]
    'Working set {0:F2} MiB; handles {1}; threads {2}' -f ($process.WorkingSet64 / 1MB), $process.HandleCount, $process.Threads.Count
} finally {
    if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id -Force }
    # Delete only the three known diagnostic files; never recurse into a
    # computed profile or touch a running user's settings.
    Remove-Item -LiteralPath $config, $ack -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $reviewDirectory -ErrorAction SilentlyContinue
}
