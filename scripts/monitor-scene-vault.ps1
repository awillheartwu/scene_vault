<#
.SYNOPSIS
在 Scene Vault 日常使用期间采样内存占用与缓存体积，输出 CSV 供事后分析。

.DESCRIPTION
每个采样间隔写一行 CSV，记录：
- Scene Vault 主进程（Rust）的 Working Set / Private Bytes / 累计 CPU 时间、线程与句柄数
- Python 视觉 worker（scene-vault-ai）的进程数与内存合计
- 归属 Scene Vault 的 WebView2 进程数与内存合计（按命令行里的应用数据目录匹配，
  不会把系统中其它程序的 WebView2 进程算进来）
- capture-output、缩略图、日志、WebView2 磁盘缓存、主库与 WAL 的体积
- 系统可用物理内存

脚本只读取数据，不改动应用、数据库或缓存。想标注"当时在做什么"，把文字写进输出目录里的
note.txt，下一行采样就会带上它（改一次可以连续多行生效）。

.PARAMETER IntervalSeconds
采样间隔，默认 60 秒。

.PARAMETER DurationMinutes
运行时长，默认 0 表示一直采样到 Ctrl+C。

.PARAMETER OutputDirectory
输出目录，默认 %LOCALAPPDATA%\com.scenevault.desktop\diagnostics。

.PARAMETER Note
整次运行的备注，会写在每一行；note.txt 存在时以 note.txt 为准。

.EXAMPLE
pwsh -File scripts\monitor-scene-vault.ps1

.EXAMPLE
pwsh -File scripts\monitor-scene-vault.ps1 -IntervalSeconds 30 -Note "连续截图"

.EXAMPLE
pwsh -File scripts\monitor-scene-vault.ps1 -DurationMinutes 120
#>
[CmdletBinding()]
param(
    [int]$IntervalSeconds = 60,
    [int]$DurationMinutes = 0,
    [string]$OutputDirectory = (Join-Path $env:LOCALAPPDATA 'com.scenevault.desktop\diagnostics'),
    [string]$Note = '',
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

$appLocalRoot = Join-Path $env:LOCALAPPDATA 'com.scenevault.desktop'
$appDataRoot = Join-Path $env:APPDATA 'com.scenevault.desktop'
$captureOutputRoot = Join-Path $appLocalRoot 'capture-output'
$webviewMarker = 'com.scenevault.desktop'

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$csvPath = Join-Path $OutputDirectory "scene-vault-usage-$stamp.csv"
$metaPath = Join-Path $OutputDirectory "scene-vault-usage-$stamp.meta.json"
$notePath = Join-Path $OutputDirectory 'note.txt'

$columns = @(
    'timestamp', 'note', 'elapsed_s',
    'rust_running', 'rust_pid', 'rust_ws_mb', 'rust_priv_mb', 'rust_cpu_s', 'rust_threads', 'rust_handles',
    'python_count', 'python_ws_mb', 'python_priv_mb', 'python_largest_ws_mb', 'python_cpu_s',
    'webview_count', 'webview_ws_mb', 'webview_priv_mb',
    'app_total_ws_mb', 'app_total_priv_mb',
    'capture_output_mb', 'capture_output_files', 'capture_output_entries',
    'thumbnails_mb', 'thumbnails_files',
    'logs_mb', 'logs_files',
    'webview_disk_mb',
    'db_mb', 'db_wal_mb',
    'system_free_mb', 'system_total_mb', 'sample_ms'
)

function Convert-Mb([double]$bytes) {
    return [math]::Round($bytes / 1MB, 1)
}

function Get-DirectoryStats([string]$path) {
    $stats = [pscustomobject]@{ mb = 0.0; files = 0; entries = 0; exists = $false }
    if (-not (Test-Path -LiteralPath $path)) {
        return $stats
    }
    $stats.exists = $true
    $stats.entries = @(Get-ChildItem -LiteralPath $path -Force -ErrorAction SilentlyContinue).Count
    $files = @(Get-ChildItem -LiteralPath $path -Recurse -Force -File -ErrorAction SilentlyContinue)
    $stats.files = $files.Count
    $sum = ($files | Measure-Object -Property Length -Sum).Sum
    if ($null -ne $sum) {
        $stats.mb = Convert-Mb $sum
    }
    return $stats
}

function Get-FileSizeMb([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) {
        return 0.0
    }
    try {
        return Convert-Mb (Get-Item -LiteralPath $path -Force).Length
    }
    catch {
        return 0.0
    }
}

function Get-SceneVaultProcesses {
    $rust = @(Get-Process -Name 'scene_vault' -ErrorAction SilentlyContinue)
    $python = @(Get-Process -Name 'scene-vault-ai' -ErrorAction SilentlyContinue)
    $webview = @()
    try {
        $candidates = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue)
        foreach ($candidate in $candidates) {
            if (-not $candidate.CommandLine) {
                continue
            }
            if (-not $candidate.CommandLine.Contains($webviewMarker)) {
                continue
            }
            $process = Get-Process -Id $candidate.ProcessId -ErrorAction SilentlyContinue
            if ($process) {
                $webview += $process
            }
        }
    }
    catch {
        $webview = @()
    }
    return [pscustomobject]@{ rust = $rust; python = $python; webview = $webview }
}

