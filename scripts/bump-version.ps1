<#
.SYNOPSIS
Bumps the unified Scene Vault version across all four version files and the
Cargo.lock root entry.

Version lives in: package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json,
python/pyproject.toml (the Cargo.lock root package stays in sync). The release
script (release-windows.ps1) enforces that all four files match.

.PARAMETER Mode
patch (default) | minor | major - computed from the current package.json version.
  1.0.0 + patch -> 1.0.1; minor -> 1.1.0; major -> 2.0.0

.PARAMETER Version
Explicit full version like 1.1.0 (overrides -Mode).

.EXAMPLE
.\scripts\bump-version.ps1 -Mode minor
.\scripts\bump-version.ps1 -Version 1.2.0
#>
param(
    [ValidateSet("patch", "minor", "major")]
    [string]$Mode = "patch",
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# Each pattern captures the current version as group 1 on its line.
$Patterns = @{
    "package.json"              = '(?m)^\s*"version":\s*"([^"]+)"'
    "src-tauri/Cargo.toml"      = '(?m)^version\s*=\s*"([^"]+)"'
    "src-tauri/tauri.conf.json" = '(?m)^\s*"version":\s*"([^"]+)"'
    "python/pyproject.toml"     = '(?m)^version\s*=\s*"([^"]+)"'
    "src-tauri/Cargo.lock"      = '(name = "scene_vault"\r?\nversion = ")([^"]+)"'
}

$Current = [regex]::Match((Get-Content -Raw "package.json"), $Patterns["package.json"])
if (-not $Current.Success) { throw "Cannot find version in package.json" }
$Current = $Current.Groups[1].Value

if (-not $Version) {
    $parts = $Current -split "\."
    if ($parts.Count -ne 3) { throw "Unexpected version format: $Current" }
    $major = [int]$parts[0]; $minor = [int]$parts[1]; $patch = [int]$parts[2]
    switch ($Mode) {
        "patch" { $patch++ }
        "minor" { $minor++; $patch = 0 }
        "major" { $major++; $minor = 0; $patch = 0 }
    }
    $Version = "$major.$minor.$patch"
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Invalid version: $Version" }
if ($Version -eq $Current) { throw "Version is already $Current; nothing to bump." }

foreach ($path in $Patterns.Keys) {
    $content = Get-Content -Raw $path
    $found = [regex]::Matches($content, $Patterns[$path])
    if ($found.Count -ne 1) {
        throw "$path : expected exactly 1 version occurrence, found $($found.Count)"
    }
    # The version is the last capture group: single-group patterns capture
    # it directly; the Cargo.lock pattern captures a prefix plus the version.
    $old = $found[0].Groups[$found[0].Groups.Count - 1].Value
    $newContent = [regex]::Replace($content, $Patterns[$path], {
        param($m)
        return $m.Value.Replace($old, $Version)
    })
    [System.IO.File]::WriteAllText(
        (Join-Path $RepoRoot $path),
        $newContent,
        (New-Object System.Text.UTF8Encoding($false))
    )
}

Write-Host "Bumped $Current -> $Version"
Write-Host "  package.json / Cargo.toml / tauri.conf.json / pyproject.toml / Cargo.lock updated."
