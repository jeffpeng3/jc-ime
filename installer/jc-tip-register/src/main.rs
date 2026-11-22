//! `jc-tip-register`：MSI 自訂動作的輔助程式。
//!
//! MSI 的安裝精靈（`msiexec`）不能直接呼叫 COM 或 `input.dll`，所以註冊與移除
//! 都經過這支程式。子命令分成兩種權限：
//!
//! | 子命令 | 權限 | 執行時機 |
//! |---|---|---|
//! | `machine-register` | 提升（deferred、`Impersonate="no"`） | `WriteRegistryValues` 之後 |
//! | `machine-unregister` | 提升 | 移除檔案之前 |
//! | `user-install` | 目前使用者（immediate） | 安裝完成之後 |
//! | `user-cleanup-legacy` | 目前使用者（immediate） | 安裝完成之後，清舊語言清單項目 |
//! | `user-uninstall <0\|1>` | 目前使用者 | 解除安裝時 |
//! | `settings` | 目前使用者 | 相容入口；Shortcut 已直連 jc-settings-ui |
//! | `legal` | 目前使用者 | 開始功能表的「授權」捷徑 |
//!
//! `machine-*` 只碰 TSF 的 profile 與分類（`HKLM\SOFTWARE\Microsoft\CTF`）。
//! **CLSID 的 `InprocServer32` 由 MSI 自己寫**：`installer/jc-ime.wxs` 在兩份 TSF
//! DLL 的 `File` 各掛一個 `<Class Advertise="no">`，WiX 因此產生 Registry table
//! 的資料列，`WriteRegistryValues` 會把 64 位元檢視與 `Wow6432Node` 兩份都寫好。
//! （`Advertise="no"` 不走 Class table，所以 `RegisterClassInfo` 不參與。）
//!
//! 這樣安排是因為 rc.5／rc.6 的 `RegisterTip` 在 `RegisterCategory` 崩
//! `0xC0000005`：當時 64 位元檢視的 CLSID 還不存在，msctf 註冊分類時拿不到類別
//! 工廠。把 CLSID 交給 MSI 之後，自訂動作排在 `WriteRegistryValues` 之後，開始跑
//! 時兩個檢視都已經就緒。
//!
//! `user-*` 寫目前使用者的 HKCU，必須以安裝使用者身分執行。`user-install` 只新增；
//! `user-uninstall` 刻意不移除 layout，理由見 [`platform::user_uninstall`]。
//! `user-cleanup-legacy` 呼叫 `RemoveLanguageProfile` 清掉升級前殘留的舊語言清單
//! 項目，不碰 HKLM 的 profile 註冊。
//!
//! 結束碼：0 成功、2 用法錯誤、3 執行失敗。MSI 以 0／非 0 判定。

use std::process::ExitCode;

#[cfg_attr(not(windows), allow(dead_code))]
mod guids;

#[cfg(windows)]
mod log;

#[cfg(windows)]
mod platform;

/// 說明文字。
#[cfg_attr(not(windows), allow(dead_code))]
fn usage() -> &'static str {
    "用法：jc-tip-register <子命令> [參數]\n\
     \n\
     \x20 machine-register                  註冊 TSF profile 與三個分類（CLSID 由 MSI 寫）\n\
     \x20 machine-unregister                反向移除 profile 與分類（CLSID 機碼由 MSI 移除）\n\
     \x20 user-install                      把 jc 注音加進目前使用者的語言清單\n\
     \x20 user-cleanup-legacy               清除已移除舊版的語言清單項目\n\
     \x20 user-uninstall <0|1>              刪除使用者資料；1 表示保留（語言清單不動）\n\
     \x20 settings                          以目前使用者的路徑啟動 jc-settings-ui.exe\n\
     \x20 legal                             用記事本開啟 legal\\THIRD-PARTY-NOTICES.md\n\
     \n\
     machine-* 與 user-* 由 installer/jc-ime.wxs 的自訂動作呼叫；settings 是開始\n\
     功能表捷徑的入口，會把 %LOCALAPPDATA% 展開後再交給設定程式；legal 是「授權」\n\
     捷徑的入口。"
}

/// 解析後的子命令。
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// 以系統權限註冊文字服務的 profile 與分類。
    ///
    /// 不吃參數：CLSID 與 DLL 路徑都由 MSI 負責，這支程式不需要知道檔案在哪。
    MachineRegister,
    /// 以系統權限移除文字服務的 profile 與分類。
    ///
    /// 不吃參數，也不刪 `HKCR\CLSID`：那是 MSI 的 `RemoveRegistryValues` 的工作。
    MachineUnregister,
    /// 把 profile 加進目前使用者的語言清單。
    UserInstall,
    /// 清除已移除舊版殘留的語言清單項目。
    UserCleanupLegacy,
    /// 刪除使用者資料；語言清單刻意不動。
    UserUninstall {
        /// `true` 時保留 `%LOCALAPPDATA%\jc-ime`。
        keep_user_data: bool,
    },
    /// 啟動設定程式（相容入口）；路徑展開現在由 jc-settings-ui 自己負責。
    Settings,
    /// 開啟授權文件（`legal\THIRD-PARTY-NOTICES.md`）。
    Legal,
}

