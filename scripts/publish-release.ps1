#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('\A[0-9]+\.[0-9]+\.[0-9]+\z')][string]$Version,
    [Parameter(Mandatory)][string]$AssetsDirectory,
    [Parameter(Mandatory)][string]$NotesFile
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$tag = "v$Version"
if ($env:GITHUB_REF -cne "refs/tags/$tag") { throw 'Publishing requires the matching GitHub tag ref.' }
if ($env:GITHUB_SHA -notmatch '\A[a-fA-F0-9]{40}\z') { throw 'GITHUB_SHA must identify the release source commit.' }
if ($env:GITHUB_REPOSITORY -notmatch '\A[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\z' -or
    @($env:GITHUB_REPOSITORY.Split('/') | Where-Object { $_ -in @('.', '..') }).Count) { throw 'GITHUB_REPOSITORY is required.' }
if (-not $env:GH_TOKEN) { throw 'Publishing requires GH_TOKEN supplied through the environment.' }
$repository = $env:GITHUB_REPOSITORY
$assets = (Resolve-Path -LiteralPath $AssetsDirectory).Path
$notes = Get-Content -LiteralPath $NotesFile -Raw
& (Join-Path $PSScriptRoot 'verify-release.ps1') -AssetsDirectory $assets -ExpectedCommit $env:GITHUB_SHA -Version $Version
$marker = '<!-- afioci-managed-release -->'
$names = @("Afiocira-$Version-Setup-Windows-x64.exe", "Afiocira-$Version-Windows-x64.zip", 'SHA256SUMS.txt')
$local = @{}
foreach ($name in $names) {
    $file = Get-Item -LiteralPath (Join-Path $assets $name)
    $local[$name] = @{ Size = $file.Length; Digest = 'sha256:' + (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Test-TransientGhError([string]$Message) {
    return $Message -match '(?i)(HTTP (408|429|500|502|503|504)\b|connection (reset|closed|refused)|TLS handshake timeout|i/o timeout|temporary failure in name resolution|unexpected EOF|network is unreachable)'
}
function Invoke-Gh {
    param([Parameter(Mandatory)][string[]]$Arguments, [switch]$ReadOnly)
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $result = & gh @Arguments 2>&1
        $code = $LASTEXITCODE
        $message = ($result | ForEach-Object { "$_" }) -join "`n"
        if ($code -eq 0) { return $message }
        if (-not $ReadOnly -or $attempt -eq 3 -or -not (Test-TransientGhError $message)) { throw "gh failed (exit $code): $message" }
        Start-Sleep -Seconds (2 * $attempt)
    }
}
function Read-GhJson([string]$Endpoint, [switch]$Paginate) {
    $arguments = @('api', '--hostname', 'github.com', '-H', 'Accept: application/vnd.github+json', '-H', 'X-GitHub-Api-Version: 2022-11-28')
    if ($Paginate) { $arguments += @('--paginate', '--slurp') }
    $value = Invoke-Gh -Arguments ($arguments + @($Endpoint)) -ReadOnly | ConvertFrom-Json
    if ($Paginate) { foreach ($page in $value) { foreach ($item in $page) { $item } } } else { $value }
}
function Assert-RemoteTag {
    $ref = Read-GhJson "repos/$repository/git/ref/tags/$tag"
    if ($ref.ref -cne "refs/tags/$tag") { throw 'Remote tag ref mismatch.' }
    $object = $ref.object
    for ($depth = 0; $object.type -eq 'tag' -and $depth -lt 3; $depth++) {
        if ($object.sha -notmatch '^[a-fA-F0-9]{40}$') { throw 'Invalid annotated tag SHA.' }
        $object = (Read-GhJson "repos/$repository/git/tags/$($object.sha)").object
    }
    if ($object.type -ne 'commit' -or $object.sha -ne $env:GITHUB_SHA) { throw 'Remote tag does not resolve to GITHUB_SHA.' }
}
function Read-Release {
    $matches = @(Read-GhJson "repos/$repository/releases?per_page=100" -Paginate | Where-Object { $_.tag_name -ceq $tag })
    if ($matches.Count -gt 1) { throw 'Multiple releases have the requested tag.' }
    if ($matches.Count) { return $matches[0] }
}
function Assert-ManagedRelease($Release) {
    if (-not $Release -or "$($Release.id)" -notmatch '^\d+$' -or $Release.tag_name -cne $tag -or
        $Release.url -ine "https://api.github.com/repos/$repository/releases/$($Release.id)") { throw 'Release identity or repository mismatch.' }
    if ("$($Release.body)".IndexOf($marker, [StringComparison]::Ordinal) -lt 0) { throw 'Release is missing the management marker.' }
}
function Assert-ManagedDraft($Release) {
    Assert-ManagedRelease $Release
    if (-not $Release.draft) { throw 'Refusing to modify a published release.' }
}
function Read-Assets($Release) {
    return @(Read-GhJson "repos/$repository/releases/$($Release.id)/assets?per_page=100" -Paginate)
}
function Test-MatchingAsset($Asset, [string]$Name) {
    return $Asset -and $Asset.name -ceq $Name -and $Asset.state -eq 'uploaded' -and
        $Asset.size -eq $local[$Name].Size -and $Asset.digest -ceq $local[$Name].Digest
}
function Assert-RemoteAssets($Release) {
    $remoteAssets = Read-Assets $Release
    if ($remoteAssets.Count -ne 3) { throw 'Remote release must contain exactly three assets.' }
    foreach ($name in $names) {
        $remote = @($remoteAssets | Where-Object { $_.name -ceq $name })
        if ($remote.Count -ne 1 -or -not (Test-MatchingAsset $remote[0] $name)) { throw "Remote asset verification failed: $name" }
    }
}

# An empty config prevents gh from falling back to credentials or aliases saved on disk.
$previousConfig = $env:GH_CONFIG_DIR
$previousHost = $env:GH_HOST
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]@('/', '\')) + [IO.Path]::DirectorySeparatorChar
$temporary = Join-Path $temporaryRoot "afiocira-publish-$([Guid]::NewGuid().ToString('N'))"
[IO.Directory]::CreateDirectory($temporary) | Out-Null
$env:GH_CONFIG_DIR = $temporary
$env:GH_HOST = 'github.com'
try {
    $notesPath = Join-Path $temporary 'release-notes.md'
    if ($notes.IndexOf($marker, [StringComparison]::Ordinal) -lt 0) { $notes += "`n`n$marker`n" }
    Set-Content -LiteralPath $notesPath -Value $notes -Encoding utf8
    Assert-RemoteTag
    $release = Read-Release
    if ($release -and -not $release.draft) {
        # A previous publication may have succeeded despite a lost edit response.
        # Confirm the entire public result using reads only; never repair public assets or notes.
        Assert-ManagedRelease $release
        Assert-RemoteAssets $release
        Assert-RemoteTag
        Write-Host "Already published and verified Afiocira ${Version}: $($release.html_url)"
        return
    }
    if ($release) { Assert-ManagedDraft $release } else {
        # Creation is intentionally not retried: on an ambiguous response, rerun to resume the marked draft.
        Invoke-Gh -Arguments @('release', 'create', $tag, '--repo', $repository, '--draft', '--verify-tag', '--target', $env:GITHUB_SHA, '--title', "Afiocira $Version", '--notes-file', $notesPath) | Out-Null
        $release = Read-Release
        Assert-ManagedDraft $release
    }
    $releaseId = $release.id
    $remoteAssets = Read-Assets $release
    if (@($remoteAssets | Where-Object { $_.name -cnotin $names }).Count) { throw 'Managed draft contains unexpected assets; refusing to publish or delete them.' }
    foreach ($name in $names) {
        $existing = @($remoteAssets | Where-Object { $_.name -ceq $name })
        if ($existing.Count -gt 1) { throw "Duplicate remote asset: $name" }
        if ($existing.Count -eq 1 -and (Test-MatchingAsset $existing[0] $name)) { continue }
        if ($existing.Count -eq 1) {
            # Recheck ownership immediately before deleting only this expected asset from our draft.
            Assert-ManagedDraft (Read-GhJson "repos/$repository/releases/$releaseId")
            if ("$($existing[0].id)" -notmatch '^\d+$') { throw 'Invalid asset ID.' }
            Invoke-Gh -Arguments @('api', '--hostname', 'github.com', '--method', 'DELETE', "repos/$repository/releases/assets/$($existing[0].id)") | Out-Null
        }
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            Assert-ManagedDraft (Read-GhJson "repos/$repository/releases/$releaseId")
            $uploadError = $null
            try { Invoke-Gh -Arguments @('release', 'upload', $tag, (Join-Path $assets $name), '--repo', $repository) | Out-Null } catch { $uploadError = $_ }
            # A lost upload response may still have created the asset. Read before deciding to retry.
            $uploaded = @(Read-Assets $release | Where-Object { $_.name -ceq $name })
            if ($uploaded.Count -eq 1 -and (Test-MatchingAsset $uploaded[0] $name)) { break }
            if ($uploaded.Count -gt 0) { throw "Remote asset digest/size mismatch after upload: $name" }
            if (-not $uploadError) { throw "Uploaded asset missing: $name" }
            if ($attempt -eq 3 -or -not (Test-TransientGhError "$uploadError")) { throw $uploadError }
            Start-Sleep -Seconds (2 * $attempt)
        }
    }
    Assert-RemoteTag
    $release = Read-GhJson "repos/$repository/releases/$releaseId"
    Assert-ManagedDraft $release
    Assert-RemoteAssets $release
    Invoke-Gh -Arguments @('release', 'edit', $tag, '--repo', $repository, '--notes-file', $notesPath, '--draft=false', '--latest') | Out-Null
    $published = Read-GhJson "repos/$repository/releases/$releaseId"
    if ($published.draft -or $published.tag_name -cne $tag) { throw 'Release publication was not confirmed.' }
    Write-Host "Published Afiocira ${Version}: $($published.html_url)"
} finally {
    $env:GH_CONFIG_DIR = $previousConfig
    $env:GH_HOST = $previousHost
    if (-not [IO.Path]::GetFullPath($temporary).StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Temporary cleanup path escaped its root.' }
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
