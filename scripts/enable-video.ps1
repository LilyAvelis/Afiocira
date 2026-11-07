param(
    [string]$OutputDirectory = (Split-Path -Parent $PSScriptRoot),
    [string]$CacheDirectory = (Join-Path $env:LOCALAPPDATA 'Afiocira\runtime-downloads')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
New-Item -ItemType Directory -Force $OutputDirectory,$CacheDirectory | Out-Null
function Download([string]$Url, [string]$File) {
    if (-not (Test-Path -LiteralPath $File)) {
        curl.exe --fail --location --retry 2 --connect-timeout 30 --max-time 900 --silent --show-error --output "$File.partial" $Url
        if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
        Move-Item -LiteralPath "$File.partial" -Destination $File -Force
    }
}
$mpv = Join-Path $CacheDirectory 'mpv.7z'
Download 'https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261107/mpv-dev-x86_64-20261107-git-9028187538.7z' $mpv
if ((Get-FileHash -LiteralPath $mpv).Hash -ne 'DF7C433A2D481FDAF6163B201F26ABBB0EFF0C215C51368397088C6D8F7A001B') { throw 'libmpv archive checksum mismatch' }
$unpack = Join-Path $CacheDirectory 'extracted'
New-Item -ItemType Directory -Force $unpack | Out-Null
tar -xf $mpv -C $unpack libmpv-2.dll
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract libmpv' }
$ffmpeg = Join-Path $CacheDirectory 'ffmpeg-8.1.2-full_build.7z'
Download 'https://github.com/GyanD/codexffmpeg/releases/download/8.1.2/ffmpeg-8.1.2-full_build.7z' $ffmpeg
if ((Get-FileHash -LiteralPath $ffmpeg).Hash -ne '0FFF188997A499B5382E0F66E845D4556C48C54F0113EBED4853D556DBDD7059') { throw 'FFmpeg archive checksum mismatch' }
$entry = @(tar -tf $ffmpeg | Where-Object { $_ -match '[/\\]bin[/\\]ffmpeg\.exe$' })
if ($LASTEXITCODE -ne 0 -or $entry.Count -ne 1) { throw 'Cannot locate FFmpeg in archive' }
tar -xf $ffmpeg -C $unpack $entry[0]
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract FFmpeg' }
$exe = Join-Path $unpack $entry[0]
if ((Get-FileHash -LiteralPath $exe).Hash -ne 'AD8F211BC894755E0061C55AB280AE00E8D3D4F15A8CC4372B24CFA247B5942E') { throw 'FFmpeg executable checksum mismatch' }
# Validate both downloads before installing either runtime. Close Afio first.
Copy-Item -LiteralPath (Join-Path $unpack 'libmpv-2.dll') -Destination $OutputDirectory -Force
Copy-Item -LiteralPath $exe -Destination $OutputDirectory -Force
Write-Output 'Video playback and thumbnails are ready. Start Afiocira.'
