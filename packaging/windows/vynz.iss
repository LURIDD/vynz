; Vynz Windows kurulumu (Inno Setup 6)
; CI derler: ISCC /DAppVersion=0.2.1 packaging\windows\vynz.iss
; Kaynak klasör: dist\vynz-windows-x86_64 (zip ile aynı içerik: vynz.exe + libmpv DLL + lisans)
;
; Kullanıcı başına kurulur (%LOCALAPPDATA%\Programs\Vynz): yönetici izni istemez ve
; uygulama içi güncelleyici exe/dll dosyalarını yerinde değiştirebilir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{72DFE41F-A2C1-4F1F-AA02-B563A21A9038}
AppName=Vynz
AppVersion={#AppVersion}
AppVerName=Vynz {#AppVersion}
AppPublisher=LURID
AppPublisherURL=https://github.com/LURIDD/vynz
AppSupportURL=https://github.com/LURIDD/vynz/issues
DefaultDirName={localappdata}\Programs\Vynz
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=vynz-windows-x86_64-setup
SetupIconFile=..\..\design\logo\vynz.ico
; Kod imzası (SignPath) ürün adı ve sürümünü dosya meta verisinden denetler
VersionInfoProductName=Vynz
VersionInfoDescription=Vynz Setup
VersionInfoVersion={#AppVersion}
VersionInfoProductVersion={#AppVersion}
VersionInfoProductTextVersion={#AppVersion}
VersionInfoCompany=LURID
UninstallDisplayIcon={app}\vynz.exe
UninstallDisplayName=Vynz
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
LicenseFile=..\..\LICENSE

[Languages]
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; İşaretli gelir; istemeyen kaldırır
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\dist\vynz-windows-x86_64\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Vynz"; Filename: "{app}\vynz.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Vynz"; Filename: "{app}\vynz.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\vynz.exe"; Description: "{cm:LaunchProgram,Vynz}"; Flags: nowait postinstall skipifsilent

; Kullanıcı verisi (%APPDATA%\vynz, önbellek) kaldırmada silinmez: yeniden kurulumda geçmiş ve listeler kalır.
