#ifndef AppVersion
  #error "Pass /DAppVersion=<version> when compiling this installer"
#endif
#ifndef PayloadDir
  #error "Pass /DPayloadDir=<release output directory> when compiling this installer"
#endif
#ifndef RepoRoot
  #error "Pass /DRepoRoot=<repo root> when compiling this installer"
#endif

[Setup]
AppId={{8F6A9C3E-2D4B-4B7A-9C1E-7E8A3D5F2B10}
AppName=Antigravity Sync
AppVersion={#AppVersion}
AppPublisher=Caspar
DefaultDirName={localappdata}\AntigravitySync
DefaultGroupName=Antigravity Sync
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
UninstallDisplayIcon={app}\AntigravitySync.exe
OutputDir=..\dist
OutputBaseFilename=AntigravitySync_Setup_x64_{#AppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
Uninstallable=yes

[Tasks]
Name: "desktopicon"; Description: "Create Desktop shortcuts"; GroupDescription: "Additional shortcuts:"; Check: not IsUpdateMode

[Files]
Source: "{#PayloadDir}\AntigravitySync.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\plugin\plugin.json"; DestDir: "{%USERPROFILE}\.gemini\antigravity\plugins\antigravity-sync"; Flags: ignoreversion
Source: "{#RepoRoot}\plugin\package.json"; DestDir: "{%USERPROFILE}\.gemini\antigravity\plugins\antigravity-sync"; Flags: ignoreversion
Source: "{#RepoRoot}\plugin\server.mjs"; DestDir: "{%USERPROFILE}\.gemini\antigravity\plugins\antigravity-sync"; Flags: ignoreversion

[Icons]
Name: "{group}\Antigravity Sync"; Filename: "{app}\AntigravitySync.exe"
Name: "{autodesktop}\Antigravity Sync"; Filename: "{app}\AntigravitySync.exe"; Tasks: desktopicon
Name: "{autodesktop}\Push Antigravity"; Filename: "{app}\AntigravitySync.exe"; Parameters: "--push --relaunch"; Tasks: desktopicon
Name: "{autodesktop}\Pull Antigravity"; Filename: "{app}\AntigravitySync.exe"; Parameters: "--pull --relaunch"; Tasks: desktopicon

[Run]
Filename: "{app}\AntigravitySync.exe"; Description: "Launch Antigravity Sync"; Flags: postinstall nowait skipifsilent

[Code]
function IsUpdateMode: Boolean;
begin
  Result := Pos('/UPDATE', Uppercase(GetCmdTail())) > 0;
end;

procedure WriteMcpConfig;
var
  PluginDir, ServerPath, JsonText: String;
begin
  PluginDir := ExpandConstant('{%USERPROFILE}\.gemini\antigravity\plugins\antigravity-sync');
  ServerPath := PluginDir + '\server.mjs';
  StringChangeEx(ServerPath, '\', '\\', True);
  JsonText := '{"mcpServers":{"antigravity-sync":{"command":"node","args":["' + ServerPath + '"]}}}';
  SaveStringToFile(PluginDir + '\mcp_config.json', JsonText, False);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    WriteMcpConfig;
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\AntigravitySync');
  end;
end;
