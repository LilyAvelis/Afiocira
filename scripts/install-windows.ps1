param(
    [string]$Executable,
    [string]$StartFolder,
    [string]$ShortcutName = 'Локальная фотомастерская — Yazi.lnk'
)

$ErrorActionPreference = 'Stop'
if (-not $Executable) {
    $Executable = Join-Path $PSScriptRoot '..\target\release\photo-gui.exe'
}
$source = (Resolve-Path -LiteralPath $Executable).Path
$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\PhotoWorkshop'
$hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.Substring(0, 12).ToLowerInvariant()
$installed = Join-Path $installDirectory ("photo-gui-$hash.exe")
New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
if (-not (Test-Path -LiteralPath $installed)) {
    Copy-Item -LiteralPath $source -Destination $installed
}

# Local video builds carry the pinned runtime alongside the GUI. Do not replace
# a different DLL underneath a running older build.
foreach ($runtimeName in @('libmpv-2.dll', 'ffmpeg.exe')) {
    $runtime = Join-Path (Split-Path -Parent $source) $runtimeName
    $target = Join-Path $installDirectory $runtimeName
    if (Test-Path -LiteralPath $runtime) {
        if (-not (Test-Path -LiteralPath $target) -or
            (Get-FileHash -LiteralPath $runtime).Hash -ne (Get-FileHash -LiteralPath $target).Hash) {
            Copy-Item -LiteralPath $runtime -Destination $target -Force
        }
    }
}

$register = Join-Path $PSScriptRoot 'register-windows.ps1'
& $register -Executable $installed -StartFolder $StartFolder -ShortcutName $ShortcutName
Write-Output "Installed application: $installed"