function Get-SystemMemory {
    try {
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
        return [pscustomobject]@{
            free_mb = [math]::Round([double]$os.FreePhysicalMemory / 1024, 1)
            total_mb = [math]::Round([double]$os.TotalVisibleMemorySize / 1024, 1)
        }
    }
    catch {
        return [pscustomobject]@{ free_mb = 0.0; total_mb = 0.0 }
    }
}

function Write-CsvLine([string[]]$values) {
    $escaped = foreach ($value in $values) {
        $text = [string]$value
        '"' + ($text -replace '"', '""') + '"'
    }
    Add-Content -LiteralPath $csvPath -Value ($escaped -join ',') -Encoding utf8
}

function Get-LiveNote {
    if (Test-Path -LiteralPath $notePath) {
        try {
            $text = (Get-Content -LiteralPath $notePath -Raw -ErrorAction Stop).Trim()
            if ($text) {
                return $text
            }
        }
        catch {
            return $Note
        }
    }
    return $Note
}

function Get-RunningExecutables {
    $list = @()
    foreach ($process in @(Get-Process -Name 'scene_vault' -ErrorAction SilentlyContinue)) {
        $path = $null
        try {
            $path = $process.Path
        }
        catch {
            $path = $null
        }
        $version = $null
        if ($path) {
            try {
                $version = (Get-Item -LiteralPath $path).VersionInfo.FileVersion
            }
            catch {
                $version = $null
            }
        }
        $started = $null
        try {
            $started = $process.StartTime.ToString('o')
        }
        catch {
            $started = $null
        }
        $list += [pscustomobject]@{ path = $path; version = $version; id = $process.Id; startTime = $started }
    }
    return $list
}

$systemInfo = Get-SystemMemory
$meta = [pscustomobject]@{
    startedAt = (Get-Date).ToString('o')
    note = $Note
    intervalSeconds = $IntervalSeconds
    durationMinutes = $DurationMinutes
    appLocalRoot = $appLocalRoot
    appDataRoot = $appDataRoot
    runningExecutables = @(Get-RunningExecutables)
    machine = [pscustomobject]@{
        name = $env:COMPUTERNAME
        user = $env:USERNAME
        os = [System.Environment]::OSVersion.VersionString
        logicalProcessors = [System.Environment]::ProcessorCount
        totalMemoryMb = $systemInfo.total_mb
    }
}
$meta | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $metaPath -Encoding utf8

$stopHint = if ($DurationMinutes -gt 0) { "Ctrl+C，或 $DurationMinutes 分钟后自动停止" } else { "Ctrl+C" }
if (-not $Quiet) {
    Write-Output 'Scene Vault 使用采样已开始'
    Write-Output "  CSV:    $csvPath"
    Write-Output "  元数据: $metaPath"
    Write-Output "  间隔:   $IntervalSeconds 秒；结束: $stopHint"
    Write-Output "  备注文件: $notePath（写入文字即可标注当前在做什么）"
    Write-Output ''
}

$startedAt = Get-Date
$deadline = if ($DurationMinutes -gt 0) { $startedAt.AddMinutes($DurationMinutes) } else { $null }
$sampleCount = 0

