//! `jc-tip-register`：MSI 自訂動作的輔助程式。
//!
//! MSI 的安裝精靈（`msiexec`）不能直接呼叫 COM 或 `input.dll`，所以註冊與移除
//! 都經過這支程式。四個子命令分成兩種權限：
//!
//! | 子命令 | 權限 | 執行時機 |
//! |---|---|---|
//! | `machine-register <jc-tsf.dll>` | 提升（deferred、`Impersonate="no"`） | 安裝檔案之後 |
//! | `machine-unregister [<jc-tsf.dll>]` | 提升 | 移除檔案之前 |
//! | `user-install` | 目前使用者（immediate） | 安裝完成之後 |
//! | `user-uninstall <0\|1>` | 目前使用者 | 解除安裝時 |
//! | `settings` | 目前使用者 | 開始功能表的設定捷徑 |
//!
//! `machine-*` 呼叫 `jc-tsf.dll` 自己的 `DllRegisterServer`／`DllUnregisterServer`，
//! 不重抄一份註冊邏輯；只有 DLL 已不在（例如被防毒隔離）時才用內建的退路直接
//! 清 CTF 機碼。`user-*` 透過 `input.dll` 的 `InstallLayoutOrTip` 修改目前使用者的
//! 語言清單，這一步**不能**在 SYSTEM 身分執行，否則記號會落在 SYSTEM 的 HKCU。
//!
//! 結束碼：0 成功、2 用法錯誤、3 執行失敗。MSI 以 0／非 0 判定。

use std::path::PathBuf;
use std::process::ExitCode;

#[cfg_attr(not(windows), allow(dead_code))]
mod guids;

#[cfg(windows)]
mod platform;

/// 說明文字。
#[cfg_attr(not(windows), allow(dead_code))]
fn usage() -> &'static str {
    "用法：jc-tip-register <子命令> [參數]\n\
     \n\
     \x20 machine-register <jc-tsf.dll>   註冊 COM、CTF profile 與分類，並設定 AppContainer 讀取權限\n\
     \x20 machine-unregister [dll]        反向移除；DLL 不在時直接清 CTF 機碼\n\
     \x20 user-install                    把 jc 注音加進目前使用者的語言清單\n\
     \x20 user-uninstall <0|1>            從目前使用者移除；1 表示保留使用者設定\n\
     \x20 settings                        以目前使用者的路徑啟動 jc-settings-ui.exe\n\
     \n\
     machine-* 與 user-* 由 installer/jc-ime.wxs 的自訂動作呼叫；settings 是開始\n\
     功能表捷徑的入口，會把 %LOCALAPPDATA% 展開後再交給設定程式。"
}

/// 解析後的子命令。
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// 以系統權限註冊文字服務。
    MachineRegister(PathBuf),
    /// 以系統權限移除文字服務；路徑可省略。
    MachineUnregister(Option<PathBuf>),
    /// 把 profile 加進目前使用者的語言清單。
    UserInstall,
    /// 從目前使用者的語言清單移除，並可選擇清掉使用者資料。
    UserUninstall {
        /// `true` 時保留 `%LOCALAPPDATA%\jc-ime`。
        keep_user_data: bool,
    },
    /// 啟動設定程式；路徑由程式自己展開，不依賴捷徑的環境變數。
    Settings,
}

/// 解析命令列。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err("沒有指定子命令".into());
    };
    match command.as_str() {
        "machine-register" => match rest {
            [dll] => Ok(Command::MachineRegister(PathBuf::from(dll))),
            _ => Err("machine-register 需要一個 jc-tsf.dll 路徑".into()),
        },
        "machine-unregister" => match rest {
            [] => Ok(Command::MachineUnregister(None)),
            [dll] => Ok(Command::MachineUnregister(Some(PathBuf::from(dll)))),
            _ => Err("machine-unregister 最多接受一個 DLL 路徑".into()),
        },
        "user-install" => match rest {
            [] => Ok(Command::UserInstall),
            _ => Err("user-install 不接受參數".into()),
        },
        "settings" => match rest {
            [] => Ok(Command::Settings),
            _ => Err("settings 不接受參數".into()),
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
        Command::MachineRegister(dll) => platform::machine_register(&dll),
        Command::MachineUnregister(dll) => platform::machine_unregister(dll.as_deref()),
        Command::UserInstall => platform::user_install(),
        Command::UserUninstall { keep_user_data } => platform::user_uninstall(keep_user_data),
        Command::Settings => platform::open_settings(),
    }
}

#[cfg(windows)]
fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(platform::Error::Usage(message)) => {
            eprintln!("jc-tip-register：{message}");
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
        Err(error) => {
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

    #[test]
    fn machine_register_requires_a_path() {
        assert!(parse_args(&args(&["machine-register"])).is_err());
        assert_eq!(
            parse_args(&args(&["machine-register", r"C:\a\jc-tsf.dll"])),
            Ok(Command::MachineRegister(PathBuf::from(r"C:\a\jc-tsf.dll")))
        );
    }

    #[test]
    fn machine_unregister_path_is_optional() {
        assert_eq!(
            parse_args(&args(&["machine-unregister"])),
            Ok(Command::MachineUnregister(None))
        );
        assert_eq!(
            parse_args(&args(&["machine-unregister", r"C:\a\jc-tsf.dll"])),
            Ok(Command::MachineUnregister(Some(PathBuf::from(
                r"C:\a\jc-tsf.dll"
            ))))
        );
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
    fn unknown_command_is_rejected() {
        assert!(parse_args(&args(&["register"])).is_err());
        assert!(parse_args(&[]).is_err());
    }
}
