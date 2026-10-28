#Requires -Version 5.1
<#
.SYNOPSIS
  Builds a release binary and assembles a Windows install package under dist/.
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

$CargoToml = Get-Content (Join-Path $Root "Cargo.toml") -Raw
if ($CargoToml -match 'version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} else {
    throw "Could not read version from Cargo.toml"
}

if (-not $SkipBuild) {
    Write-Host "Building release binary..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }
}

$Exe = Join-Path $Root "target\release\comruvi.exe"
if (-not (Test-Path -LiteralPath $Exe)) {
    throw "Missing $Exe - build the release binary first."
}

$StageName = "comruvi-$Version-windows-x64"
$DistRoot = Join-Path $Root "dist"
$Stage = Join-Path $DistRoot $StageName
$ZipPath = Join-Path $DistRoot "$StageName.zip"

if (Test-Path -LiteralPath $Stage) { Remove-Item -LiteralPath $Stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $Stage "parsers") | Out-Null

Copy-Item -LiteralPath $Exe -Destination (Join-Path $Stage "comruvi.exe")
Copy-Item -LiteralPath (Join-Path $Root "installer\install.ps1") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $Root "installer\Install-Config.ps1") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $Root "installer\uninstall.ps1") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $Root "installer\install.bat") -Destination $Stage
Set-Content -LiteralPath (Join-Path $Stage "version.txt") -Value $Version -Encoding ascii -NoNewline
Get-ChildItem -LiteralPath (Join-Path $Root "parsers") -Filter "*.toml" | Copy-Item -Destination (Join-Path $Stage "parsers")

@"
comruvi $Version - Windows install package
==========================================

ZIP / script installer
----------------------
1. Unzip this folder anywhere.
2. Double-click install.bat  (or run: powershell -ExecutionPolicy Bypass -File .\install.ps1)
3. Optional desktop shortcut:  powershell -ExecutionPolicy Bypass -File .\install.ps1 -DesktopShortcut

MSI installer (preferred)
-------------------------
Build with:  powershell -ExecutionPolicy Bypass -File .\installer\build-msi.ps1
Then run:    msiexec /i dist\comruvi-$Version-windows-x64.msi
Quiet keep:  msiexec /i dist\comruvi-$Version-windows-x64.msi CONFLICTACTION=Keep /qn

Install locations
-----------------
  Program : %LOCALAPPDATA%\comruvi\comruvi.exe
  Config  : %USERPROFILE%\.config\comruvi\config.toml
  Parsers : %USERPROFILE%\.config\comruvi\parsers\

If config.toml or shipped parsers already exist, the installer asks:
  Overwrite - backup existing files as name_YYMMDD_HHMM.ext, then install defaults
  Keep      - leave existing; save installer copies as name_$Version.ext
Non-interactive:  CONFLICTACTION=Overwrite|Keep  (MSI) or -ConflictAction (PS1)

Default config (fresh install or Overwrite)
------------------------------------------
  lastfolder    = your Pictures folder
  thumbnailsize = "160x160"
  parserdir     = %USERPROFILE%\.config\comruvi\parsers

Uninstall
---------
  msiexec /x dist\comruvi-$Version-windows-x64.msi
  or: powershell -ExecutionPolicy Bypass -File .\uninstall.ps1
  Add -RemoveConfig to also delete config and parsers.
"@ | Set-Content -LiteralPath (Join-Path $Stage "README.txt") -Encoding utf8

if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
Compress-Archive -Path $Stage -DestinationPath $ZipPath -Force

Write-Host ""
Write-Host "Zip package ready:"
Write-Host "  Folder : $Stage"
Write-Host "  Zip    : $ZipPath"

Write-Host ""
Write-Host "Building MSI..."
& (Join-Path $Root "installer\build-msi.ps1") -SkipBuild

