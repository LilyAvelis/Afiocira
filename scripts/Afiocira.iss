#define Stage "..\target\package-stage"

[Setup]
AppId=Afiocira.LilyAvelis
AppName=Afiocira
AppVersion=0.1.0-alpha
AppPublisher=LilyAvelis
DefaultDirName={localappdata}\Programs\Afiocira
DefaultGroupName=Afiocira
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
LicenseFile=..\LICENSE
OutputDir=..\target\setup
OutputBaseFilename=Afiocira-Setup-Windows11-x64
UninstallDisplayIcon={app}\Afiocira.exe
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#Stage}\Afiocira.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\photo-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\yazi\*"; DestDir: "{app}\yazi"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Stage}\test-images\*"; DestDir: "{app}\test-images"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Stage}\vc_redist.x64.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Afiocira"; Filename: "{app}\Afiocira.exe"; Parameters: """{app}\test-images"""
Name: "{autodesktop}\Afiocira"; Filename: "{app}\Afiocira.exe"; Parameters: """{app}\test-images"""; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Afiocira"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\shell\open\command"; ValueType: string; ValueData: """{app}\Afiocira.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".jpg"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".jpeg"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".png"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".webp"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".gif"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".bmp"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".tif"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\Afiocira.exe\SupportedTypes"; ValueType: string; ValueName: ".tiff"; ValueData: ""

[Run]
Filename: "{app}\vc_redist.x64.exe"; Parameters: "/install /passive /norestart"; StatusMsg: "Installing Microsoft Visual C++ Runtime for Yazi..."; Check: not VCRuntimePresent; Flags: waituntilterminated
Filename: "{app}\Afiocira.exe"; Parameters: """{app}\test-images"""; Description: "Launch Afiocira"; Flags: postinstall nowait skipifsilent

[Code]
function VCRuntimePresent: Boolean;
begin
  Result := FileExists(ExpandConstant('{sys}\vcruntime140.dll'));
end;
