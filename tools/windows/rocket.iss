; Optional per-user installer. The portable/update payload remains byte-for-byte intact.
; Package-Installer.ps1 generates the exact [Files] list and pins the payload manifest.
#ifndef PayloadInclude
  #error Build with tools/Package-Installer.ps1; an explicit verified payload is required.
#endif
#ifndef ProductId
  #define ProductId "Rocket.Desktop.9b64bde0-90e8-4696-b681-7d383e8c3f0b"
#endif
#ifndef ProductName
  #define ProductName "Rocket"
#endif
#ifndef OutputBase
  #define OutputBase "Rocket-" + RocketVersion + "-Setup-x64"
#endif

[Setup]
AppId={#ProductId}
AppName={#ProductName}
AppVersion={#RocketVersion}
AppPublisher=JasXTD
AppPublisherURL=https://github.com/JasXTD/Rocket
AppSupportURL=https://github.com/JasXTD/Rocket/issues
AppUpdatesURL=https://github.com/JasXTD/Rocket/releases
DefaultDirName={localappdata}\Programs\{#ProductName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
DisableProgramGroupPage=yes
DisableReadyPage=no
UsePreviousAppDir=yes
UninstallFilesDir={app}\uninstall
UninstallDisplayIcon={app}\app\rocket.exe
SetupIconFile={#SourcePath}..\..\crates\rocket-ui\src\rocket.ico
OutputDir={#OutputDirectory}
OutputBaseFilename={#OutputBase}
Compression=lzma2/normal
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
AllowCancelDuringInstall=yes
SetupLogging=yes
UninstallLogging=yes
ChangesEnvironment=no

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
#include PayloadInclude

[Icons]
Name: "{autoprograms}\{#ProductName}"; Filename: "{app}\app\rocket.exe"; WorkingDir: "{app}\app"
Name: "{autodesktop}\{#ProductName}"; Filename: "{app}\app\rocket.exe"; WorkingDir: "{app}\app"; Tasks: desktopicon

[INI]
Filename: "{app}\uninstall\rocket-install.ini"; Section: "Rocket"; Key: "Identity"; String: "{#ProductId}"; Flags: uninsdeleteentry uninsdeletesectionifempty
Filename: "{app}\uninstall\rocket-install.ini"; Section: "Rocket"; Key: "Layout"; String: "1"; Flags: uninsdeleteentry uninsdeletesectionifempty

[Run]
Filename: "{app}\app\rocket.exe"; Description: "Open {#ProductName}"; Flags: nowait postinstall skipifsilent unchecked; Check: PayloadIsReady

[Code]
type
  { Inno Setup 6 runs a 32-bit wizard even when it installs a 64-bit app. }
  NativeHandle = Integer;
const
  LayoutIdentity = '{#ProductId}';
  ManifestName = 'SHA256SUMS.txt';
  ReparsePoint = $400;
  DirectoryAttribute = $10;
  InvalidAttributes = $FFFFFFFF;
var
  StageRoot, PreviousRoot, OldManifestHash, FailureText: String;
  Prepared, PayloadPublished: Boolean;
  DirectoryHandles: array of NativeHandle;
  DirectoryNames: TArrayOfString;

function GetFileAttributesW(Name: String): Cardinal;
  external 'GetFileAttributesW@kernel32.dll stdcall';
function CreateFileW(Name: String; Access, Sharing: Cardinal; Security: NativeHandle;
  Creation, Flags: Cardinal; Template: NativeHandle): NativeHandle;
  external 'CreateFileW@kernel32.dll stdcall';
function CloseHandle(Handle: NativeHandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';
function GetCurrentProcessId: Cardinal;
  external 'GetCurrentProcessId@kernel32.dll stdcall';

procedure ReleaseDirectories;
var I: Integer;
begin
  for I := GetArrayLength(DirectoryHandles)-1 downto 0 do CloseHandle(DirectoryHandles[I]);
  SetArrayLength(DirectoryHandles, 0);
  SetArrayLength(DirectoryNames, 0);
end;

procedure PinDirectory(const Name: String);
var H: NativeHandle; N: Integer; A: Cardinal;
begin
  A := GetFileAttributesW(Name);
  if (A = InvalidAttributes) or ((A and ReparsePoint) <> 0) or
     ((A and DirectoryAttribute) = 0) then RaiseException('Unsafe or unavailable installation directory. Files were preserved.');
  { Prevent replacement of a directory while its children are checked or removed. }
  H := CreateFileW(Name, $80, 3, 0, 3, $02200000, 0);
  if H = -1 then RaiseException('Could not protect the installation directory. Close other setup/update processes and retry.');
  A := GetFileAttributesW(Name);
  if (A = InvalidAttributes) or ((A and ReparsePoint) <> 0) then begin
    CloseHandle(H); RaiseException('The installation directory changed. Files were preserved.');
  end;
  N := GetArrayLength(DirectoryHandles); SetArrayLength(DirectoryHandles, N+1); DirectoryHandles[N] := H;
  SetArrayLength(DirectoryNames, N+1); DirectoryNames[N] := Name;
end;

procedure CheckAncestors(const Name: String);
var P, Parent: String; A: Cardinal;
begin
  P := RemoveBackslashUnlessRoot(ExpandFileName(Name));
  if (Length(ExtractFileDrive(P)) <> 2) or (Length(P) < 5) then
    RaiseException('Choose a dedicated folder on a local drive.');
  while Length(P) > 3 do begin
    A := GetFileAttributesW(P);
    if (A <> InvalidAttributes) and (((A and ReparsePoint) <> 0) or ((A and DirectoryAttribute) = 0)) then
      RaiseException('Installation through a link or a file is not allowed.');
    Parent := ExtractFileDir(P); if Parent = P then Break; P := Parent;
  end;
end;

function SafeRelative(const Name: String): Boolean;
var Parts: TArrayOfString; I, J: Integer; C: String;
begin
  Result := False;
  if (Name = '') or (Length(Name) > 1024) or (Pos('\', Name) <> 0) then Exit;
  Parts := StringSplit(Name, ['/'], stAll);
  for I := 0 to GetArrayLength(Parts)-1 do begin
    if (Parts[I] = '') or (Parts[I] = '.') or (Parts[I] = '..') or
       (CompareText(Parts[I], '.git') = 0) or (CompareText(Parts[I], '.ssh') = 0) then Exit;
    C := Copy(Parts[I],Length(Parts[I]),1); if (C = '.') or (C = ' ') then Exit;
    for J := 1 to Length(Parts[I]) do begin
      C := Copy(Parts[I],J,1);
      if (Ord(C[1]) < 32) or (Pos(C, ':<>"|?*') <> 0) then Exit;
    end;
  end;
  Result := True;
end;

function NativeRelative(Name: String): String;
begin StringChangeEx(Name, '/', '\', True); Result := Name; end;

function LoadManifest(const Root: String; Names, Hashes: TStringList): Boolean;
var Lines: TArrayOfString; I, J: Integer; Name, Digest, Text: String; Raw: AnsiString; Size: Int64;
begin
  Result := False;
  if not FileSize64(Root+'\'+ManifestName, Size) or (Size > 4194304) then RaiseException('Package manifest is missing or exceeds its size limit.');
  if not LoadStringFromFile(Root+'\'+ManifestName, Raw) then RaiseException('Package manifest could not be read.');
  { Release manifests are UTF-8 without a BOM. Decode explicitly; Inno 6 otherwise
    treats an unmarked file as the local Windows code page. }
  Text := UTF8Decode(Raw);
  if (Length(Text) > 0) and (Text[1] = #$FEFF) then Delete(Text,1,1);
  StringChangeEx(Text,#13,'',True); Lines := StringSplit(Text,[#10],stAll);
  if GetArrayLength(Lines) > 20000 then RaiseException('Package manifest has too many records.');
  for I := 0 to GetArrayLength(Lines)-1 do begin
    if Lines[I] = '' then Continue;
    if (Length(Lines[I]) < 67) or (Copy(Lines[I],65,2) <> '  ') then RaiseException('Malformed package manifest record '+IntToStr(I+1)+'.');
    Digest := LowerCase(Copy(Lines[I],1,64)); Name := Copy(Lines[I],67,Length(Lines[I]));
    for J := 1 to 64 do if Pos(Digest[J], '0123456789abcdef') = 0 then RaiseException('Invalid manifest digest at record '+IntToStr(I+1)+'.');
    if not SafeRelative(Name) or (CompareText(Name, ManifestName) = 0) or
       (Names.IndexOf(LowerCase(Name)) >= 0) then RaiseException('Unsafe or duplicate manifest path at record '+IntToStr(I+1)+'.');
    Names.Add(LowerCase(Name)); Hashes.Add(Digest+'  '+Name);
  end;
  Result := (Names.IndexOf('rocket.exe') >= 0) and (Names.IndexOf('rocket-cli.exe') >= 0) and
    (Names.IndexOf('rocket-mcp.exe') >= 0) and (Names.IndexOf('rocket-lua-host.exe') >= 0) and
    (Names.IndexOf('rocket-update.json') >= 0);
  Names.Add(LowerCase(ManifestName));
end;

procedure CheckTree(const Root, Relative: String; Names: TStringList; var Count: Integer);
var Find: TFindRec; Dir, Child: String;
begin
  Dir := Root; if Relative <> '' then Dir := Dir+'\'+NativeRelative(Relative);
  PinDirectory(Dir);
  if FindFirst(Dir+'\*', Find) then try
    repeat
      if (Find.Name <> '.') and (Find.Name <> '..') then begin
        if (Find.Attributes and ReparsePoint) <> 0 then RaiseException('Installation contains a link. Files were preserved.');
        Child := Find.Name; if Relative <> '' then Child := Relative+'/'+Child;
        if (CompareText(Find.Name,'.git') = 0) or (CompareText(Find.Name,'.ssh') = 0) then
          RaiseException('Installation contains repository or key data. Move it out before retrying; nothing was removed.');
        if (Find.Attributes and DirectoryAttribute) <> 0 then CheckTree(Root,Child,Names,Count)
        else begin
          if Names.IndexOf(LowerCase(Child)) < 0 then RaiseException('Installation contains extra files. They were preserved; move personal files out before retrying.');
          Count := Count+1;
        end;
      end;
    until not FindNext(Find);
  finally FindClose(Find); end;
end;

procedure VerifyPayload(const Root: String; Names, Hashes: TStringList);
var I, Count: Integer; FileName: String;
begin
  CheckAncestors(Root); PinDirectory(ExtractFileDir(Root)); Names.Sorted := True;
  if not LoadManifest(Root,Names,Hashes) then RaiseException('Rocket package manifest is missing or invalid. Files were preserved.');
  Count := 0; CheckTree(Root,'',Names,Count);
  if Count <> Names.Count then RaiseException('Rocket package is incomplete. Files were preserved.');
  for I := 0 to Hashes.Count-1 do begin
    FileName := Root+'\'+NativeRelative(Copy(Hashes[I],67,Length(Hashes[I])));
    if LowerCase(GetSHA256OfFile(FileName)) <> Copy(Hashes[I],1,64) then
      RaiseException('Rocket installation contains changed files. They were preserved; use a fresh installation folder.');
  end;
end;

procedure VerifyAndRelease(const Root: String);
var Names, Hashes: TStringList;
begin
  Names := TStringList.Create; Hashes := TStringList.Create;
  try VerifyPayload(Root,Names,Hashes);
  finally ReleaseDirectories; Names.Free; Hashes.Free; end;
end;

function NonEmpty(const Dir: String): Boolean;
var Find: TFindRec;
begin
  Result := False;
  if FindFirst(Dir+'\*',Find) then try
    repeat if (Find.Name <> '.') and (Find.Name <> '..') then begin Result := True; Break; end;
    until not FindNext(Find);
  finally FindClose(Find); end;
end;

function MarkerPath: String;
begin Result := ExpandConstant('{app}\uninstall\rocket-install.ini'); end;

function GetStageDir(Param: String): String;
begin
  Result := ExpandConstant('{app}\.rocket-setup-')+IntToStr(GetCurrentProcessId);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var Target: String;
begin
  Result := '';
  try
    Target := ExpandConstant('{app}'); CheckAncestors(Target);
    CheckAncestors(Target+'\uninstall');
    if FileOrDirExists(Target+'\.app.rocket-update-lock') then RaiseException('An update is active or needs recovery. Finish it before running Setup.');
    if NonEmpty(Target) then begin
      if GetIniString('Rocket','Identity','',MarkerPath) <> LayoutIdentity then
        RaiseException('This folder is not a managed Rocket installation. Choose a new empty folder; existing files will not be overwritten.');
      VerifyAndRelease(Target+'\app');
      OldManifestHash := GetSHA256OfFile(Target+'\app\'+ManifestName);
    end;
    StageRoot := GetStageDir(''); PreviousRoot := Target+'\.rocket-setup-previous-'+IntToStr(GetCurrentProcessId);
    if not Prepared and (FileOrDirExists(StageRoot) or FileOrDirExists(PreviousRoot)) then
      RaiseException('A previous setup needs recovery. Its files were preserved; choose another folder or inspect the setup log.');
    Prepared := True;
  except Result := GetExceptionMessage; Log('Rocket setup refused: '+Result); end;
end;

procedure PublishPayload;
var Target: String; HadPrevious: Boolean;
begin
  Target := ExpandConstant('{app}\app');
  VerifyAndRelease(StageRoot);
  if CompareText(GetSHA256OfFile(StageRoot+'\'+ManifestName),'{#PayloadManifestHash}') <> 0 then
    RaiseException('Prepared installation changed. Nothing was replaced.');
  if FileOrDirExists(ExpandConstant('{app}\.app.rocket-update-lock')) then
    RaiseException('An update started during Setup. Finish it before retrying; staged files were retained.');
  HadPrevious := DirExists(Target);
  if HadPrevious then begin
    VerifyAndRelease(Target);
    if GetSHA256OfFile(Target+'\'+ManifestName) <> OldManifestHash then RaiseException('Installed files changed during Setup. Nothing was replaced.');
    if not RenameFile(Target,PreviousRoot) then RaiseException('Close Rocket and its helpers, then retry. The current installation was preserved.');
  end;
  if not RenameFile(StageRoot,Target) then begin
    if HadPrevious and not RenameFile(PreviousRoot,Target) then
      RaiseException('Setup could not restore the previous installation. Both folders are preserved beside the setup log; do not delete them.');
    RaiseException('Could not publish the new installation. The previous app was restored.');
  end;
  if GetIniString('Rocket','Identity','',MarkerPath) <> LayoutIdentity then
    RaiseException('Installation ownership record could not be written. Keep the previous app folder for recovery.');
  PayloadPublished := True;
  Log('Rocket verified payload installed. Settings, repositories and key stores were not accessed.');
  if HadPrevious then Log('Previous app retained at: '+PreviousRoot);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssPostInstall then Exit;
  try PublishPayload;
  except
    FailureText := GetExceptionMessage;
    Log('Rocket installation failed: '+FailureText);
    SuppressibleMsgBox(FailureText,mbError,MB_OK,IDOK);
  end;
end;

function PayloadIsReady: Boolean;
begin Result := PayloadPublished; end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and not PayloadPublished then begin
    WizardForm.FinishedHeadingLabel.Caption := 'Rocket installation did not complete';
    WizardForm.FinishedLabel.Caption := FailureText+#13#10+#13#10+
      'Review the Setup log before retrying. Retained staging and recovery folders have not been deleted.';
  end;
end;

function GetCustomSetupExitCode: Integer;
begin
  Result := 0;
  { Inno normally reports success even if a post-install event raised an error. }
  if not PayloadPublished then Result := 1;
end;

procedure DeinitializeSetup;
begin
  ReleaseDirectories;
  if Prepared and not PayloadPublished and DirExists(StageRoot) then
    Log('Setup did not publish this candidate; any remaining stage is retained at: '+StageRoot);
end;

{ The payload is uninsneveruninstall: Inno's initial file list must not delete a
  changed file after an in-app update. Verify the current complete payload first,
  then delete only those manifest-listed files. Never recursively delete the installation root. }
function InitializeUninstall: Boolean;
begin
  Result := False;
  try
    CheckAncestors(ExpandConstant('{app}'));
    CheckAncestors(ExpandConstant('{app}\uninstall'));
    if GetIniString('Rocket','Identity','',MarkerPath) <> LayoutIdentity then RaiseException('Installation ownership record is missing. Files were preserved.');
    if FileOrDirExists(ExpandConstant('{app}\.app.rocket-update-lock')) then RaiseException('An update is active or needs recovery. Finish it before uninstalling.');
    VerifyAndRelease(ExpandConstant('{app}\app'));
    Result := True;
  except
    FailureText := GetExceptionMessage; Log('Rocket uninstall refused: '+FailureText);
    SuppressibleMsgBox(FailureText,mbError,MB_OK,IDOK);
  end;
end;

procedure RemoveVerifiedEmptyDirectories(const Root: String);
var I: Integer; Dir: String;
begin
  { Children were pinned after their parents. Close only the child being removed;
    parents remain pinned so cleanup cannot be redirected through a renamed tree. }
  for I := GetArrayLength(DirectoryHandles)-1 downto 0 do begin
    Dir := DirectoryNames[I]; CloseHandle(DirectoryHandles[I]);
    SetArrayLength(DirectoryHandles,I); SetArrayLength(DirectoryNames,I);
    if (CompareText(Dir,Root) = 0) or (CompareText(Copy(Dir,1,Length(Root)+1),Root+'\') = 0) then
      RemoveDir(Dir); { Empty directories only; new or unknown data always stays. }
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var Names, Hashes: TStringList; Target, Quarantine, FileName: String; I: Integer;
begin
  if CurUninstallStep <> usUninstall then Exit;
  Target := ExpandConstant('{app}\app'); Names := TStringList.Create; Hashes := TStringList.Create;
  Quarantine := ExpandConstant('{app}\.rocket-uninstall-')+IntToStr(GetCurrentProcessId);
  try
    VerifyAndRelease(Target);
    if FileOrDirExists(Quarantine) then RaiseException('An old uninstall needs recovery. Files were preserved.');
    if not RenameFile(Target,Quarantine) then RaiseException('Close Rocket and its helpers, then uninstall again. No app files were removed.');
    try VerifyPayload(Quarantine,Names,Hashes);
    except
      ReleaseDirectories;
      if not RenameFile(Quarantine,Target) then Log('Restore the retained uninstall folder manually: '+Quarantine);
      RaiseLastException;
    end;
    for I := 0 to Hashes.Count-1 do begin
      FileName := Quarantine+'\'+NativeRelative(Copy(Hashes[I],67,Length(Hashes[I])));
      if LowerCase(GetSHA256OfFile(FileName)) <> Copy(Hashes[I],1,64) then RaiseException('A file changed during uninstall. Remaining files are retained in '+Quarantine+'. Inspect this folder before recovery.');
      if not DeleteFile(FileName) then RaiseException('Uninstall stopped while removing an app file. Remaining files are retained in '+Quarantine+'. Close Rocket and inspect the uninstall log before recovery.');
    end;
    if not DeleteFile(Quarantine+'\'+ManifestName) then RaiseException('Could not remove the package manifest.');
    RemoveVerifiedEmptyDirectories(Quarantine);
  finally ReleaseDirectories; Names.Free; Hashes.Free; end;
  Log('Rocket payload removed. Settings, repositories, SSH keys and recovery backups were preserved.');
end;
