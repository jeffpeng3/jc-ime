//! 0c 設定程式：用 GPUI Kit 讀寫 `settings.json`。
//!
//! 進入點照 GPUI Kit 的用法：`application().run(...)` 內先 `init`，再開視窗。
//! 設定檔位置與平台預設值都由 `jc-settings` 決定，這個 crate 只負責畫面。

// 開始功能表的捷徑直接啟動這支程式；windows subsystem 讓 Windows 不要配置
// console 視窗。命令列模式的 --help 輸出跟著看不到，可接受。
#![windows_subsystem = "windows"]

mod data;
mod options;
mod state;
mod ui;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::state::{settings_path_for, LaunchOptions};

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("jc-settings-ui：{message}");
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    };
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::component::set_locale("zh-TW");
        let view = state::SettingsView::load(&options);
        gpui_kit::open_window(window_options(), cx, move |window, cx| {
            follow_system_theme(window, cx);
            cx.new(|_| view)
        })
        .expect("開啟設定視窗失敗");
    });
    ExitCode::SUCCESS
}

/// 啟動時套用系統外觀，之後系統深色／淺色切換也跟著換主題。
///
/// `WindowOptions` 沒有 appearance 欄位；`Window::appearance()` 是 GPUI 從平台
/// 讀到的值。`observe_window_appearance` 的訂閱要 `detach()`，才會活到視窗關閉。
fn follow_system_theme(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
    window
        .observe_window_appearance(|window, cx| Theme::sync_system_appearance(Some(window), cx))
        .detach();
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(120.), px(120.)),
            size: size(px(1080.), px(720.)),
        })),
        titlebar: Some(TitlebarOptions {
            title: Some("0c 設定".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 解析命令列；`Ok(None)` 表示只印說明。
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<LaunchOptions>, String> {
    let mut data_root: Option<PathBuf> = None;
    let mut settings: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                let value = args.next().ok_or("--data 需要一個目錄")?;
                data_root = Some(PathBuf::from(value));
            }
            "--settings" => {
                let value = args.next().ok_or("--settings 需要一個檔案路徑")?;
                settings = Some(PathBuf::from(value));
            }
            "--help" | "-h" => return Ok(None),
            other => return Err(format!("不認得的參數：{other}")),
        }
    }
    let data_root = match data_root {
        Some(path) => path,
        None => default_data_root()?,
    };
    let settings_path = match settings {
        Some(path) => path,
        None => default_settings_path(&data_root)?,
    };
    Ok(Some(LaunchOptions {
        data_root,
        settings_path,
    }))
}

/// 資料根目錄：Windows 安裝版用 `%ProgramData%\jc-ime\data`（與 MSI 的
/// `[DATADIR]` 一致），其餘平台照 `JC_DATA` → XDG → `HOME` 的順序找。
fn default_data_root() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        if let Some(value) = std::env::var_os("ProgramData").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(value).join("jc-ime").join("data"));
        }
    }
    if let Some(value) = std::env::var_os("JC_DATA") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value).join("0c"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home).join(".local").join("share").join("0c"));
        }
    }
    Err("找不到資料根目錄；請用 --data 指定，或設定 JC_DATA".to_string())
}

/// 設定檔路徑：Windows 安裝版展開 `%LOCALAPPDATA%\jc-ime\settings.json`，
/// 其餘平台沿用資料根目錄下的 `settings.json`。
///
/// `.lnk` 的參數在啟動時不會展開環境變數，per-machine 捷徑也不能寫死單一
/// 使用者的路徑，因此展開留給程式在執行時做。這原本是
/// `jc-tip-register settings` 的工作；Shortcut 改為直連本程式後，職責跟著
/// 搬過來。
fn default_settings_path(data_root: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        if let Some(value) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
            let dir = PathBuf::from(value).join("jc-ime");
            std::fs::create_dir_all(&dir)
                .map_err(|error| format!("無法建立 {}：{error}", dir.display()))?;
            return Ok(dir.join("settings.json"));
        }
    }
    Ok(settings_path_for(data_root, None))
}

fn usage() -> &'static str {
    "用法：jc-settings-ui [--data <資料根目錄>] [--settings <設定檔>]\n\
     \n\
     \x20 --data <dir>       資料根目錄；預設讀 JC_DATA，再退回 XDG 資料目錄的 0c\n\
     \x20 --settings <path>  直接指定 settings.json\n\
     \x20 -h, --help         顯示這份說明"
}

#[cfg(test)]
mod tests {
    use super::parse_args;
    use std::path::PathBuf;

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| value.to_string())
    }

    #[test]
    fn test_data_and_settings_are_parsed() {
        let options = parse_args(args(&["--data", "/tmp/d", "--settings", "/tmp/s.json"]))
            .expect("解析成功")
            .expect("有工作");
        assert_eq!(options.data_root, PathBuf::from("/tmp/d"));
        assert_eq!(options.settings_path, PathBuf::from("/tmp/s.json"));
    }

    #[test]
    fn test_settings_defaults_to_data_root() {
        let options = parse_args(args(&["--data", "/tmp/d"]))
            .expect("解析成功")
            .expect("有工作");
        // 預期值照 default_settings_path 的分支算：Windows 展開 %LOCALAPPDATA%，
        // 其餘平台沿用傳入的資料根目錄。這樣 CI windows job 不會拿真實使用者
        // 路徑去比寫死的 /tmp/d/settings.json。函式本身的行為不變。
        let fallback = PathBuf::from("/tmp/d/settings.json");
        let expected = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join("jc-ime").join("settings.json"))
                .unwrap_or(fallback)
        } else {
            fallback
        };
        assert_eq!(options.settings_path, expected);
    }

    #[test]
    fn test_missing_value_is_rejected() {
        assert!(parse_args(args(&["--data"])).is_err());
        assert!(parse_args(args(&["--settings"])).is_err());
        assert!(parse_args(args(&["--nope"])).is_err());
    }

    #[test]
    fn test_help_returns_none() {
        assert!(parse_args(args(&["--help"])).expect("解析成功").is_none());
    }
}
