param(
    [Parameter(Mandatory = $true)][string]$GameDirectory,
    [Parameter(Mandatory = $true)][string]$ClientExecutable,
    [ValidateRange(1024,65534)][int]$Port = 19270,
    [ValidateRange(30,86400)][int]$Seconds = 1800
)
$ErrorActionPreference = 'Stop'
$assets = Join-Path $PSScriptRoot 'target\compat-7148'
$testDirectory = Join-Path $PSScriptRoot 'target\compat-test'
$server = Join-Path $PSScriptRoot 'target\compat-build\release\pyrealium.exe'
$client = (Resolve-Path -LiteralPath $ClientExecutable).Path
$GameDirectory = (Resolve-Path -LiteralPath $GameDirectory).Path
if (Get-Process ac2client -ErrorAction SilentlyContinue) { throw 'Close the previous game before starting another world-entry test.' }
if ((Get-Item -LiteralPath $client).VersionInfo.FileVersion -notmatch '7148$') { throw 'This test requires the recovered 7148 client.' }
if (Test-Path -LiteralPath (Join-Path $testDirectory 'data\server.lock')) { throw 'The world test server is already running. Connect through the launcher, or stop it before restarting.' }
if (-not (Test-Path -LiteralPath $assets)) {
    New-Item -ItemType Directory -Force -Path (Split-Path $assets -Parent) | Out-Null
    & python (Join-Path $PSScriptRoot 'tools\make_7148_test_assets.py') $GameDirectory $assets
    if ($LASTEXITCODE -ne 0) { throw 'Compatibility asset preparation failed.' }
}
if (-not (Test-Path -LiteralPath (Join-Path $assets 'pyrealium-compatibility.json'))) { throw 'The compatibility directory is incomplete; inspect it before retrying.' }
& python (Join-Path $PSScriptRoot 'tools\check_7148_scripts.py') (Join-Path $assets 'portal.dat')
if ($LASTEXITCODE -eq 2) {
    & python (Join-Path $PSScriptRoot 'tools\repair_7148_examination.py') $assets
    if ($LASTEXITCODE -ne 0) { throw 'The pinned examination compatibility repair failed. Test launch canceled.' }
} elseif ($LASTEXITCODE -ne 0) {
    throw 'Script compatibility check failed to read the test assets.'
}
& python (Join-Path $PSScriptRoot 'tools\repair_7148_skill_panels.py') $assets | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'The skill panel compatibility repair failed. Test launch canceled.' }
Push-Location $PSScriptRoot
try {
    & cargo build --release --target-dir target/compat-build
    if ($LASTEXITCODE -ne 0) { throw 'Server build failed.' }
} finally { Pop-Location }
New-Item -ItemType Directory -Force -Path $testDirectory | Out-Null
if (-not (Test-Path -LiteralPath (Join-Path $testDirectory 'data\characters.journal'))) {
    & $server character-create-human --account local --name Pyrealium --data (Join-Path $testDirectory 'data')
    if ($LASTEXITCODE -ne 0) { throw 'Test character creation failed.' }
}
$process = Start-Process -FilePath $server -ArgumentList @('serve','--bind',"127.0.0.1:$Port",'--account','local','--seconds',$Seconds,'--data','data','--game-data',('"'+$assets+'"')) -WorkingDirectory $testDirectory -WindowStyle Hidden -RedirectStandardOutput (Join-Path $testDirectory 'server.log') -RedirectStandardError (Join-Path $testDirectory 'server-error.log') -PassThru
Start-Sleep -Milliseconds 750
if ($process.HasExited) { Get-Content (Join-Path $testDirectory 'server-error.log'); throw 'World test server failed to start.' }
try {
    # Other large DAT files are read-only hardlinks. Never omit --rodat.
    Start-Process -FilePath $client -ArgumentList @('--account','local','--host','127.0.0.1','--port',$Port,'--rodat','--width','1024','--height','768') -WorkingDirectory $assets -PassThru | Select-Object Id
} catch {
    & $server stop --data (Join-Path $testDirectory 'data')
    throw
}
Write-Host "World-entry experiment on $Port/$($Port+1). Windows Run prompts must be reviewed by you."
Write-Host 'Native world rendering was verified; movement, gameplay and audio compatibility remain unfinished.'
