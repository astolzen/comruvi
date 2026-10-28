#Requires -Version 5.1
<#
.SYNOPSIS
  Installs / merges comruvi config.toml and shipped parsers.

.DESCRIPTION
  Used by install.ps1 and the MSI post-install custom action.
  Conflict policy: Ask (prompt), Overwrite (backup _YYMMDD_HHMM), Keep (save as _version).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PayloadDir,

    [ValidateSet("Ask", "Overwrite", "Keep")]
    [string]$ConflictAction = "Ask",

    [string]$Version = "",

    [string]$LogFile = ""
)

$ErrorActionPreference = "Stop"

function Write-InstallLog([string]$Message) {
    Write-Host $Message
    if ($LogFile) {
        Add-Content -LiteralPath $LogFile -Value $Message -ErrorAction SilentlyContinue
    }
}

function Escape-TomlString([string]$Value) {
    return $Value.Replace('\', '\\').Replace('"', '\"')
}

function Format-ConfigVersion([string]$Ver) {
    if ($Ver -match '^(\d+)\.(\d+)\.(\d+)$') {
        return "{0}.{1}{2}" -f $Matches[1], $Matches[2], $Matches[3]
    }
    return $Ver
}

function Get-StampSuffix {
    return (Get-Date).ToString("_yyMMdd_HHmm")
}

function Get-VersionedPath([string]$Path, [string]$Ver) {
    $dir = Split-Path -Parent $Path
    $base = [System.IO.Path]::GetFileNameWithoutExtension($Path)
    $ext = [System.IO.Path]::GetExtension($Path)
    $safe = ($Ver -replace '[^\w\.\-]', '_')
    return (Join-Path $dir ("{0}_{1}{2}" -f $base, $safe, $ext))
}

function Get-BackupPath([string]$Path, [string]$Stamp) {
    $dir = Split-Path -Parent $Path
    $base = [System.IO.Path]::GetFileNameWithoutExtension($Path)
    $ext = [System.IO.Path]::GetExtension($Path)
    return (Join-Path $dir ("{0}{1}{2}" -f $base, $Stamp, $ext))
}

function Backup-ExistingFile([string]$Path, [string]$Stamp) {
    $backup = Get-BackupPath $Path $Stamp
    Copy-Item -LiteralPath $Path -Destination $backup -Force
    Write-Host "  Backup  $([IO.Path]::GetFileName($Path)) -> $([IO.Path]::GetFileName($backup))"
}

function New-DefaultConfigText([string]$ParserDir, [string]$Pictures, [string]$Ver) {
    $parserEsc = Escape-TomlString $ParserDir
    $picturesEsc = Escape-TomlString $Pictures
    $configVersion = Format-ConfigVersion $Ver
    return @"
version = $configVersion
thumbnailsize = "160x160"
fonttype = "default"
fontsize = 14
displaymode = "dark"
parserdir = "$parserEsc"
lastfolder = "$picturesEsc"
"@
}

function Resolve-ConflictAction([string]$Action, [string[]]$ConflictNames, [string]$Ver) {
    if ($Action -ne "Ask") { return $Action }

    $list = ($ConflictNames | ForEach-Object { "  - $_" }) -join "`n"
    $message = "Existing comruvi config and/or parsers were found:`n`n$list`n`nYes = Overwrite (backup as *_YYMMDD_HHMM)`nNo  = Keep (save installer copies as *_$Ver)"

    try {
        Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
        $result = [System.Windows.Forms.MessageBox]::Show(
            $message,
            "comruvi setup",
            [System.Windows.Forms.MessageBoxButtons]::YesNo,
            [System.Windows.Forms.MessageBoxIcon]::Question,
            [System.Windows.Forms.MessageBoxDefaultButton]::Button2
        )
        if ($result -eq [System.Windows.Forms.DialogResult]::Yes) { return "Overwrite" }
        return "Keep"
    } catch {
        Write-Host $message
        while ($true) {
            $answer = Read-Host "Choose Overwrite or Keep [O/K]"
            switch -Regex ($answer.Trim()) {
                '^(o|overwrite)$' { return "Overwrite" }
                '^(k|keep)$'      { return "Keep" }
                default { Write-Host "Please enter O or K." }
            }
        }
    }
}

try {
    if ($LogFile) {
        $logDir = Split-Path -Parent $LogFile
        if ($logDir) { New-Item -ItemType Directory -Force -Path $logDir -ErrorAction SilentlyContinue | Out-Null }
        Set-Content -LiteralPath $LogFile -Value "comruvi Install-Config $(Get-Date -Format o)" -ErrorAction SilentlyContinue
    }

    $ParsersSource = Join-Path $PayloadDir "parsers"
    $VersionFile = Join-Path $PayloadDir "version.txt"
    if (-not $Version -and (Test-Path -LiteralPath $VersionFile)) {
        $Version = (Get-Content -LiteralPath $VersionFile -TotalCount 1).Trim()
    }
    if (-not $Version) { $Version = "0.0.0" }

    Write-InstallLog "PayloadDir=$PayloadDir ConflictAction=$ConflictAction Version=$Version"

    if (-not (Test-Path -LiteralPath $ParsersSource)) {
        throw "parsers folder not found under payload: $ParsersSource"
    }

    $ConfigDir = Join-Path $env:USERPROFILE ".config\comruvi"
    $ParserDir = Join-Path $ConfigDir "parsers"
    $ConfigPath = Join-Path $ConfigDir "config.toml"
    $Pictures = [Environment]::GetFolderPath("MyPictures")
    if ([string]::IsNullOrWhiteSpace($Pictures)) {
        $Pictures = Join-Path $env:USERPROFILE "Pictures"
    }

    New-Item -ItemType Directory -Force -Path $ParserDir | Out-Null

    $ShippedParsers = @(Get-ChildItem -LiteralPath $ParsersSource -Filter "*.toml")
    $ConflictConfig = Test-Path -LiteralPath $ConfigPath
    $ConflictParsers = @($ShippedParsers | Where-Object { Test-Path -LiteralPath (Join-Path $ParserDir $_.Name) })
    $HasConflicts = $ConflictConfig -or ($ConflictParsers.Count -gt 0)

    $Mode = "Install"
    if ($HasConflicts) {
        $names = @()
        if ($ConflictConfig) { $names += "config.toml" }
        $names += @($ConflictParsers | ForEach-Object { "parsers\$($_.Name)" })
        $Mode = Resolve-ConflictAction $ConflictAction $names $Version
        Write-InstallLog "  Conflict policy: $Mode"
    }

    $Stamp = Get-StampSuffix
    $ConfigText = New-DefaultConfigText $ParserDir $Pictures $Version

    if (-not $ConflictConfig) {
        Set-Content -LiteralPath $ConfigPath -Value $ConfigText -Encoding utf8
        Write-InstallLog "  Wrote config.toml (lastfolder = Pictures, thumbnailsize = 160x160)"
    } elseif ($Mode -eq "Overwrite") {
        Backup-ExistingFile $ConfigPath $Stamp
        Set-Content -LiteralPath $ConfigPath -Value $ConfigText -Encoding utf8
        Write-InstallLog "  Overwrote config.toml with installer defaults"
    } else {
        $versioned = Get-VersionedPath $ConfigPath $Version
        Set-Content -LiteralPath $versioned -Value $ConfigText -Encoding utf8
        Write-InstallLog "  Kept config.toml; installer defaults saved as $([IO.Path]::GetFileName($versioned))"
    }

    foreach ($parser in $ShippedParsers) {
        $dest = Join-Path $ParserDir $parser.Name
        $exists = Test-Path -LiteralPath $dest
        if (-not $exists) {
            Copy-Item -LiteralPath $parser.FullName -Destination $dest
            Write-InstallLog "  Added parser $($parser.Name)"
        } elseif ($Mode -eq "Overwrite") {
            Backup-ExistingFile $dest $Stamp
            Copy-Item -LiteralPath $parser.FullName -Destination $dest -Force
            Write-InstallLog "  Overwrote parser $($parser.Name)"
        } else {
            $versioned = Get-VersionedPath $dest $Version
            Copy-Item -LiteralPath $parser.FullName -Destination $versioned -Force
            Write-InstallLog "  Kept $($parser.Name); installer copy saved as $([IO.Path]::GetFileName($versioned))"
        }
    }

    Write-InstallLog "Install-Config finished OK"
    exit 0
} catch {
    Write-InstallLog "Install-Config FAILED: $($_.Exception.Message)"
    # Do not fail the MSI hard; binary is already installed.
    exit 0
}
