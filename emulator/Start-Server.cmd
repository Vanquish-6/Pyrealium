@echo off
rem Double-click to build (if needed) and start the local server.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Server.ps1" %*
pause
