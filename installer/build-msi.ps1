#Requires -Version 5.1
<#
.SYNOPSIS
  Builds a per-user MSI for comruvi using WiX Toolset 5 (dotnet tool: wix).
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

# WiX ProductVersion requires up to 3 numeric fields (major.minor.patch).
$MsiVersion = $Version
if ($Version -match '^(\d+)\.(\d+)\.(\d+)') {
    $MsiVersion = "$($Matches[1]).$($Matches[2]).$($Matches[3])"
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

$env:Path = "$env:USERPROFILE\.dotnet\tools;" + $env:Path
$wixCmd = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wixCmd) {
    Write-Host "Installing WiX tool (dotnet tool install -g wix)..."
    dotnet tool install --global wix --version 5.0.2
    $env:Path = "$env:USERPROFILE\.dotnet\tools;" + $env:Path
    $wixCmd = Get-Command wix -ErrorAction SilentlyContinue
    if (-not $wixCmd) { throw "wix tool not found after install. Open a new shell and retry." }
}

Write-Host "Ensuring WiX UI extension..."
& wix extension add "WixToolset.UI.wixext/5.0.2" | Out-Null
if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) {
    # Already installed is fine; continue.
    Write-Host "  (extension add exit $LASTEXITCODE - continuing)"
}

$WixDir = Join-Path $Root "installer\wix"
$DistRoot = Join-Path $Root "dist"
New-Item -ItemType Directory -Force -Path $DistRoot | Out-Null

$VersionFile = Join-Path $WixDir "version.txt"
Set-Content -LiteralPath $VersionFile -Value $Version -Encoding ascii -NoNewline

# Generate ParserComponents fragment
$ParsersDir = Join-Path $Root "parsers"
$ParsersWxs = Join-Path $WixDir "Parsers.wxs"
$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine('<?xml version="1.0" encoding="UTF-8"?>')
[void]$sb.AppendLine('<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">')
[void]$sb.AppendLine('  <Fragment>')
[void]$sb.AppendLine('    <ComponentGroup Id="ParserComponents" Directory="PARSERSFOLDER">')
$index = 0
Get-ChildItem -LiteralPath $ParsersDir -Filter "*.toml" | Sort-Object Name | ForEach-Object {
    $index++
    $id = "Parser$index"
    $guid = [guid]::NewGuid().ToString().ToUpperInvariant()
    $src = $_.FullName
    $name = $_.Name
    [void]$sb.AppendLine("      <Component Id=`"$id`" Guid=`"$guid`">")
    [void]$sb.AppendLine("        <File Id=`"${id}File`" Source=`"$src`" Name=`"$name`" />")
    [void]$sb.AppendLine("        <RegistryValue Root=`"HKCU`" Key=`"Software\comruvi\parsers`" Name=`"$name`" Type=`"integer`" Value=`"1`" KeyPath=`"yes`" />")
    [void]$sb.AppendLine("      </Component>")
}
[void]$sb.AppendLine('    </ComponentGroup>')
[void]$sb.AppendLine('  </Fragment>')
[void]$sb.AppendLine('</Wix>')
[System.IO.File]::WriteAllText($ParsersWxs, $sb.ToString())

$OutMsi = Join-Path $DistRoot "comruvi-$Version-windows-x64.msi"
if (Test-Path -LiteralPath $OutMsi) { Remove-Item -LiteralPath $OutMsi -Force }

$ConfigScript = Join-Path $Root "installer\Install-Config.ps1"
$RunConfigCmd = Join-Path $Root "installer\run-config.cmd"

$LicenseRtf = Join-Path $WixDir "License.rtf"
$LicenseSrc = Join-Path $Root "LICENSE"
if (-not (Test-Path -LiteralPath $LicenseSrc)) { throw "Missing LICENSE" }
$licenseText = [IO.File]::ReadAllText($LicenseSrc)
$escaped = $licenseText.Replace('\', '\\').Replace('{', '\{').Replace('}', '\}')
$escaped = $escaped -replace "`r`n", "\par`r`n" -replace "`n", "\par`r`n"
$rtf = "{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fmodern\fcharset0 Courier New;}}\f0\fs16`r`n$escaped`r`n}"
[IO.File]::WriteAllText($LicenseRtf, $rtf, [Text.UTF8Encoding]::new($false))
Write-Host "Wrote AGPL License.rtf"

$IconBmp = Join-Path $WixDir "comruvi-icon.bmp"
Write-Host "Exporting icon BMP for warning dialog..."
cargo run --release --bin export-icon -- $IconBmp
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $IconBmp)) {
    throw "Failed to export $IconBmp"
}

Write-Host "Building MSI $OutMsi ..."
wix build `
    (Join-Path $WixDir "Package.wxs") `
    $ParsersWxs `
    -arch x64 `
    -ext WixToolset.UI.wixext `
    -d "ProductVersion=$MsiVersion" `
    -d "ExeSource=$Exe" `
    -d "ConfigScript=$ConfigScript" `
    -d "RunConfigCmd=$RunConfigCmd" `
    -d "VersionFile=$VersionFile" `
    -d "LicenseRtf=$LicenseRtf" `
    -d "IconBmp=$IconBmp" `
    -d "AskConflictVbs=$(Join-Path $WixDir 'AskConflict.vbs')" `
    -o $OutMsi
if ($LASTEXITCODE -ne 0) { throw "wix build failed" }

Write-Host ""
Write-Host "MSI ready: $OutMsi"
Write-Host "Install:   msiexec /i `"$OutMsi`""
Write-Host "Quiet:     msiexec /i `"$OutMsi`" CONFLICTACTION=Keep /qn"
Write-Host "Uninstall: msiexec /x `"$OutMsi`""
