#Requires -Version 7
<# User-invoked frontend switch. Changes only Termielle's own Windhawk enable bit.
   Preserves profiles, persistent Off, other mods and the standalone task definition.
   Graceful stop only; no taskbar/Explorer manipulation or force termination. #>
param(
    [Parameter(Mandatory)][ValidateSet('Native','Windhawk')][string]$Edition,
    [string]$ManifestPath = (Join-Path $env:LOCALAPPDATA 'Termielle\windhawk\edition.json')
)
$ErrorActionPreference = 'Stop'
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ($manifest.ModId -ne 'termielle') { throw 'Unsupported mod identity' }
$native = [IO.Path]::GetFullPath($manifest.NativeExecutable)
$windhawk = [IO.Path]::GetFullPath($manifest.WindhawkExecutable)
$ini = [IO.Path]::GetFullPath($manifest.ModConfig)
$hostExe = [IO.Path]::GetFullPath($manifest.HostExecutable)
$hosting = [string]$manifest.Hosting
if ($hosting -notin @('', 'LegacyX64', 'WindhawkX86')) { throw 'Unsupported hosting mode' }
$expectedInclude = if ($hosting -eq 'WindhawkX86') { 'windhawk.exe' } else { 'windhawk.exe|termielle-windhawk-host.exe' }
$hostName = if ($hosting -eq 'WindhawkX86') { 'windhawk.exe' } else { 'termielle-windhawk-host.exe' }
if ($hosting -eq 'WindhawkX86' -and $hostExe -ine $windhawk) { throw 'Consolidated host must be the registered Windhawk executable' }
if (-not (Test-Path -LiteralPath $native) -or -not (Test-Path -LiteralPath $windhawk)) { throw 'An edition executable is missing' }
if (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.termielle\disabled')) {
    throw 'Termielle is persistently Off. Explicitly run termielle-app.exe --enable-only before choosing an edition.'
}
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class TermielleEditionControl {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    public static uint[] VisibleOverlayPids() { var ids=new List<uint>(); EnumWindows((h,p) => {var name=new StringBuilder(128);GetClassName(h,name,128);if(name.ToString()=="termielle_overlay" && IsWindowVisible(h)){uint pid;GetWindowThreadProcessId(h,out pid);ids.Add(pid);} return true;},IntPtr.Zero);return ids.ToArray(); }
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern uint GetPrivateProfileString(string section,string key,string fallback,StringBuilder output,uint size,string file);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern bool WritePrivateProfileString(string section,string key,string value,string file);
    public static string Read(string section,string key,string file) {var text=new StringBuilder(32768);GetPrivateProfileString(section,key,"",text,32768,file);return text.ToString();}
    public static void Close(uint pid) {
        EnumWindows((h,p) => {uint owner;GetWindowThreadProcessId(h,out owner);if(owner==pid) {var name=new StringBuilder(128);GetClassName(h,name,128);if(name.ToString()=="termielle_overlay") PostMessage(h,0x0010,IntPtr.Zero,IntPtr.Zero);}return true;},IntPtr.Zero);
    }
}
'@
if ([TermielleEditionControl]::Read('Mod','LibraryFileName',$ini) -ne $manifest.LibraryFileName -or
    [TermielleEditionControl]::Read('Mod','Include',$ini) -ne $expectedInclude) { throw 'Termielle mod registration no longer matches its installation manifest' }
