param([string]$OutputDirectory = (Join-Path $PSScriptRoot '..\target\debug'))
$ErrorActionPreference = 'Stop'
# Pin the tested Windows x64 runtime; never silently track a daily build.
$url = 'https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261107/mpv-dev-x86_64-20261107-git-9028187538.7z'
$expected = 'DF7C433A2D481FDAF6163B201F26ABBB0EFF0C215C51368397088C6D8F7A001B'
$cache = Join-Path $PSScriptRoot '..\target\video-runtime'
New-Item -ItemType Directory -Force $cache, $OutputDirectory | Out-Null
$archive = Join-Path $cache 'mpv.7z'
if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest $url -OutFile $archive }
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'libmpv archive SHA-256 mismatch' }
tar -xf $archive -C $cache
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract libmpv archive' }
$source = Join-Path $cache 'libmpv-2.dll'
$destination = Join-Path $OutputDirectory 'libmpv-2.dll'
if (-not (Test-Path -LiteralPath $destination) -or
    (Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $destination).Hash) {
    Copy-Item -LiteralPath $source -Destination $destination -Force
}
Write-Output "Video runtime ready: $destination"
Write-Output 'FFmpeg on PATH (or AFIO_FFMPEG) enables video thumbnails; playback itself uses libmpv.'
