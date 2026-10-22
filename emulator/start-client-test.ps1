param(
    [Parameter(Mandatory = $true)][string]$GameDirectory,
    [Parameter(Mandatory = $true)][string]$ClientExecutable,
    [ValidateRange(1024, 65534)][int]$Port = 19070,
    [ValidateRange(30, 86400)][int]$Seconds = 600
)
$ErrorActionPreference = 'Stop'
$recoveredClient = (Resolve-Path -LiteralPath $ClientExecutable).Path
$GameDirectory = (Resolve-Path -LiteralPath $GameDirectory).Path
$serverExe = Join-Path $PSScriptRoot 'target\release\pyrealium.exe'
$testDirectory = Join-Path $PSScriptRoot 'target\connect-test'
foreach ($requiredFile in @($recoveredClient, $serverExe, (Join-Path $GameDirectory 'portal.dat'), (Join-Path $GameDirectory 'ijl15.dll'))) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) { throw "Missing required file: $requiredFile" }
}
if ((Get-Item -LiteralPath $recoveredClient).VersionInfo.FileVersion -notmatch '7148$') { throw 'This test requires the recovered build 7148 executable.' }
if (Get-Process ac2client -ErrorAction SilentlyContinue) { throw 'Close the previous AC2 test client before starting a new test.' }
if (Test-Path -LiteralPath (Join-Path $testDirectory 'data\server.lock')) { throw 'A test-server lock already exists. Stop the previous test server before relaunching; inspect a stale lock only after verifying no server is running.' }
New-Item -ItemType Directory -Force -Path $testDirectory | Out-Null
$serverProcess = Start-Process -FilePath $serverExe -ArgumentList @('serve', '--bind', "127.0.0.1:$Port", '--account', 'local', '--seconds', $Seconds, '--data', 'data') -WorkingDirectory $testDirectory -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $testDirectory 'server.log') -RedirectStandardError (Join-Path $testDirectory 'server-error.log')
Start-Sleep -Milliseconds 750
if ($serverProcess.HasExited) { Get-Content -LiteralPath (Join-Path $testDirectory 'server-error.log'); throw 'Pyrealium could not start. Check the port pair and data lock.' }
Write-Host "Pyrealium PID $($serverProcess.Id), listening on ports $Port and $($Port + 1)."
Write-Host "Logs: $testDirectory"
Write-Host 'Launching the recovered 7148 executable with installed assets and --rodat.'
Write-Host 'If Windows shows its Run warning, review and approve it yourself.'
try {
    Start-Process -FilePath $recoveredClient -ArgumentList @('--account','local','--host','127.0.0.1','--port', $Port, '--rodat','--width','1024','--height','768') -WorkingDirectory $GameDirectory -PassThru | Select-Object Id, ProcessName
} catch {
    & $serverExe stop --data (Join-Path $testDirectory 'data')
    throw
}
Write-Host "Server stops automatically after $Seconds seconds. This mode tests character selection; use start-world-test.ps1 for the experimental world bootstrap."
