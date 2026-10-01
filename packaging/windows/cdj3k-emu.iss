; SPDX-License-Identifier: MIT OR Apache-2.0
; cdj3k-emu.iss - the Windows installer. Inno Setup 6.6 or newer.
;
; One installer per architecture, built by build.ps1 from a tree stage.ps1
; made. Defines passed to ISCC:
;
;   /DAppVersion=0.2.0       Cargo.toml's workspace version
;   /DArch=x64|arm64         x64 installs on x64 Windows only, arm64 on arm64
;   /DStageDir=DIR           the staged tree
;   /DSign                   sign the installer, uninstaller and our executables
;
; build.ps1 defines Sign and registers the SignTool "cdj3k" when
; CDJ3K_SIGN_CERT_SHA1 is set.
;
; Per-machine under {autopf}\cdj3k-emu, as administrator (driver store,
; firewall rules). {app}\installer\setup-helper.ps1 does the privileged steps.

#ifndef AppVersion
  #error AppVersion is not defined; build with build.ps1
#endif
#ifndef Arch
  #error Arch is not defined; build with build.ps1
#endif
#ifndef StageDir
  #error StageDir is not defined; build with build.ps1
#endif
#if Ver < 0x06060000
  #error Inno Setup 6.6 or newer is required
#endif

; AppName is what Windows shows; DirName names the install directory.
#define AppName "CDJ3K Emulator"
#define DirName "cdj3k-emu"
; app_meta::BUNDLE_ID: the per-user data directory under %LOCALAPPDATA%.
#define DataDirName "com.cdj3k.emu"
#define QemuExe "qemu-system-aarch64.exe"

#ifdef Sign
  #define SignFlag " sign"
#else
  #define SignFlag ""
#endif

