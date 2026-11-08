param([string]$BuildDirectory = 'target\distribution\release', [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ci-common.ps1')
. (Join-Path $PSScriptRoot 'release-version.ps1')
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$version = Get-ReleaseVersion
if (-not $OutputDirectory) { $OutputDirectory = "target/package-$version" }
$buildPath = if ([IO.Path]::IsPathRooted($BuildDirectory)) {$BuildDirectory} else {Join-Path $root $BuildDirectory}
$build = (Resolve-Path -LiteralPath $buildPath).Path
$stage = if ([IO.Path]::IsPathRooted($OutputDirectory)) {$OutputDirectory} else {[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
$cache = Get-CiCachePath -Name release-dependencies
$targetRoot = [IO.Path]::GetFullPath((Join-Path $root 'target'))
$expectedStage = [IO.Path]::GetFullPath((Join-Path $targetRoot "package-$version"))
$isDefaultStage = [IO.Path]::GetFullPath($stage) -eq $expectedStage
$ownerPath = Join-Path $targetRoot "package-$version.owner.json"
if ($isDefaultStage -and (Test-Path -LiteralPath $targetRoot) -and
    ((Get-Item -LiteralPath $targetRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Default target directory must not be a link.' }
if ((Test-Path -LiteralPath $stage) -and ((Get-Item -LiteralPath $stage).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Staging directory must not be a link.' }
# Restaging may remove only our marked default output inside this checkout's target.
if (Test-Path -LiteralPath $stage) {
    $existing = Get-ChildItem -LiteralPath $stage -Force | Select-Object -First 1
    if ($existing) {
        $resolvedStage = (Resolve-Path -LiteralPath $stage).ProviderPath
        $marker = Join-Path $stage 'PACKAGE-MANIFEST.json'
        if (-not $isDefaultStage -or $resolvedStage -ne $expectedStage) {
            throw 'Output directory is not an owned default stage; choose an empty output directory.'
        }
        $owned = $false
        if (Test-Path -LiteralPath $ownerPath -PathType Leaf) {
            $owner = Get-Content -LiteralPath $ownerPath -Raw | ConvertFrom-Json
            $owned = $owner.owner -eq 'afiocira-release-stage-v1' -and $owner.version -eq $version
        } elseif (Test-Path -LiteralPath $marker -PathType Leaf) {
            $previous = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
            $owned = $previous.version -eq $version -and $previous.source_commit -match '^[0-9a-f]{40}$'
        }
        if (-not $owned) { throw 'Unrecognized stage marker; directory preserved.' }
        if (@(Get-ChildItem -LiteralPath $resolvedStage -Recurse -Force | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) {
            throw 'Owned stage contains links; automatic cleanup refused.'
        }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
}
New-Item -ItemType Directory -Force $stage,$cache | Out-Null
if ($isDefaultStage) {
    # A separate marker survives failed downloads/copies, without entering the package.
    $temporaryOwner = "$ownerPath.$([Guid]::NewGuid().ToString('N')).next"
    try {
        [IO.File]::WriteAllText($temporaryOwner, (@{ owner = 'afiocira-release-stage-v1'; version = $version } | ConvertTo-Json))
        [IO.File]::Move($temporaryOwner, $ownerPath, $true)
    } finally { if (Test-Path -LiteralPath $temporaryOwner) { Remove-Item -LiteralPath $temporaryOwner -Force } }
}
$lock = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'release-dependencies.json') -Raw | ConvertFrom-Json
# Video runtimes are downloaded directly from upstream on the end user machine.
& (Join-Path $PSScriptRoot 'setup-everything.ps1') -OutputDirectory $stage
Copy-Item -LiteralPath (Join-Path $build 'photo-gui.exe') -Destination (Join-Path $stage 'Afiocira.exe') -Force
Copy-Item -LiteralPath (Join-Path $build 'photo-cli.exe') -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $build 'photo-launcher.exe') -Destination (Join-Path $stage 'AfioLauncher.exe') -Force
[IO.File]::WriteAllText((Join-Path $stage 'current-version.txt'),'Afiocira.exe')
$yaziArchive = Join-Path $cache "yazi-$($lock.yazi.version)-windows-x64.zip"
Invoke-CheckedDownload $lock.yazi.url $yaziArchive $lock.yazi.archive_sha256
$yaziUnpack = Join-Path $cache 'yazi-unpack'
Expand-Archive -LiteralPath $yaziArchive -DestinationPath $yaziUnpack -Force
$yazi = Get-ChildItem -LiteralPath $yaziUnpack -Filter yazi.exe -Recurse -File | Select-Object -First 1
$yaziStage = Join-Path $stage 'yazi'
New-Item -ItemType Directory -Force $yaziStage | Out-Null
Copy-Item -LiteralPath (Join-Path $yazi.DirectoryName 'yazi.exe'),(Join-Path $yazi.DirectoryName 'ya.exe') -Destination $yaziStage -Force
$fontArchive = Join-Path $cache "JetBrainsMono-$($lock.font.version).zip"
Invoke-CheckedDownload $lock.font.url $fontArchive $lock.font.archive_sha256
$fontUnpack = Join-Path $cache 'font-unpack'
Expand-Archive -LiteralPath $fontArchive -DestinationPath $fontUnpack -Force
$font = Get-ChildItem -LiteralPath $fontUnpack -Filter JetBrainsMonoNerdFontMono-Regular.ttf -Recurse -File | Select-Object -First 1
$fonts = Join-Path $stage 'fonts'
New-Item -ItemType Directory -Force $fonts | Out-Null
Copy-Item -LiteralPath $font.FullName -Destination $fonts -Force
$redist = Join-Path $cache 'vc_redist.x64.exe'
Invoke-CheckedDownload $lock.vc_redist.url $redist $lock.vc_redist.sha256
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
@"
Afiocira source: https://github.com/LilyAvelis/Afiocira (v$version), MIT.
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
"@ | Set-Content -LiteralPath (Join-Path $notices 'RUNTIME-SOURCES.txt') -Encoding utf8
$files = Get-ChildItem -LiteralPath $stage -Recurse -File | Where-Object Name -ne 'PACKAGE-MANIFEST.json' | ForEach-Object {
    [pscustomobject]@{path=[IO.Path]::GetRelativePath($stage,$_.FullName);bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
}
$commit = git -C $root rev-parse HEAD
[pscustomobject]@{version=$version;source_commit=$commit;files=@($files)} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stage 'PACKAGE-MANIFEST.json') -Encoding utf8
Write-Output "Staged release: $stage"
