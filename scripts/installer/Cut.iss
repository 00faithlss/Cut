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
MinVersion=10.0.22000
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CloseApplications=no
SetupIconFile=cut.ico
UninstallDisplayIcon={app}\cut.ico
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
spanish.WebView2Error=No se pudo instalar Microsoft Edge WebView2. Comprueba tu conexión a Internet y vuelve a ejecutar el instalador.
english.LaunchCut=Open Cut
english.WebView2Error=Microsoft Edge WebView2 could not be installed. Check your Internet connection and run Setup again.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#Bootstrapper}"; Flags: dontcopy
Source: "{#BuildDirectory}\cut.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildDirectory}\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "cut.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\docs\superpowers\concepts\ui-assets\*-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Cut"; Filename: "{app}\cut.exe"; WorkingDir: "{app}"; IconFilename: "{app}\cut.ico"
Name: "{autodesktop}\Cut"; Filename: "{app}\cut.exe"; WorkingDir: "{app}"; IconFilename: "{app}\cut.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\cut.exe"; Description: "{cm:LaunchCut}"; Flags: postinstall nowait skipifsilent

[Code]
const
  WebView2Key = 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  BootstrapperName = 'MicrosoftEdgeWebview2Setup.exe';
  StartupKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

function HasWebView2(RootKey: Integer): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(RootKey, WebView2Key, 'pv', Version) and
    (Version <> '') and (Version <> '0.0.0.0');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if HasWebView2(HKLM32) or HasWebView2(HKCU) then
    Exit;

  Log('Instalando Microsoft Edge WebView2 Runtime.');
  ExtractTemporaryFile(BootstrapperName);
  if Exec(ExpandConstant('{tmp}\' + BootstrapperName), '/silent /install', '',
    SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Log(Format('Resultado de WebView2: %d', [ExitCode]));

  if not (HasWebView2(HKLM32) or HasWebView2(HKCU)) then
    Result := CustomMessage('WebView2Error');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StartupCommand: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  if RegQueryStringValue(HKCU, StartupKey, 'Cut', StartupCommand) and
    (CompareText(StartupCommand, '"' + ExpandConstant('{app}\cut.exe') + '"') = 0) then
    RegDeleteValue(HKCU, StartupKey, 'Cut');
end;
