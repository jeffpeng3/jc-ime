//! 0c 設定程式：用 GPUI Kit 讀寫 `settings.json`。
//!
//! 進入點照 GPUI Kit 的用法：`application().run(...)` 內先 `init`，再開視窗。
//! 設定檔位置與平台預設值都由 `jc-settings` 決定，這個 crate 只負責畫面。

mod data;
mod options;
mod state;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

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
        gpui_kit::open_window(window_options(), cx, move |_, cx| cx.new(|_| view))
            .expect("開啟設定視窗失敗");
    });
    ExitCode::SUCCESS
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
    let settings_path = settings_path_for(&data_root, settings);
    Ok(Some(LaunchOptions {
        data_root,
        settings_path,
    }))
}

/// 資料根目錄：`JC_DATA`，否則 XDG 資料目錄的 `0c`。
fn default_data_root() -> Result<PathBuf, String> {
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
        assert_eq!(options.settings_path, PathBuf::from("/tmp/d/settings.json"));
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
