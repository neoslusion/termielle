<#
.SYNOPSIS
    Removes the Termielle overlay and reverses its installer's changes.

.DESCRIPTION
    Reverses exactly what install.ps1 did, using its install record
    (%LOCALAPPDATA%\Termielle\installed.json):

      - Stops the overlay and unregisters the crash-watchdog task.
      - Restores ~/.claude/settings.json, ~/.codex/config.toml, and
        ~/.gemini/config/hooks.json from the pre-Termielle backups the
        installer saved (falling back to a surgical removal of only the
        Termielle-owned entries).
      - Removes the opencode and OpenCode 2 plugin files.
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

function Get-ClaudeEntriesWithoutTermielle {
    param([object[]]$Entries)
    $kept = [System.Collections.Generic.List[object]]::new()
    foreach ($entry in @($Entries)) {
        if ($null -eq $entry) { continue }
        if ($entry -isnot [System.Collections.IDictionary]) {
            $kept.Add($entry)
            continue
        }
        $entryHooks = @($entry['hooks'] | Where-Object {
            -not ($_['type'] -eq 'command' -and $_['command'] -match 'termielle-emit(?:\.exe)?')
        })
        if ($entryHooks.Count -eq 0) { continue }
        $copy = @{}
        foreach ($key in $entry.Keys) { $copy[$key] = $entry[$key] }
        $copy['hooks'] = $entryHooks
        $kept.Add($copy)
    }
    return $kept.ToArray()
}

# Removes only Termielle command entries, retaining user hooks registered on
# the same Claude events.
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
        if (-not $settings['hooks'].ContainsKey($key)) { continue }
        $kept = @(Get-ClaudeEntriesWithoutTermielle @($settings['hooks'][$key]))
        if ($kept.Count -eq 0) {
            $null = $settings['hooks'].Remove($key)
        } else {
            $settings['hooks'][$key] = $kept
        }
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
    $prefix = $existing.Substring(0, $start).TrimEnd()
    $suffix = $existing.Substring($end).TrimStart("`r", "`n")
    $kept = $prefix + $(if ($suffix) { "`r`n`r`n$suffix`r`n" } else { "`r`n" })
    Write-Atomic $ConfigPath $kept
}

# Removes the Termielle handler from ~/.gemini/config/hooks.json when no
# pre-Termielle backup exists. A file left holding nothing else is removed:
# it can only have been created for Termielle's sake.
function Remove-AgyHook {
    param([string]$HooksPath)
    if (-not (Test-Path -LiteralPath $HooksPath)) { return }
    $hooks = Read-Json $HooksPath
    if ($null -eq $hooks -or $hooks -isnot [System.Collections.IDictionary]) { return }
    $null = $hooks.Remove('termielle')
    if ($hooks.Count -eq 0) {
        Remove-Item -LiteralPath $HooksPath -Force
    } else {
        Write-Atomic $HooksPath ($hooks | ConvertTo-Json -Depth 20)
    }
}

# Prefer surgical removal so edits made after installation survive. Restore the
# first-run backup only when the current file cannot be cleaned safely.
function Restore-Config {
    param(
        [string]$ConfigPath,
        [string]$Backup,
        [scriptblock]$Surgical,
        [string]$Label
    )
    if (-not (Test-Path -LiteralPath $ConfigPath)) {
        Write-Good "$Label had no config file"
        return
    }
    try {
        & $Surgical
        Write-Good "$Label cleaned without discarding newer user edits"
    } catch {
        if ($Backup -and (Test-Path -LiteralPath $Backup)) {
            Copy-Item -LiteralPath $Backup -Destination $ConfigPath -Force
            Write-Good "$Label restored from backup after surgical cleanup failed"
        } else {
            throw
        }
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
        if ($record.opencode2 -and $record.opencode2.plugin) {
            Write-Step 'Removing the OpenCode 2 plugin'
            Remove-Item -LiteralPath $record.opencode2.plugin -Force -ErrorAction SilentlyContinue
            Write-Good 'OpenCode 2 plugin removed'
        }
        if ($record.agy) {
            Write-Step 'Restoring Antigravity CLI hooks'
            Restore-Config -ConfigPath $record.agy.hooks -Backup $record.agy.backup `
                -Label 'Antigravity CLI hooks' `
                -Surgical { Remove-AgyHook $record.agy.hooks }
        }
    } else {
        Write-Host '==> No install record found; cleaning configs surgically'
        $claudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'
        $codexConfig = Join-Path $env:USERPROFILE '.codex\config.toml'
        $agyHooks = Join-Path $env:USERPROFILE '.gemini\config\hooks.json'
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
        if (Test-Path $agyHooks) {
            Write-Step 'Cleaning Antigravity CLI hooks'
            Remove-AgyHook $agyHooks
            Write-Good 'agy hooks removed'
        }
        $opencodePlugin = Join-Path $env:USERPROFILE '.config\opencode\plugins\termielle.plugin.ts'
        if (Test-Path $opencodePlugin) {
            Remove-Item -LiteralPath $opencodePlugin -Force -ErrorAction SilentlyContinue
            Write-Good 'opencode plugin removed'
        }
        $opencode2Plugin = Join-Path $env:USERPROFILE '.config\opencode\plugins\termielle.opencode2.plugin.ts'
        if (Test-Path $opencode2Plugin) {
            Remove-Item -LiteralPath $opencode2Plugin -Force -ErrorAction SilentlyContinue
            Write-Good 'OpenCode 2 plugin removed'
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
