; Inno Setup script for Ella Desktop (Windows 11 x64).
;
; Inno Setup rather than MSIX (§11's open question): MSIX wants a Store-style
; signing and packaging flow, and this ships to campuses by USB stick and
; direct download. Inno gives a single signed .exe, works offline, and does not
; care where the file came from.
;
; Build (from the repo root, after building the Flutter app and orchestrator):
;   iscc desktop\installer\ella.iss
;
; STILL REQUIRED, and not solvable in this file (§10):
;   * A code-signing certificate. Unsigned installers hit SmartScreen and
;     students will simply not get past it. Obtaining one takes WEEKS — start
;     in Phase 1, not here. Sign both the installer and ella-orchestrator.exe.
;   * Antivirus submission. PyInstaller bundles draw false positives; submit
;     the signed binary to the major vendors before a wide rollout.

#define AppName "Ella"
#define AppPublisher "NavGurukul"
#define AppExeName "ella_app.exe"
#define AppVersion "1.3.7"

; Set on the command line for a models-in-installer build:
;   iscc /DIncludeModels desktop\installer\ella.iss
; Without it the installer is ~120 MB and models download on first run.
; With it the installer is ~4 GB but needs no connection at all — which is the
; right trade on a campus with a bad line and a working USB stick.

[Setup]
AppId={{9E4A1C7B-3F52-4A0D-9C1E-7B2D6A8F1E34}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
; Per-user install by default: campus machines rarely give students admin, and
; requiring it is the difference between "it installed" and "it did not".
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=EllaSetup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
; A 4 GB payload needs the disk check to be honest about it.
ExtraDiskSpaceRequired=0
UninstallDisplayIcon={app}\{#AppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; \
    GroupDescription: "{cm:AdditionalIcons}"

[Files]
; The Flutter release build.
Source: "..\..\ella_app\build\windows\x64\runner\Release\*"; \
    DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

; The PyInstaller one-dir orchestrator, plus the native engines.
Source: "..\dist\ella-orchestrator\*"; \
    DestDir: "{app}\engines\bin\ella-orchestrator"; \
    Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\engines\bin\*"; DestDir: "{app}\engines\bin"; \
    Flags: ignoreversion recursesubdirs createallsubdirs

#ifdef IncludeModels
; ~3.5 GB. Slow to build, but the student needs no connection.
Source: "..\engines\models\*"; DestDir: "{app}\engines\models"; \
    Flags: ignoreversion recursesubdirs createallsubdirs
#endif

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; \
    Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; \
    Flags: nowait postinstall skipifsilent

[UninstallDelete]
; The local database and any downloaded models live outside [Files], so the
; uninstaller has to name them. zoe.db holds the student's progress, so it is
; deliberately NOT deleted here — a reinstall should find their garden intact.
Type: filesandordirs; Name: "{app}\engines\models"
Type: dirifempty; Name: "{app}\engines"

[Code]
function InitializeSetup(): Boolean;
var
  FreeMB, TotalMB: Int64;
begin
  Result := True;
  { Models plus the app need roughly 6 GB of headroom. Failing here with a
    clear message beats failing halfway through a 4 GB copy. }
  if GetSpaceOnDisk(ExpandConstant('{autopf}'), True, FreeMB, TotalMB) then
  begin
    if FreeMB < 6144 then
    begin
      MsgBox('Ella needs about 6 GB of free disk space. ' + #13#10 +
             'This drive has ' + IntToStr(FreeMB) + ' MB free.',
             mbError, MB_OK);
      Result := False;
    end;
  end;
end;
