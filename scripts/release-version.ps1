#requires -Version 7.0
function Get-ReleaseVersion {
    $root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    $versions = foreach ($crate in @('photo-core', 'photo-cli', 'photo-gui', 'photo-launcher')) {
        $manifest = Get-Content -LiteralPath (Join-Path $root "crates/$crate/Cargo.toml") -Raw
        $package = [regex]::Match($manifest, '(?ms)^\[package\]\s*\r?\n(.*?)(?=^\[|\z)')
        $version = [regex]::Match($package.Groups[1].Value, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
        if (-not $version.Success) { throw "Missing stable package version in $crate/Cargo.toml" }
        $version.Groups[1].Value
    }
    if (@($versions | Select-Object -Unique).Count -ne 1) { throw 'The four application package versions must agree before release.' }
    $versions[0]
}