[Setup]
AppId={{2D243441-2852-435C-B88F-239852BCCD81}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=CDJ3K Emulator authors
DefaultDirName={autopf}\{#DirName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
MinVersion=10.0.19041
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
#endif
OutputBaseFilename=cdj3k-emu-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile={#StageDir}\cdj3k-emu.ico
UninstallDisplayIcon={app}\cdj3k-emu.ico
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
; The app holds Global\cdj3k-emu while it runs; Setup and Uninstall wait for it.
AppMutex=Global\cdj3k-emu
SetupLogging=yes
#ifdef Sign
SignTool=cdj3k
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#StageDir}\bin\cdj3k-emu.exe"; DestDir: "{app}\bin"; Flags: ignoreversion{#SignFlag}
Source: "{#StageDir}\bin\{#QemuExe}"; DestDir: "{app}\bin"; Flags: ignoreversion{#SignFlag}
Source: "{#StageDir}\bin\qemu-img.exe"; DestDir: "{app}\bin"; Flags: ignoreversion{#SignFlag}
Source: "{#StageDir}\bin\*"; DestDir: "{app}\bin"; Excludes: "cdj3k-emu.exe,{#QemuExe},qemu-img.exe"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\share\*"; DestDir: "{app}\share"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#StageDir}\cdj3k-emu.ico"; DestDir: "{app}"; Flags: ignoreversion
; Last, so every file is in place when its AfterInstall runs the helper.
Source: "{#StageDir}\installer\setup-helper.ps1"; DestDir: "{app}\installer"; Flags: ignoreversion; AfterInstall: RunHelperSteps

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\bin\cdj3k-emu.exe"; WorkingDir: "{app}\bin"; IconFilename: "{app}\cdj3k-emu.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\bin\cdj3k-emu.exe"; WorkingDir: "{app}\bin"; IconFilename: "{app}\cdj3k-emu.ico"; Tasks: desktopicon

[Run]
; Inbound QEMU rules for ProLink discovery; deleted first so a reinstall does
; not duplicate them.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""CDJ3K Emulator QEMU UDP"" program=""{app}\bin\{#QemuExe}"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""CDJ3K Emulator QEMU TCP"" program=""{app}\bin\{#QemuExe}"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""CDJ3K Emulator QEMU UDP"" dir=in action=allow protocol=UDP program=""{app}\bin\{#QemuExe}"" profile=any enable=yes"; StatusMsg: "Adding firewall rules..."; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""CDJ3K Emulator QEMU TCP"" dir=in action=allow protocol=TCP program=""{app}\bin\{#QemuExe}"" profile=any enable=yes"; Flags: runhidden
Filename: "{app}\bin\cdj3k-emu.exe"; WorkingDir: "{app}\bin"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""CDJ3K Emulator QEMU UDP"" program=""{app}\bin\{#QemuExe}"""; Flags: runhidden; RunOnceId: "FirewallUdp"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""CDJ3K Emulator QEMU TCP"" program=""{app}\bin\{#QemuExe}"""; Flags: runhidden; RunOnceId: "FirewallTcp"

[UninstallDelete]
; tap-owned is written by setup-helper.ps1, not by the installer.
Type: filesandordirs; Name: "{app}\installer"

[Code]
var
  RestartPending: Boolean;
  // Uninstall: the user ticked "delete my data" in the confirmation dialog.
  DeleteData: Boolean;

// Run one setup-helper.ps1 action with the native PowerShell; ExitCode is its
// exit code. False when PowerShell itself could not start.
function RunHelper(const Action, Extra: String; var ExitCode: Integer): Boolean;
var
  Params: String;
begin
  Params := '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
    ExpandConstant('{app}\installer\setup-helper.ps1') + '" -Action ' + Action + Extra;
  Log('setup-helper: ' + Params);
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Params, '', SW_HIDE, ewWaitUntilTerminated, ExitCode);
  if Result then
    Log('setup-helper exit code ' + IntToStr(ExitCode));
end;

// 0 is done, 3010 is done with a restart pending.
function HelperSucceeded(const Action, Extra: String; const Failure: String): Boolean;
var
  Code: Integer;
begin
  Result := RunHelper(Action, Extra, Code) and ((Code = 0) or (Code = 3010));
  if Result then
  begin
    if Code = 3010 then
      RestartPending := True;
  end
  else
    SuppressibleMsgBox(Failure + ' The details are in the setup log.',
      mbError, MB_OK, IDOK);
end;

// AfterInstall of setup-helper.ps1. Setup calls NeedRestart before
// ssPostInstall, so RestartPending must be set here.
procedure RunHelperSteps();
begin
  WizardForm.StatusLabel.Caption := 'Installing the TAP-Windows6 driver...';
  // Bridged networking needs the driver; the emulator runs without it.
  HelperSucceeded('InstallTap',
    ' -DriverDir "' + ExpandConstant('{app}\share\cdj3k-emu\tap-windows6') + '"',
    'The TAP-Windows6 driver could not be installed. Bridged networking will be unavailable.');
#if Arch == "arm64"
  WizardForm.StatusLabel.Caption := 'Enabling Windows Hypervisor Platform...';
  // The arm64 emulator accelerates through WHPX.
  HelperSucceeded('EnableHypervisorPlatform', '',
    'Windows Hypervisor Platform could not be enabled. Enable it in "Turn Windows features on or off", then restart.');
#endif
end;

// Apps & features runs UninstallString. /SILENT skips Uninstall's own Yes/No
// box, which cannot hold a checkbox; /ASK puts up ConfirmUninstall in its place.
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RegWriteStringValue(HKA,
      ExpandConstant('Software\Microsoft\Windows\CurrentVersion\Uninstall\{#emit SetupSetting("AppId")}_is1'),
      'UninstallString', '"' + ExpandConstant('{uninstallexe}') + '" /SILENT /ASK');
end;

function NeedRestart(): Boolean;
begin
  Result := RestartPending;
end;

function DataDir(): String;
begin
  Result := ExpandConstant('{localappdata}\{#DataDirName}');
end;

function HasParam(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then
      Result := True;
end;

// The uninstall confirmation, with the choice to delete the user's data
// (app_meta::BUNDLE_ID under %LOCALAPPDATA%). False when the user cancels.
function ConfirmUninstall(): Boolean;
var
  Form: TSetupForm;
  Prompt, PathLabel: TNewStaticText;
  Check: TNewCheckBox;
  OkButton, CancelButton: TNewButton;
  HaveData: Boolean;
begin
  HaveData := DirExists(DataDir());
  Form := CreateCustomForm(ScaleX(440), ScaleY(170), False, False);
  try
    Form.Caption := '{#AppName} Uninstall';

    Prompt := TNewStaticText.Create(Form);
    Prompt.Parent := Form;
    Prompt.AutoSize := False;
    Prompt.WordWrap := True;
    Prompt.SetBounds(ScaleX(16), ScaleY(16), Form.ClientWidth - ScaleX(32), ScaleY(36));
    Prompt.Caption := FmtMessage(SetupMessage(msgConfirmUninstall), ['{#AppName}']);

    Check := TNewCheckBox.Create(Form);
    Check.Parent := Form;
    Check.SetBounds(ScaleX(16), ScaleY(60), Form.ClientWidth - ScaleX(32), ScaleY(20));
    Check.Caption := 'Also delete my data: installed firmware, every slot''s eMMC and settings';
    Check.Checked := False;
    Check.Enabled := HaveData;

    PathLabel := TNewStaticText.Create(Form);
    PathLabel.Parent := Form;
    PathLabel.AutoSize := False;
    PathLabel.SetBounds(ScaleX(34), ScaleY(82), Form.ClientWidth - ScaleX(50), ScaleY(32));
    PathLabel.WordWrap := True;
    if HaveData then
      PathLabel.Caption := DataDir()
    else
      PathLabel.Caption := 'No data in ' + DataDir();

    OkButton := TNewButton.Create(Form);
    OkButton.Parent := Form;
    OkButton.SetBounds(Form.ClientWidth - ScaleX(180), Form.ClientHeight - ScaleY(39), ScaleX(80), ScaleY(25));
    OkButton.Caption := 'Uninstall';
    OkButton.ModalResult := mrOk;

    CancelButton := TNewButton.Create(Form);
    CancelButton.Parent := Form;
    CancelButton.SetBounds(Form.ClientWidth - ScaleX(92), Form.ClientHeight - ScaleY(39), ScaleX(80), ScaleY(25));
    CancelButton.Caption := SetupMessage(msgButtonCancel);
    CancelButton.ModalResult := mrCancel;
    CancelButton.Cancel := True;
    CancelButton.Default := True;

    Form.ActiveControl := CancelButton;
    Result := Form.ShowModal() = mrOk;
    DeleteData := Result and Check.Checked;
  finally
    Form.Free();
  end;
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  DeleteData := False;
  if HasParam('/ASK') then
    Result := ConfirmUninstall();
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  // The helper lives in {app}, which is still there at this step.
  if CurUninstallStep = usUninstall then
    RunHelper('RemoveTap', '', Code);
  if (CurUninstallStep = usPostUninstall) and DeleteData then
    if not DelTree(DataDir(), True, True, True) then
      MsgBox('Some files in ' + DataDir() + ' could not be deleted.', mbError, MB_OK);
  // /SILENT also skips Uninstall's closing message.
  if (CurUninstallStep = usDone) and HasParam('/ASK') then
    MsgBox(FmtMessage(SetupMessage(msgUninstalledAll), ['{#AppName}']), mbInformation, MB_OK);
end;
