#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$BuildDirectory = 'target/distribution/release',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ci-common.ps1')
. (Join-Path $PSScriptRoot 'release-version.ps1')
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$version = Get-ReleaseVersion
$stage = Join-Path $root "target/package-$version"
$assets = Join-Path $root 'target/release-assets'
$requestedBuild = [IO.Path]::GetFullPath($(if ([IO.Path]::IsPathRooted($BuildDirectory)) { $BuildDirectory } else { Join-Path $root $BuildDirectory }))
$defaultBuild = [IO.Path]::GetFullPath((Join-Path $root 'target/distribution/release'))
if (-not $SkipBuild -and $requestedBuild -ne $defaultBuild) { throw 'A custom BuildDirectory requires SkipBuild; the normal build always stages its own distribution output.' }
if ($SkipBuild -and $env:GITHUB_ACTIONS -eq 'true') { throw 'CI release candidates must build their exact source; SkipBuild is local smoke-test only.' }

if (-not $SkipBuild) {
    $sep = [char]31
    $env:CARGO_ENCODED_RUSTFLAGS = '-C' + $sep + 'target-feature=+crt-static' + $sep + "--remap-path-prefix=$env:USERPROFILE=AFIOCIRA_BUILD" + $sep + "--remap-path-prefix=$root=AFIOCIRA_SOURCE"
    Push-Location $root
    try { Invoke-CargoBuild -Arguments @('build', '--locked', '--release', '--workspace', '--target-dir', 'target/distribution') }
    finally { Pop-Location }
}
& (Join-Path $PSScriptRoot 'stage-windows-release.ps1') -BuildDirectory $BuildDirectory -OutputDirectory $stage
New-Item -ItemType Directory -Force $assets | Out-Null
& ISCC.exe "/DVersion=$version" "/DStage=$stage" "/O$assets" /Q (Join-Path $PSScriptRoot 'Afiocira.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed: $LASTEXITCODE" }
$zip = Join-Path $assets "Afiocira-$version-Windows-x64.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
@("Afiocira-$version-Setup-Windows-x64.exe", "Afiocira-$version-Windows-x64.zip") | ForEach-Object {
    '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $assets $_) -Algorithm SHA256).Hash.ToLowerInvariant(), $_
} | Set-Content -LiteralPath (Join-Path $assets 'SHA256SUMS.txt') -Encoding ascii
& (Join-Path $PSScriptRoot 'verify-release.ps1') -Version $version -StageDirectory $stage -AssetsDirectory $assets
Write-Host "Verified release candidate: $assets"
