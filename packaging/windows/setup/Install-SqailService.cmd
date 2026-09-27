@echo off
rem Right-click > "Run as administrator". Extra arguments are passed on, e.g.
rem   Install-SqailService.cmd -Network
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Install-SqailService.ps1" %*
echo.
pause
