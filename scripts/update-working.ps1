param([string]$StartFolder, [string]$BuildDirectory)
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$release = if ($BuildDirectory) { (Resolve-Path -LiteralPath $BuildDirectory).Path } else { Join-Path $project 'target\release' }
$app = Join-Path $release 'photo-gui.exe'
$runtime = Join-Path $release 'libmpv-2.dll'
$launcherSource = Join-Path $release 'photo-launcher.exe'
foreach ($file in @($app, $runtime, $launcherSource, (Join-Path $release 'es.exe'), (Join-Path $release 'Everything-ES-LICENSE.txt'), (Join-Path $release 'Everything-ES-PROVENANCE.txt'))) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Build/setup required: $file" }
}
$root = Join-Path $env:LOCALAPPDATA 'Programs\Afiocira'
$version = (Get-FileHash -LiteralPath $app).Hash.Substring(0, 12).ToLowerInvariant()
$relative = "working-versions\$version\photo-gui.exe"
$directory = Join-Path $root "working-versions\$version"
New-Item -ItemType Directory -Force $directory | Out-Null
foreach ($name in @('photo-gui.exe', 'photo-cli.exe', 'libmpv-2.dll', 'es.exe', 'Everything-ES-LICENSE.txt', 'Everything-ES-PROVENANCE.txt')) {
    $source = Join-Path $release $name
    $destination = Join-Path $directory $name
    if (-not (Test-Path -LiteralPath $destination) -or
        (Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $destination).Hash) {
        Copy-Item -LiteralPath $source -Destination $destination -Force
    }
}
$launcher = Join-Path $root 'AfioLauncher.exe'
if (-not (Test-Path -LiteralPath $launcher) -or
    (Get-FileHash -LiteralPath $launcherSource).Hash -ne (Get-FileHash -LiteralPath $launcher).Hash) {
    Copy-Item -LiteralPath $launcherSource -Destination $launcher -Force
}
$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
if ($ffmpeg) { [IO.File]::WriteAllText((Join-Path $root 'ffmpeg-path.txt'), $ffmpeg.Source) }
# Publish the pointer only after all files are ready. Existing open versions stay intact.
$temporary = Join-Path $root 'current-version.next'
$pointer = Join-Path $root 'current-version.txt'
[IO.File]::WriteAllText($temporary, $relative)
if (Test-Path -LiteralPath $pointer) { [IO.File]::Replace($temporary, $pointer, "$pointer.previous") }
else { [IO.File]::Move($temporary, $pointer) }
$desktop = [Environment]::GetFolderPath('DesktopDirectory')
$shortcutPath = Join-Path $desktop 'Afiocira.lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
# Preserve the existing starting folder and custom shortcut arguments.
if ($StartFolder) { $shortcut.Arguments = '"{0}"' -f $StartFolder }
$shortcut.TargetPath = $launcher
$shortcut.WorkingDirectory = $root
$shortcut.Description = 'Afiocira — актуальная рабочая версия'
$shortcut.IconLocation = "$(Join-Path $directory 'photo-gui.exe'),0"
$shortcut.Save()
Write-Output "Working version: $(Join-Path $root $relative)"
Write-Output "Permanent shortcut: $shortcutPath"
