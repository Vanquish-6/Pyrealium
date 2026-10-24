# SPDX-License-Identifier: AGPL-3.0-only
param(
    [Parameter(Mandatory=$true)][uri]$AccountsUrl,
    [Parameter(Mandatory=$true)][string]$ClientExecutable,
    [Parameter(Mandatory=$true)][string]$GameDirectory,
    [string]$Account = ''
)
$ErrorActionPreference = 'Stop'
if ($AccountsUrl.Scheme -ne 'https' -or $AccountsUrl.UserInfo -or $AccountsUrl.Query -or $AccountsUrl.Fragment -or $AccountsUrl.AbsolutePath -ne '/') {
    throw 'Use the HTTPS address of the account website, without an extra path.'
}
if (-not (Test-Path -LiteralPath $ClientExecutable -PathType Leaf) -or -not (Test-Path -LiteralPath (Join-Path $GameDirectory 'portal.dat') -PathType Leaf)) {
    throw 'Select the game client executable and its matching game data folder.'
}
if (-not $Account) { $Account = Read-Host 'Account name' }
if ($Account -notmatch '^[A-Za-z0-9_]{1,32}$') { throw 'Invalid account name.' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$gate = New-Object Threading.Mutex($false, ('Local\PyrealiumTicket_' + $identity))
$held = $false
$ticket = $null
$credentials = $null
$registry = 'HKCU:\Software\Turbine\ac2'
try {
    try { $held = $gate.WaitOne(0) } catch [Threading.AbandonedMutexException] { $held = $true }
    if (-not $held) { throw 'Another online launch is already in progress.' }
    $credentials = Get-Credential -UserName $Account -Message 'Sign in to your Pyrealium account'
    if ($null -eq $credentials) { throw 'Sign-in cancelled.' }
    $plain = $credentials.GetNetworkCredential().Password
    $body = @{ account = $credentials.UserName; password = $plain } | ConvertTo-Json -Compress
    try {
        $bodyBytes = [Text.Encoding]::UTF8.GetBytes($body)
        $login = Invoke-RestMethod -Uri ([uri]::new($AccountsUrl, 'api/login')) -Method Post -ContentType 'application/json; charset=utf-8' -Body $bodyBytes -TimeoutSec 30
    } catch { throw 'Sign-in failed. Check your account details and account website address.' }
    finally {
        if ($bodyBytes) { [Array]::Clear($bodyBytes,0,$bodyBytes.Length) }
        $plain = $null; $body = $null; $bodyBytes = $null
    }
    if ($login.account -notmatch '^[a-z0-9_]{1,32}$' -or $login.ticket -notmatch '^[a-f0-9]{64}$' -or $login.game_host -notmatch '^[A-Za-z0-9][A-Za-z0-9.-]{0,251}[A-Za-z0-9]$' -or [int]$login.game_port -lt 1024 -or [int]$login.game_port -gt 65534) {
        throw 'The account service returned invalid connection details.'
    }
    $ticket = [string]$login.ticket
    $login.ticket = $null
    New-Item -Path $registry -Force | Out-Null
    New-ItemProperty -Path $registry -Name 'GLSTicket' -PropertyType String -Value $ticket -Force | Out-Null
    # The native client consumes/deletes this value. No ticket or password is
    # included in the process command line, settings file, or console output.
    $game = Start-Process -FilePath ([IO.Path]::GetFullPath($ClientExecutable)) -WorkingDirectory ([IO.Path]::GetFullPath($GameDirectory)) -ArgumentList @('--account', $login.account, '--host', $login.game_host, '--port', [string]$login.game_port, '/glsticket', '--rodat', '--width', '1024', '--height', '768') -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $deadline -and -not $game.HasExited) {
        $remaining = Get-ItemProperty -LiteralPath $registry -Name 'GLSTicket' -ErrorAction SilentlyContinue
        if ($null -eq $remaining) { break }
        Start-Sleep -Milliseconds 200
        $game.Refresh()
    }
} finally {
    if ($ticket) {
        $remaining = Get-ItemProperty -LiteralPath $registry -Name 'GLSTicket' -ErrorAction SilentlyContinue
        if ($null -ne $remaining -and $remaining.GLSTicket -ceq $ticket) { Remove-ItemProperty -LiteralPath $registry -Name 'GLSTicket' -ErrorAction SilentlyContinue }
    }
    $ticket = $null
    if ($credentials) { $credentials.Password.Dispose() }
    if ($held) { $gate.ReleaseMutex() }
    $gate.Dispose()
}
