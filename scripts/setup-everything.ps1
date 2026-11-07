param([string]$OutputDirectory = (Join-Path $PSScriptRoot '..\target\debug'))
$ErrorActionPreference = 'Stop'
# Official voidtools ES, MIT licensed. No installer, index, or Everything settings changes.
# Provenance: https://www.voidtools.com/downloads/
# Protocol: https://www.voidtools.com/support/everything/command_line_interface/
$cache = Join-Path $PSScriptRoot '..\target\everything-runtime'
New-Item -ItemType Directory -Force $cache, $OutputDirectory | Out-Null
$packages = @(
    @{ Name = 'ES-1.1.0.38.x64.zip'; Hash = '5E0C70CBF4F694080C34AA7C6C745E606C16FE76A4B5423B93EBF9DC34274C99'; Directory = 'binary' },
    @{ Name = 'ES-1.1.0.38.src.zip'; Hash = '2E36D1177633218821CBF1CE6430BA88E6FC5192CC81A1C254D16E72DF63A8EE'; Directory = 'source' }
)
foreach ($package in $packages) {
    $archive = Join-Path $cache $package.Name
    if (-not (Test-Path -LiteralPath $archive)) {
        Invoke-WebRequest ('https://www.voidtools.com/' + $package.Name) -OutFile $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.Hash) {
        throw ('Everything archive SHA-256 mismatch: ' + $package.Name)
    }
    Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $cache $package.Directory) -Force
}
Copy-Item -LiteralPath (Join-Path $cache 'binary\es.exe') -Destination (Join-Path $OutputDirectory 'es.exe') -Force
Copy-Item -LiteralPath (Join-Path $cache 'source\ES-1.1.0.38\LICENSE') -Destination (Join-Path $OutputDirectory 'Everything-ES-LICENSE.txt') -Force
@'
Everything ES 1.1.0.38 x64, Copyright (c) 2025 voidtools, MIT License.
Official binary: https://www.voidtools.com/ES-1.1.0.38.x64.zip
Binary archive SHA256: 5E0C70CBF4F694080C34AA7C6C745E606C16FE76A4B5423B93EBF9DC34274C99
Official source and license: https://www.voidtools.com/ES-1.1.0.38.src.zip
Source archive SHA256: 2E36D1177633218821CBF1CE6430BA88E6FC5192CC81A1C254D16E72DF63A8EE
Requires an already installed and running Everything desktop application.
'@ | Set-Content -LiteralPath (Join-Path $OutputDirectory 'Everything-ES-PROVENANCE.txt') -Encoding utf8
Write-Output "Everything search helper ready: $OutputDirectory"
