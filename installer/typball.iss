; Typball の Windows インストーラー (Inno Setup 7)
;
; ビルド (release.yml / ci.yml から呼ぶ。CI では .github/actions/setup-innosetup が Inno Setup 7 を入れる):
;   "C:\Program Files\Inno Setup 7\ISCC.exe" /DAppVersion=0.1.0 installer\typball.iss
; 出力: dist\typball-<AppVersion>-windows-x86_64-setup.exe
;
; 方針:
; - 既定はユーザー単位 (%LOCALAPPDATA%\Programs\Typball)。管理者権限が要らず、アプリ内の自動更新も UAC 無しで動く。
;   「すべてのユーザー」に入れたいときは最初の画面で選べる (PrivilegesRequiredOverridesAllowed)
; - 自動更新はこの setup.exe を /VERYSILENT で実行し直す。前回のタスク (関連付けなど) はそのまま引き継がれる
; - 隣に unins000.exe があることを Typball が「インストーラー版」の印に使う (update.rs の install_kind)

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif
; 出力ファイル名 (拡張子なし)。nightly は ci.yml が typball-nightly-windows-x86_64-setup を渡す
#ifndef OutputName
  #define OutputName "typball-" + AppVersion + "-windows-x86_64-setup"
#endif
#define AppName "Typball"
#define AppExe "Typball.exe"
#define RepoUrl "https://github.com/Typball/Typball"

