//! `jc-tip-register`：MSI 自訂動作的輔助程式。
//!
//! MSI 的安裝精靈（`msiexec`）不能直接呼叫 COM 或 `input.dll`，所以註冊與移除
//! 都經過這支程式。四個子命令分成兩種權限：
//!
//! | 子命令 | 權限 | 執行時機 |
//! |---|---|---|
//! | `machine-register <dll>...` | 提升（deferred、`Impersonate="no"`） | 安裝檔案之後 |
//! | `machine-unregister [<dll>...]` | 提升 | 移除檔案之前 |
//! | `user-install` | 目前使用者（immediate） | 安裝完成之後 |
//! | `user-uninstall <0\|1>` | 目前使用者 | 解除安裝時 |
//! | `settings` | 目前使用者 | 開始功能表的設定捷徑 |
//! | `legal` | 目前使用者 | 開始功能表的「授權」捷徑 |
//!
//! `machine-*` 優先呼叫 `jc-tsf.dll` 自己的 `DllRegisterServer`／
//! `DllUnregisterServer`，不重抄一份註冊邏輯；DLL 不在（例如被防毒隔離）時才用
//! 內建的退路直接清 CTF 機碼。`user-*` 透過 `input.dll` 的 `InstallLayoutOrTip`
//! 修改目前使用者的語言清單，這一步**不能**在 SYSTEM 身分執行，否則記號會落在
//! SYSTEM 的 HKCU。
//!
//! 產品有 x64 與 x86 兩份 `jc-tsf`，**共用同一個 CLSID 與 profile**，只差註冊在
//! 哪一個機碼檢視。這支程式是 x64，載入不了 32 位元 DLL，所以 `machine-*` 對每個
//! 路徑判斷 PE 架構（`--arch` 可覆寫）：同架構才呼叫 DLL 的匯出函式，跨架構由
//! 這裡直接寫對應檢視並呼叫 TSF 介面。詳見 `docs/port/x86-tip.md`。
//!
//! 結束碼：0 成功、2 用法錯誤、3 執行失敗。MSI 以 0／非 0 判定。

use std::path::PathBuf;
use std::process::ExitCode;

#[cfg_attr(not(windows), allow(dead_code))]
mod arch;

#[cfg_attr(not(windows), allow(dead_code))]
mod guids;

#[cfg(windows)]
mod platform;

/// 說明文字。
#[cfg_attr(not(windows), allow(dead_code))]
fn usage() -> &'static str {
    "用法：jc-tip-register <子命令> [參數]\n\
     \n\
     \x20 machine-register [--arch x64|x86] <dll>...  註冊 COM、CTF profile 與分類，並設定 AppContainer 讀取權限\n\
     \x20 machine-unregister [dll...]                 反向移除；DLL 不在時直接清 CTF 機碼（兩個檢視都清）\n\
     \x20 user-install                    把 jc 注音加進目前使用者的語言清單\n\
     \x20 user-uninstall <0|1>            從目前使用者移除；1 表示保留使用者設定\n\
     \x20 settings                        以目前使用者的路徑啟動 jc-settings-ui.exe\n\
     \x20 legal                           用記事本開啟 legal\\THIRD-PARTY-NOTICES.md\n\
     \n\
     machine-register 通常一次給兩個 DLL（x64 與 x86）；--arch 省略時由 PE 標頭\n\
     自動判斷，只在手動修復單一架構時才需要指定。machine-* 與 user-* 由\n\
     installer/jc-ime.wxs 的自訂動作呼叫；settings 是開始功能表捷徑的入口，會把\n\
     %LOCALAPPDATA% 展開後再交給設定程式；legal 是「授權」捷徑的入口。"
}

/// 解析後的子命令。
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// 以系統權限註冊文字服務；每個 DLL 寫自己那個機碼檢視。
    MachineRegister {
        /// `jc-tsf.dll` 的路徑；通常 x64 與 x86 各一個。
        dlls: Vec<PathBuf>,
        /// `--arch` 指定的架構（只給一個 DLL 時才有意義）。
        arch: Option<arch::Arch>,
    },
    /// 以系統權限移除文字服務；路徑可省略（兩個檢視都清）。
    MachineUnregister {
        /// `jc-tsf.dll` 的路徑。
        dlls: Vec<PathBuf>,
    },
    /// 把 profile 加進目前使用者的語言清單。
    UserInstall,
    /// 從目前使用者的語言清單移除，並可選擇清掉使用者資料。
    UserUninstall {
        /// `true` 時保留 `%LOCALAPPDATA%\jc-ime`。
        keep_user_data: bool,
    },
    /// 啟動設定程式；路徑由程式自己展開，不依賴捷徑的環境變數。
    Settings,
    /// 開啟授權文件（`legal\THIRD-PARTY-NOTICES.md`）。
    Legal,
}

