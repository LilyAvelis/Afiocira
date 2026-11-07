param(
    [string]$Executable,
    [string]$ShortcutName = 'Afiocira.lnk',
    [string]$StartFolder,
    [switch]$SkipOpenWith
)

$ErrorActionPreference = 'Stop'

if (-not $Executable) {
    $Executable = Join-Path $PSScriptRoot '..\target\release\photo-gui.exe'
}
$appPath = (Resolve-Path -LiteralPath $Executable).Path
$projectDirectory = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$desktopDirectory = Join-Path $env:USERPROFILE 'OneDrive\Desktop'
if (-not (Test-Path -LiteralPath $desktopDirectory -PathType Container)) {
    $desktopDirectory = [Environment]::GetFolderPath('DesktopDirectory')
}

$shortcutPath = Join-Path $desktopDirectory $ShortcutName
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $appPath
$shortcut.Arguments = if ($StartFolder) { '"{0}"' -f $StartFolder } else { '' }
$shortcut.WorkingDirectory = $projectDirectory
$shortcut.Description = 'Open Afiocira'
$shortcut.IconLocation = "$appPath,0"
$shortcut.Save()

# Register as an available Open With application for formats the current decoder supports.
# This does not change any default file association.
if (-not $SkipOpenWith) {
    $applicationKey = 'HKCU:\Software\Classes\Applications\photo-gui.exe'
    $commandKey = Join-Path $applicationKey 'shell\open\command'
    $typesKey = Join-Path $applicationKey 'SupportedTypes'
    New-Item -Path $commandKey -Force | Out-Null
    New-Item -Path $typesKey -Force | Out-Null
    New-ItemProperty -Path $applicationKey -Name 'FriendlyAppName' -Value 'Afiocira' -PropertyType String -Force | Out-Null
    Set-Item -Path $commandKey -Value ('"{0}" "%1"' -f $appPath)
    foreach ($extension in @('.jpg', '.jpeg', '.png', '.webp', '.gif', '.bmp', '.tif', '.tiff')) {
        New-ItemProperty -Path $typesKey -Name $extension -Value '' -PropertyType String -Force | Out-Null
    }

    $appPathsKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\photo-gui.exe'
    New-Item -Path $appPathsKey -Force | Out-Null
    Set-Item -Path $appPathsKey -Value $appPath
}

Write-Output "Shortcut: $shortcutPath"
if (-not $SkipOpenWith) {
    Write-Output "Open With application: $appPath"
}
