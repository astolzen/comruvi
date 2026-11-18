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

function Get-ComruviCargoTargetDir {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    # Cursor/CI often set CARGO_TARGET_DIR away from repo `target/` — pack that path.
    if ($env:CARGO_TARGET_DIR -and $env:CARGO_TARGET_DIR.Trim()) {
        return $env:CARGO_TARGET_DIR.Trim()
    }
    return (Join-Path $Root "target")
}

function Get-ComruviReleaseExe {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root
    )
    $exe = Join-Path (Get-ComruviCargoTargetDir -Root $Root) "release\comruvi.exe"
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Missing $exe (CARGO_TARGET_DIR='$($env:CARGO_TARGET_DIR)'). Build the release binary first."
    }
    return $exe
}

function Assert-ComruviExeVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Exe,
        [Parameter(Mandatory = $true)]
        [string]$Version
    )
    $product = (Get-Item -LiteralPath $Exe).VersionInfo.ProductVersion
    if (-not $product) { $product = "" }
    # Windows resources sometimes append ".0"; About uses the dotted VERSION string.
    $normalized = ($product -replace '\.0$', '')
    if ($product -ne $Version -and $normalized -ne $Version -and -not $product.StartsWith("$Version.")) {
        throw "Release exe version mismatch: $Exe reports ProductVersion='$product' but VERSION is '$Version'. Rebuild so About/installer match."
    }
}
