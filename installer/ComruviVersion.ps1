#Requires -Version 5.1
<#
.SYNOPSIS
  Read the repo-root VERSION file (single source of truth for package version).
#>

function Get-ComruviVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    $path = Join-Path $Root "VERSION"
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing VERSION file at $path"
    }
    $version = (Get-Content -LiteralPath $path -Raw).Trim()
    if (-not $version) {
        throw "VERSION file is empty: $path"
    }
    return $version
}
