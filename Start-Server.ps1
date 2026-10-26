# Start the local server from the repository root. Same as emulator\Start-Server.ps1.
param([int]$Port = 0)
$script = Join-Path $PSScriptRoot 'emulator\Start-Server.ps1'
if ($Port -ne 0) { & $script -Port $Port } else { & $script }
