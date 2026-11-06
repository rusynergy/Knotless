param([ValidateSet('x64','arm64')][string[]]$Architectures = @('x64','arm64'))
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$headers = @{ 'User-Agent' = 'Knotless-build'; 'Accept' = 'application/vnd.github+json' }
$release = Invoke-RestMethod 'https://api.github.com/repos/XTLS/Xray-core/releases/latest' -Headers $headers
$version = [string]$release.tag_name
if ($release.draft -or $release.prerelease -or $version -notmatch '^v\d+(\.\d+)+$') { throw 'Invalid stable Xray release' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($arch in $Architectures) {
    $archiveName = if ($arch -eq 'arm64') { 'Xray-windows-arm64-v8a.zip' } else { 'Xray-windows-64.zip' }
    $asset = $release.assets | Where-Object name -eq $archiveName
    $checksumAsset = $release.assets | Where-Object name -eq "$archiveName.dgst"
    if (!$asset -or !$checksumAsset) { throw "Release $version has no verified $arch package" }
    $cache = Join-Path $projectRoot ".tools\bundled-xray\$version\$arch"
    New-Item -ItemType Directory -Force $cache | Out-Null
    $archive = Join-Path $cache $archiveName
    $checksum = (Invoke-WebRequest $checksumAsset.browser_download_url -Headers $headers).Content
    if ($checksum -is [byte[]]) { $checksum = [Text.Encoding]::UTF8.GetString($checksum) }
    if (!(Test-Path -LiteralPath $archive)) { Invoke-WebRequest $asset.browser_download_url -Headers $headers -OutFile $archive }
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
    if ($checksum -notmatch "(?i)(?<![a-f0-9])$hash(?![a-f0-9])") { throw "SHA-256 mismatch for $archiveName. Remove the cached archive and retry." }
    $destination = Join-Path $projectRoot "src-tauri\resources\bundled-xray\$arch"
    New-Item -ItemType Directory -Force $destination | Out-Null
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    $files = @{}
    try {
        foreach ($name in @('xray.exe','geoip.dat','geosite.dat','LICENSE')) {
            $entry = $zip.GetEntry($name)
            if (!$entry) { throw "Missing required bundled file: $name" }
            $file = Join-Path $destination $name
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$file,$true)
            $files[$name] = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    } finally { $zip.Dispose() }
    $metadata = @{ version=$version; architecture=$arch; archiveSha256=$hash.ToLowerInvariant(); files=$files }
    [IO.File]::WriteAllText((Join-Path $destination 'manifest.json'),($metadata | ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
    Write-Host "Bundled Xray $version ($arch), SHA-256 verified"
}
