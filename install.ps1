<#
.SYNOPSIS
    One-shot installer for the Termielle overlay.

.DESCRIPTION
    Downloads the latest release, verifies its checksum, installs to
    %LOCALAPPDATA%\Termielle, adds the emitter to PATH, registers the
    crash-watchdog scheduled task, and wires up the CLI integrations it
    finds (opencode, Claude Code, Codex). Safe to re-run: it upgrades the
    binaries in place and leaves the user configuration alone.

    Usage:
        irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex

    Options (via $args or -Repo): pass -Repo owner/name to override the
    source repository and -NoStart to skip launching the overlay.

.EXAMPLE
    irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex
#>
[CmdletBinding()]
param(
    [string]$Repo = 'neoslusion/termielle',
    [string]$BaseUrl = '',
    [switch]$NoStart
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$asset = 'termielle-windows-x64.zip'
if (-not $BaseUrl) {
    $BaseUrl = "https://github.com/$Repo/releases/latest/download"
}
$zipUrl = "$BaseUrl/$asset"
$shaUrl = "$BaseUrl/$asset.sha256"

$dest = Join-Path $env:LOCALAPPDATA 'Termielle'
$bin = Join-Path $dest 'bin'
$temp = Join-Path $env:TEMP 'termielle-install'
$zip = Join-Path $temp $asset

function Write-Step($message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Good($message) { Write-Host "    $message" -ForegroundColor Green }

try {
    New-Item -ItemType Directory -Force -Path $temp | Out-Null

    Write-Step "Downloading $asset from $Repo"
    Invoke-WebRequest -Uri $zipUrl -OutFile $zip
    $shaResponse = Invoke-WebRequest -Uri $shaUrl -UseBasicParsing
    $expected = if ($shaResponse.Content -is [string]) {
        $shaResponse.Content.Trim()
    } else {
        [System.Text.Encoding]::UTF8.GetString($shaResponse.Content).Trim()
    }

    Write-Step 'Verifying SHA256 checksum'
    $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash
    if ($actual -ne $expected) {
        throw "Checksum mismatch: expected $expected, got $actual"
    }
    Write-Good 'Checksum OK'

    Write-Step "Installing to $dest"
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    if (Test-Path $bin) {
        # Stop a running overlay first so its files are not locked, then
        # wait for the termination to settle before replacing them.
        Stop-Process -Name 'termielle-app' -Force -ErrorAction SilentlyContinue
        $deadline = (Get-Date).AddSeconds(10)
        while (Get-Process 'termielle-app' -ErrorAction SilentlyContinue) {
            if ((Get-Date) -gt $deadline) { break }
            Start-Sleep -Milliseconds 200
        }
        Remove-Item -Recurse -Force $bin
    }
    New-Item -ItemType Directory -Force -Path $bin | Out-Null
    Expand-Archive -Path $zip -DestinationPath $bin -Force
    if (-not (Test-Path (Join-Path $bin 'termielle-app.exe'))) {
        throw 'Release archive does not contain termielle-app.exe'
    }
    $version = if (Test-Path (Join-Path $bin 'VERSION')) {
        (Get-Content (Join-Path $bin 'VERSION') -Raw).Trim()
    } else { 'unknown' }
    Write-Good "Termielle $version installed"

    Write-Step 'Adding emitter to PATH'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -notlike "*$bin*") {
        [Environment]::SetEnvironmentVariable(
            'Path',
            (($userPath.TrimEnd(';') + ';' + $bin).TrimStart(';')),
            'User'
        )
        Write-Good "Added $bin to user PATH"
    } else {
        Write-Good 'PATH already contains the emitter directory'
    }

    Write-Step 'Registering the crash-watchdog task'
    $task = Get-ScheduledTask -TaskName 'Termielle' -ErrorAction SilentlyContinue
    if (-not $task) {
        $action = New-ScheduledTaskAction -Execute (Join-Path $bin 'termielle-app.exe')
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
        $settings = New-ScheduledTaskSettingsSet -RestartCount 3 `
            -RestartInterval (New-TimeSpan -Minutes 1) `
            -ExecutionTimeLimit (New-TimeSpan -Days 3650) `
            -MultipleInstances IgnoreNew -StartWhenAvailable
        Register-ScheduledTask -TaskName 'Termielle' -Action $action -Trigger $trigger `
            -Settings $settings -Description 'Termielle overlay companion' -Force | Out-Null
        Write-Good 'Task registered'
    } else {
        Write-Good 'Task already registered'
    }

    $integrations = Join-Path $dest 'integrations'
    if (Test-Path $integrations) {
        Write-Step 'Wiring CLI integrations'

        $opencode = Join-Path $env:USERPROFILE '.config\opencode\plugins'
        if ((Test-Path $opencode) -or (Get-Command opencode -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $opencode | Out-Null
            Copy-Item (Join-Path $integrations 'opencode\termielle.plugin.ts') $opencode -Force
            Write-Good 'opencode plugin installed (restart opencode to load it)'
        }

        $claudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'
        $claudeDir = Join-Path $env:USERPROFILE '.claude'
        if ((Test-Path $claudeDir) -or (Get-Command claude -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $claudeDir | Out-Null
            $fragment = Get-Content (Join-Path $integrations 'claude\settings.fragment.json') -Raw |
                ConvertFrom-Json
            $settings = @{}
            if (Test-Path $claudeSettings) {
                $settings = Get-Content $claudeSettings -Raw | ConvertFrom-Json
            }
            if (-not $settings.hooks) {
                $settings | Add-Member -NotePropertyName hooks -NotePropertyValue $fragment.hooks
                $settings | ConvertTo-Json -Depth 10 | Set-Content $claudeSettings
                Write-Good 'Claude Code hooks installed'
            } else {
                Write-Good 'Claude Code hooks already present'
            }
        }

        $codexConfig = Join-Path $env:USERPROFILE '.codex\config.toml'
        $codexDir = Join-Path $env:USERPROFILE '.codex'
        if ((Test-Path $codexDir) -or (Get-Command codex -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $codexDir | Out-Null
            $hooksText = Get-Content (Join-Path $integrations 'codex\hooks.toml') -Raw
            if (Test-Path $codexConfig) {
                if ((Get-Content $codexConfig -Raw) -notmatch '\[hooks\.') {
                    Add-Content -Path $codexConfig -Value "`n$hooksText"
                    Write-Good 'Codex hooks appended'
                } else {
                    Write-Good 'Codex hooks already present'
                }
            } else {
                Set-Content -Path $codexConfig -Value $hooksText
                Write-Good 'Codex hooks installed'
            }
        }
    }

    if (-not $NoStart -and -not (Get-Process 'termielle-app' -ErrorAction SilentlyContinue)) {
        Write-Step 'Starting the overlay'
        if (Get-ScheduledTask -TaskName 'Termielle' -ErrorAction SilentlyContinue) {
            Start-ScheduledTask -TaskName 'Termielle' | Out-Null
        } else {
            Start-Process (Join-Path $bin 'termielle-app.exe')
        }
        Write-Good 'Overlay is running; look for the character and the tray icon'
    }

    Write-Host ''
    Write-Host 'Termielle installed.' -ForegroundColor Green
    Write-Host "  - Binaries:    $bin"
    Write-Host "  - Config:      $(Join-Path $dest 'config.json')"
    Write-Host '  - Restart opencode once if you use it, so the plugin loads.'
    Write-Host '  - Uninstall:   Stop-Process termielle-app; Unregister-ScheduledTask Termielle;'
    Write-Host "                 Remove-Item -Recurse $dest; remove the bin entry from PATH."
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
