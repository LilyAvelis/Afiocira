param(
    [string]$ShortcutName = 'Локальная фотомастерская — Yazi.lnk'
)

$ErrorActionPreference = 'Stop'
$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\PhotoWorkshop'
$prefix = [System.IO.Path]::GetFullPath($installDirectory).TrimEnd('\') + '\'
$appPathsKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\photo-gui.exe'
$installed = if (Test-Path -LiteralPath $appPathsKey) {
    (Get-Item -LiteralPath $appPathsKey).GetValue('')
} else { $null }
if (-not $installed -or -not $installed.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    Write-Output 'No Photo Workshop registration in the install directory.'
    exit 0
}
$applicationKey = 'HKCU:\Software\Classes\Applications\photo-gui.exe'
$commandKey = Join-Path $applicationKey 'shell\open\command'
$expectedCommand = '"{0}" "%1"' -f $installed
if ((Test-Path -LiteralPath $commandKey) -and
    (Get-Item -LiteralPath $commandKey).GetValue('') -eq $expectedCommand) {
    Remove-Item -LiteralPath $applicationKey -Recurse
}

if ((Test-Path -LiteralPath $appPathsKey) -and
    (Get-Item -LiteralPath $appPathsKey).GetValue('') -eq $installed) {
    Remove-Item -LiteralPath $appPathsKey -Recurse
}

$desktopDirectory = Join-Path $env:USERPROFILE 'OneDrive\Desktop'
if (-not (Test-Path -LiteralPath $desktopDirectory -PathType Container)) {
    $desktopDirectory = [Environment]::GetFolderPath('DesktopDirectory')
}
$shortcutPath = Join-Path $desktopDirectory $ShortcutName
if (Test-Path -LiteralPath $shortcutPath) {
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    if ($shortcut.TargetPath -eq $installed) {
        Remove-Item -LiteralPath $shortcutPath
    }
}

Write-Output 'Registration removed where it pointed to this installation. The executable was kept.'
