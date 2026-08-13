<#
.SYNOPSIS
    Removes the Termielle overlay and reverses its installer's changes.

.DESCRIPTION
    Reverses exactly what install.ps1 did, using its install record
    (%LOCALAPPDATA%\Termielle\installed.json):

      - Stops the overlay and unregisters the crash-watchdog task.
      - Restores ~/.claude/settings.json and ~/.codex/config.toml from the
        pre-Termielle backups the installer saved (falling back to a
        surgical removal of only the Termielle-owned keys).
      - Removes the opencode plugin file.
      - Removes the Termielle directory and its PATH entry.

    The overlay's data (~/.termielle: config, assets, logs) is kept by
    default; pass -RemoveData to delete it too.

    Usage:
        pwsh -File scripts\uninstall.ps1
        pwsh -File scripts\uninstall.ps1 -RemoveData
#>
[CmdletBinding()]
param(
    [switch]$RemoveData
)

$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Termielle'
$bin = Join-Path $dest 'bin'
$recordPath = Join-Path $dest 'installed.json'
$backupDir = Join-Path $dest 'backups'

function Write-Step($message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Good($message) { Write-Host "    $message" -ForegroundColor Green }

function Read-Json {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try {
        return Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json -AsHashtable
    } catch {
        throw "$Path is not valid JSON: $($_.Exception.Message)"
    }
}

function Write-Atomic {
    param([string]$Path, [string]$Content)
    $directory = Split-Path -Parent $Path
    if (-not (Test-Path $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $tempFile = Join-Path $directory ('.' + (Split-Path -Leaf $Path) + '.tmp-' + [guid]::NewGuid().ToString('N'))
    try {
        [System.IO.File]::WriteAllText($tempFile, $Content)
        Move-Item -LiteralPath $tempFile -Destination $Path -Force
    } finally {
        Remove-Item -LiteralPath $tempFile -ErrorAction SilentlyContinue
    }
}

# The hook keys Termielle owns in ~/.claude/settings.json. Fixed by the
# protocol contract, and used by the surgical removal even when the install
# directory (and its fragment copy) is already gone.
$TermielleHookKeys = @(
    'SessionStart', 'UserPromptSubmit', 'PermissionRequest',
    'Stop', 'StopFailure', 'SessionEnd'
)

# Removes the Termielle-owned hook keys from ~/.claude/settings.json when no
# pre-Termielle backup exists, validating the result before writing.
function Remove-ClaudeHooks {
    param([string]$SettingsPath, [string]$FragmentPath)
    $keys = $TermielleHookKeys
    if ($FragmentPath -and (Test-Path -LiteralPath $FragmentPath)) {
        $fragment = Read-Json $FragmentPath
        if ($fragment -and $fragment['hooks'] -is [System.Collections.IDictionary]) {
            $keys = @($fragment['hooks'].Keys)
        }
    }
    $settings = Read-Json $SettingsPath
    if ($null -eq $settings) {
        Write-Host '    no claude settings file to clean'
        return
    }
    if ($settings['hooks'] -isnot [System.Collections.IDictionary]) {
        Write-Host '    hooks key is not an object; leaving it alone'
        return
    }
    foreach ($key in $keys) {
        $null = $settings['hooks'].Remove($key)
    }
    if ($settings['hooks'].Count -eq 0) {
        $null = $settings.Remove('hooks')
    }
    $json = $settings | ConvertTo-Json -Depth 20
    $null = $json | ConvertFrom-Json -AsHashtable
    Write-Atomic $SettingsPath $json
}

# Removes the guarded Termielle block from ~/.codex/config.toml when no
# pre-Termielle backup exists.
function Remove-CodexBlock {
    param([string]$ConfigPath)
    if (-not (Test-Path -LiteralPath $ConfigPath)) { return }
    $existing = Get-Content -Raw -LiteralPath $ConfigPath
    $start = $existing.IndexOf('# --- Termielle hooks (managed by Termielle) ---')
    if ($start -lt 0) { return }
    $endMarker = '# --- Termielle hooks end ---'
    $endAt = $existing.IndexOf($endMarker, $start)
    $end = if ($endAt -ge 0) { $endAt + $endMarker.Length } else { $existing.Length }
    if ($end -lt $existing.Length -and $existing[$end] -eq "`r") { $end++ }
    if ($end -lt $existing.Length -and $existing[$end] -eq "`n") { $end++ }
    $kept = $existing.Substring(0, $start).TrimEnd()
    Write-Atomic $ConfigPath ($kept + "`r`n")
}

# Reverses one configuration using its pre-Termielle backup, or the
# surgical removal when no backup exists.
function Restore-Config {
    param(
        [string]$ConfigPath,
        [string]$Backup,
        [scriptblock]$Surgical,
        [string]$Label
    )
    if (Test-Path -LiteralPath $ConfigPath) {
        if ($Backup -and (Test-Path -LiteralPath $Backup)) {
            Copy-Item -LiteralPath $Backup -Destination $ConfigPath -Force
            Write-Good "$Label restored from backup"
        } else {
            & $Surgical
            Write-Good "$Label cleaned (no backup was available)"
        }
    } else {
        Write-Good "$Label had no config file"
    }
}

$record = Read-Json $recordPath

try {
    Write-Step 'Stopping the overlay'
    Stop-Process -Name 'termielle-app' -Force -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds(10)
    while (Get-Process 'termielle-app' -ErrorAction SilentlyContinue) {
        if ((Get-Date) -gt $deadline) { break }
        Start-Sleep -Milliseconds 200
    }
    Write-Good 'Overlay stopped'

    if ($record -and $record.task) {
        Write-Step 'Removing the crash-watchdog task'
        Unregister-ScheduledTask -TaskName 'Termielle' -Confirm:$false -ErrorAction SilentlyContinue
        Write-Good 'Task removed'
    } else {
        Write-Host '==> Skipping the scheduled task (not recorded as installed)'
    }

    # The release archive extracts its payload (including integrations/) into
    # the bin directory.
    $integrations = Join-Path $bin 'integrations'
    $fragment = Join-Path $integrations 'claude\settings.fragment.json'
    $hooks = Join-Path $integrations 'codex\hooks.toml'

    if ($record) {
        if ($record.claude) {
            Write-Step 'Restoring Claude Code settings'
            Restore-Config -ConfigPath $record.claude.settings -Backup $record.claude.backup `
                -Label 'Claude Code settings' `
                -Surgical { Remove-ClaudeHooks $record.claude.settings $fragment }
        }
        if ($record.codex) {
            Write-Step 'Restoring Codex config'
            Restore-Config -ConfigPath $record.codex.config -Backup $record.codex.backup `
                -Label 'Codex config' `
                -Surgical { Remove-CodexBlock $record.codex.config }
        }
        if ($record.opencode -and $record.opencode.plugin) {
            Write-Step 'Removing the opencode plugin'
            Remove-Item -LiteralPath $record.opencode.plugin -Force -ErrorAction SilentlyContinue
            Write-Good 'opencode plugin removed'
        }
    } else {
        Write-Host '==> No install record found; cleaning configs surgically'
        $claudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'
        $codexConfig = Join-Path $env:USERPROFILE '.codex\config.toml'
        if (Test-Path $claudeSettings) {
            Write-Step 'Cleaning Claude Code settings'
            Remove-ClaudeHooks $claudeSettings $fragment
            Write-Good 'Claude Code hooks removed'
        }
        if (Test-Path $codexConfig) {
            Write-Step 'Cleaning Codex config'
            Remove-CodexBlock $codexConfig
            Write-Good 'Codex hooks removed'
        }
        $opencodePlugin = Join-Path $env:USERPROFILE '.config\opencode\plugins\termielle.plugin.ts'
        if (Test-Path $opencodePlugin) {
            Remove-Item -LiteralPath $opencodePlugin -Force -ErrorAction SilentlyContinue
            Write-Good 'opencode plugin removed'
        }
    }

    Write-Step 'Removing PATH entry'
    if ($record -and $record.path) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = $userPath -split ';' | Where-Object { $_ -and $_ -ne $bin }
        [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
        Write-Good 'PATH entry removed'
    } else {
        Write-Host '    PATH entry not recorded; leaving PATH unchanged'
    }

    Write-Step "Removing $dest"
    if (Test-Path -LiteralPath $dest) {
        Remove-Item -Recurse -Force -LiteralPath $dest
    }
    Write-Good 'Termielle directory removed'

    if ($RemoveData) {
        Write-Step 'Removing ~/.termielle'
        $dataDir = Join-Path $env:USERPROFILE '.termielle'
        if (Test-Path -LiteralPath $dataDir) {
            Remove-Item -Recurse -Force -LiteralPath $dataDir
            Write-Good 'Overlay data removed'
        }
    }

    Write-Host ''
    Write-Host 'Termielle uninstalled.' -ForegroundColor Green
    if (-not $RemoveData) {
        Write-Host "  - Overlay data kept at $(Join-Path $env:USERPROFILE '.termielle') (pass -RemoveData to delete)"
    }
} finally { }