function Native-Processes { @(Get-Process -Name termielle-app -ErrorAction SilentlyContinue | Where-Object Path -EQ $native) }
function Tool-Processes { @(Get-CimInstance Win32_Process -Filter "Name='$hostName'" | Where-Object {
    $_.ExecutablePath -ieq $hostExe -and $_.CommandLine -match '(?:^|\s)-tool-mod\s+"?termielle"?(?:\s|$)'
}) }
function Set-ModEnabled([bool]$enable) {
    if (-not [TermielleEditionControl]::WritePrivateProfileString('Mod','Disabled',$(if($enable){'0'}else{'1'}),$ini)) { throw 'Cannot change Termielle mod enable state' }
}
function Wait-Empty([scriptblock]$get) {
    $end=[DateTime]::UtcNow.AddSeconds(20)
    while (@(& $get).Count -gt 0 -and [DateTime]::UtcNow -lt $end) { Start-Sleep -Milliseconds 250 }
    if (@(& $get).Count -gt 0) { throw 'Previous frontend did not stop gracefully. No process was force-terminated.' }
}
function Start-Native {
    $task=Get-ScheduledTask -TaskName Termielle
    if (@($task.Actions).Count -ne 1 -or $task.Actions[0].Execute -ine $native) { throw 'Standalone task action differs; refusing to change it' }
    if (@(Native-Processes).Count -eq 0) {
        $end=[DateTime]::UtcNow.AddSeconds(5)
        while ((Get-ScheduledTask -TaskName Termielle).State -eq 'Running' -and [DateTime]::UtcNow -lt $end) {Start-Sleep -Milliseconds 200}
        Start-ScheduledTask -TaskName Termielle
    }
}
if ($Edition -eq 'Native') {
    Set-ModEnabled $false
    Wait-Empty {Tool-Processes}
    Start-Native
} else {
    if (@(Tool-Processes).Count -gt 0 -and @(Native-Processes).Count -eq 0) { Write-Output 'Windhawk edition is already active'; exit 0 }
    # Disable before closing standalone, avoiding premature host collisions.
    Set-ModEnabled $false
    Wait-Empty {Tool-Processes}
    $end=[DateTime]::UtcNow.AddSeconds(20)
    while (@(Native-Processes).Count -gt 0 -and [DateTime]::UtcNow -lt $end) {
        foreach($p in Native-Processes) {[TermielleEditionControl]::Close([uint32]$p.Id)}
        Start-Sleep -Milliseconds 300
    }
    Wait-Empty {Native-Processes}
    try {
        Set-ModEnabled $true
        $manager=@(Get-CimInstance Win32_Process -Filter "Name='windhawk.exe'" | Where-Object {$_.ExecutablePath -ieq $windhawk -and $_.CommandLine -notmatch '-tool-mod|-service(?:-start|-stop)?(?:\s|$)'})
        if($manager.Count -eq 0) {Start-Process -FilePath $windhawk -ArgumentList '-tray-only'}
        $end=[DateTime]::UtcNow.AddSeconds(3)
        while (@(Tool-Processes).Count -ne 1 -and [DateTime]::UtcNow -lt $end) {Start-Sleep -Milliseconds 250}
        if (@(Tool-Processes).Count -eq 0) {
            # Portable 1.7.3's daemon is not itself a mod-launcher instance.
            # Explicitly create the documented dedicated tool process; Windhawk
            # loads only the enabled mod matching this flag, never Explorer.
            Start-Process -FilePath $hostExe -ArgumentList '-tool-mod "termielle"'
        }
        $end=[DateTime]::UtcNow.AddSeconds(15)
        while (@(Tool-Processes).Count -ne 1 -and [DateTime]::UtcNow -lt $end) {Start-Sleep -Milliseconds 250}
        if (@(Tool-Processes).Count -ne 1) { throw 'Windhawk tool host did not start' }
        Start-Sleep -Seconds 2
        if (@(Tool-Processes).Count -ne 1 -or @(Native-Processes).Count -ne 0) { throw 'Windhawk host exited or another frontend claimed startup' }
    } catch {
        $failure=$_
        Set-ModEnabled $false
        Wait-Empty {Tool-Processes}
        if (-not (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.termielle\disabled'))) {Start-Native}
        throw $failure
    }
}
$end=[DateTime]::UtcNow.AddSeconds(10)
do {
    $owners = if ($Edition -eq 'Native') {@(Native-Processes | ForEach-Object Id)} else {@(Tool-Processes | ForEach-Object ProcessId)}
    $visible = @([TermielleEditionControl]::VisibleOverlayPids() | Where-Object { $owners -contains $_ })
    if ($owners.Count -eq 1 -and $visible.Count -eq 1) { break }
    Start-Sleep -Milliseconds 250
} while ([DateTime]::UtcNow -lt $end)
if ($owners.Count -ne 1 -or $visible.Count -ne 1) { throw 'Frontend did not produce one visible overlay. Check fullscreen preferences and host logs.' }
Write-Output "Active edition: $Edition (PID $($owners[0])). Profile, persistent Off and startup task definition were not changed."
