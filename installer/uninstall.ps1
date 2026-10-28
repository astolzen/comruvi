#Requires -Version 5.1
<#
.SYNOPSIS
  Removes the comruvi binary and shortcuts for the current user.

.DESCRIPTION
  Deletes %LOCALAPPDATA%\comruvi\ and Start Menu / Desktop shortcuts.
  Config and parsers under %USERPROFILE%\.config\comruvi are kept unless
  -RemoveConfig is passed.
#>
[CmdletBinding()]
param(
    [switch]$RemoveConfig
)

$ErrorActionPreference = "Stop"

$BinDir = Join-Path $env:LOCALAPPDATA "comruvi"
$ConfigDir = Join-Path $env:USERPROFILE ".config\comruvi"
$StartMenuLnk = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\comruvi.lnk"
$DesktopLnk = Join-Path ([Environment]::GetFolderPath("Desktop")) "comruvi.lnk"

Write-Host "Uninstalling comruvi..."

if (Test-Path -LiteralPath $BinDir) {
    Remove-Item -LiteralPath $BinDir -Recurse -Force
    Write-Host "  Removed $BinDir"
} else {
    Write-Host "  Binary folder already absent"
}

foreach ($lnk in @($StartMenuLnk, $DesktopLnk)) {
    if (Test-Path -LiteralPath $lnk) {
        Remove-Item -LiteralPath $lnk -Force
        Write-Host "  Removed shortcut $lnk"
    }
}

if ($RemoveConfig) {
    if (Test-Path -LiteralPath $ConfigDir) {
        Remove-Item -LiteralPath $ConfigDir -Recurse -Force
        Write-Host "  Removed $ConfigDir"
    }
} else {
    Write-Host "  Kept config/parsers in $ConfigDir (pass -RemoveConfig to delete)"
}

Write-Host "Uninstall complete."
