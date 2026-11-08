#requires -Version 7.0
# Offline DevOps regression suite; all executables, downloads and publication calls are mocked.
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'test-ci-common.ps1')
& python (Join-Path $PSScriptRoot 'test-release.py')
if ($LASTEXITCODE -ne 0) { throw "Release self-tests failed: $LASTEXITCODE" }
Write-Host 'PASS: CI provisioning and release self-tests.'
