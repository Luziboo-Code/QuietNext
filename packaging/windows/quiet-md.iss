#ifndef AppVersion
  #define AppVersion "0.1.20"
#endif
#ifndef AppSource
  #define AppSource "..\..\target\dist"
#endif

[Setup]
AppId={{59DC98EE-A136-40C9-A7BD-EB0CC50E37C2}
AppName=QuietNext
AppVersion={#AppVersion}
AppPublisher=QuietNext
AppPublisherURL=https://github.com/s1dashu/quiet-md
DefaultDirName={localappdata}\Programs\quietnext
DisableDirPage=no
DisableProgramGroupPage=yes
DisableWelcomePage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\..\dist\windows
OutputBaseFilename=quietnext-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\branding\app-icon.ico
UninstallDisplayIcon={app}\QuietNext.exe
WizardStyle=modern light
WizardSmallImageFile=wizard-logo.bmp
WizardSmallImageBackColor=$FFFFFF
WizardImageStretch=no
WizardSizePercent=105,105
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes
SetupLogging=yes
#ifdef Signed
SignTool=quietmd
SignedUninstaller=yes
SignedUninstallerDir={#AppSource}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
SelectDirLabel3=Choose an install folder. Your notes are stored separately.
SelectTasksLabel2=Make QuietNext easier to reach.
FinishedLabel=QuietNext is ready. A quiet place for your notes.

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#AppSource}\QuietNext.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"
Source: "..\..\THIRD_PARTY.md"; DestDir: "{app}"

[Icons]
Name: "{userprograms}\QuietNext"; Filename: "{app}\QuietNext.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\QuietNext"; Filename: "{app}\QuietNext.exe"; WorkingDir: "{app}"; Tasks: desktopicon

; "Open with" for Markdown files and folders (not the default app): they are edited in place.
[Registry]
Root: HKA; Subkey: "Software\Classes\QuietMd.Markdown"; ValueType: string; ValueName: ""; ValueData: "Markdown"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\QuietMd.Markdown\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\QuietNext.exe,0"
Root: HKA; Subkey: "Software\Classes\QuietMd.Markdown\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\QuietNext.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "QuietMd.Markdown"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "QuietMd.Markdown"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\QuietNext.exe\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\QuietNext.exe\SupportedTypes"; ValueType: string; ValueName: ".markdown"; ValueData: ""
; Folders: "Open with QuietNext" in the Explorer's context menu, listed under 本机文件.
Root: HKA; Subkey: "Software\Classes\Directory\shell\QuietMd"; ValueType: string; ValueName: ""; ValueData: "Open with QuietNext"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\QuietMd"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\QuietNext.exe,0"
Root: HKA; Subkey: "Software\Classes\Directory\shell\QuietMd\command"; ValueType: string; ValueName: ""; ValueData: """{app}\QuietNext.exe"" ""%1"""

[Run]
Filename: "{app}\QuietNext.exe"; Description: "Launch QuietNext"; Flags: nowait postinstall skipifsilent
; The in-app updater installs silently with /UPDATE=1 and expects the app back afterwards.
Filename: "{app}\QuietNext.exe"; Flags: nowait; Check: IsUpdate

[Code]
function IsUpdate: Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;

procedure InitializeWizard;
begin
  WizardForm.Caption := 'QuietNext';
end;
