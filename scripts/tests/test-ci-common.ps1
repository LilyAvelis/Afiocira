#requires -Version 7.0
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/../ci-common.ps1"
function Start-Sleep { param($Seconds) }
function Assert($Condition, $Message) { if (-not $Condition) { throw $Message } }
foreach ($name in @('tools', 'release-dependencies', 'everything-runtime', 'zig-global-cache')) {
    $expected = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../.ci-cache/$name"))
    Assert ((Get-CiCachePath -Name $name) -eq $expected) 'Native cache must stay outside Rust target pruning'
}
$rejectedCacheName = $false
try { Get-CiCachePath -Name '../target' | Out-Null } catch { $rejectedCacheName = $true }
Assert $rejectedCacheName 'Cache names must reject traversal'
$testRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../target/ci-self-tests'))
$case = Join-Path $testRoot "common-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $case -Force | Out-Null
try {
$payload = Join-Path $case 'payload'
Set-Content -LiteralPath $payload -Value 'verified-payload' -NoNewline
$sha = (Get-FileHash $payload -Algorithm SHA256).Hash
$script:downloadAttempts = 0
$script:downloadMode = 'transient'
function Invoke-WebRequest {
    param($Uri, $OutFile, $TimeoutSec, $ErrorAction)
    $script:downloadAttempts++
    Assert ($OutFile -ne $script:destination -and -not (Test-Path -LiteralPath $script:destination)) 'Incomplete transfer must never be written to the cache destination'
    if ($script:downloadMode -eq 'transient' -and $script:downloadAttempts -lt 3) {
        Set-Content -LiteralPath $OutFile -Value 'incomplete transfer'
        throw [System.Net.Http.HttpRequestException]::new('connection closed')
    }
    if ($script:downloadMode -eq 'always-transient') {
        Set-Content -LiteralPath $OutFile -Value 'incomplete transfer'
        throw [System.Net.Http.HttpRequestException]::new('connection closed')
    }
    if ($script:downloadMode -eq 'deterministic') { throw [System.IO.IOException]::new('permission denied') }
    Copy-Item $payload $OutFile -Force
}
$path = Join-Path $case "download-$([Guid]::NewGuid()).zip"
$script:destination = $path
Invoke-CheckedDownload https://example.invalid $path $sha
Assert ($script:downloadAttempts -eq 3) 'Expected three download attempts'
Invoke-CheckedDownload https://example.invalid $path $sha
Assert ($script:downloadAttempts -eq 3) 'Cached verified archive should not download'
$caught = $false
try { Invoke-CheckedDownload https://example.invalid $path ('0' * 64) } catch { $caught = $_.ToString() -match 'Checksum mismatch' }
Assert $caught 'Corrupt cached archive must fail'
$script:downloadMode = 'success'; $script:downloadAttempts = 0
$bad = Join-Path $case "bad-$([Guid]::NewGuid()).zip"
$script:destination = $bad
$caught = $false
try { Invoke-CheckedDownload https://example.invalid $bad ('0' * 64) } catch { $caught = $_.ToString() -match 'Checksum mismatch' }
Assert ($caught -and $script:downloadAttempts -eq 1 -and -not (Test-Path $bad)) 'Downloaded mismatch must fail without retry/promotion'
$script:downloadMode = 'always-transient'; $script:downloadAttempts = 0
try { Invoke-CheckedDownload https://example.invalid $bad $sha } catch {}
Assert ($script:downloadAttempts -eq 3) 'Network download must stop at three'
$script:downloadMode = 'deterministic'; $script:downloadAttempts = 0
try { Invoke-CheckedDownload https://example.invalid $bad $sha } catch {}
Assert ($script:downloadAttempts -eq 1) 'Non-network download failure must not retry'
Assert (@(Get-ChildItem $case -Filter '*.partial').Count -eq 0) 'Partial download remains'

$script:buildAttempts = 0; $script:buildMode = 'transient'
function cargo {
    $script:seenArguments = @($args)
    $script:buildAttempts++
    if ($script:buildMode -eq 'success') { $global:LASTEXITCODE = 0; return }
    if ($script:buildMode -eq 'transient' -and $script:buildAttempts -eq 3) { $global:LASTEXITCODE = 0; return }
    if ($script:buildMode -eq 'deterministic') { Write-Output 'error[E0308]: mismatched types' }
    else { Write-Output 'error: unable to fetch tarball: HttpConnectionClosing' }
    if ($script:buildMode -eq 'mixed') { Write-Output 'src/main.zig:45:3: error: expected type int' }
    if ($script:buildMode -eq 'ansi-mixed') { Write-Output "$([char]27)[1;31merror$([char]27)[0m$([char]27)[1m[E0308]$([char]27)[0m: mismatched types" }
    $global:LASTEXITCODE = 1
}
Invoke-CargoBuild -Arguments @('build', '--locked')
Assert ($script:buildAttempts -eq 3) 'Build transient retry failed'
Assert ($script:seenArguments[0] -eq 'build') 'Default build must use project toolchain without an explicit override'
foreach ($mode in @('deterministic', 'mixed', 'ansi-mixed', 'always-transient')) {
    $script:buildMode = $mode; $script:buildAttempts = 0; $caught = $false
    try { Invoke-CargoBuild -Arguments @('build', '--locked') } catch { $caught = $true }
    $expected = if ($mode -eq 'always-transient') {3} else {1}
    Assert ($caught -and $script:buildAttempts -eq $expected) "Build $mode had incorrect retries"
}
$script:buildMode = 'success'
Invoke-CargoBuild -Arguments @('build', '--locked') -Toolchain '1.95.0'
Assert ($script:seenArguments[0] -eq '+1.95.0' -and $script:seenArguments[1] -eq 'build') 'Explicit toolchain override must remain supported'
$script:buildAttempts = 0; $caught = $false
try { Invoke-CargoBuild -Arguments @('test') } catch { $caught = $true }
Assert ($caught -and $script:buildAttempts -eq 0) 'Tests must never run through retry helper'
Write-Output 'PASS: download cache/SHA/atomic cleanup/bounded retries; Zig fetch retries/compiler failures/test exclusion'
} finally {
    $cleanup = [IO.Path]::GetFullPath($case)
    if (-not $cleanup.StartsWith($testRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Test cleanup escaped its directory.' }
    Remove-Item -LiteralPath $cleanup -Recurse -Force
}
