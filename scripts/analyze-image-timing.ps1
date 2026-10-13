<#
.SYNOPSIS
汇总 Scene Vault 的图片读取、显示、保存、AI 处理和应用资源性能日志。

.DESCRIPTION
读取 logs\perf 与 logs 目录里的 *.jsonl（性能日志默认写在 perf 子目录，兼容旧的
主目录记录；“性能日志写入独立文件”开关关闭时不会有新记录），解析每次成功读图的记录：
- 后端记录（module capture.image）带分段毫秒值：guard/item/validate/settings/thumb 与
  permit/lookup/decode/publish；kind=thumb 表示 320px 缩略图，kind=full 表示原图直读。
- 前端记录（module ui.capture.image）只有 durationMs，即前端观测到的读取耗时
  （invoke 往返，含 IPC 传输；不含 Blob 构建与 img 解码）。
同时读取 load_timing、归档分段、vision.request 与 app.resources 采样。
按环节、变体、缓存命中、结果分组输出次数、均值、P50、P90、最大值，并可导出 CSV。
耗时单位为 ms，内存/缓存为 MiB，CPU 为 %，线程/句柄/文件为 count。

怎么读结果：
- validate 大：每次读图都在做源文件校验（当前会整文件哈希）。
- decode 大或 hit=false 多：正在生成缩略图，属于首次浏览或缓存失效。
- permit 大：在等生成许可（并发上限为后台处理并行度，1–32）。
- lookup 大：缓存命中路径的指纹与哈希校验偏慢（含网络盘/杀软扫描）。
- 前端 durationMs 明显大于后端 total：差值包含 IPC 传输、序列化与前端调度等待。

性能日志默认在 %LOCALAPPDATA%\com.scenevault.desktop\logs\perf\perf-*.jsonl，不会出现在
应用"历史 → 调试日志"面板里；主日志目录仍会被本脚本一并扫描（旧数据）。

.PARAMETER LogDirectory
日志目录，默认 %LOCALAPPDATA%\com.scenevault.desktop\logs。

.PARAMETER Minutes
只统计最近 N 分钟的记录，0 表示全部。

.PARAMETER Csv
可选的 CSV 输出路径。

.EXAMPLE
pwsh -File scripts\analyze-image-timing.ps1 -Minutes 10

.EXAMPLE
pwsh -File scripts\analyze-image-timing.ps1 -Csv work\image-timing.csv
#>
[CmdletBinding()]
param(
    [string]$LogDirectory = (Join-Path $env:LOCALAPPDATA 'com.scenevault.desktop\logs'),
    [int]$Minutes = 0,
    [string]$Csv = ''
)

$ErrorActionPreference = 'Stop'
$invariant = [System.Globalization.CultureInfo]::InvariantCulture

if (-not (Test-Path -LiteralPath $LogDirectory)) {
    throw "日志目录不存在：$LogDirectory"
}

$since = $null
if ($Minutes -gt 0) {
    $since = [DateTime]::UtcNow.AddMinutes(-$Minutes)
}

function ConvertTo-Values([string]$message) {
    $values = @{}
    foreach ($match in [regex]::Matches($message, '([A-Za-z_]+)=([^\s]+)')) {
        $values[$match.Groups[1].Value] = $match.Groups[2].Value
    }
    return $values
}

function ConvertTo-Double([string]$value) {
    $parsed = 0.0
    if ([double]::TryParse($value, [System.Globalization.NumberStyles]::Float, $invariant, [ref]$parsed)) {
        return $parsed
    }
    return $null
}

function Get-Stats($values) {
    if ($values.Count -eq 0) {
        return $null
    }
    $sorted = @($values | Sort-Object)
    return [pscustomobject]@{
        Count = $sorted.Count
        Mean  = ($sorted | Measure-Object -Average).Average
        P50   = $sorted[[Math]::Max(0, [int][Math]::Ceiling($sorted.Count * 0.5) - 1)]
        P90   = $sorted[[Math]::Max(0, [int][Math]::Ceiling($sorted.Count * 0.9) - 1)]
        Max   = $sorted[$sorted.Count - 1]
    }
}

function Format-Ms([double]$value) {
    return $value.ToString('F1', $invariant)
}