/// 解析命令列。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err("沒有指定子命令".into());
    };
    match command.as_str() {
        "machine-register" => match rest {
            [] => Ok(Command::MachineRegister),
            [extra, ..] => Err(format!(
                "machine-register 不接受參數（收到 {extra}）；CLSID 由 MSI 寫入"
            )),
        },
        "machine-unregister" => match rest {
            [] => Ok(Command::MachineUnregister),
            [extra, ..] => Err(format!(
                "machine-unregister 不接受參數（收到 {extra}）；CLSID 機碼由 MSI 移除"
            )),
        },
        "user-install" => match rest {
            [] => Ok(Command::UserInstall),
            _ => Err("user-install 不接受參數".into()),
        },
        "user-cleanup-legacy" => match rest {
            [] => Ok(Command::UserCleanupLegacy),
            _ => Err("user-cleanup-legacy 不接受參數".into()),
        },
        "settings" => match rest {
            [] => Ok(Command::Settings),
            _ => Err("settings 不接受參數".into()),
        },
        "legal" => match rest {
            [] => Ok(Command::Legal),
            _ => Err("legal 不接受參數".into()),
        },
        "user-uninstall" => match rest {
            [] => Ok(Command::UserUninstall {
                keep_user_data: false,
            }),
            [flag] => match flag.as_str() {
                "0" => Ok(Command::UserUninstall {
                    keep_user_data: false,
                }),
                "1" => Ok(Command::UserUninstall {
                    keep_user_data: true,
                }),
                other => Err(format!("user-uninstall 的參數必須是 0 或 1，收到 {other}")),
            },
            _ => Err("user-uninstall 最多接受一個參數".into()),
        },
        other => Err(format!("未知的子命令：{other}")),
    }
}

#[cfg(windows)]
fn run(args: &[String]) -> Result<(), platform::Error> {
    match parse_args(args).map_err(platform::Error::Usage)? {
        Command::MachineRegister => platform::machine_register(),
        Command::MachineUnregister => platform::machine_unregister(),
        Command::UserInstall => platform::user_install(),
        Command::UserCleanupLegacy => platform::user_cleanup_legacy(),
        Command::UserUninstall { keep_user_data } => platform::user_uninstall(keep_user_data),
        Command::Settings => platform::open_settings(),
        Command::Legal => platform::open_legal(),
    }
}

#[cfg(windows)]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log::init();
    log::step(&format!("啟動參數：{}", args.join(" ")));
    match run(&args) {
        Ok(()) => {
            log::step("結束：成功");
            ExitCode::SUCCESS
        }
        Err(platform::Error::Usage(message)) => {
            log::step(&format!("結束：用法錯誤（{message}）"));
            eprintln!("jc-tip-register：{message}");
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
        Err(error) => {
            log::step(&format!("結束：失敗（{error}）"));
            eprintln!("jc-tip-register：{error}");
            ExitCode::from(3)
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("jc-tip-register 只能在 Windows 上執行；其他平台只支援 cargo check。");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// CLSID 由 MSI 寫，這支程式不需要 DLL 路徑。
    #[test]
    fn machine_register_takes_no_arguments() {
        assert_eq!(
            parse_args(&args(&["machine-register"])),
            Ok(Command::MachineRegister)
        );
        assert!(parse_args(&args(&["machine-register", r"C:\a\jc-tsf.dll"])).is_err());
        assert!(parse_args(&args(&["machine-register", "--arch", "x86"])).is_err());
    }

    /// 反安裝同樣不吃參數，也不分架構：CLSID 機碼由 MSI 的
    /// `RemoveRegistryValues` 移除。
    #[test]
    fn machine_unregister_takes_no_arguments() {
        assert_eq!(
            parse_args(&args(&["machine-unregister"])),
            Ok(Command::MachineUnregister)
        );
        assert!(parse_args(&args(&["machine-unregister", r"C:\a\jc-tsf.dll"])).is_err());
        assert!(parse_args(&args(&["machine-unregister", "--arch", "x86"])).is_err());
    }

    #[test]
    fn user_install_takes_no_arguments() {
        assert_eq!(parse_args(&args(&["user-install"])), Ok(Command::UserInstall));
        assert!(parse_args(&args(&["user-install", "x"])).is_err());
    }

    #[test]
    fn user_cleanup_legacy_is_parsed() {
        assert_eq!(
            parse_args(&args(&["user-cleanup-legacy"])),
            Ok(Command::UserCleanupLegacy)
        );
        assert!(parse_args(&args(&["user-cleanup-legacy", "x"])).is_err());
        // 已移除的平行建置子命令不再接受。
        assert!(parse_args(&args(&["machine-register-next"])).is_err());
        assert!(parse_args(&args(&["user-install-next"])).is_err());
    }

    #[test]
    fn user_uninstall_reads_keep_flag() {
        assert_eq!(
            parse_args(&args(&["user-uninstall"])),
            Ok(Command::UserUninstall {
                keep_user_data: false
            })
        );
        assert_eq!(
            parse_args(&args(&["user-uninstall", "1"])),
            Ok(Command::UserUninstall {
                keep_user_data: true
            })
        );
        assert!(parse_args(&args(&["user-uninstall", "2"])).is_err());
    }

    #[test]
    fn settings_takes_no_arguments() {
        assert_eq!(parse_args(&args(&["settings"])), Ok(Command::Settings));
        assert!(parse_args(&args(&["settings", "x"])).is_err());
    }

    #[test]
    fn legal_takes_no_arguments() {
        assert_eq!(parse_args(&args(&["legal"])), Ok(Command::Legal));
        assert!(parse_args(&args(&["legal", "x"])).is_err());
    }

    #[test]
    fn unknown_command_is_rejected() {
        assert!(parse_args(&args(&["register"])).is_err());
        assert!(parse_args(&[]).is_err());
    }
}
