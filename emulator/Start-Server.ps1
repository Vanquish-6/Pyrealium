# Start the local server using the folders already saved in the launcher.
# No extra flags. Leave the window open, then click Enter Dereth.
param(
    [ValidateRange(1024, 65534)][int]$Port = 0
)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$data = Join-Path $root 'data'
$settingsPath = Join-Path $env:LOCALAPPDATA 'Pyrealium\launcher.xml'
$install = "C:\Asheron's Call 2"

# Newest built server; $null when none exists or the source is newer than every build.
function Find-Server {
    $builds = @(
        (Join-Path $root 'target\release\pyrealium.exe'),
        (Join-Path $root 'target\local-build\release\pyrealium.exe'),
        (Join-Path $root 'target\debug\pyrealium.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | ForEach-Object { Get-Item -LiteralPath $_ } |
        Sort-Object LastWriteTime -Descending
    if (-not $builds) { return $null }
    $source = Get-ChildItem -LiteralPath (Join-Path $root 'src'), (Join-Path $root 'content') -Recurse -File |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($source -and $source.LastWriteTime -gt $builds[0].LastWriteTime) { return $null }
    return $builds[0].FullName
}

$account = 'local'
$hostName = '127.0.0.1'
$game = $null
if (Test-Path -LiteralPath $settingsPath) {
    [xml]$settings = Get-Content -LiteralPath $settingsPath
    $node = $settings.Settings
    # The existing no-ticket profile stays local; other names require password accounts.
    if ($node.Host) { $hostName = [string]$node.Host }
    if ($Port -eq 0 -and $node.Port) { $Port = [int]$node.Port }
    if ($node.GameDirectory -and (Test-Path -LiteralPath (Join-Path ([string]$node.GameDirectory) 'pyrealium-compatibility.json'))) {
        $game = [string]$node.GameDirectory
    }
}
if ($Port -eq 0) { $Port = 19070 }
if ($hostName -ne '127.0.0.1' -and $hostName -ne 'localhost') {
    throw "The launcher is set to $hostName. This command only starts the server on this PC."
}

if (-not $game) {
    $game = Join-Path $root 'target\compat-7148'
    if (-not (Test-Path -LiteralPath (Join-Path $game 'pyrealium-compatibility.json'))) {
        if (-not (Test-Path -LiteralPath (Join-Path $install 'portal.dat') -PathType Leaf)) {
            throw "The launcher has no prepared game folder, and $install was not found."
        }
        Write-Host 'First launch: copying your AC2 data into a 7148-compatible folder. This happens once.'
        & python (Join-Path $root 'tools\make_7148_test_assets.py') $install $game
        if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the compatible game data.' }
    }
    & python (Join-Path $root 'tools\check_7148_scripts.py') (Join-Path $game 'portal.dat')
    if ($LASTEXITCODE -eq 2) {
        & python (Join-Path $root 'tools\repair_7148_examination.py') $game
        if ($LASTEXITCODE -ne 0) { throw 'The examination compatibility repair failed.' }
    } elseif ($LASTEXITCODE -ne 0) {
        throw 'Could not read the prepared game data.'
    }
    & python (Join-Path $root 'tools\repair_7148_skill_panels.py') $game | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'The skill panel compatibility repair failed.' }
}

$server = Find-Server
if (-not $server) {
    Write-Host 'Building the latest server.'
    Push-Location $root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'Server build failed.' }
    } finally { Pop-Location }
    $server = Join-Path $root 'target\release\pyrealium.exe'
}

$processes = @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'pyrealium.exe'" -ErrorAction SilentlyContinue)
$running = @($processes | Where-Object { $_.CommandLine -match '\bserve(?:-training|-world)?\s' })
if ($running.Count -gt 0) {
    $current = $running | Where-Object { $_.ExecutablePath -eq $server }
    if ($current) {
        Write-Host 'The latest server is already running. Leave it open and click Enter Dereth.'
        exit 0
    }
    Write-Host 'Replacing the older running server with the latest build.'
    & $running[0].ExecutablePath stop --data $data --seconds 20 2>$null
    $accounts = Join-Path $data 'auth'
    if (Test-Path -LiteralPath (Join-Path $accounts 'accounts.lock')) {
        & $running[0].ExecutablePath account-stop --auth-data $accounts --seconds 20 2>$null
    }
    foreach ($process in $processes) {
        if ($process.CommandLine -match '\b(serve(?:-training|-world)?|account-web)\s') {
            Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
        }
    }
    Start-Sleep -Milliseconds 500
}
$stale = @(Join-Path $data 'server.lock')
$accountsRunning = @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'pyrealium.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -match '\baccount-web\s' }).Count -gt 0
if (-not $accountsRunning) { $stale += Join-Path $data 'auth\accounts.lock' }
foreach ($lock in $stale) {
    if (Test-Path -LiteralPath $lock) { Remove-Item -LiteralPath $lock }
}

Write-Host ''
Write-Host "Server: ${hostName}:$Port    Account: $account"
Write-Host 'Leave this window open, then click Enter Dereth in the launcher.'
Write-Host "Game data: $game"
Write-Host ''

& (Join-Path $root 'Start-LocalAccounts.ps1') -Server $server -Data $data -GameData $game -Port $Port
