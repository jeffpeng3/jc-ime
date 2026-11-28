; jc 注音的 Inno Setup 安裝腳本。
;
; # 怎麼建
;
;     .\installer\build.ps1 -DataRoot <資料根目錄>
;
; 建置腳本會先 cargo build、暫存 payload、跑 check-payload.py、簽章，最後叫
; ISCC 編這個檔。版本號由腳本用 /DMyAppVersion= 與 /DIsoVersion= 傳進來；
; 直接開 Inno Setup IDE 按編譯會用到下面的預設值，不要那樣發版。
;
; # 為什麼是 Inno 而不是 MSI
;
; TSF DLL 被 explorer 等宿主以 image section 載入，MSI 的 InstallFiles 直接
; 覆寫會失敗，Restart Manager 又會嘗試關閉宿主。Inno 可以關掉 Restart Manager，
; 並在檔案複製前用 PrepareToInstall 把舊檔讓開。
;
; # 授權
;
; PrepareToInstall／SweepLeftovers／CurUninstallStepChanged 的「先刪、刪不掉
; 改名 .old-<時間戳>、再不行排開機刪除」做法參考 Zosuya/Tsunagi-IME
; （GPL-3.0-or-later）。這裡是照同樣策略重寫的 jc 版本，沒有逐行複製；
; jc 是 AGPL-3.0-only，GPLv3 與 AGPLv3 相容。
;
; # 升級時不能改的東西
;
; AppId 一旦發布就不能改。Windows 靠它認出「同一個程式的新版本」；改了會變成
; 兩個項目並存，舊版還會留在「應用程式與功能」裡。

#define MyAppName "jc 注音"
#define MyAppPublisher "jc-ime"
#define MyAppURL "https://github.com/jeffpeng3/jc-ime"

; 顯示版本與檔名用完整字串（可含 -rc.1）；VersionInfoVersion 只吃四段數字。
#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef IsoVersion
  #define IsoVersion "0.1.0.0"
#endif
#ifndef PayloadDir
  #define PayloadDir "..\target\installer\payload"
#endif
#define Root ".."

