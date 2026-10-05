<#
.SYNOPSIS
    One-shot installer for the Termielle overlay.

.DESCRIPTION
    Downloads the latest release, verifies its checksum, installs to
    %LOCALAPPDATA%\Termielle\bin, adds the emitter to PATH, registers the
    crash-watchdog scheduled task, and wires up the CLI integrations it
    finds (opencode, opencode2, Claude Code, Codex, agy).

    Safe to re-run: it upgrades the binaries in place, refreshes the
    Termielle-owned configuration on every run, and never touches anything
    it does not own. Before the first modification of a user config, the
    original is saved once to %LOCALAPPDATA%\Termielle\backups, and every
    write is atomic (temp file + rename) and validated before it lands.
    What was installed is recorded in
    %LOCALAPPDATA%\Termielle\installed.json, which scripts\uninstall.ps1
    uses to reverse everything exactly.

    Usage:
        irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex

    Options (via $args or -Repo): pass -Repo owner/name to override the
    source repository, -NoStart to skip launching the overlay, -NoTask to
    skip the crash-watchdog scheduled task, -NoPath to skip the PATH entry.

.EXAMPLE
    irm https://github.com/neoslusion/termielle/releases/latest/download/install.ps1 | iex
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [string]$Repo = 'neoslusion/termielle',
    [string]$BaseUrl = '',
    [switch]$NoStart,
    [switch]$NoTask,
    [switch]$NoPath
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
$backupDir = Join-Path $dest 'backups'
$recordPath = Join-Path $dest 'installed.json'
$temp = Join-Path $env:TEMP 'termielle-install'
$zip = Join-Path $temp $asset

function Write-Step($message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Good($message) { Write-Host "    $message" -ForegroundColor Green }

# Reads a JSON file as a hashtable. A missing file is an empty hashtable; a
# malformed file is an error, never a silent reset.
function Read-Json {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return @{} }
    $raw = Get-Content -Raw -LiteralPath $Path
    if ([string]::IsNullOrWhiteSpace($raw)) { return @{} }
    try {
        return $raw | ConvertFrom-Json -AsHashtable
    } catch {
        throw "$Path is not valid JSON: $($_.Exception.Message)"
    }
}

# Writes atomically: the content lands in a same-directory temp file, is
# flushed, then renamed over the target. A crash can never leave a torn file.
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

# Saves the pre-Termielle state of a config once. Re-runs never overwrite
# it, so uninstall always has the true original to restore.
function Backup-Once {
    param([string]$Path, [string]$Name)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
    $backup = Join-Path $backupDir $Name
    if (-not (Test-Path -LiteralPath $backup)) {
        Copy-Item -LiteralPath $Path -Destination $backup -Force
    }
    return $backup
}

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

# Refreshes only Termielle command entries while preserving every other hook
# registered under the same Claude event.
function Merge-ClaudeSettings {
    param([string]$SettingsPath, [string]$FragmentPath)
    $fragment = Read-Json $FragmentPath
    if (-not $fragment.ContainsKey('hooks') -or $fragment['hooks'] -isnot [System.Collections.IDictionary]) {
        throw 'the claude fragment has no hooks object'
    }
    $settings = Read-Json $SettingsPath
    if (-not $settings.ContainsKey('hooks')) { $settings['hooks'] = @{} }
    if ($settings['hooks'] -isnot [System.Collections.IDictionary]) {
        throw 'the hooks key in settings.json is not an object; refusing to modify'
    }
    foreach ($key in $fragment['hooks'].Keys) {
        $existing = if ($settings['hooks'].ContainsKey($key)) {
            @(Get-ClaudeEntriesWithoutTermielle @($settings['hooks'][$key]))
        } else {
            @()
        }
        $settings['hooks'][$key] = @($existing) + @($fragment['hooks'][$key])
    }
    $json = $settings | ConvertTo-Json -Depth 20
    $null = $json | ConvertFrom-Json -AsHashtable
    Write-Atomic $SettingsPath $json
}