try {
    while ($true) {
        $sampleStarted = Get-Date
        $sampleCount += 1

        $processes = Get-SceneVaultProcesses
        $rust = @($processes.rust)
        $python = @($processes.python)
        $webview = @($processes.webview)
        $rustProcess = if ($rust.Count -gt 0) { $rust[0] } else { $null }

        $rustCpu = 0.0
        $rustThreads = 0
        $rustHandles = 0
        $rustPid = ''
        if ($rustProcess) {
            $rustPid = $rustProcess.Id
            $rustThreads = $rustProcess.Threads.Count
            $rustHandles = $rustProcess.HandleCount
            try {
                $rustCpu = [math]::Round($rustProcess.CPU, 1)
            }
            catch {
                $rustCpu = 0.0
            }
        }

        $pythonWs = 0.0
        $pythonPriv = 0.0
        $pythonCpu = 0.0
        $pythonLargest = 0.0
        foreach ($process in $python) {
            $pythonWs += $process.WorkingSet64
            $pythonPriv += $process.PrivateMemorySize64
            if ($process.WorkingSet64 -gt $pythonLargest) {
                $pythonLargest = $process.WorkingSet64
            }
            try {
                $pythonCpu += $process.CPU
            }
            catch {
            }
        }

        $webviewWs = 0.0
        $webviewPriv = 0.0
        foreach ($process in $webview) {
            $webviewWs += $process.WorkingSet64
            $webviewPriv += $process.PrivateMemorySize64
        }

        $rustWs = if ($rustProcess) { [double]$rustProcess.WorkingSet64 } else { 0.0 }
        $rustPriv = if ($rustProcess) { [double]$rustProcess.PrivateMemorySize64 } else { 0.0 }

        $capture = Get-DirectoryStats $captureOutputRoot
        $thumbnails = Get-DirectoryStats (Join-Path $appLocalRoot 'thumbnails')
        $logs = Get-DirectoryStats (Join-Path $appLocalRoot 'logs')
        $webviewDisk = Get-DirectoryStats (Join-Path $appLocalRoot 'EBWebView')
        $system = Get-SystemMemory

        $values = @(
            (Get-Date).ToString('o'),
            (Get-LiveNote),
            [math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1),
            ($(if ($rustProcess) { 1 } else { 0 })),
            $rustPid,
            (Convert-Mb $rustWs),
            (Convert-Mb $rustPriv),
            $rustCpu,
            $rustThreads,
            $rustHandles,
            $python.Count,
            (Convert-Mb $pythonWs),
            (Convert-Mb $pythonPriv),
            (Convert-Mb $pythonLargest),
            [math]::Round($pythonCpu, 1),
            $webview.Count,
            (Convert-Mb $webviewWs),
            (Convert-Mb $webviewPriv),
            (Convert-Mb ($rustWs + $pythonWs + $webviewWs)),
            (Convert-Mb ($rustPriv + $pythonPriv + $webviewPriv)),
            $capture.mb,
            $capture.files,
            $capture.entries,
            $thumbnails.mb,
            $thumbnails.files,
            $logs.mb,
            $logs.files,
            $webviewDisk.mb,
            (Get-FileSizeMb (Join-Path $appDataRoot 'scene-vault.db')),
            (Get-FileSizeMb (Join-Path $appDataRoot 'scene-vault.db-wal')),
            $system.free_mb,
            $system.total_mb,
            [math]::Round(((Get-Date) - $sampleStarted).TotalMilliseconds, 0)
        )

        if ($sampleCount -eq 1) {
            Write-CsvLine ([string[]]$columns)
        }
        Write-CsvLine ([string[]]$values)

        if (-not $Quiet) {
            $progress = '[{0}] 主进程 {1}/{2}MB  Python {3}MB/{4}进程  WebView2 {5}MB/{6}进程  缓存 {7}MB/{8}个' -f (Get-Date).ToString('HH:mm:ss'), (Convert-Mb $rustWs), (Convert-Mb $rustPriv), (Convert-Mb $pythonWs), $python.Count, (Convert-Mb $webviewWs), $webview.Count, $capture.mb, $capture.entries
            Write-Output $progress
        }

        if ($deadline -and (Get-Date) -ge $deadline) {
            break
        }
        Start-Sleep -Seconds $IntervalSeconds
    }
}
finally {
    $rows = @()
    if (Test-Path -LiteralPath $csvPath) {
        $rows = @(Import-Csv -LiteralPath $csvPath)
    }
    if ($rows.Count -gt 0) {
        $first = $rows[0]
        $last = $rows[-1]
        Write-Output ''
        Write-Output ('采样结束：{0} 行，{1} → {2}' -f $rows.Count, $first.timestamp, $last.timestamp)
        Write-Output ('  主进程 Working Set: {0} → {1} MB' -f $first.rust_ws_mb, $last.rust_ws_mb)
        Write-Output ('  Python worker:      {0} → {1} MB' -f $first.python_ws_mb, $last.python_ws_mb)
        Write-Output ('  WebView2(本应用):   {0} → {1} MB' -f $first.webview_ws_mb, $last.webview_ws_mb)
        Write-Output ('  capture-output:     {0} → {1} MB（{2} → {3} 个条目）' -f $first.capture_output_mb, $last.capture_output_mb, $first.capture_output_entries, $last.capture_output_entries)
        Write-Output ''
        Write-Output "把 CSV 和分析一起看即可：$csvPath"
        Write-Output '（同目录 .meta.json 记录了应用路径与版本）'
    }
    else {
        Write-Output '没有采集到任何样本'
    }
}
