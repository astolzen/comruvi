@echo off
setlocal
set "PAYLOAD=%~dp0"
set "PAYLOAD=%PAYLOAD:~0,-1%"
set "ACTION=%~1"
if "%ACTION%"=="" set "ACTION=Keep"
set "VER=%~2"
if "%VER%"=="" set "VER=0.0.0"
set "LOG=%TEMP%\comruvi-msi-config.log"
echo run-config.cmd start %DATE% %TIME% > "%LOG%"
echo PAYLOAD=%PAYLOAD% ACTION=%ACTION% VER=%VER% >> "%LOG%"
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%PAYLOAD%\Install-Config.ps1" -PayloadDir "%PAYLOAD%" -ConflictAction "%ACTION%" -Version "%VER%" -LogFile "%LOG%"
set "EC=%ERRORLEVEL%"
echo powershell exit=%EC% >> "%LOG%"
exit /b 0
