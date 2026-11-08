#ifndef Version
  #define Version "3.0.0"
#endif
#ifndef Stage
  #define Stage "..\target\package-" + Version
#endif
[Setup]
AppId={code:ApplicationId}
AppName=Afiocira
AppVersion={#Version}
AppPublisher=LilyAvelis
DefaultDirName={localappdata}\Programs\Afiocira
DefaultGroupName=Afiocira
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UsePreviousLanguage=no
LicenseFile=..\LICENSE
OutputDir=..\target\release-assets
OutputBaseFilename=Afiocira-{#Version}-Setup-Windows-x64
UninstallDisplayIcon={app}\releases\{#Version}\Afiocira.exe
Compression=lzma2/fast
SolidCompression=yes
[Tasks]
Name: videosupport; Description: "Download video playback and thumbnail components from upstream (Internet required)"; GroupDescription: "Video support:"
Name: desktopicon; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"
[Files]
Source: "{#Stage}\*"; DestDir: "{app}\releases\{#Version}"; Excludes: "AfioLauncher.exe,current-version.txt"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Stage}\AfioLauncher.exe"; DestDir: "{app}"; Flags: ignoreversion
[Icons]
Name: "{autoprograms}\Afiocira"; Filename: "{app}\AfioLauncher.exe"; Check: not TestMode
Name: "{autodesktop}\Afiocira"; Filename: "{app}\AfioLauncher.exe"; Tasks: desktopicon; Check: not TestMode
[Registry]
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Afiocira"; Flags: uninsdeletekey; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\shell\open\command"; ValueType: string; ValueData: """{app}\AfioLauncher.exe"" ""%1"""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".jpg"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".jpeg"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".png"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".webp"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".gif"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".bmp"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".tif"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".tiff"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".avi"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".m4v"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".wmv"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mpeg"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mpg"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".ts"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".mts"; ValueData: ""; Check: not TestMode
Root: HKCU; Subkey: "Software\Classes\Applications\AfioLauncher.exe\SupportedTypes"; ValueType: string; ValueName: ".m2ts"; ValueData: ""; Check: not TestMode
[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\releases\{#Version}\scripts\enable-video.ps1"" -OutputDirectory ""{app}\releases\{#Version}"""; Tasks: videosupport; StatusMsg: "Downloading and verifying video components..."; Flags: runhidden waituntilterminated; AfterInstall: CheckVideoSetup
Filename: "{app}\releases\{#Version}\vc_redist.x64.exe"; Parameters: "/install /passive /norestart"; StatusMsg: "Installing Microsoft Visual C++ Runtime..."; Check: not VCRuntimePresent; Flags: waituntilterminated
Filename: "{app}\AfioLauncher.exe"; Description: "Launch Afiocira"; Flags: postinstall nowait skipifsilent
[Code]
function TestMode: Boolean;
begin
 Result := ExpandConstant('{param:TESTMODE|0}') = '1';
end;
function ApplicationId(Param: String): String;
begin
 if TestMode then Result := 'Afiocira.PackageTest.{#Version}' else Result := 'Afiocira.LilyAvelis';
end;
procedure CheckVideoSetup;
begin
 if not FileExists(ExpandConstant('{app}\releases\{#Version}\libmpv-2.dll')) or not FileExists(ExpandConstant('{app}\releases\{#Version}\ffmpeg.exe')) then begin
  Log('Video download failed; Enable-Video.cmd can retry.');
  if not WizardSilent then MsgBox('Video components could not be downloaded. Photo browsing is available. Close Afiocira and run Enable-Video.cmd from its release folder to retry.',mbInformation,MB_OK);
 end;
end;
function VCRuntimePresent: Boolean;
begin
 Result := FileExists(ExpandConstant('{sys}\vcruntime140.dll'));
end;
procedure CurStepChanged(CurStep: TSetupStep);
begin
 if CurStep = ssPostInstall then
  if not SaveStringToFile(ExpandConstant('{app}\current-version.txt'),'releases\{#Version}\Afiocira.exe',False) then
   RaiseException('Cannot publish the application launcher target.');
end;
