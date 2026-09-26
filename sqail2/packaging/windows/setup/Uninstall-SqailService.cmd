@echo off
rem Right-click > "Run as administrator". Extra arguments are passed on, e.g.
rem   Uninstall-SqailService.cmd -Network
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Uninstall-SqailService.ps1" %*
echo.
pause