$segments = @('total', 'guard', 'item', 'validate', 'settings', 'thumb', 'permit', 'lookup', 'decode', 'publish', 'read',
    'vstat', 'vread', 'vhash', 'vdb')
$samples = @{}
$fileCount = 0
$recordCount = 0

function Add-Sample([string]$SampleScope, [string]$Group, [string]$Segment, $Value, [string]$Unit = 'ms', [double]$Scale = 1) {
    if ($null -eq $Value) { return }
    $number = if ($Value -is [string]) { ConvertTo-Double $Value } else { [double]$Value }
    if ($null -eq $number -or [double]::IsNaN($number) -or [double]::IsInfinity($number)) { return }
    $key = "$SampleScope|$Group|$Segment|$Unit"
    if (-not $samples.ContainsKey($key)) {
        $samples[$key] = [pscustomobject]@{
            Scope = $SampleScope; Group = $Group; Segment = $Segment; Unit = $Unit
            Values = [System.Collections.Generic.List[double]]::new()
        }
    }
    $samples[$key].Values.Add($number / $Scale)
}

# The performance records live in a dedicated perf/ subdirectory; the main
# directory is still scanned so older data keeps working.
$scanDirectories = @($LogDirectory, (Join-Path $LogDirectory 'perf')) | Where-Object { Test-Path -LiteralPath $_ }
$scanFiles = foreach ($directory in $scanDirectories) {
    Get-ChildItem -LiteralPath $directory -Filter '*.jsonl' -File
}
foreach ($file in $scanFiles) {
    $fileCount++
    foreach ($line in Get-Content -LiteralPath $file.FullName) {
        if ($line -notmatch 'read_timing|load_timing|archive_resize|archive_copy|archive_total|archive_fingerprint_saved|vision\.request|app\.resources') {
            continue
        }
        $record = $null
        try {
            $record = $line | ConvertFrom-Json
        } catch {
            continue
        }
        $module = [string]$record.module
        if ($since) {
            $stamp = $null
            try {
                $stamp = [DateTimeOffset]::Parse([string]$record.timestamp, $invariant).UtcDateTime
            } catch {
                continue
            }
            if ($stamp -lt $since) { continue }
        }
        $recordCount++
        $values = ConvertTo-Values ([string]$record.message)
        $variant = if ($values.ContainsKey('variant')) { $values['variant'] } else { 'unknown' }
        $outcome = if ($record.outcome) { [string]$record.outcome } else { 'unknown' }
        if ($module -eq 'capture.image' -and $record.event -eq 'read_timing') {
            $kind = 'thumb'
            if ($values.ContainsKey('kind')) { $kind = $values['kind'] }
            $hit = 'n/a'
            if ($values.ContainsKey('hit')) { $hit = $values['hit'] }
            $group = "$kind variant=$variant hit=$hit outcome=$outcome"
            foreach ($segment in $segments) {
                if (-not $values.ContainsKey($segment)) { continue }
                Add-Sample 'server' $group $segment $values[$segment]
            }
        } elseif ($module -eq 'ui.capture.image' -and $record.event -eq 'read_timing') {
            $operation = [string]$record.operationId
            if (-not $operation) { $operation = 'ui.capture.image' }
            Add-Sample 'client' "$operation variant=$variant outcome=$outcome" 'duration' $record.durationMs
        } elseif ($module -eq 'ui.capture.image' -and $record.event -eq 'load_timing') {
            $group = "$($values['kind']) variant=$variant outcome=$outcome"
            foreach ($segment in $values.Keys) {
                if ($segment -like '*_ms') { Add-Sample 'client_load' $group $segment $values[$segment] }
            }
        } elseif ($module -eq 'vision.request') {
            $group = "mode=$($values['mode']) outcome=$($values['outcome'])"
            foreach ($segment in $values.Keys) {
                if ($segment -like '*_ms') { Add-Sample 'vision' $group $segment $values[$segment] }
            }
        } elseif ($module -eq 'capture.archive') {
            $group = "$($record.event) kind=$($record.fileKind) policy=$($record.policy) outcome=$outcome"
            Add-Sample 'archive' $group 'duration' $record.durationMs
            if ($record.metrics) {
                foreach ($metric in $record.metrics.PSObject.Properties) {
                    Add-Sample 'archive' $group $metric.Name $metric.Value
                }
            }
        } elseif ($module -eq 'app.resources' -and $record.event -eq 'process_sample') {
            $processes = $record.metrics.processes
            $details = $record.metrics.details
            $groups = @($processes.groups) + @([pscustomobject]@{
                role = 'total'; workingSetBytes = $processes.totalWorkingSetBytes
                privateBytes = $processes.totalPrivateBytes; cpuPercent = $processes.totalCpuPercent
            })
            foreach ($group in $groups) {
                Add-Sample 'resources' $group.role 'working_set' $group.workingSetBytes 'MiB' (1MB)
                Add-Sample 'resources' $group.role 'private_bytes' $group.privateBytes 'MiB' (1MB)
                Add-Sample 'resources' $group.role 'cpu' $group.cpuPercent '%'
                Add-Sample 'resources' $group.role 'processes' $group.processCount 'count'
            }
            Add-Sample 'resources' 'rust' 'threads' $details.rustThreads 'count'
            Add-Sample 'resources' 'rust' 'handles' $details.rustHandles 'count'
            Add-Sample 'resources' 'rust' 'cpu_seconds' $details.rustCpuSeconds 's'
            Add-Sample 'resources' 'system' 'free_memory' $details.systemFreePhysicalBytes 'MiB' (1MB)
            Add-Sample 'resources' 'system' 'total_memory' $details.systemTotalPhysicalBytes 'MiB' (1MB)
            Add-Sample 'sampler' 'process' 'duration' $record.durationMs
        } elseif ($module -eq 'app.resources' -and $record.event -eq 'storage_sample') {
            foreach ($entry in $record.metrics.entries) {
                $group = "$($entry.kind) complete=$($entry.complete)"
                Add-Sample 'storage' $group 'bytes' $entry.bytes 'MiB' (1MB)
                Add-Sample 'storage' $group 'files' $entry.files 'count'
            }
            Add-Sample 'sampler' 'storage' 'duration' $record.durationMs
        }
    }
}

