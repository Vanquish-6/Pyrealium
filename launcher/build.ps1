$ErrorActionPreference = 'Stop'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $compiler)) { $compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe' }
if (-not (Test-Path -LiteralPath $compiler)) { throw 'Install .NET Framework 4.x to build the launcher.' }
$output = Join-Path $PSScriptRoot 'dist'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$executable = Join-Path $output 'PyrealiumLauncher.exe'
& $compiler /nologo /target:winexe /platform:anycpu /optimize+ /reference:System.Windows.Forms.dll /reference:System.Drawing.dll "/out:$executable" (Join-Path $PSScriptRoot 'Launcher.cs')
if ($LASTEXITCODE -ne 0) { throw 'Launcher compilation failed.' }
$report = Join-Path $output 'self-test.txt'
$process = Start-Process -FilePath $executable -ArgumentList @('--self-test', ('"' + $report + '"')) -PassThru -Wait -WindowStyle Hidden
if ($process.ExitCode -ne 0) { throw "Launcher validation failed. See $report" }
Get-Content -LiteralPath $report
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $output 'README.md') -Force
Compress-Archive -LiteralPath $executable, (Join-Path $output 'README.md') -DestinationPath (Join-Path $PSScriptRoot 'PyrealiumLauncher-windows.zip') -Force
Write-Host "Built $executable and PyrealiumLauncher-windows.zip"