[Setup]
; 固定 AppId：升級判斷全靠它。這個值與舊 MSI 的 UpgradeCode 無關；第一次換成
; Inno 時，PrepareToInstall 會先移除舊 MSI，再照這裡的 AppId 建立新項目。
AppId={{EF7AD1D4-1BA4-41AE-AD3A-768BEA4D64B4}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
VersionInfoVersion={#IsoVersion}

; 與現行 C:\Program Files\jc-ime 一致。
DefaultDirName={autopf}\jc-ime
; 使用者按「瀏覽」選到既有目錄時，不要再接一層 AppName。
AppendDefaultDirName=no
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile={#Root}\LICENSE
OutputDir={#Root}\target\installer
OutputBaseFilename=jc-ime-{#MyAppVersion}-x64-setup
UninstallDisplayIcon={app}\jc-settings-ui.exe
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
SetupLogging=yes

; TSF 註冊寫 HKLM、安裝到 Program Files、排程開機刪除，三件都需要管理員。
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; 關掉 Restart Manager。Inno 預設會偵測占用檔案的程式並嘗試關閉；對輸入法
; 而言，占用 jc-tsf.dll 的通常是 explorer，關掉等於桌面消失，而且它會立刻
; 重啟又載回 DLL。檔案讓路改由 [Code] 的 PrepareToInstall 處理。
CloseApplications=no
RestartApplications=no

[Languages]
Name: "zh"; MessagesFile: "languages\ChineseTraditional.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
zh.EnableIme=安裝後把 jc 注音加入輸入法清單
en.EnableIme=Add jc Bopomofo to the input method list
zh.RunSmoke=安裝後執行診斷（會花幾秒；結果寫到 %LOCALAPPDATA%\jc-ime\doctor）
en.RunSmoke=Run diagnostics after installation (takes a few seconds; results go to %LOCALAPPDATA%\jc-ime\doctor)
zh.MsgRegistering=正在向系統註冊輸入法…
en.MsgRegistering=Registering the input method...
zh.MsgEnabling=正在把 jc 注音加入輸入法清單…
en.MsgEnabling=Adding jc Bopomofo to the input method list...
zh.MsgCleanup=正在清理舊版語言清單…
en.MsgCleanup=Cleaning up legacy language list entries...
zh.OpenSettings=開啟 jc 注音設定
en.OpenSettings=Open jc Bopomofo settings
zh.ErrLocked=無法更新 jc 注音：舊的程式檔正在使用中，而且無法讓路。請重新開機後再安裝一次。
en.ErrLocked=Cannot update jc Bopomofo: an old program file is in use and could not be moved aside. Please restart Windows and run the installer again.
zh.ErrEngineRunning=無法停止舊的 jc 引擎，檔案可能還是舊版。請關閉所有使用 jc 注音的程式，或重新開機後再安裝一次。
en.ErrEngineRunning=Cannot stop the old jc engine, so the new files may not take effect. Close all programs using jc Bopomofo, or restart Windows and run the installer again.

[Tasks]
Name: "enableime"; Description: "{cm:EnableIme}"
; 安裝後診斷預設不跑：它只是疑難排解紀錄，不影響能否打字。使用者要的話
; 自己勾；靜默安裝（/VERYSILENT）也維持不跑。
Name: "smoke"; Description: "{cm:RunSmoke}"; Flags: unchecked

[Files]
; 升級／反安裝前用來優雅停止舊引擎。這份不進 {app}，只由 [Code] 的
; ExtractTemporaryFile 解到 {tmp} 執行；`--stop` 會先要求舊監督者收尾，
; 真的不回應才送 Shutdown 存檔後強制結束。檔名加 -stop 是為了讓 log 與
; 工作管理員看得出這是安裝器帶起來的輔助行程。
;
; 放在 [Files] 最前面：SolidCompression=yes 時，ExtractTemporaryFile 要先把
; 前面的檔案解到記憶體；它又是 PrepareToInstall 最早用到的檔案。
Source: "{#PayloadDir}\jc-engine.exe"; DestDir: "{tmp}"; DestName: "jc-engine-stop.exe"; Flags: dontcopy

; 程式。32 位元 DLL 與 x64 DLL 放在同一層，檔名分架構；CLSID 的機碼檢視由
; 下面 [Registry] 的 HKLM64／HKLM32 決定。ignoreversion：版本升級由 AppId／
; VersionInfoVersion 與 PrepareToInstall 讓路處理，不靠檔案版本比對。
Source: "{#PayloadDir}\jc-tsf.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-tsf-x86.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-engine.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-engine-launcher.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-settings-ui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-tsf-smoke.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\jc-tip-register.exe"; DestDir: "{app}"; Flags: ignoreversion
; 安裝後診斷；只有勾選 smoke 工作時，下面的 [Run] 才以原使用者身分執行，
; log 寫到該使用者的 %LOCALAPPDATA%\jc-ime\doctor。
Source: "{#PayloadDir}\post-install-doctor.ps1"; DestDir: "{app}"; Flags: ignoreversion

; 授權文件：payload 內的路徑與 repo 相同，check-payload.py 直接比對雜湊。
Source: "{#PayloadDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\THIRD-PARTY-NOTICES.md"; DestDir: "{app}\legal"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\RUST-THIRD-PARTY-NOTICES.md"; DestDir: "{app}\legal"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\README.md"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\Apache-2.0.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\CC-BY-4.0.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\CC-BY-SA-3.0.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\CC-BY-SA-4.0.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\ENGLISH-DATA-NOTICE.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\ESDB-SCOWL-LICENSE.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\IPADIC-COPYING.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\MIT.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\MIT-Funmula.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\Mozc-LICENSE.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\Mozc-README.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\UNIDIC-AUTHORS.md"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\UNIDIC-BSD-2025.12-artifact.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\UNIDIC-BSD-consortium.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\UNIDIC-COPYING.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion
Source: "{#PayloadDir}\legal\licenses\ZH-DATA-NOTICE.txt"; DestDir: "{app}\legal\licenses"; Flags: ignoreversion

; 資料：清單來自資料根目錄 manifest.json 的 shipped: true。安裝到共用位置，
; 引擎以 mmap 讀取；setup 不碰使用者自己的 %LOCALAPPDATA%\jc-ime。
Source: "{#PayloadDir}\data\manifest.json"; DestDir: "{commonappdata}\jc-ime\data"; Flags: ignoreversion
Source: "{#PayloadDir}\data\packs\ja-lexicon-mozc-v2.pack"; DestDir: "{commonappdata}\jc-ime\data\packs"; Flags: ignoreversion
Source: "{#PayloadDir}\data\build-zh\lexicon.bin"; DestDir: "{commonappdata}\jc-ime\data\build-zh"; Flags: ignoreversion
Source: "{#PayloadDir}\data\build-zh\manifest.json"; DestDir: "{commonappdata}\jc-ime\data\build-zh"; Flags: ignoreversion
Source: "{#PayloadDir}\data\build-zh-bigram\lm.bin"; DestDir: "{commonappdata}\jc-ime\data\build-zh-bigram"; Flags: ignoreversion
Source: "{#PayloadDir}\data\english\words.bin"; DestDir: "{commonappdata}\jc-ime\data\english"; Flags: ignoreversion
Source: "{#PayloadDir}\data\english\manifest.json"; DestDir: "{commonappdata}\jc-ime\data\english"; Flags: ignoreversion
Source: "{#PayloadDir}\data\english\NOTICE.txt"; DestDir: "{commonappdata}\jc-ime\data\english"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\manifest.json"; DestDir: "{commonappdata}\jc-ime\data\models"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\zh-select\model.q8_0.gguf"; DestDir: "{commonappdata}\jc-ime\data\models\zh-select"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\zh-select\vocab.json"; DestDir: "{commonappdata}\jc-ime\data\models\zh-select"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\zh-select\manifest.json"; DestDir: "{commonappdata}\jc-ime\data\models\zh-select"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\external\jinen-v2-small\jinen-v2-small-Q4_K_M.gguf"; DestDir: "{commonappdata}\jc-ime\data\models\external\jinen-v2-small"; Flags: ignoreversion
Source: "{#PayloadDir}\data\models\quant\jinen-v2-xsmall\tokenizer.json"; DestDir: "{commonappdata}\jc-ime\data\models\quant\jinen-v2-xsmall"; Flags: ignoreversion

[Icons]
Name: "{group}\jc 注音設定"; Filename: "{app}\jc-settings-ui.exe"; WorkingDir: "{app}"
Name: "{group}\jc 注音授權"; Filename: "{app}\jc-tip-register.exe"; Parameters: "legal"; WorkingDir: "{app}"

[Registry]
; CLSID 的兩個機碼檢視。TSF 依宿主位元數挑選：64 位元行程讀 64 位元檢視，
; 32 位元行程讀 32 位元檢視（實體位置是 Wow6432Node）。兩份共用同一個 CLSID，
; 使用者的語言清單只有一項。
;
; 檢視用 Root 的 HKLM64／HKLM32 指定，不要用 Flags。Inno 的 [Registry] 從
; 來沒有 32bit／64bit 旗標，任何版本都沒有；那是 [Files]／[Run] 用來指定
; 「這個檔案／行程是 32 位元」的旗標，名字一樣但意義不同。
; HKLM\Software\Classes 就是 HKCR 的機器層級來源，效果與寫 HKCR 相同。
; PipePrefix 是 jc 自己的值，啟動器靠它知道要連 jc-ime-<rid> 的 pipe。
Root: HKLM64; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32"; ValueType: string; ValueName: ""; ValueData: "{app}\jc-tsf.dll"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32"; ValueType: string; ValueName: "ThreadingModel"; ValueData: "Apartment"
Root: HKLM64; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}"; ValueType: string; ValueName: "PipePrefix"; ValueData: "jc-ime"; Flags: uninsdeletekey

Root: HKLM32; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32"; ValueType: string; ValueName: ""; ValueData: "{app}\jc-tsf-x86.dll"; Flags: uninsdeletekey
Root: HKLM32; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32"; ValueType: string; ValueName: "ThreadingModel"; ValueData: "Apartment"
Root: HKLM32; Subkey: "Software\Classes\CLSID\{{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}"; ValueType: string; ValueName: "PipePrefix"; ValueData: "jc-ime"; Flags: uninsdeletekey

; 引擎登入自動啟動。REG_EXPAND_SZ 讓 %LOCALAPPDATA% 在每位使用者登入時各自展開。
; 必須用 --supervise；直接伺服模式沒給 socket 會退回 stdio，TIP 連不上。
; 呼叫 windows subsystem 的 jc-engine-launcher.exe，由它用 CREATE_NO_WINDOW
; 啟動 jc-engine.exe。jc-engine 本身是 console subsystem，Run 直接指它會在
; 登入時開一個終端機視窗，關掉視窗等於殺掉引擎。
; 升級時先刪舊值再寫新值，避免新舊兩條登入命令同時存在、各帶起一份引擎。
; pipe 名由 jc_windows::resolve_pipe_name() 決定，TIP 與引擎都呼叫同一函式，
; 所以 autostart 絕對不能帶 --socket（帶了反而和 TIP 腳踏）。
; --personal-dir 必須指向使用者目錄：資料根目錄是 ProgramData，寫入會失敗。
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "jc-ime-engine"
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: expandsz; ValueName: "jc-ime-engine"; ValueData: """{app}\jc-engine-launcher.exe"" --supervise --data ""{commonappdata}\jc-ime\data"" --settings ""%LOCALAPPDATA%\jc-ime\settings.json"" --personal-dir ""%LOCALAPPDATA%\jc-ime"" --personal-prefix personal"; Flags: uninsdeletevalue

[Run]
; 機器層級註冊（HKLM\SOFTWARE\Microsoft\CTF）。安裝程式已提權，直接跑。
Filename: "{app}\jc-tip-register.exe"; Parameters: "machine-register"; StatusMsg: "{cm:MsgRegistering}"; Flags: runhidden waituntilterminated

; 使用者層級啟用。runasoriginaluser 不能省：安裝程式是管理員身分，直接跑會
; 寫到 Administrator 的語言清單，對實際使用者沒有作用。
;
; [Run] 沒有 ignoreerrors 這個旗標（任何版本都沒有），所以這兩步失敗會讓安裝
; 失敗。這是刻意的：使用者清單沒加進去，裝完也不能打字，靜靜成功只會讓問題
; 延後到使用者身上。真的需要「失敗也繼續」，得改用 [Code] 的 Exec／
; ExecAsOriginalUser 自己處理結束碼。
Filename: "{app}\jc-tip-register.exe"; Parameters: "user-install"; Tasks: enableime; StatusMsg: "{cm:MsgEnabling}"; Flags: runhidden waituntilterminated runasoriginaluser

; 清掉已移除舊 CLSID 的語言清單殘留。
Filename: "{app}\jc-tip-register.exe"; Parameters: "user-cleanup-legacy"; StatusMsg: "{cm:MsgCleanup}"; Flags: runhidden waituntilterminated runasoriginaluser

; 裝完就喚起引擎，不等下次登入。
; 走啟動器，和登入自動啟動同一條路徑；runhidden 留著，就算啟動器未來改成
; 有 console 的版本也不會閃視窗。
; 啟動器只負責 CreateProcess，會立刻結束；等待它才能確保下面的驗證不會
; 早於「新引擎被帶起來」。
Filename: "{app}\jc-engine-launcher.exe"; Parameters: "--supervise --data ""{commonappdata}\jc-ime\data"" --settings ""{%LOCALAPPDATA}\jc-ime\settings.json"" --personal-dir ""{%LOCALAPPDATA}\jc-ime"" --personal-prefix personal"; Tasks: enableime; Flags: runhidden waituntilterminated runasoriginaluser

; 驗證執行中的引擎就是剛安裝的這顆 binary（版本＋SHA-256），不只確認
; 「單例鎖有人在」。
; 舊版引擎不理會停止事件，新引擎接手時會先送 Shutdown 存檔再強制結束；
; 這一步把接手結果變成安裝時就看得到的失敗，不會拖到下次登入。
Filename: "{app}\jc-engine.exe"; Parameters: "--verify-running"; Tasks: enableime; Flags: runhidden waituntilterminated runasoriginaluser

; 安裝完成後跑一次 doctor；預設不跑，要使用者在 [Tasks] 勾選 smoke 才會執行。
; runasoriginaluser 不能省：診斷要讀原使用者的
; 語言清單與 TSF 啟用狀態，log 與剪貼簿也必須落在原使用者的
; %LOCALAPPDATA%。
;
; -NoUi 不能省：GitHub runner 是互動工作階段，doctor 只要有 FAIL 就會跳
; MessageBox；[Run] 的 waituntilterminated 會永遠等不到關閉，安裝就卡住
; （run 45624582847）。安裝期只負責留下紀錄，結果由 install-smoke 的
; 「安裝後 doctor」以 -NoUi -ExitWithSmokeCode 斷言。
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\post-install-doctor.ps1"" -NoUi"; Tasks: smoke; Flags: runhidden waituntilterminated runasoriginaluser

Filename: "{app}\jc-settings-ui.exe"; Description: "{cm:OpenSettings}"; Flags: postinstall nowait skipifsilent unchecked

[UninstallRun]
; 先停引擎與啟動器。`--stop` 走和升級同一條路徑：先要求監督者優雅收尾，
; 不回應就送 Shutdown 存檔後強制結束；`[Code]` 的 usUninstall 還會再確認一次，
; 避免反安裝刪檔時還握著 jc-engine.exe。
Filename: "{app}\jc-engine.exe"; Parameters: "--stop"; Flags: runhidden waituntilterminated; RunOnceId: "StopEngine"

; 再做 machine-unregister。user-uninstall 需要原使用者身分，但 [UninstallRun]
; 不支援 runasoriginaluser；profile 移除後語言清單的項目自然失效。
Filename: "{app}\jc-tip-register.exe"; Parameters: "machine-unregister"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterTip"

[UninstallDelete]
Type: files; Name: "{app}\*.old-*"
Type: files; Name: "{commonappdata}\jc-ime\msi-uninstall.log"
; 安裝後診斷的 log 與 last.txt；反安裝不需要這支腳本留下的紀錄。
Type: filesandordirs; Name: "{localappdata}\jc-ime\doctor"

[Code]
const
  MOVEFILE_DELAY_UNTIL_REBOOT = $4;
  LEGACY_DISPLAY_PREFIX = 'jc 注音';

var
  GScheduledDelete: Boolean;

// 第二個參數宣告成 Cardinal 才傳得了 NULL；傳 0 才是「刪除」，空字串不是 NULL。
function MoveFileExDelete(lpExistingFileName: String; lpNewFileName: Cardinal;
  dwFlags: Cardinal): Boolean;
  external 'MoveFileExW@kernel32.dll stdcall';

// 能刪就刪；刪不掉（DLL 被載入時是 image section）就登記下次開機刪。
procedure DeleteOrScheduleDelete(const Path: String);
begin
  if not FileExists(Path) then
    Exit;
  if DeleteFile(Path) then
    Log('已刪除：' + Path)
  else if MoveFileExDelete(Path, 0, MOVEFILE_DELAY_UNTIL_REBOOT) then begin
    GScheduledDelete := True;
    Log('刪不掉，已登記下次開機刪除：' + Path);
  end
  else
    Log('刪不掉，登記開機刪除也失敗：' + Path);
end;

// 清掉先前讓路留下的 .old-*。時間戳命名讓每一輪都能改名，不會被上一輪的
// 殘骸卡住；殘骸仍被占用時就排開機刪除。
procedure SweepOneLeftover(const Dir, BaseName: String);
var
  R: TFindRec;
begin
  if FindFirst(Dir + '\' + BaseName + '.old-*', R) then begin
    try
      repeat
        DeleteOrScheduleDelete(Dir + '\' + R.Name);
      until not FindNext(R);
    finally
      FindClose(R);
    end;
  end;
end;

procedure SweepLeftovers(const Dir: String);
begin
  SweepOneLeftover(Dir, 'jc-tsf.dll');
  SweepOneLeftover(Dir, 'jc-tsf-x86.dll');
  SweepOneLeftover(Dir, 'jc-engine.exe');
  SweepOneLeftover(Dir, 'jc-engine-launcher.exe');
end;

// 先刪、刪不掉改名 .old-<yyyymmddhhnnss>、再失敗才回錯誤訊息。
function ParkFile(const Path: String): String;
var
  Stamp: String;
begin
  Result := '';
  if not FileExists(Path) then
    Exit;
  if DeleteFile(Path) then begin
    Log('舊檔沒被占用，直接刪除：' + Path);
    Exit;
  end;
  Stamp := Path + '.old-' + GetDateTimeString('yyyymmddhhnnss', #0, #0);
  if RenameFile(Path, Stamp) then
    Log('舊檔被占用，已改名讓路：' + Stamp)
  else
    Result := ExpandConstant('{cm:ErrLocked}') + #13#10 + Path;
end;

function ParkProtectedFiles(const Dir: String): String;
begin
  Result := ParkFile(Dir + '\jc-tsf.dll');
  if Result <> '' then
    Exit;
  Result := ParkFile(Dir + '\jc-tsf-x86.dll');
  if Result <> '' then
    Exit;
  Result := ParkFile(Dir + '\jc-engine.exe');
  if Result <> '' then
    Exit;
  Result := ParkFile(Dir + '\jc-engine-launcher.exe');
end;

// 從 MsiExec.exe /X{GUID} 或 /I{GUID} 取出 ProductCode（含大括號）。
function ExtractProductCode(const Command: String): String;
var
  StartPos, EndPos: Integer;
begin
  Result := '';
  StartPos := Pos('{', Command);
  if StartPos = 0 then
    Exit;
  EndPos := Pos('}', Copy(Command, StartPos, Length(Command)));
  if EndPos = 0 then
    Exit;
  Result := Copy(Command, StartPos, EndPos);
end;

// 停掉引擎與載入 TSF DLL 的行程。舊 MSI 反安裝會直接刪檔案，explorer 還載著
// jc-tsf.dll 時會失敗（1603）；讓路策略對 msiexec 不適用，只能先讓行程結束。
procedure StopTsfHosts;
var
  Script, ScriptFile: String;
  Code: Integer;
begin
  Script :=
    '$names = @("jc-tsf.dll", "jc-tsf-x86.dll")' + #13#10 +
    'Get-Process -ErrorAction SilentlyContinue | ForEach-Object {' + #13#10 +
    '  $p = $_' + #13#10 +
    '  try {' + #13#10 +
    '    $hit = $p.Modules | Where-Object { $names -contains $_.ModuleName } | Select-Object -First 1' + #13#10 +
    '    if ($hit) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }' + #13#10 +
    '  } catch { }' + #13#10 +
    '}' + #13#10 +
    'Get-Process jc-engine -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue';
  ScriptFile := ExpandConstant('{tmp}\jc-stop-tsf.ps1');
  if not SaveStringToFile(ScriptFile, Script, False) then begin
    Log('無法寫入停止行程的腳本，跳過：' + ScriptFile);
    Exit;
  end;
  if Exec('powershell.exe',
      '-NoProfile -ExecutionPolicy Bypass -File "' + ScriptFile + '"',
      '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Log('已停止載入 jc-tsf 的行程（powershell exit ' + IntToStr(Code) + '）')
  else
    Log('無法執行 powershell 停止行程；舊 MSI 反安裝可能因檔案占用失敗');
end;

// 掃兩個 Uninstall 檢視，找出舊 MSI（WindowsInstaller=1 且 DisplayName 以
// 「jc 注音」開頭），逐一 msiexec /x。任何一步失敗都只寫 log，不中斷安裝。
procedure RemoveLegacyMsi;
var
  Root: Integer;
  Names: TArrayOfString;
  Products: TStringList;
  I, J, Code: Integer;
  SubKey, DisplayName, UninstallString, ProductCode, MsiLog: String;
  IsMsi: Cardinal;
begin
  Products := TStringList.Create;
  try
    for I := 0 to 1 do begin
      if I = 0 then
        Root := HKLM64
      else
        Root := HKLM32;
      if RegGetSubkeyNames(Root,
          'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', Names) then begin
        for J := 0 to GetArrayLength(Names) - 1 do begin
          SubKey := 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\' + Names[J];
          if RegQueryDWordValue(Root, SubKey, 'WindowsInstaller', IsMsi) then
            if IsMsi = 1 then
              if RegQueryStringValue(Root, SubKey, 'DisplayName', DisplayName) then
                if Copy(DisplayName, 1, Length(LEGACY_DISPLAY_PREFIX)) = LEGACY_DISPLAY_PREFIX then
                  if RegQueryStringValue(Root, SubKey, 'UninstallString', UninstallString) then begin
                    ProductCode := ExtractProductCode(UninstallString);
                    if (ProductCode <> '') and (Products.IndexOf(ProductCode) < 0) then begin
                      Products.Add(ProductCode);
                      Log('找到舊 MSI：' + DisplayName + '（' + ProductCode + '）');
                    end;
                  end;
        end;
      end;
    end;

    if Products.Count = 0 then
      Exit;

    StopTsfHosts;
    ForceDirectories(ExpandConstant('{commonappdata}\jc-ime'));
    MsiLog := ExpandConstant('{commonappdata}\jc-ime\msi-uninstall.log');
    for I := 0 to Products.Count - 1 do begin
      Code := 0;
      if Exec('msiexec.exe',
          '/x ' + Products[I] + ' /qn /norestart /l*v "' + MsiLog + '"',
          '', SW_HIDE, ewWaitUntilTerminated, Code) then
        Log('舊 MSI 反安裝結束：' + Products[I] + '（exit ' + IntToStr(Code) + '）')
      else
        Log('無法執行 msiexec 反安裝舊 MSI：' + Products[I]);
      // 1605 是「這個產品沒有安裝」；半殘登錄很常見，不當成錯誤。
      if Code = 1605 then
        Log('舊 MSI 登錄殘留，產品已不存在：' + Products[I]);
    end;
  finally
    Products.Free;
  end;
end;

// 用 tasklist 的輸出確認引擎行程。PowerShell 不可用時的備援。
//
// tasklist 沒有 session 過濾，這個備援是 machine-wide：其他登入使用者的
// 引擎會讓結果變成「可能還在跑」。這是刻意的 fail-closed，呼叫端會停下
// 安裝而不是冒險覆蓋檔案。
//
// 回傳 False 只代表 tasklist 明確列出「沒有這兩個行程」；工具本身失敗時
// 呼叫端仍要當成「可能還在跑」。
function TasklistShowsEngine(var Known: Boolean): Boolean;
var
  Code, I: Integer;
  TempFile: String;
  Lines: TArrayOfString;
  Lower: String;
begin
  Result := False;
  Known := False;
  TempFile := ExpandConstant('{tmp}\jc-engine-processes.txt');
  DeleteFile(TempFile);
  if not Exec(ExpandConstant('{cmd}'),
      '/C tasklist /NH /FO CSV > "' + TempFile + '" 2>NUL',
      '', SW_HIDE, ewWaitUntilTerminated, Code) then begin
    Log('tasklist 執行失敗，無法確認引擎行程');
    Exit;
  end;
  if Code <> 0 then begin
    Log(Format('tasklist 結束碼 %d，無法確認引擎行程', [Code]));
    Exit;
  end;
  if not FileExists(TempFile) then begin
    Log('tasklist 沒有產生輸出，無法確認引擎行程');
    Exit;
  end;
  if not LoadStringsFromFile(TempFile, Lines) then begin
    Log('讀取 tasklist 輸出失敗，無法確認引擎行程');
    Exit;
  end;
  Known := True;
  for I := 0 to GetArrayLength(Lines) - 1 do begin
    Lower := Lowercase(Lines[I]);
    if (Pos('jc-engine.exe', Lower) > 0) or
        (Pos('jc-engine-launcher.exe', Lower) > 0) then begin
      Result := True;
      Exit;
    end;
  end;
end;

// 引擎／啟動器是否還在執行。
//
// 確認工具本身壞掉時要保守回 True（可能還在跑），不能把「不知道」當成
// 「沒有在跑」而繼續覆蓋檔案。呼叫端會因此停下安裝，讓使用者看到訊息。
function EngineProcessesRunning: Boolean;
var
  Code: Integer;
  Known: Boolean;
begin
  Result := True;
  // 只檢查目前 session：提權安裝器不該因為其他登入使用者有引擎而停下。
  if Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command ' +
      '"$s=[System.Diagnostics.Process]::GetCurrentProcess().SessionId; ' +
      'if (Get-Process jc-engine,jc-engine-launcher -ErrorAction SilentlyContinue | ' +
      'Where-Object { $_.SessionId -eq $s }) { exit 1 } else { exit 0 }"',
      '', SW_HIDE, ewWaitUntilTerminated, Code) then begin
    Result := Code <> 0;
    Exit;
  end;
  Log('PowerShell 無法執行；改用 tasklist 確認引擎行程（machine-wide、fail-closed）');
  Result := TasklistShowsEngine(Known);
  if not Known then begin
    Result := True;
    Log('無法確認 jc-engine／jc-engine-launcher 是否結束，保守視為仍在執行');
  end;
end;

// 用指定版本的 jc-engine 執行 --stop。安裝時傳 {tmp} 的新版（舊版引擎不認得
// 停止事件，新版會送 Shutdown 存檔後強制結束）；反安裝時傳 {app} 的版本。
//
// AllowOriginalUser 只在安裝階段為 True：ExecAsOriginalUser 在反安裝模式不可用，
// 反安裝直接走提權身分。
function RunEngineStop(const Helper: String; const AllowOriginalUser: Boolean): Boolean;
var
  Code: Integer;
  OriginalOk: Boolean;
begin
  Result := False;
  if not FileExists(Helper) then begin
    Log('找不到停止引擎的輔助程式：' + Helper);
    Exit;
  end;
  Code := 0;
  // Inno 的 Pascal 不保證 and 短路，先明確判斷再呼叫 ExecAsOriginalUser。
  // 文件註明極端情況下它會丟例外，不能讓例外中斷 PrepareToInstall。
  if AllowOriginalUser then begin
    try
      OriginalOk := ExecAsOriginalUser(Helper, '--stop', '', SW_HIDE,
        ewWaitUntilTerminated, Code);
    except
      OriginalOk := False;
      Log('ExecAsOriginalUser 失敗；改用提權身分執行 --stop');
    end;
  end else
    OriginalOk := False;
  if OriginalOk then
    Log(Format('jc-engine --stop（原使用者）結束：exit=%d', [Code]))
  else if Exec(Helper, '--stop', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Log(Format('jc-engine --stop（提權）結束：exit=%d', [Code]))
  else begin
    Log('無法執行 jc-engine --stop');
    Exit;
  end;
  Result := Code = 0;
end;

// 最後手段：--stop 無效時才直接結束行程。
//
// 先用 PowerShell 只停目前 session 的引擎與 launcher；PowerShell 不可用時
// 才退回 machine-wide taskkill，並在 log 寫清楚影響範圍。先關 launcher，
// 避免它又帶起引擎。
procedure KillEngineProcesses;
var
  Code: Integer;
begin
  Code := 1;
  if Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command ' +
      '"$s=[System.Diagnostics.Process]::GetCurrentProcess().SessionId; ' +
      'Get-Process jc-engine,jc-engine-launcher -ErrorAction SilentlyContinue | ' +
      'Where-Object { $_.SessionId -eq $s } | Stop-Process -Force -ErrorAction SilentlyContinue"',
      '', SW_HIDE, ewWaitUntilTerminated, Code) then begin
    if Code = 0 then begin
      Log('已用 PowerShell 結束目前 session 的引擎與 launcher');
      Sleep(500);
      Exit;
    end;
  end;
  Log('無法只停目前 session；改用 machine-wide taskkill（可能影響其他登入使用者）');
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM jc-engine-launcher.exe', '',
      SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM jc-engine.exe', '',
      SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(500);
end;

// 先 graceful（--stop），再確認行程真的結束；不得已才 taskkill。
procedure StopEngine(const Helper: String; const AllowOriginalUser: Boolean);
begin
  RunEngineStop(Helper, AllowOriginalUser);
  if EngineProcessesRunning then begin
    Log('--stop 之後仍有引擎行程；改用 taskkill（最後手段，可能損失最後一次 flush）');
    KillEngineProcesses;
  end;
  if EngineProcessesRunning then
    Log('警告：jc-engine 或 jc-engine-launcher 仍在執行；讓路可能失敗')
  else
    Log('jc-engine 與 jc-engine-launcher 已結束，可以替換檔案');
end;

// 檔案複製之前唯一的動手時機。先停引擎、移除舊 MSI，再清 .old-* 殘骸，
// 最後把四個程式檔讓開（其中兩個 TSF DLL 可能被載入，引擎與啟動器可能被執行）。
//
// 它從 {app} 執行並握著 jc-engine.exe；不先收掉，Inno 只能把舊檔改名成
// .old-<時間戳> 並排到重開機刪除，使用者會出現「DLL 是新的、引擎是舊的」
// 這種半套升級。新版引擎會用身分檔比對版本與 SHA-256，接手時再擋一次。
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Dir: String;
begin
  Dir := ExpandConstant('{app}');
  // 舊版引擎不認得停止事件，所以用這次安裝帶來的新版執行 --stop。
  ExtractTemporaryFile('jc-engine-stop.exe');
  StopEngine(ExpandConstant('{tmp}\jc-engine-stop.exe'), True);
  if EngineProcessesRunning then begin
    // 還沒動任何檔案就停下來，不要留下「引擎被刪、DLL 已讓路」的半套狀態。
    Result := ExpandConstant('{cm:ErrEngineRunning}');
    Exit;
  end;
  RemoveLegacyMsi;
  SweepLeftovers(Dir);
  Result := ParkProtectedFiles(Dir);
end;

// 反安裝時 DLL 同樣可能被載著刪不掉：清殘骸、把仍在的程式檔排開機刪除，
// 最後收掉資料夾。檔案先登記、目錄後登記，PendingFileRenameOperations 會
// 照順序執行。
procedure CurUninstallStepChanged(CurStep: TUninstallStep);
var
  Dir: String;
begin
  if CurStep = usUninstall then
    StopEngine(ExpandConstant('{app}\jc-engine.exe'), False);
  if CurStep = usPostUninstall then begin
    Dir := ExpandConstant('{app}');
    SweepLeftovers(Dir);
    DeleteOrScheduleDelete(Dir + '\jc-tsf.dll');
    DeleteOrScheduleDelete(Dir + '\jc-tsf-x86.dll');
    DeleteOrScheduleDelete(Dir + '\jc-engine.exe');
    DeleteOrScheduleDelete(Dir + '\jc-engine-launcher.exe');

    RemoveDir(Dir + '\legal\licenses');
    RemoveDir(Dir + '\legal');
    if not RemoveDir(Dir) then
      if GScheduledDelete then begin
        if MoveFileExDelete(Dir, 0, MOVEFILE_DELAY_UNTIL_REBOOT) then
          Log('資料夾還不能刪，已登記下次開機刪除：' + Dir)
        else
          Log('資料夾刪不掉，登記開機刪除也失敗：' + Dir);
      end
      else
        Log('資料夾還有反安裝程式檔案，交給 Inno 收尾：' + Dir);
  end;
end;
