<#
.SYNOPSIS
Builds the face-benchmark dataset manifest from a folder of character-named
screenshots (the layout used by the game screenshot script, e.g.
\\NAS\Pictures\Games\<game>\<Character>.png with "2"/"3" suffixes for
duplicates).

The manifest references the original files in place -- no images are copied.
Characters with at least -MinCount images are split deterministically (sorted
by filename) into bank/query roles: the first min(-BankCap, count-1) files go
to bank, the rest to query.

Usage:
    powershell -ExecutionPolicy Bypass -File scripts/build_face_eval_manifest.ps1 `
        -Root <screenshot-dataset-directory>

Outputs:
    python/tests/fixtures/face_eval/dataset.json (gitignored)
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$Root,
    [string]$Out = "$PSScriptRoot\..\python\tests\fixtures\face_eval\dataset.json",
    [int]$MinCount = 2,
    [int]$BankCap = 5
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

if (-not (Test-Path -LiteralPath $Root)) {
    throw "data root not found: $Root"
}

$extensions = @(".png", ".jpg", ".jpeg", ".webp", ".bmp")
$entries = @()
$summary = @()

Get-ChildItem -LiteralPath $Root -Directory | Sort-Object Name | ForEach-Object {
    $game = $_.Name
    $images = @(
        Get-ChildItem -LiteralPath $_.FullName -File -ErrorAction SilentlyContinue |
            Where-Object { $extensions -contains $_.Extension.ToLowerInvariant() } |
            Sort-Object Name
    )
    if ($images.Count -eq 0) {
        return
    }

    # Group by character: the naming script appends "2"/"3"/... for duplicates,
    # sometimes with a space before the number ("Name 2.png").
    $groups = @{}
    foreach ($image in $images) {
        $base = [System.IO.Path]::GetFileNameWithoutExtension($image.Name)
        $name = $base.TrimEnd("0123456789".ToCharArray()).TrimEnd(" ", "`t")
        if ($groups.ContainsKey($name)) {
            $groups[$name] += $image
        } else {
            $groups[$name] = @($image)
        }
    }

    $gameCharacters = 0
    foreach ($name in ($groups.Keys | Sort-Object)) {
        $files = @($groups[$name])
        if ($files.Count -lt $MinCount) {
            continue
        }
        $gameCharacters += 1
        $bankCount = [Math]::Min($BankCap, $files.Count - 1)
        for ($index = 0; $index -lt $files.Count; $index++) {
            $role = if ($index -lt $bankCount) { "bank" } else { "query" }
            $entries += [pscustomobject]@{
                game      = $game
                character = $name
                role      = $role
                path      = $files[$index].FullName
            }
        }
    }
    if ($gameCharacters -gt 0) {
        $summary += [pscustomobject]@{
            game       = $game
            characters = $gameCharacters
            entries    = $entries.Count - ($summary | Measure-Object -Property entries -Sum).Sum
        }
    }
}

if ($entries.Count -eq 0) {
    throw "no characters with at least $MinCount images found under $Root"
}

$manifest = [ordered]@{
    version = 1
    generatedAt = (Get-Date -Format "yyyy-MM-ddTHH:mm:sszzz")
    root = $Root
    bankCap = $BankCap
    minCount = $MinCount
    entries = $entries
}

$outDir = Split-Path -Parent $Out
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$json = $manifest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($Out, $json, (New-Object System.Text.UTF8Encoding($false)))

$bankCount = @($entries | Where-Object { $_.role -eq "bank" }).Count
$queryCount = $entries.Count - $bankCount
Write-Output ("games=" + $summary.Count + " characters=" + ($summary | Measure-Object -Property characters -Sum).Sum)
Write-Output ("entries=" + $entries.Count + " bank=" + $bankCount + " query=" + $queryCount)
Write-Output ("manifest written to " + (Resolve-Path -LiteralPath $Out).Path)
