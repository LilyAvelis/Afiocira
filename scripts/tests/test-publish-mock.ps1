#requires -Version 7.0
param([Parameter(Mandatory)][string]$AssetsDirectory)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$assets = (Resolve-Path -LiteralPath $AssetsDirectory).Path
$notes = Join-Path (Split-Path -Parent $assets) 'mock-release-notes.md'
Set-Content $notes 'Release notes'
$env:GITHUB_REF = 'refs/tags/v2.5.0'; $env:GITHUB_SHA = 'a'*40
$env:GITHUB_REPOSITORY = 'owner/repo'; $env:GH_TOKEN = 'nonfunctional-local-mock'
function Assert($Condition, $Message) { if (-not $Condition) { throw $Message } }
function global:gh {
    $a = @($args); $global:LASTEXITCODE = 0
    $global:calls.Add(($a -join ' '))
    if ($a[0] -eq 'api') {
        $endpoint = $a[-1]
        if ($endpoint -like '*/git/ref/*') { return @{ref='refs/tags/v2.5.0';object=@{type='commit';sha=$(if ($global:mode -eq 'tag') {'b'*40} else {'a'*40})}} | ConvertTo-Json -Depth 6 -Compress }
        if ($endpoint -eq 'repos/owner/repo/releases?per_page=100') {
            if ($global:release) { return '[[' + ($global:release | ConvertTo-Json -Compress -Depth 5) + ']]' }
            return '[[]]'
        }
        if ($endpoint -eq 'repos/owner/repo/releases/123') { return $global:release | ConvertTo-Json -Compress -Depth 5 }
        if ($endpoint -eq 'repos/owner/repo/releases/123/assets?per_page=100') { return '[' + (ConvertTo-Json -InputObject @($global:remoteAssets) -Compress -Depth 5) + ']' }
        if ($a -contains 'DELETE') { $global:remoteAssets.Clear(); return '' }
        throw "Unexpected API: $endpoint"
    }
    if ($a[0] -ne 'release') { throw 'Unknown mocked command' }
    switch ($a[1]) {
        create { $global:release = @{id=123;tag_name='v2.5.0';draft=$true;url='https://api.github.com/repos/owner/repo/releases/123';body='<!-- afioci-managed-release -->';html_url='https://example.invalid/release'}; return '' }
        upload {
            $file = Get-Item $a[3]; $hash = (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            $global:remoteAssets.Add(@{id=($global:remoteAssets.Count+1);name=$file.Name;state='uploaded';size=$file.Length;digest=$(if ($global:mode -eq 'digest') {'sha256:' + '0'*64} else {'sha256:'+$hash})})
            return ''
        }
        edit { $global:release.draft = $false; $global:published = $true; return '' }
        default { throw 'Unknown mocked release command' }
    }
}
$global:calls = [Collections.Generic.List[string]]::new()
foreach ($gate in @('ref', 'sha', 'repository', 'token')) {
    $env:GITHUB_REF = 'refs/tags/v2.5.0'; $env:GITHUB_SHA = 'a' * 40
    $env:GITHUB_REPOSITORY = 'owner/repo'; $env:GH_TOKEN = 'nonfunctional-local-mock'
    $expected = switch ($gate) {
        ref { $env:GITHUB_REF = 'refs/heads/main'; 'matching GitHub tag ref' }
        sha { $env:GITHUB_SHA = 'invalid'; 'GITHUB_SHA must' }
        repository { $env:GITHUB_REPOSITORY = '../repo'; 'GITHUB_REPOSITORY is required' }
        token { $env:GH_TOKEN = ''; 'requires GH_TOKEN' }
    }
    $failure = ''
    try { & "$root/scripts/publish-release.ps1" -Version '2.5.0' -AssetsDirectory $assets -NotesFile $notes } catch { $failure = "$_" }
    Assert ($failure.Contains($expected) -and $global:calls.Count -eq 0) "Publisher did not enforce $gate gate before invoking gh"
    Write-Host "PASS publish gate: $gate"
}
$env:GH_TOKEN = 'nonfunctional-local-mock'
$failures = @{ 'public-mismatch' = 'Remote asset verification failed'; 'public-unmarked' = 'management marker'; unmarked = 'management marker'; tag = 'does not resolve'; digest = 'digest/size mismatch'; unknown = 'unexpected assets' }
foreach ($mode in @('public-exact','public-mismatch','public-unmarked','unmarked','tag','digest','unknown','fresh','resume')) {
    $global:mode = $mode; $global:published = $false
    $global:calls = [Collections.Generic.List[string]]::new()
    $global:remoteAssets = [Collections.Generic.List[object]]::new()
    $global:release = $null
    if ($mode -like 'public-*' -or $mode -in @('unmarked','resume','unknown')) {
        $global:release = @{id=123;tag_name='v2.5.0';draft=($mode -notlike 'public-*');url='https://api.github.com/repos/owner/repo/releases/123';body=$(if ($mode -in @('unmarked','public-unmarked')) {'user release'} else {'<!-- afioci-managed-release -->'});html_url='https://example.invalid/release'}
    }
    if ($mode -like 'public-*') {
        foreach ($file in Get-ChildItem -LiteralPath $assets -File) {
            $digest = 'sha256:' + (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($mode -eq 'public-mismatch') { $digest = 'sha256:' + ('0' * 64) }
            $global:remoteAssets.Add(@{ id = $global:remoteAssets.Count + 1; name = $file.Name; size = $file.Length; state = 'uploaded'; digest = $digest })
        }
    }
    if ($mode -eq 'unknown') { $global:remoteAssets.Add(@{ id = 99; name = 'keep-user-asset.txt'; state = 'uploaded'; size = 1; digest = 'sha256:' + ('0' * 64) }) }
    $caught = $false; $failure = ''
    try { & "$root/scripts/publish-release.ps1" -Version '2.5.0' -AssetsDirectory $assets -NotesFile $notes } catch { $caught = $true; $failure = "$_" }
    $success = $mode -in @('fresh','resume','public-exact')
    $mutationExpected = $mode -in @('fresh','resume')
    Assert (($global:published -eq $mutationExpected) -and ($caught -eq (-not $success))) "Incorrect result: $mode"
    if (-not $success) { Assert ($failure.Contains($failures[$mode])) "Incorrect failure reason for $mode`: $failure" }
    if ($mode -like 'public-*' -or $mode -in @('unmarked','tag')) { Assert (@($global:calls | Where-Object { $_ -match '^release (create|upload|edit)|--method DELETE' }).Count -eq 0) "Unsafe mutation: $mode" }
    if ($success) { Assert ($global:remoteAssets.Count -eq 3) 'Expected 3 verified uploaded assets' }
    if ($mode -eq 'unknown') { Assert ($global:remoteAssets.Count -eq 1 -and $global:remoteAssets[0].id -eq 99) 'Unknown draft asset must be preserved' }
    Write-Host "PASS publish mock: $mode"
}
Write-Host 'PASS: only the local gh function was invoked; no external gh or network writes.'
