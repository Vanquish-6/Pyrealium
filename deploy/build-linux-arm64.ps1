param(
    [string]$SourceRoot = (Split-Path -Parent $PSScriptRoot),
    [Parameter(Mandatory = $true)][string]$SourceUrl,
    [string]$TargetDirectory
)
$ErrorActionPreference = 'Stop'
$sysroot = (& rustc --print sysroot).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Rust toolchain.' }
$hostTriple = ((& rustc -vV) | Where-Object { $_ -like 'host: *' }) -replace '^host: ', ''
$linker = Join-Path $sysroot "lib/rustlib/$hostTriple/bin/rust-lld.exe"
if (!(Test-Path -LiteralPath $linker)) { throw 'Rust LLD is missing.' }
$names = @('PYREALIUM_SOURCE_URL', 'CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER', 'CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:PYREALIUM_SOURCE_URL = $SourceUrl
    $env:CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER = $linker
    $env:CARGO_BUILD_JOBS = '2'
    $env:CARGO_INCREMENTAL = '0'
    $buildArgs = @('build', '--locked', '--release', '--target', 'aarch64-unknown-linux-musl', '--manifest-path', (Join-Path $SourceRoot 'emulator/Cargo.toml'))
    if ($TargetDirectory) { $buildArgs += @('--target-dir', $TargetDirectory) }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "ARM64 compilation failed ($LASTEXITCODE)." }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
