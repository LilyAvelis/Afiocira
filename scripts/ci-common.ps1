#requires -Version 7.0
# Shared by CI provisioning and release staging. Downloads are trusted only after SHA256 verification.
function Invoke-CheckedDownload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory, Position = 0)][string]$Url,
        [Parameter(Mandatory, Position = 1)][string]$Path,
        [Parameter(Mandatory, Position = 2)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$Sha256
    )
    $Path = [IO.Path]::GetFullPath($Path)
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Sha256) {
            throw "Checksum mismatch in cached archive: $Path"
        }
        return
    }
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path)) | Out-Null
    $temporary = "$Path.$([Guid]::NewGuid().ToString('N')).partial"
    try {
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                Invoke-WebRequest -Uri $Url -OutFile $temporary -TimeoutSec 180 -ErrorAction Stop
                break
            } catch {
                $status = $_.Exception.Response.StatusCode
                $retryable = if ($null -ne $status) {
                    [int]$status -in @(408, 425, 429, 500, 502, 503, 504)
                } else {
                    $_.Exception -is [System.Net.Http.HttpRequestException] -or
                    $_.Exception -is [System.Net.WebException] -or
                    $_.Exception -is [System.Threading.Tasks.TaskCanceledException]
                }
                if (-not $retryable -or $attempt -eq 3) { throw }
                Write-Warning "Download attempt $attempt failed for $Url; retrying."
                Start-Sleep -Seconds (2 * $attempt)
            }
        }
        if ((Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash -ne $Sha256) {
            throw "Checksum mismatch in downloaded archive: $Url"
        }
        # The temporary file is on the same volume; an incomplete transfer never becomes a cache hit.
        [IO.File]::Move($temporary, $Path, $true)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Invoke-CargoBuild {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string[]]$Arguments,
        [string]$Toolchain = ''
    )
    if ($Arguments.Count -eq 0 -or $Arguments[0] -ne 'build') {
        throw 'Invoke-CargoBuild accepts cargo build only; run tests directly so test failures are never retried.'
    }
    $PSNativeCommandUseErrorActionPreference = $false
    $cargoArguments = if ($Toolchain) { @("+$Toolchain") + $Arguments } else { $Arguments }
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $output = @()
        & cargo @cargoArguments 2>&1 | Tee-Object -Variable output | ForEach-Object { Write-Host $_ }
        $exitCode = $LASTEXITCODE
        if ($exitCode -eq 0) { return }
        $log = [regex]::Replace(($output -join "`n"), '\x1b\[[0-?]*[ -/]*[@-~]', '')
        # Zig's package fetcher can fail after a dependency server closes an HTTP connection.
        # Require fetch context and a named transport failure; compiler diagnostics are never retryable.
        $fetchFailure = $log -match '(?i)(unable to fetch|failed to fetch|fetching|zig\s+fetch|download.*tarball)'
        $zigContext = $log -match '(?i)(libghostty|\.zig(?:\.zon)?|\bzig(?:\.exe)?\s|\bHttpConnectionClosing\b)'
        $transportFailure = $log -match '(?i)\b(HttpConnectionClosing|ConnectionResetByPeer|ConnectionTimedOut|ConnectionRefused|TemporaryNameServerFailure|NetworkUnreachable|TlsConnectionTruncated)\b'
        $compilerFailure = $log -match '(?m)(^\s*error\[E\d+\]|\.zig:\d+:\d+: error:)'
        if ($attempt -eq 3 -or -not $zigContext -or -not $fetchFailure -or -not $transportFailure -or $compilerFailure) {
            throw "cargo build failed with exit code $exitCode (attempt $attempt)."
        }
        Write-Warning "Zig dependency fetch failed transiently; retrying cargo build ($attempt/3)."
        Start-Sleep -Seconds (2 * $attempt)
    }
}
