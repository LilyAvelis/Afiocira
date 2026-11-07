param(
    [string]$GuiExecutable = 'target\remapped\release\photo-gui.exe',
    [string]$CliExecutable = 'target\remapped\release\photo-cli.exe'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target = Join-Path $root 'target'
$stage = [System.IO.Path]::GetFullPath((Join-Path $target 'package-stage'))
$unpack = [System.IO.Path]::GetFullPath((Join-Path $target 'package-yazi-unpack'))
$downloads = Join-Path $target 'package-downloads'
$allowed = [System.IO.Path]::GetFullPath($target).TrimEnd('\') + '\'
foreach ($path in @($stage, $unpack)) {
    if (-not $path.StartsWith($allowed, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Package path escapes target: $path"
    }
}

$gui = (Resolve-Path -LiteralPath (Join-Path $root $GuiExecutable)).Path
$cli = (Resolve-Path -LiteralPath (Join-Path $root $CliExecutable)).Path
New-Item -ItemType Directory -Path $downloads -Force | Out-Null

function Get-CheckedDownload {
    param([string]$Url, [string]$Destination, [string]$Sha256)
    if ((-not (Test-Path -LiteralPath $Destination)) -or
        ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash -ne $Sha256)) {
        Invoke-WebRequest -Uri $Url -OutFile $Destination
    }
    $actual = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
    if ($actual -ne $Sha256) {
        throw "Download hash mismatch for $Destination. Expected $Sha256; got $actual"
    }
}

$yaziZip = Join-Path $downloads 'yazi-26.5.6-windows-x64.zip'
Get-CheckedDownload `
    -Url 'https://github.com/sxyazi/yazi/releases/download/v26.5.6/yazi-x86_64-pc-windows-msvc.zip' `
    -Destination $yaziZip `
    -Sha256 '6C6C52A4B2648E179F917BDAA7C57E793D18561B380A8BFA025F10CD1B9B2AD1'
$vcRedist = Join-Path $downloads 'vc_redist.x64.exe'
Get-CheckedDownload `
    -Url 'https://aka.ms/vc14/vc_redist.x64.exe' `
    -Destination $vcRedist `
    -Sha256 '843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C'

foreach ($path in @($stage, $unpack)) {
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}
New-Item -ItemType Directory -Path $stage,$unpack -Force | Out-Null
Expand-Archive -LiteralPath $yaziZip -DestinationPath $unpack
$yazi = Get-ChildItem -LiteralPath $unpack -Filter yazi.exe -Recurse -File |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.DirectoryName 'ya.exe') } |
    Select-Object -First 1
if (-not $yazi) { throw 'The verified Yazi archive has no matching yazi.exe and ya.exe pair' }

New-Item -ItemType Directory -Path (Join-Path $stage 'yazi'),(Join-Path $stage 'licenses'),(Join-Path $stage 'test-images') -Force | Out-Null
Copy-Item -LiteralPath $gui -Destination (Join-Path $stage 'Afiocira.exe')
Copy-Item -LiteralPath $cli -Destination (Join-Path $stage 'photo-cli.exe')
Copy-Item -LiteralPath $vcRedist -Destination (Join-Path $stage 'vc_redist.x64.exe')
Copy-Item -LiteralPath (Join-Path $yazi.DirectoryName 'yazi.exe'),(Join-Path $yazi.DirectoryName 'ya.exe') -Destination (Join-Path $stage 'yazi')
foreach ($name in @('LICENSE','README.md')) {
    $source = Join-Path $yazi.DirectoryName $name
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination (Join-Path $stage 'yazi')
    }
}
Copy-Item -Path (Join-Path $root 'licenses\*') -Destination (Join-Path $stage 'licenses')
Copy-Item -Path (Join-Path $root 'assets\test-images\*') -Destination (Join-Path $stage 'test-images')
Write-Output "Staged release files: $stage"
