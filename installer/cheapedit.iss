; Inno Setup script for cheapedit.
; Normally built by scripts\release.ps1, which passes the version:
;   iscc /DAppVersion=1.2.3 installer\cheapedit.iss
; Output: installer\Output\cheapedit-Setup-<version>.exe
;
; The in-app updater runs this installer with /SILENT /RELAUNCH=1: it updates
; in place (same folder, same install mode) and reopens cheapedit afterwards.

#define AppName "cheapedit"
#ifndef AppVersion
  #define AppVersion "0.0.0-local"
#endif
#define AppExe "cheapedit.exe"

[Setup]
AppId={{6E1F2C7A-4B8D-4C3E-9A51-0C7D2E9F8B13}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=cheapedit
; Per-user install by default (no admin); the wizard offers "Install for all users".
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; {autopf} = %LOCALAPPDATA%\Programs for per-user, Program Files for all users.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
SetupIconFile=..\assets\cheapedit.ico
WizardStyle=modern
OutputDir=Output
OutputBaseFilename=cheapedit-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "contextmenu"; Description: "Add ""Edit with cheapedit"" to the right-click menu for video files"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\SETUP.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; "Edit with cheapedit" on video files. HKA = the current user for per-user installs,
; all users for admin installs. Each key is removed again on uninstall.
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp4\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp4\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mp4\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mkv\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mkv\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mkv\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mov\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mov\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mov\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.webm\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.webm\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.webm\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.avi\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.avi\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.avi\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4v\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4v\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m4v\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wmv\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wmv\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.wmv\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ts\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ts\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.ts\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mts\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mts\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mts\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m2ts\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m2ts\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.m2ts\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flv\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flv\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.flv\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpg\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpg\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpg\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpeg\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpeg\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.mpeg\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.3gp\shell\cheapedit"; ValueType: string; ValueName: ""; ValueData: "Edit with cheapedit"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.3gp\shell\cheapedit"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe}"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.3gp\shell\cheapedit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch cheapedit"; Flags: nowait postinstall skipifsilent
; After an in-app update, reopen cheapedit (as the normal user, never elevated).
Filename: "{app}\{#AppExe}"; Flags: nowait runasoriginaluser; Check: IsRelaunch

[Messages]
FinishedLabel=cheapedit is installed.%n%ncheapedit uses FFmpeg and libmpv, which you install once for your whole PC. If they aren't set up yet, cheapedit will walk you through it the first time it opens.

[Code]
// True when started by cheapedit's updater (/RELAUNCH=1).
function IsRelaunch: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

// On uninstall, ask whether to also remove settings and logs (default: keep them).
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Dir := ExpandConstant('{userappdata}\cheapedit');
    if DirExists(Dir) and not UninstallSilent then
      if MsgBox('Also remove your cheapedit settings and logs?' + #13#10 + #13#10 + Dir,
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(Dir, True, True, True);
  end;
end;
