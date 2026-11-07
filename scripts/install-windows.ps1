param(
    [string]$Executable,
    [string]$StartFolder,
    [string]$ShortcutName = 'Afiocira.lnk'
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

$register = Join-Path $PSScriptRoot 'register-windows.ps1'
& $register -Executable $installed -StartFolder $StartFolder -ShortcutName $ShortcutName
Write-Output "Installed application: $installed"
