#requires -Version 7.0
[CmdletBinding()]
param([switch]$Installer)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
. (Join-Path $PSScriptRoot 'ci-common.ps1')
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$cache = Get-CiCachePath -Name tools
$toolchainPath = Join-Path $root 'rust-toolchain.toml'
$toolchainConfig = Get-Content -LiteralPath $toolchainPath -Raw
$channels = [regex]::Matches($toolchainConfig, '(?m)^\s*channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*(?:#.*)?$')
if ($channels.Count -ne 1) { throw 'rust-toolchain.toml must pin exactly one numeric x.y.z channel.' }
$rustVersion = $channels[0].Groups[1].Value
$zigVersion = '0.15.2'
# Upstream https://ziglang.org/download/index.json, 0.15.2 -> x86_64-windows (verified 2026-11-08).
$zigSha256 = '3a0ed1e8799a2f8ce2a6e6290a9ff22e6906f8227865911fb7ddedc3cc14cb0c'
$zigDirectory = Join-Path $cache "zig-x86_64-windows-$zigVersion"
$zigArchive = "$zigDirectory.zip"

function Add-CiPath([string]$Directory) {
    $env:PATH = "$Directory$([IO.Path]::PathSeparator)$env:PATH"
    if ($env:GITHUB_PATH) { Add-Content -LiteralPath $env:GITHUB_PATH -Value $Directory -Encoding utf8 }
}
function Set-CiOutput([string]$Name, [string]$Value) {
    Write-Host "$Name=$Value"
    if ($env:GITHUB_OUTPUT) { Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "$Name=$Value" -Encoding utf8 }
}
function Get-CompatibleInnoVersion {
    param($VersionInfo)
    foreach ($candidate in @($VersionInfo.ProductVersion, $VersionInfo.FileVersion)) {
        $match = [regex]::Match("$candidate", '\A\s*([0-9]+\.[0-9]+\.[0-9]+)(?:\D|\z)')
        if (-not $match.Success) { continue }
        $version = [version]$match.Groups[1].Value
        if ($version -eq [version]'0.0.0') { continue }
        if ($version -lt [version]'6.7.1' -or $version -ge [version]'7.0.0') {
            throw "Unsupported Inno Setup version $version; require >=6.7.1 and <7.0.0."
        }
        return $version.ToString(3)
    }
    # Some official Inno builds use 0.0.0 PE metadata; query the engine through a no-output compilation.
    return $null
}

# Installing an explicit toolchain never changes the developer's global Rust default.
& rustup toolchain install $rustVersion --profile minimal --component rustfmt --component clippy --no-self-update --no-update
if ($LASTEXITCODE -ne 0) { throw "rustup toolchain install failed: $LASTEXITCODE" }
$rustIdentity = (& rustc "+$rustVersion" -vV) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "rustc version check failed: $LASTEXITCODE" }
if ($rustIdentity -notmatch "(?m)^release: $([regex]::Escape($rustVersion))$") {
    throw "Unexpected Rust toolchain: $rustIdentity"
}

Invoke-CheckedDownload -Url "https://ziglang.org/download/$zigVersion/zig-x86_64-windows-$zigVersion.zip" -Path $zigArchive -Sha256 $zigSha256
$zigMarker = Join-Path $zigDirectory '.archive-sha256'
if (-not (Test-Path -LiteralPath (Join-Path $zigDirectory 'zig.exe') -PathType Leaf) -or
    -not (Test-Path -LiteralPath $zigMarker) -or (Get-Content -LiteralPath $zigMarker -Raw).Trim() -ne $zigSha256) {
    Expand-Archive -LiteralPath $zigArchive -DestinationPath $cache -Force
    Set-Content -LiteralPath $zigMarker -Value $zigSha256 -Encoding ascii
}
$installedZig = & (Join-Path $zigDirectory 'zig.exe') version
if ($LASTEXITCODE -ne 0 -or "$installedZig".Trim() -ne $zigVersion) {
    throw "Unexpected Zig toolchain: $installedZig (exit $LASTEXITCODE)"
}
Add-CiPath $zigDirectory

if ($Installer) {
    $innoDirectories = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6'),
        (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6')
    )
    $inno = $innoDirectories | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'ISCC.exe') } | Select-Object -First 1
    if (-not $inno) {
        & choco install innosetup --version=6.7.1 --yes --no-progress
        if ($LASTEXITCODE -ne 0) { throw "Inno Setup installation failed: $LASTEXITCODE" }
        $inno = $innoDirectories | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'ISCC.exe') } | Select-Object -First 1
    }
    if (-not $inno) { throw 'Inno Setup 6 ISCC.exe was not found after installation.' }
    $innoCompiler = Join-Path $inno 'ISCC.exe'
    $innoVersion = Get-CompatibleInnoVersion (Get-Item -LiteralPath $innoCompiler).VersionInfo
    if (-not $innoVersion) {
        $probe = Join-Path $cache 'inno-version-probe.iss'
        @'
[Setup]
AppName=Afiocira CI compiler version probe
AppVersion=0.0.0
DefaultDirName={tmp}\AfiociraCompilerVersionProbe
Uninstallable=no
Output=no
'@ | Set-Content -LiteralPath $probe -Encoding ascii
        $banner = (& $innoCompiler '/O-' $probe 2>&1) -join "`n"
        if ($LASTEXITCODE -ne 0) { throw "Inno Setup version probe failed (exit $LASTEXITCODE): $banner" }
        $match = [regex]::Match($banner, 'Compiler engine version:\s*Inno Setup\s+([0-9]+\.[0-9]+\.[0-9]+)\b')
        if (-not $match.Success) { throw 'Inno Setup compiler did not report its engine version.' }
        $innoVersion = Get-CompatibleInnoVersion ([pscustomobject]@{ ProductVersion = $match.Groups[1].Value; FileVersion = $null })
        if (-not $innoVersion) { throw 'Inno Setup compiler reported an invalid engine version.' }
    }
    Set-CiOutput 'inno-version' $innoVersion
    Add-CiPath $inno
}

$imageVersion = if ($env:ImageVersion) { $env:ImageVersion } else { [Environment]::OSVersion.Version.ToString() }
$toolchainHash = (Get-FileHash -LiteralPath $toolchainPath -Algorithm SHA256).Hash
$identity = "$rustIdentity`n$($env:ImageOS)`n$imageVersion`n$toolchainHash"
$hasher = [Security.Cryptography.SHA256]::Create()
try { $environmentKey = [BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($identity))).Replace('-', '').ToLowerInvariant() } finally { $hasher.Dispose() }
Set-CiOutput 'environment-key' $environmentKey
Set-CiOutput 'image-version' $imageVersion
Set-CiOutput 'rust-version' $rustVersion
Set-CiOutput 'zig-version' $zigVersion
