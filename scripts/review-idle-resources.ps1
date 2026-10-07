#Requires -Version 7
param(
    [Parameter(Mandatory)][string]$HostExecutable,
    [Parameter(Mandatory)][string]$BaselineDll,
    [Parameter(Mandatory)][string]$CurrentDll,
    [ValidateRange(5,120)][int]$DurationSeconds=20,
    [ValidateRange(1,10)][int]$Runs=3,
    [string]$OutputPath
)
# Same fresh hidden disposable host for both DLLs, no live mod/shell changes.
# The host uses a private profile/pipe and stops itself normally. Never kill
# or unload a runtime if a deadline fails; report the failure instead.
$ErrorActionPreference='Stop'
$exe=(Resolve-Path -LiteralPath $HostExecutable).Path
$dlls=@{baseline=(Resolve-Path -LiteralPath $BaselineDll).Path; current=(Resolve-Path -LiteralPath $CurrentDll).Path}
$results=[Collections.Generic.List[object]]::new()
for ($run=0; $run -lt $Runs; $run++) {
    # Alternate order to reduce systematic warm-cache/order bias.
    $order=if ($run % 2 -eq 0) {@('baseline','current')} else {@('current','baseline')}
    foreach ($edition in $order) {
        $info=[Diagnostics.ProcessStartInfo]::new($exe)
        $info.UseShellExecute=$false; $info.CreateNoWindow=$true
        $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
        foreach ($arg in @($dlls[$edition],'perf',"$DurationSeconds")) {$info.ArgumentList.Add($arg)}
        $process=[Diagnostics.Process]::Start($info)
        try {
            $ready=$process.StandardOutput.ReadLineAsync()
            if (!$ready.Wait([TimeSpan]::FromSeconds(20)) -or $ready.Result -ne 'PERF_READY') {
                throw 'Hidden host did not reach its ready marker; no process was force-terminated'
            }
            $private=[Collections.Generic.List[double]]::new()
            $working=[Collections.Generic.List[double]]::new()
            $cpuLine=$process.StandardOutput.ReadLineAsync()
            $watch=[Diagnostics.Stopwatch]::StartNew()
            while (!$cpuLine.IsCompleted -and $watch.Elapsed.TotalSeconds -lt ($DurationSeconds+10)) {
                $process.Refresh()
                if ($process.HasExited) {break}
                $private.Add($process.PrivateMemorySize64/1MB); $working.Add($process.WorkingSet64/1MB)
                Start-Sleep -Milliseconds 250
            }
            if (!$cpuLine.IsCompleted -or $cpuLine.Result -notmatch '^PERF_CPU (\d+) (\d+)$') {
                throw 'No complete CPU measurement; no process was force-terminated'
            }
            $cpuMs=[double]$Matches[1]; $wallMs=[double]$Matches[2]
            if (!$process.WaitForExit(10000) -or $process.ExitCode -ne 0) {throw 'Host failed normal stop/profile preservation review'}
            $results.Add([ordered]@{
                run=$run+1; edition=$edition; dll=$dlls[$edition]
                dll_sha256=(Get-FileHash -LiteralPath $dlls[$edition]).Hash
                cpu_ms=$cpuMs; measured_seconds=$wallMs/1000
                cpu_percent_one_core=$cpuMs/$wallMs*100
                private_mib_mean=($private|Measure-Object -Average).Average
                working_set_mib_mean=($working|Measure-Object -Average).Average
            })
        } finally {$process.Dispose()}
    }
}
$report=[ordered]@{
    timestamp=(Get-Date).ToString('o'); host=$exe; host_sha256=(Get-FileHash -LiteralPath $exe).Hash
    fixture='Isolated x86/x64 DLL host (matching bitness), hidden center-only Bar, flat recording-safe glass, no side modules/face/media/tasks/toasts, private pipe/profile, five-second warm-up. Not a live Windhawk/GPU/transition benchmark.'
    samples=$results.ToArray()
}
$json=$report|ConvertTo-Json -Depth 8
if ($OutputPath) {$json|Set-Content -LiteralPath $OutputPath -Encoding utf8}
$json
