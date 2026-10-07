#Requires -Version 7
# Compile/link only. Never inject, load artifacts, or change mod enable state.
param(
    [Parameter(Mandatory)][string]$WindhawkDirectory,
    [string]$OutputPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\termielle-windhawk-check.dll')
)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$compiler=Join-Path $WindhawkDirectory 'Compiler\bin\clang++.exe'
$engine=Get-ChildItem -LiteralPath (Join-Path $WindhawkDirectory 'Engine') -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName '64\windhawk.lib') } |
    Sort-Object Name -Descending | Select-Object -First 1
if(-not $engine){throw 'No Windhawk engine import library found'}
$null=New-Item -ItemType Directory -Force -Path (Split-Path $OutputPath -Parent)
function Compile([string[]]$arguments){
    $info=[Diagnostics.ProcessStartInfo]::new();$info.FileName=$compiler;$info.UseShellExecute=$false;$info.CreateNoWindow=$true
    foreach($argument in $arguments){$null=$info.ArgumentList.Add($argument)}
    $p=[Diagnostics.Process]::Start($info);$p.WaitForExit();$code=$p.ExitCode;$p.Dispose()
    if($code -ne 0){throw "Compilation/link failed ($code)"}
}
foreach($arch in @(@('x86_64-w64-mingw32','64',$OutputPath),@('i686-w64-mingw32','32',($OutputPath -replace '\.dll$','-x86.dll')))){
    Compile @('-target',$arch[0],'-std=c++23','-O2','-shared','-DWH_MOD','-DWH_MOD_ID=L"termielle"','-DWH_MOD_VERSION=L"0.3.0"',
        '-I',(Join-Path $WindhawkDirectory 'Compiler\include'),(Join-Path $root 'windhawk\termielle.wh.cpp'),
        (Join-Path $engine.FullName "$($arch[1])\windhawk.lib"),'-lshell32','-Wl,--export-all-symbols','-o',$arch[2])
}
Write-Output "PASS x86 manager adapter and x64 runtime adapter compiled against engine $($engine.Name); artifacts NOT loaded. Build the x64 bootstrap with cargo build -p termielle-runtime --release."