/// 從參數裡挑出 `--arch`，其餘依原順序留下。
///
/// 兩種寫法都接受：`--arch x86` 與 `--arch=x86`。
fn split_arch(args: &[String]) -> Result<(Option<arch::Arch>, Vec<&String>), String> {
    let mut selected = None;
    let mut positional = Vec::new();
    let mut values = args.iter();
    while let Some(argument) = values.next() {
        if argument == "--arch" {
            let value = values
                .next()
                .ok_or_else(|| "--arch 需要一個值（x64 或 x86）".to_string())?;
            selected = Some(arch::parse_arch(value)?);
        } else if let Some(value) = argument.strip_prefix("--arch=") {
            selected = Some(arch::parse_arch(value)?);
        } else {
            positional.push(argument);
        }
    }
    Ok((selected, positional))
}

/// 解析命令列。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        return Err("沒有指定子命令".into());
    };
    match command.as_str() {
        "machine-register" => {
            let (arch, positional) = split_arch(rest)?;
            if positional.is_empty() {
                return Err("machine-register 需要至少一個 jc-tsf.dll 路徑".into());
            }
            Ok(Command::MachineRegister {
                dlls: positional.iter().map(PathBuf::from).collect(),
                arch,
            })
        }
        "machine-unregister" => {
            let (arch, positional) = split_arch(rest)?;
            if arch.is_some() {
                return Err("machine-unregister 一律清兩個機碼檢視，不接受 --arch".into());
            }
            Ok(Command::MachineUnregister {
                dlls: positional.iter().map(PathBuf::from).collect(),
            })
        }
        "user-install" => match rest {
            [] => Ok(Command::UserInstall),
            _ => Err("user-install 不接受參數".into()),
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
        Command::MachineRegister { dlls, arch } => platform::machine_register(&dlls, arch),
        Command::MachineUnregister { dlls } => platform::machine_unregister(&dlls),
        Command::UserInstall => platform::user_install(),
        Command::UserUninstall { keep_user_data } => platform::user_uninstall(keep_user_data),
        Command::Settings => platform::open_settings(),
        Command::Legal => platform::open_legal(),
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
    use arch::Arch;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn paths(values: &[&str]) -> Vec<PathBuf> {
        values.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn machine_register_requires_a_path() {
        assert!(parse_args(&args(&["machine-register"])).is_err());
        assert_eq!(
            parse_args(&args(&["machine-register", r"C:\a\jc-tsf.dll"])),
            Ok(Command::MachineRegister {
                dlls: paths(&[r"C:\a\jc-tsf.dll"]),
                arch: None,
            })
        );
    }

    /// WXS 一次給兩個 DLL；順序就是參數順序。
    #[test]
    fn machine_register_accepts_both_architectures_in_one_call() {
        assert_eq!(
            parse_args(&args(&[
                "machine-register",
                r"C:\a\jc-tsf.dll",
                r"C:\a\jc-tsf-x86.dll"
            ])),
            Ok(Command::MachineRegister {
                dlls: paths(&[r"C:\a\jc-tsf.dll", r"C:\a\jc-tsf-x86.dll"]),
                arch: None,
            })
        );
    }

    /// 手動修復單一架構時可以指定 `--arch`，放在路徑前後都行。
    #[test]
    fn machine_register_accepts_arch_before_or_after_the_path() {
        let expected = Ok(Command::MachineRegister {
            dlls: paths(&[r"C:\a\jc-tsf-x86.dll"]),
            arch: Some(Arch::X86),
        });
        assert_eq!(
            parse_args(&args(&[
                "machine-register",
                "--arch",
                "x86",
                r"C:\a\jc-tsf-x86.dll"
            ])),
            expected
        );
        assert_eq!(
            parse_args(&args(&[
                "machine-register",
                r"C:\a\jc-tsf-x86.dll",
                "--arch=x86"
            ])),
            expected
        );
        assert!(parse_args(&args(&["machine-register", "--arch"])).is_err());
        assert!(parse_args(&args(&[
            "machine-register",
            "--arch",
            "arm64",
            r"C:\a\jc-tsf.dll"
        ]))
        .is_err());
    }

    #[test]
    fn machine_unregister_path_is_optional() {
        assert_eq!(
            parse_args(&args(&["machine-unregister"])),
            Ok(Command::MachineUnregister { dlls: Vec::new() })
        );
        assert_eq!(
            parse_args(&args(&[
                "machine-unregister",
                r"C:\a\jc-tsf.dll",
                r"C:\a\jc-tsf-x86.dll"
            ])),
            Ok(Command::MachineUnregister {
                dlls: paths(&[r"C:\a\jc-tsf.dll", r"C:\a\jc-tsf-x86.dll"]),
            })
        );
        // 反安裝不分架構：兩個檢視都清，所以不接受 --arch。
        assert!(parse_args(&args(&["machine-unregister", "--arch", "x86"])).is_err());
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
