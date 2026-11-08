#requires -Version 7.0
[CmdletBinding()]
param([string]$TargetDirectory = 'target/debug/deps')
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ci-common.ps1')
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target = if ([IO.Path]::IsPathRooted($TargetDirectory)) { $TargetDirectory } else { Join-Path $root $TargetDirectory }
$cache = Join-Path $root 'target/release-dependencies'
$lock = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'release-dependencies.json') -Raw | ConvertFrom-Json

# Match release staging's cache names so CI test and packaging jobs can share the verified archives.
$yaziArchive = Join-Path $cache "yazi-$($lock.yazi.version)-windows-x64.zip"
$fontArchive = Join-Path $cache "JetBrainsMono-$($lock.font.version).zip"
Invoke-CheckedDownload $lock.yazi.url $yaziArchive $lock.yazi.archive_sha256
Invoke-CheckedDownload $lock.font.url $fontArchive $lock.font.archive_sha256
$yaziUnpack = Join-Path $cache 'yazi-unpack'
$fontUnpack = Join-Path $cache 'font-unpack'
foreach ($dependency in @(
    @{ Archive = $yaziArchive; Directory = $yaziUnpack; Hash = $lock.yazi.archive_sha256 },
    @{ Archive = $fontArchive; Directory = $fontUnpack; Hash = $lock.font.archive_sha256 }
)) {
    $marker = Join-Path $dependency.Directory '.archive-sha256'
    if (-not (Test-Path -LiteralPath $marker) -or (Get-Content -LiteralPath $marker -Raw).Trim() -ne $dependency.Hash) {
        Expand-Archive -LiteralPath $dependency.Archive -DestinationPath $dependency.Directory -Force
        Set-Content -LiteralPath $marker -Value $dependency.Hash -Encoding ascii
    }
}
$yazi = Get-ChildItem -LiteralPath $yaziUnpack -Filter yazi.exe -Recurse -File | Select-Object -First 1
$font = Get-ChildItem -LiteralPath $fontUnpack -Filter JetBrainsMonoNerdFontMono-Regular.ttf -Recurse -File | Select-Object -First 1
if (-not $yazi -or -not (Test-Path -LiteralPath (Join-Path $yazi.DirectoryName 'ya.exe'))) { throw 'The pinned Yazi archive is missing yazi.exe or ya.exe.' }
if (-not $font) { throw 'The pinned font archive is missing JetBrainsMonoNerdFontMono-Regular.ttf.' }
$yaziTarget = Join-Path $target 'yazi'
$fontTarget = Join-Path $target 'fonts'
New-Item -ItemType Directory -Path $yaziTarget,$fontTarget -Force | Out-Null
Copy-Item -LiteralPath $yazi.FullName,(Join-Path $yazi.DirectoryName 'ya.exe') -Destination $yaziTarget -Force
Copy-Item -LiteralPath $font.FullName -Destination $fontTarget -Force
Write-Host "Native test fixtures ready: $target"