# Merges the Termielle hooks block into ~/.codex/config.toml, guarded by
# start/end markers so re-runs replace only the Termielle block and
# uninstall can remove exactly it.
function Merge-CodexConfig {
    param([string]$ConfigPath, [string]$HooksPath)
    $hooksText = Get-Content -Raw -LiteralPath $HooksPath
    $block = "# --- Termielle hooks (managed by Termielle) ---`r`n$hooksText" +
        "`r`n# --- Termielle hooks end ---`r`n"
    $existing = ''
    if (Test-Path -LiteralPath $ConfigPath) {
        $existing = Get-Content -Raw -LiteralPath $ConfigPath
    }
    $startMarker = '# --- Termielle hooks (managed by Termielle) ---'
    $endMarker = '# --- Termielle hooks end ---'
    $start = $existing.IndexOf($startMarker)
    if ($start -ge 0) {
        # Cut from the start marker to just past the end marker (or to the
        # end of the file when the block predates the end marker), then
        # consume the trailing newline so no blank line is left behind.
        $endAt = $existing.IndexOf($endMarker, $start)
        $end = if ($endAt -ge 0) { $endAt + $endMarker.Length } else { $existing.Length }
        $prefix = $existing.Substring(0, $start).TrimEnd()
        $suffix = $existing.Substring($end).TrimStart("`r", "`n")
        $existing = $prefix + "`r`n`r`n$block" +
            $(if ($suffix) { "`r`n$suffix" } else { '' })
    } else {
        $existing = $existing.TrimEnd() + "`r`n`r`n$block"
    }
    Write-Atomic $ConfigPath $existing
}

