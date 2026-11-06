param([ValidateSet('x64','arm64','all')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $projectRoot
if (Test-Path -LiteralPath "$projectRoot\.tools\cargo\bin\cargo.exe") {
    $env:CARGO_HOME = "$projectRoot\.tools\cargo"
    $env:RUSTUP_HOME = "$projectRoot\.tools\rustup"
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
}
$architectures = if ($Architecture -eq 'all') { @('x64','arm64') } else { @($Architecture) }
& "$PSScriptRoot\prepare-xray.ps1" -Architectures $architectures
if (Test-Path -LiteralPath "$projectRoot\.tools\llvm\bin\clang.exe") {
    $env:PATH = "$projectRoot\.tools\llvm\bin;$env:PATH"
}
foreach ($arch in $architectures) {
    $staging = Join-Path $projectRoot 'src-tauri\resources\bundled-xray\current'
    New-Item -ItemType Directory -Force (Join-Path $staging 'xray') | Out-Null
    foreach ($name in @('xray.exe','geoip.dat','geosite.dat','LICENSE','manifest.json')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "src-tauri\resources\bundled-xray\$arch\$name") -Destination (Join-Path $staging "xray\$name") -Force
    }
    $driver = if ($arch -eq 'arm64') { 'wintun-arm64.dll' } else { 'wintun.dll' }
    Copy-Item -LiteralPath (Join-Path $projectRoot "src-tauri\resources\$driver") -Destination (Join-Path $staging 'wintun.dll') -Force
    if ($arch -eq 'arm64') {
        & npm.cmd run tauri -- build --target aarch64-pc-windows-msvc --config src-tauri/tauri.arm64.conf.json
    } else {
        & npm.cmd run tauri -- build
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
exit 0
