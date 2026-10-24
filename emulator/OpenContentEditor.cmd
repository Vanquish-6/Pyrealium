@echo off
setlocal
where pythonw >nul 2>nul
if errorlevel 1 (
    echo Python with Tk support is required. Install Python for Windows and enable its launcher on PATH.
    pause
    exit /b 1
)
start "" /b pythonw "%~dp0tools\content_editor.py" %*