# Merges the Termielle handler into ~/.gemini/config/hooks.json (the global
# Antigravity CLI hook configuration). Termielle-owned keys are refreshed on
# every run; user-owned handlers are preserved. The result is validated
# before it replaces the file.
function Merge-AgyHooks {
    param([string]$HooksPath, [string]$FragmentPath, [string]$EmitterPath)
    $template = Get-Content -Raw -LiteralPath $FragmentPath
    # agy resolves hook commands as absolute paths only, so the fragment's
    # placeholder becomes the installed emitter's absolute path here; a bare
    # PATH name would fail with exit 127. Backslashes double first because
    # the path lands inside a JSON string.
    $escaped = $EmitterPath.Replace('\', '\\').Replace('"', '\"')
    $text = $template.Replace('{{TERMIELLE_EMIT}}', $escaped)
    $ours = $text | ConvertFrom-Json -AsHashtable
    if (-not $ours.ContainsKey('termielle')) {
        throw 'the agy fragment has no termielle handler'
    }
    $hooks = Read-Json $HooksPath
    if ($hooks -isnot [System.Collections.IDictionary]) {
        throw "$HooksPath is not a JSON object; refusing to modify"
    }
    foreach ($key in $ours.Keys) {
        $hooks[$key] = $ours[$key]
    }
    $json = $hooks | ConvertTo-Json -Depth 20
    # The serialized result must parse before it may replace the file.
    $null = $json | ConvertFrom-Json -AsHashtable
    Write-Atomic $HooksPath $json
}

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
    if (-not (Test-Path (Join-Path $bin 'termielle-emit.exe'))) {
        throw 'Release archive does not contain termielle-emit.exe'
    }
    $version = if (Test-Path (Join-Path $bin 'VERSION')) {
        (Get-Content (Join-Path $bin 'VERSION') -Raw).Trim()
    } else { 'unknown' }
    Write-Good "Termielle $version installed"

    $prior = Read-Json $recordPath
    $record = @{
        version   = 1
        bin       = $bin
        task      = [bool]$prior['task']
        path      = [bool]$prior['path']
        claude    = $prior['claude']
        codex     = $prior['codex']
        opencode  = $prior['opencode']
        opencode2 = $prior['opencode2']
        agy       = $prior['agy']
    }

    # These launchers are the control surface when Off has removed all resident
    # Termielle UI. They target only this installation, not Explorer or another app.
    if (Test-Path -LiteralPath (Join-Path $bin 'integrations\termielle-power-v1')) {
        Write-Step 'Adding on/off Start-menu shortcuts'
        $powerShortcuts = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Termielle'
        New-Item -ItemType Directory -Force -Path $powerShortcuts | Out-Null
        $shortcutShell = New-Object -ComObject WScript.Shell
        foreach ($entry in @(
            @{ Name = 'Turn Termielle On'; Argument = '--enable'; Description = 'Enable Termielle and launch with your saved settings' },
            @{ Name = 'Turn Termielle Off'; Argument = '--disable'; Description = 'Exit Termielle completely and stay off across logins' }
        )) {
            $link = Join-Path $powerShortcuts ($entry.Name + '.lnk')
            $shortcut = $shortcutShell.CreateShortcut($link)
            if ((Test-Path -LiteralPath $link) -and $shortcut.TargetPath -ine (Join-Path $bin 'termielle-app.exe')) {
                Write-Warning "Leaving unrelated shortcut unchanged: $link"
                continue
            }
            $shortcut.TargetPath = Join-Path $bin 'termielle-app.exe'
            $shortcut.Arguments = $entry.Argument
            $shortcut.WorkingDirectory = $bin
            $shortcut.Description = $entry.Description
            $shortcut.Save()
        }
    }

    if (-not $NoPath) {
        Write-Step 'Adding emitter to PATH'
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $pathEntries = @($userPath -split ';' | Where-Object { $_ })
        $alreadyPresent = $pathEntries | Where-Object {
            $_.Trim().TrimEnd('\') -ieq $bin.TrimEnd('\')
        }
        if (-not $alreadyPresent) {
            [Environment]::SetEnvironmentVariable(
                'Path',
                (($userPath.TrimEnd(';') + ';' + $bin).TrimStart(';')),
                'User'
            )
            Write-Good "Added $bin to user PATH"
            $record.path = $true
        } else {
            Write-Good 'PATH already contains the emitter directory'
        }
    }

    if (-not $NoTask) {
        Write-Step 'Registering the crash-watchdog task'
        $action = New-ScheduledTaskAction -Execute (Join-Path $bin 'termielle-app.exe')
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
        # Boot-race guard: the session is still initializing when AtLogOn
        # fires (launching then fails and exhausts the retries), so wait a
        # minute. The 3x1min restarts cover residual flakiness after that.
        $trigger.Delay = 'PT1M'
        $settings = New-ScheduledTaskSettingsSet -RestartCount 3 `
            -RestartInterval (New-TimeSpan -Minutes 1) `
            -ExecutionTimeLimit (New-TimeSpan -Days 3650) `
            -MultipleInstances IgnoreNew -StartWhenAvailable `
            -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        $task = Get-ScheduledTask -TaskName 'Termielle' -ErrorAction SilentlyContinue
        if (-not $task) {
            Register-ScheduledTask -TaskName 'Termielle' -Action $action -Trigger $trigger `
                -Settings $settings -Description 'Termielle overlay companion' -Force | Out-Null
            Write-Good 'Task registered'
            $record.task = $true
        } elseif ($task.State -eq 'Disabled') {
            Write-Good 'Task left disabled (deliberate opt-out untouched)'
        } else {
            $logon = @($task.Triggers | Where-Object { $_.CimClass.CimClassName -match 'Logon' })
            $needsDelay = $logon.Count -eq 0 -or $logon[0].Delay -notmatch '^PT1M'
            $needsPowerPolicy = $task.Settings.DisallowStartIfOnBatteries -or $task.Settings.StopIfGoingOnBatteries
            if ($needsDelay -or $needsPowerPolicy) {
                Set-ScheduledTask -TaskName 'Termielle' -Action $action -Trigger $trigger `
                    -Settings $settings | Out-Null
                Write-Good 'Task upgraded with current power and logon settings'
            } else {
                Write-Good 'Task already registered'
            }
            $record.task = [bool]$prior['task']
        }
    }

# The overlay is a user-facing status surface, not a background maintenance
# job. It must remain available after AC power is disconnected.

    # The release archive extracts its payload (including integrations/) into
    # the bin directory.
    $integrations = Join-Path $bin 'integrations'
    if (Test-Path $integrations) {
        Write-Step 'Wiring CLI integrations'

        $opencodeDir = Join-Path $env:USERPROFILE '.config\opencode\plugins'
        if ((Test-Path $opencodeDir) -or (Get-Command opencode -ErrorAction SilentlyContinue) -or (Get-Command opencode2 -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $opencodeDir | Out-Null
            $pluginTarget = Join-Path $opencodeDir 'termielle.plugin.ts'
            Copy-Item (Join-Path $integrations 'opencode\termielle.plugin.ts') $pluginTarget -Force
            Write-Good 'opencode plugin installed (restart opencode to load it)'
            $record.opencode = @{ plugin = $pluginTarget }
        }

        # OpenCode 2 reads the same plugin directories; its variant emits
        # under the distinct `opencode2` source word so sessions from both
        # runtimes stay apart when they run side by side.
        if ((Get-Command opencode2 -ErrorAction SilentlyContinue) -or (Test-Path (Join-Path $env:USERPROFILE '.config\opencode2'))) {
            New-Item -ItemType Directory -Force -Path $opencodeDir | Out-Null
            $v2PluginTarget = Join-Path $opencodeDir 'termielle.opencode2.plugin.ts'
            Copy-Item (Join-Path $integrations 'opencode2\termielle.plugin.ts') $v2PluginTarget -Force
            Write-Good 'OpenCode 2 plugin installed (restart opencode2 to load it)'
            $record.opencode2 = @{ plugin = $v2PluginTarget }
        }

        $claudeSettings = Join-Path $env:USERPROFILE '.claude\settings.json'
        $claudeDir = Join-Path $env:USERPROFILE '.claude'
        if ((Test-Path $claudeDir) -or (Get-Command claude -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $claudeDir | Out-Null
            $claudeBackup = Backup-Once -Path $claudeSettings -Name 'claude.settings.json.pre'
            Merge-ClaudeSettings -SettingsPath $claudeSettings `
                -FragmentPath (Join-Path $integrations 'claude\settings.fragment.json')
            Write-Good 'Claude Code hooks installed'
            $record.claude = @{ settings = $claudeSettings; backup = $claudeBackup }
        }

        $codexConfig = Join-Path $env:USERPROFILE '.codex\config.toml'
        $codexDir = Join-Path $env:USERPROFILE '.codex'
        if ((Test-Path $codexDir) -or (Get-Command codex -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path $codexDir | Out-Null
            $codexBackup = Backup-Once -Path $codexConfig -Name 'codex.config.toml.pre'
            Merge-CodexConfig -ConfigPath $codexConfig `
                -HooksPath (Join-Path $integrations 'codex\hooks.toml')
            Write-Good 'Codex hooks installed'
            $record.codex = @{ config = $codexConfig; backup = $codexBackup }
        }

        $agyHooks = Join-Path $env:USERPROFILE '.gemini\config\hooks.json'
        if ((Test-Path (Join-Path $env:USERPROFILE '.gemini')) -or (Get-Command agy -ErrorAction SilentlyContinue)) {
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $agyHooks) | Out-Null
            $agyBackup = Backup-Once -Path $agyHooks -Name 'agy.hooks.json.pre'
            Merge-AgyHooks -HooksPath $agyHooks `
                -FragmentPath (Join-Path $integrations 'agy\hooks.fragment.json') `
                -EmitterPath (Join-Path $bin 'termielle-emit.exe')
            Write-Good 'Antigravity CLI (agy) hooks installed'
            $record.agy = @{ hooks = $agyHooks; backup = $agyBackup }
        }

    }

    Write-Atomic $recordPath ($record | ConvertTo-Json -Depth 10)

    $powerDisabled = Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.termielle\disabled')
    if ($powerDisabled) {
        Write-Good 'Termielle left off (deliberate opt-out preserved); use Start > Turn Termielle On to resume'
    }
    if (-not $NoStart -and -not $powerDisabled -and -not (Get-Process 'termielle-app' -ErrorAction SilentlyContinue)) {
        Write-Step 'Starting the overlay'
        if (Get-ScheduledTask -TaskName 'Termielle' -ErrorAction SilentlyContinue) {
            Start-ScheduledTask -TaskName 'Termielle' | Out-Null
        } else {
            Start-Process (Join-Path $bin 'termielle-app.exe')
        }
        Write-Good 'Overlay is running; look for the character and the tray icon'
    }
    Write-Host "  - Uninstall:   pwsh -File $(Join-Path $bin 'uninstall.ps1')"
    Write-Host ''
    Write-Host 'Termielle installed.' -ForegroundColor Green
    Write-Host "  - Binaries:    $bin"
    Write-Host "  - Config:      $(Join-Path $env:USERPROFILE '.termielle\config.json')"
    Write-Host '  - Restart opencode / opencode2 once if you use them, so the plugins load.'
    Write-Host '  - Restart any running agy session so its hooks reload.'
    Write-Host "  - Uninstall:   pwsh -File $(Join-Path $PSScriptRoot 'uninstall.ps1')"
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
