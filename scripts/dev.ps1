$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $projectRoot
if (Test-Path -LiteralPath "$projectRoot\.tools\cargo\bin\cargo.exe") {
    $env:CARGO_HOME = "$projectRoot\.tools\cargo"
    $env:RUSTUP_HOME = "$projectRoot\.tools\rustup"
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
}
# A previous cross-build may have staged ARM64 resources. Always stage the
# development host's resources before starting Tauri.
$arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -eq 'Arm64') { 'arm64' } else { 'x64' }
$bundles = Join-Path $projectRoot 'src-tauri\resources\bundled-xray'
if (!(Test-Path -LiteralPath "$bundles\$arch\manifest.json")) {
    & "$PSScriptRoot\prepare-xray.ps1" -Architectures @($arch)
}
New-Item -ItemType Directory -Force "$bundles\current\xray" | Out-Null
foreach ($name in @('xray.exe','geoip.dat','geosite.dat','LICENSE','manifest.json')) {
    Copy-Item -LiteralPath "$bundles\$arch\$name" -Destination "$bundles\current\xray\$name" -Force
}
$driver = if ($arch -eq 'arm64') { 'wintun-arm64.dll' } else { 'wintun.dll' }
Copy-Item -LiteralPath "$projectRoot\src-tauri\resources\$driver" -Destination "$bundles\current\wintun.dll" -Force
& npm.cmd run desktop
exit $LASTEXITCODE
