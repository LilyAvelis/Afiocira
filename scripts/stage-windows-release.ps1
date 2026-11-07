param([string]$BuildDirectory = 'target\distribution\release', [string]$OutputDirectory = 'target\package-2.0.0')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$buildPath = if ([IO.Path]::IsPathRooted($BuildDirectory)) {$BuildDirectory} else {Join-Path $root $BuildDirectory}
$build = (Resolve-Path -LiteralPath $buildPath).Path
$stage = if ([IO.Path]::IsPathRooted($OutputDirectory)) {$OutputDirectory} else {[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
$cache = Join-Path $root 'target\release-dependencies'
New-Item -ItemType Directory -Force $stage,$cache | Out-Null
$lock = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'release-dependencies.json') -Raw | ConvertFrom-Json
function CheckedDownload([string]$url,[string]$path,[string]$hash) {
    if (-not (Test-Path -LiteralPath $path)) { Invoke-WebRequest -Uri $url -OutFile $path }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $hash) {throw "Checksum mismatch: $path"}
}
# Video runtimes are downloaded directly from upstream on the end user machine.
& (Join-Path $PSScriptRoot 'setup-everything.ps1') -OutputDirectory $stage
Copy-Item -LiteralPath (Join-Path $build 'photo-gui.exe') -Destination (Join-Path $stage 'Afiocira.exe') -Force
Copy-Item -LiteralPath (Join-Path $build 'photo-cli.exe') -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $build 'photo-launcher.exe') -Destination (Join-Path $stage 'AfioLauncher.exe') -Force
[IO.File]::WriteAllText((Join-Path $stage 'current-version.txt'),'Afiocira.exe')
$yaziArchive = Join-Path $cache 'yazi-26.5.6-windows-x64.zip'
CheckedDownload $lock.yazi.url $yaziArchive $lock.yazi.archive_sha256
$yaziUnpack = Join-Path $cache 'yazi-unpack'
Expand-Archive -LiteralPath $yaziArchive -DestinationPath $yaziUnpack -Force
$yazi = Get-ChildItem -LiteralPath $yaziUnpack -Filter yazi.exe -Recurse -File | Select-Object -First 1
$yaziStage = Join-Path $stage 'yazi'
New-Item -ItemType Directory -Force $yaziStage | Out-Null
Copy-Item -LiteralPath (Join-Path $yazi.DirectoryName 'yazi.exe'),(Join-Path $yazi.DirectoryName 'ya.exe') -Destination $yaziStage -Force
$fontArchive = Join-Path $cache 'JetBrainsMono-3.5.1.zip'
CheckedDownload $lock.font.url $fontArchive $lock.font.archive_sha256
$fontUnpack = Join-Path $cache 'font-unpack'
Expand-Archive -LiteralPath $fontArchive -DestinationPath $fontUnpack -Force
$font = Get-ChildItem -LiteralPath $fontUnpack -Filter JetBrainsMonoNerdFontMono-Regular.ttf -Recurse -File | Select-Object -First 1
$fonts = Join-Path $stage 'fonts'
New-Item -ItemType Directory -Force $fonts | Out-Null
Copy-Item -LiteralPath $font.FullName -Destination $fonts -Force
$redist = Join-Path $cache 'vc_redist.x64.exe'
CheckedDownload $lock.vc_redist.url $redist $lock.vc_redist.sha256
Copy-Item -LiteralPath $redist -Destination $stage -Force
$notices = Join-Path $stage 'licenses'
New-Item -ItemType Directory -Force $notices | Out-Null
Copy-Item -Path (Join-Path $root 'licenses\*') -Destination $notices -Force
Copy-Item -LiteralPath (Join-Path $yazi.DirectoryName 'LICENSE') -Destination (Join-Path $notices 'Yazi-LICENSE.txt') -Force
Copy-Item -LiteralPath (Join-Path $root 'LICENSE'),(Join-Path $root 'README.md'),(Join-Path $root 'CHANGELOG.md') -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'release-dependencies.json') -Destination $notices -Force
Copy-Item -LiteralPath (Join-Path $root 'assets\test-images') -Destination $stage -Recurse -Force
Copy-Item -LiteralPath (Join-Path $root 'docs') -Destination $stage -Recurse -Force
$runtimeScripts = Join-Path $stage 'scripts'
New-Item -ItemType Directory -Force $runtimeScripts | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'enable-video.ps1') -Destination $runtimeScripts -Force
'@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\enable-video.ps1" -OutputDirectory "%~dp0."
if errorlevel 1 (echo Video setup failed. Check the connection and retry. & pause & exit /b 1)
pause' | Set-Content -LiteralPath (Join-Path $stage 'Enable-Video.cmd') -Encoding ascii
@'
Afiocira source: https://github.com/LilyAvelis/Afiocira (v2.0.0), MIT.
Yazi 26.5.6: https://github.com/sxyazi/yazi/tree/v26.5.6, MIT.
Everything ES: see Everything-ES-LICENSE.txt and Everything-ES-PROVENANCE.txt.
Nerd Fonts JetBrains Mono3.5.1: https://github.com/ryanoasis/nerd-fonts/tree/v3.5.1; OFL included.
libmpv (downloaded on user machine, not bundled): https://github.com/shinchiro/mpv-winbuild-cmake/releases/tag/20261107
mpv source: https://github.com/mpv-player/mpv/tree/9028187538
Build recipes: https://github.com/shinchiro/mpv-winbuild-cmake/actions/runs/42315459116
mpv Copyright/GPL/LGPL included. Linked components retain their own licenses.
FFmpeg8.1.2 (downloaded on user machine, not bundled): https://www.gyan.dev/ffmpeg/builds/; exact source/config provided by upstream.
VC++ Runtime: https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist
Rust dependency notices: THIRD_PARTY_RUST.html; local egui_tty patch MIT.
'@ | Set-Content -LiteralPath (Join-Path $notices 'RUNTIME-SOURCES.txt') -Encoding utf8
$files = Get-ChildItem -LiteralPath $stage -Recurse -File | Where-Object Name -ne 'PACKAGE-MANIFEST.json' | ForEach-Object {
    [pscustomobject]@{path=[IO.Path]::GetRelativePath($stage,$_.FullName);bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
}
$commit = git -C $root rev-parse HEAD
[pscustomobject]@{version='2.0.0';source_commit=$commit;files=@($files)} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stage 'PACKAGE-MANIFEST.json') -Encoding utf8
Write-Output "Staged release: $stage"
