#Requires -Version 5.1
<#
.SYNOPSIS
  Installs comruvi for the current Windows user.

.DESCRIPTION
  Binary  -> %LOCALAPPDATA%\comruvi\comruvi.exe
  Config  -> %USERPROFILE%\.config\comruvi\config.toml
  Parsers -> %USERPROFILE%\.config\comruvi\parsers\

  If config.toml or any shipped parser already exists, you are asked to
  Overwrite (backup with _YYMMDD_HHMM) or Keep (save installer copies with
  the build version in the name). Use -ConflictAction to skip the prompt.
#>
[CmdletBinding()]
param(
    [switch]$NoStartMenu,
    [switch]$DesktopShortcut,
    [ValidateSet("Ask", "Overwrite", "Keep")]
    [string]$ConflictAction = "Ask"
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ExeSource = Join-Path $ScriptDir "comruvi.exe"
$ParsersSource = Join-Path $ScriptDir "parsers"
$VersionFile = Join-Path $ScriptDir "version.txt"
$ConfigScript = Join-Path $ScriptDir "Install-Config.ps1"

if (-not (Test-Path -LiteralPath $ExeSource)) {
    throw "comruvi.exe not found next to install.ps1 ($ScriptDir). Run pack-windows.ps1 first."
}
if (-not (Test-Path -LiteralPath $ParsersSource)) {
    throw "parsers\ folder not found next to install.ps1."
}
if (-not (Test-Path -LiteralPath $ConfigScript)) {
    throw "Install-Config.ps1 not found next to install.ps1."
}

$InstallVersion = "0.0.0"
if (Test-Path -LiteralPath $VersionFile) {
    $InstallVersion = (Get-Content -LiteralPath $VersionFile -TotalCount 1).Trim()
}

$BinDir = Join-Path $env:LOCALAPPDATA "comruvi"

Write-Host "Installing comruvi $InstallVersion ..."
Write-Host "  Binary : $BinDir"

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Copy-Item -LiteralPath $ExeSource -Destination (Join-Path $BinDir "comruvi.exe") -Force

& $ConfigScript -PayloadDir $ScriptDir -ConflictAction $ConflictAction -Version $InstallVersion
if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { throw "Install-Config.ps1 failed" }

$ExePath = Join-Path $BinDir "comruvi.exe"
$Wsh = New-Object -ComObject WScript.Shell

if (-not $NoStartMenu) {
    $StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
    $lnk = Join-Path $StartMenu "comruvi.lnk"
    $shortcut = $Wsh.CreateShortcut($lnk)
    $shortcut.TargetPath = $ExePath
    $shortcut.WorkingDirectory = $BinDir
    $shortcut.Description = "comruvi - ComfyUI Rust Viewer"
    $shortcut.IconLocation = "$ExePath,0"
    $shortcut.Save()
    Write-Host "  Start Menu shortcut: $lnk"
}

if ($DesktopShortcut) {
    $Desktop = [Environment]::GetFolderPath("Desktop")
    $lnk = Join-Path $Desktop "comruvi.lnk"
    $shortcut = $Wsh.CreateShortcut($lnk)
    $shortcut.TargetPath = $ExePath
    $shortcut.WorkingDirectory = $BinDir
    $shortcut.Description = "comruvi - ComfyUI Rust Viewer"
    $shortcut.IconLocation = "$ExePath,0"
    $shortcut.Save()
    Write-Host "  Desktop shortcut: $lnk"
}

Write-Host ""
Write-Host "Installation complete."
Write-Host "Run: $ExePath"
