<#
.SYNOPSIS
汇总 Scene Vault 图片读取耗时（module capture.image / ui.capture.image，event read_timing）。

.DESCRIPTION
读取 logs\perf 与 logs 目录里的 *.jsonl（性能日志默认写在 perf 子目录，兼容旧的
主目录记录；“性能日志写入独立文件”开关关闭时不会有新记录），解析每次成功读图的记录：
- 后端记录（module capture.image）带分段毫秒值：guard/item/validate/settings/thumb 与
  permit/lookup/decode/publish；kind=thumb 表示 320px 缩略图，kind=full 表示原图直读。
- 前端记录（module ui.capture.image）只有 durationMs，即前端观测到的读取耗时
  （invoke 往返，含 IPC 传输；不含 Blob 构建与 img 解码）。
脚本按 变体、缓存命中与否分组输出 count、均值、p50、p90、最大值，并可导出 CSV。

怎么读结果：
- validate 大：每次读图都在做源文件校验（当前会整文件哈希）。
- decode 大或 hit=false 多：正在生成缩略图，属于首次浏览或缓存失效。
- permit 大：在等生成许可（并发上限为图片处理核心设置，1-4）。
- lookup 大：缓存命中路径的指纹与哈希校验偏慢（含网络盘/杀软扫描）。
- 前端 durationMs 明显大于后端 total：差值来自 IPC 传输与序列化往返。

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
$serverValues = @{}
$clientValues = @{}
$fileCount = 0
$recordCount = 0

# The performance records live in a dedicated perf/ subdirectory; the main
# directory is still scanned so older data keeps working.
$scanDirectories = @($LogDirectory, (Join-Path $LogDirectory 'perf')) | Where-Object { Test-Path -LiteralPath $_ }
$scanFiles = foreach ($directory in $scanDirectories) {
    Get-ChildItem -LiteralPath $directory -Filter '*.jsonl' -File
}
foreach ($file in $scanFiles) {
    $fileCount++
    foreach ($line in Get-Content -LiteralPath $file.FullName) {
        if ($line -notmatch 'read_timing' -or $line -notmatch 'capture\.image') {
            continue
        }
        $record = $null
        try {
            $record = $line | ConvertFrom-Json
        } catch {
            continue
        }
        if ($record.event -ne 'read_timing') { continue }
        $module = [string]$record.module
        if ($module -notlike '*capture.image*') { continue }
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
        if ($module -eq 'capture.image') {
            $kind = 'thumb'
            if ($values.ContainsKey('kind')) { $kind = $values['kind'] }
            $hit = 'n/a'
            if ($values.ContainsKey('hit')) { $hit = $values['hit'] }
            $group = "$kind hit=$hit"
            if (-not $serverValues.ContainsKey($group)) { $serverValues[$group] = @{} }
            foreach ($segment in $segments) {
                if (-not $values.ContainsKey($segment)) { continue }
                $number = ConvertTo-Double $values[$segment]
                if ($null -eq $number) { continue }
                if (-not $serverValues[$group].ContainsKey($segment)) {
                    $serverValues[$group][$segment] = [System.Collections.Generic.List[double]]::new()
                }
                $serverValues[$group][$segment].Add($number)
            }
        } else {
            $operation = [string]$record.operationId
            if (-not $operation) { $operation = 'ui.capture.image' }
            if (-not $clientValues.ContainsKey($operation)) {
                $clientValues[$operation] = [System.Collections.Generic.List[double]]::new()
            }
            if ($null -ne $record.durationMs) {
                $clientValues[$operation].Add([double]$record.durationMs)
            }
        }
    }
}

Write-Output ("日志目录：{0}" -f $LogDirectory)
Write-Output ("扫描文件：{0} 个；匹配记录：{1} 条" -f $fileCount, $recordCount)
if ($recordCount -eq 0) {
    Write-Output ''
    Write-Output '没有匹配记录。先在应用里浏览截图网格，再重跑本脚本；'
    Write-Output '确认应用版本包含 capture.image / read_timing 打点（日志级别 debug）。'
    return
}

$rows = [System.Collections.Generic.List[object]]::new()
foreach ($group in ($serverValues.Keys | Sort-Object)) {
    foreach ($segment in $segments) {
        if (-not $serverValues[$group].ContainsKey($segment)) { continue }
        $stats = Get-Stats $serverValues[$group][$segment]
        if (-not $stats) { continue }
        $rows.Add([pscustomobject]@{
            Scope   = 'server'
            Group   = $group
            Segment = $segment
            Count   = $stats.Count
            Mean    = $stats.Mean
            P50     = $stats.P50
            P90     = $stats.P90
            Max     = $stats.Max
        })
    }
}
foreach ($operation in ($clientValues.Keys | Sort-Object)) {
    $stats = Get-Stats $clientValues[$operation]
    if (-not $stats) { continue }
    $rows.Add([pscustomobject]@{
        Scope   = 'client'
        Group   = $operation
        Segment = 'duration'
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
    Format-Table Scope, Group, Segment, Count,
        @{ Label = 'Mean'; Expression = { Format-Ms $_.Mean } },
        @{ Label = 'P50'; Expression = { Format-Ms $_.P50 } },
        @{ Label = 'P90'; Expression = { Format-Ms $_.P90 } },
        @{ Label = 'Max'; Expression = { Format-Ms $_.Max } } -AutoSize

Write-Output '单位：毫秒。server 为 Rust 侧分段，client 为前端观测到的 invoke 往返耗时。'

if ($Csv) {
    $rows | Sort-Object Scope, Group, Segment | Export-Csv -LiteralPath $Csv -NoTypeInformation -Encoding utf8
    Write-Output ("已导出 CSV：{0}" -f $Csv)
}
