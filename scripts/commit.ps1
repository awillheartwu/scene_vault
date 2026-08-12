<#
.SYNOPSIS
Commits the working tree with an automatic patch version bump (1.0.0 -> 1.0.1).

Every commit advances the patch number; minor/major versions are bumped
explicitly with release-windows.ps1 -BumpVersion when packaging.

.PARAMETER Message
Commit message (Conventional Commits style, e.g. "feat(vision): ...").

.PARAMETER NoBump
Commit without touching the version files.

.PARAMETER NoStageAll
Stage only already-staged paths instead of `git add -A` (use when unrelated
work is sitting in the working tree).

.EXAMPLE
.\scripts\commit.ps1 "feat(settings): add storage diagnosis cards"
#>
param(
    [Parameter(Mandatory = $true)][string]$Message,
    [switch]$NoBump,
    [switch]$NoStageAll
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

if (-not $NoBump) {
    & (Join-Path $PSScriptRoot "bump-version.ps1") -Mode patch
}
if (-not $NoStageAll) {
    git add -A
}
git commit -m $Message
