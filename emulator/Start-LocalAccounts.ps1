# SPDX-License-Identifier: AGPL-3.0-only
# Start password accounts alongside the local game server, preserving the local profile.
param(
    [Parameter(Mandatory=$true)][string]$Server,
    [Parameter(Mandatory=$true)][string]$Data,
    [Parameter(Mandatory=$true)][string]$GameData,
    [ValidateRange(1024,65534)][int]$Port = 19070
)
$ErrorActionPreference = 'Stop'
$auth = Join-Path $Data 'auth'
$logs = Join-Path $PSScriptRoot 'target\account-runtime'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backend = $null
$arguments = @('account-web', '--auth-data', ('"' + $auth + '"'), '--data', ('"' + $Data + '"'), '--web-bind', '127.0.0.1:19400', '--public-origin', 'http://127.0.0.1:19400', '--public-host', '127.0.0.1', '--public-port', $Port)
try {
    $backend = Start-Process -FilePath $Server -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logs "accounts-$stamp.log") -RedirectStandardError (Join-Path $logs "accounts-$stamp-error.log")
    $ready = $false
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        Start-Sleep -Milliseconds 100
        $backend.Refresh()
        if ($backend.HasExited) { throw "The account service could not start. See $logs." }
        if (Test-Path -LiteralPath (Join-Path $auth 'accounts.journal')) {
            try {
                $health = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:19400/health' -TimeoutSec 1
                if ($health.StatusCode -eq 200) { $ready = $true; break }
            } catch { }
        }
    }
    if (-not $ready) { throw 'The account service did not become ready.' }
    Write-Host 'Password accounts: http://127.0.0.1:19400/ (Create account in the launcher)'
    & $Server serve --bind "127.0.0.1:$Port" --account local --data $Data --game-data $GameData --local-accounts $auth
} finally {
    if ($backend) {
        $backend.Refresh()
        if (-not $backend.HasExited) {
            & $Server account-stop --auth-data $auth --seconds 20
        }
        $backend.Dispose()
    }
}