Write-Output ("日志目录：{0}" -f $LogDirectory)
Write-Output ("扫描文件：{0} 个；匹配记录：{1} 条" -f $fileCount, $recordCount)
if ($recordCount -eq 0) {
    Write-Output ''
    Write-Output '没有匹配记录。确认性能日志开关已开启，并在应用中浏览、处理或归档图片；'
    Write-Output '应用进程资源每 60 秒采样，缓存和数据库体积每 5 分钟采样。'
    return
}

$rows = [System.Collections.Generic.List[object]]::new()
foreach ($sample in $samples.Values) {
    $stats = Get-Stats $sample.Values
    if (-not $stats) { continue }
    $rows.Add([pscustomobject]@{
        Scope   = $sample.Scope
        Group   = $sample.Group
        Segment = $sample.Segment
        Unit    = $sample.Unit
        Count   = $stats.Count
        Mean    = $stats.Mean
        P50     = $stats.P50
        P90     = $stats.P90
        Max     = $stats.Max
    })
}

if ($rows.Count -eq 0) {
    Write-Output '匹配记录里没有可用数值。'
    return
}

Write-Output ''
$rows |
    Sort-Object Scope, Group, Segment |
    Format-Table Scope, Group, Segment, Unit, Count,
        @{ Label = 'Mean'; Expression = { Format-Ms $_.Mean } },
        @{ Label = 'P50'; Expression = { Format-Ms $_.P50 } },
        @{ Label = 'P90'; Expression = { Format-Ms $_.P90 } },
        @{ Label = 'Max'; Expression = { Format-Ms $_.Max } } -AutoSize

Write-Output '单位见 Unit 列。client 为 invoke 往返，client_load 为前端解码/显示就绪；resources/storage 为后台采样。'

if ($Csv) {
    $rows | Sort-Object Scope, Group, Segment | Export-Csv -LiteralPath $Csv -NoTypeInformation -Encoding utf8
    Write-Output ("已导出 CSV：{0}" -f $Csv)
}
