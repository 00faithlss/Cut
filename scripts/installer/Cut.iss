[Setup]
AppId=CutClipper
AppName=Cut
AppVersion={#AppVersion}
AppPublisher=00faithlss
AppPublisherURL=https://github.com/00faithlss/Cut
AppMutex=Local\CutClipper.Instancia
DefaultDirName={localappdata}\Programs\Cut
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
; Windows 10 1809 (LTSC 2019) or later. Cut no longer needs WebView2.
MinVersion=10.0.17763
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CloseApplications=no
SetupIconFile=cut.ico
UninstallDisplayIcon={app}\Cut.exe
VersionInfoVersion={#AppVersion}
OutputBaseFilename=Cut-{#AppVersion}-Setup-x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
spanish.LaunchCut=Abrir Cut
english.LaunchCut=Open Cut

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[InstallDelete]
; Left over by the WebView2 interface (0.2.x).
Type: files; Name: "{app}\WebView2Loader.dll"

[Files]
Source: "{#BuildDirectory}\Cut.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "cut.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\licenses\*-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Cut"; Filename: "{app}\Cut.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Cut"; Filename: "{app}\Cut.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\Cut.exe"; Description: "{cm:LaunchCut}"; Flags: postinstall nowait skipifsilent

[Code]
const
  StartupKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StartupCommand: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  if RegQueryStringValue(HKCU, StartupKey, 'Cut', StartupCommand) and
    (CompareText(StartupCommand, '"' + ExpandConstant('{app}\Cut.exe') + '"') = 0) then
    RegDeleteValue(HKCU, StartupKey, 'Cut');
end;