[Setup]
AppId={{1D55724C-9BCC-41A2-B1E3-55340BE618D0}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Typball contributors
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; 更新時に動いている Typball を閉じる (サイレント実行では自動で閉じる)
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes
LicenseFile=..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"

[CustomMessages]
en.AssocTyp=Open .typ (Typst) files with {#AppName}
ja.AssocTyp=.typ (Typst) ファイルを {#AppName} で開く
en.AssocMd=Open .md (Markdown) files with {#AppName}
ja.AssocMd=.md (Markdown) ファイルを {#AppName} で開く
en.AssocTbdg=Open .tbdg (diagram) files with {#AppName}
ja.AssocTbdg=.tbdg (図) ファイルを {#AppName} で開く
en.VerbOpen=Open with {#AppName}
ja.VerbOpen={#AppName} で開く
en.VerbEdit=Edit with {#AppName}
ja.VerbEdit={#AppName} で編集
en.VerbView=Preview with {#AppName}
ja.VerbView={#AppName} でプレビュー
en.WebView2Missing=The Markdown preview needs the Microsoft Edge WebView2 Runtime, which is not installed. Download and install it now?
ja.WebView2Missing=Markdown プレビューに必要な Microsoft Edge WebView2 ランタイムが入っていません。今すぐダウンロードしてインストールしますか？
en.WebView2Failed=The WebView2 Runtime could not be installed. You can install it later from https://developer.microsoft.com/microsoft-edge/webview2/
ja.WebView2Failed=WebView2 ランタイムをインストールできませんでした。後から https://developer.microsoft.com/microsoft-edge/webview2/ で導入できます。

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked
Name: "assoc_typ"; Description: "{cm:AssocTyp}"
Name: "assoc_md"; Description: "{cm:AssocMd}"; Flags: unchecked
Name: "assoc_tbdg"; Description: "{cm:AssocTbdg}"

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; 同梱フォント (Noto Sans JP・Typst のフォント) とライブラリのライセンス全文 (tools/gen_third_party_notices.py が作る)
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; ファイルの関連付け。HKA はユーザー単位なら HKCU、管理者なら HKLM の Software\Classes になる
Root: HKA; Subkey: "Software\Classes\.typ\OpenWithProgids"; ValueType: string; ValueName: "Typball.typ"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\.typ"; ValueType: string; ValueName: ""; ValueData: "Typball.typ"; Flags: uninsdeletevalue; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ"; ValueType: string; ValueName: ""; ValueData: "Typst Document"; Flags: uninsdeletekey; Tasks: assoc_typ
; 書類のアイコンは exe に埋め込んだもの (crates/typball/assets/icons/typball.rc)。負の数はリソース ID (101=.typ, 102=.md, 104=.tbdg)
Root: HKA; Subkey: "Software\Classes\Typball.typ\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},-101"; Tasks: assoc_typ
; ダブルクリック (open) は引数無しで渡し、設定 [filetypes.*] open_mode に従ってエディタかプレビューウィンドウで開く。
; 右クリックには「{#AppName} で編集」(--edit) と「{#AppName} でプレビュー」(--view) も出す
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell"; ValueType: string; ValueName: ""; ValueData: "open"; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\open"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbOpen}"; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\edit"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbEdit}"; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\edit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --edit ""%1"""; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\view"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbView}"; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\Typball.typ\shell\view\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --view ""%1"""; Tasks: assoc_typ
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "Typball.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\.md"; ValueType: string; ValueName: ""; ValueData: "Typball.md"; Flags: uninsdeletevalue; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md"; ValueType: string; ValueName: ""; ValueData: "Markdown Document"; Flags: uninsdeletekey; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},-102"; Tasks: assoc_md
; ダブルクリック (open) は引数無しで渡し、設定 [filetypes.*] open_mode に従ってエディタかプレビューウィンドウで開く。
; 右クリックには「{#AppName} で編集」(--edit) と「{#AppName} でプレビュー」(--view) も出す
Root: HKA; Subkey: "Software\Classes\Typball.md\shell"; ValueType: string; ValueName: ""; ValueData: "open"; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\open"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbOpen}"; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\edit"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbEdit}"; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\edit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --edit ""%1"""; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\view"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbView}"; Tasks: assoc_md
Root: HKA; Subkey: "Software\Classes\Typball.md\shell\view\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --view ""%1"""; Tasks: assoc_md
; 図 (.tbdg)。Typball の図エディタの形式なので、既定で関連付ける。右クリックの項目は .typ / .md と同じ
Root: HKA; Subkey: "Software\Classes\.tbdg\OpenWithProgids"; ValueType: string; ValueName: "Typball.tbdg"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\.tbdg"; ValueType: string; ValueName: ""; ValueData: "Typball.tbdg"; Flags: uninsdeletevalue; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg"; ValueType: string; ValueName: ""; ValueData: "Typball Diagram"; Flags: uninsdeletekey; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},-104"; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell"; ValueType: string; ValueName: ""; ValueData: "open"; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\open"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbOpen}"; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\edit"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbEdit}"; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\edit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --edit ""%1"""; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\view"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbView}"; Tasks: assoc_tbdg
Root: HKA; Subkey: "Software\Classes\Typball.tbdg\shell\view\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --view ""%1"""; Tasks: assoc_tbdg
; 「プログラムから開く」の一覧に出す (関連付けのタスクを選ばなくても)
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\edit"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbEdit}"
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\edit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --edit ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\view"; ValueType: string; ValueName: ""; ValueData: "{cm:VerbView}"
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\view\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" --view ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".typ"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".txt"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\SupportedTypes"; ValueType: string; ValueName: ".tbdg"; ValueData: ""

[Run]
; 対話インストールの最後に起動する。自動更新 (サイレント) では Typball 側のバッチが起動し直す
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; アプリの 設定 → 関連付け で、この Typball に後から登録した関連付け (.txt など) を解除する。
; 関連付ける前の既定のアプリに戻すところまでアプリ側がやる (crates/typball/src/assoc/windows.rs)。
; ファイルとレジストリを消す前に実行される
Filename: "{app}\{#AppExe}"; Parameters: "--unregister-associations"; RunOnceId: "TypballAssociations"; Flags: waituntilterminated

[UninstallDelete]
; WebView2 のプロファイル (キャッシュ)。設定 (%APPDATA%\Typball\Settings.toml) は残す
Type: filesandordirs; Name: "{localappdata}\{#AppName}\EBWebView"

[Code]
// Pascal Script は関数内の const を受け付けない ('BEGIN' expected) ので、定数はグローバルに置く
const
  WebView2ClientKey = 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WebView2ClientKey32 = 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

// WebView2 ランタイム (Evergreen) が入っているか。Typball の webview2_loader.rs と同じキーを見る
function IsWebView2Installed: Boolean;
var
  Version: String;
begin
  Result := (RegQueryStringValue(HKLM, WebView2ClientKey32, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'))
    or (RegQueryStringValue(HKLM, WebView2ClientKey, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'))
    or (RegQueryStringValue(HKCU, WebView2ClientKey, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

// 無ければ Microsoft の Evergreen Bootstrapper (約 2 MB) を落として実行する (対話インストールのときだけ)
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if (CurStep = ssPostInstall) and (not WizardSilent) and (not IsWebView2Installed) then
  begin
    if MsgBox(CustomMessage('WebView2Missing'), mbConfirmation, MB_YESNO) = IDYES then
    begin
      try
        DownloadTemporaryFile('https://go.microsoft.com/fwlink/p/?LinkId=2124703', 'MicrosoftEdgeWebview2Setup.exe', '', nil);
        if not Exec(ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'), '/install', '', SW_SHOW, ewWaitUntilTerminated, ResultCode) then
          MsgBox(CustomMessage('WebView2Failed'), mbError, MB_OK);
      except
        MsgBox(CustomMessage('WebView2Failed'), mbError, MB_OK);
      end;
    end;
  end;
end;
