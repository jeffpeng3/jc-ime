//! 登入自動啟動用的無視窗啟動器：命令列切割與組裝。
//!
//! Windows 的 argv 解析規則由 `CommandLineToArgvW` 決定，引號、反斜線與空白
//! 的組合很容易寫錯。啟動器**不重新組裝**使用者傳進來的參數，只把 argv[0]
//! 之後的原始命令列原封不動轉給 `jc-engine.exe`，因此安裝器寫的引號與空路徑
//! 會原樣保留。

use std::path::{Path, PathBuf};

/// 引擎執行檔名。它與啟動器放在同一個安裝目錄。
pub const ENGINE_EXE: &str = "jc-engine.exe";

/// 從啟動器路徑推出引擎路徑。兩者永遠同目錄。
pub fn engine_path(launcher: &Path) -> PathBuf {
    launcher
        .parent()
        .map(|dir| dir.join(ENGINE_EXE))
        .unwrap_or_else(|| PathBuf::from(ENGINE_EXE))
}

/// 回傳命令列中 argv[0] 之後的原始片段，去掉分隔用的空白。
///
/// `command_line` 是不含結尾 NUL 的 UTF-16 命令列，Windows 的空白只有空格與
/// Tab。引號內的空白不算分隔；argv[0] 是啟動器自己的路徑，不會有逸出引號。
pub fn args_after_program(command_line: &[u16]) -> &[u16] {
    let mut index = 0;
    let mut in_quotes = false;
    while index < command_line.len() {
        let ch = command_line[index];
        if ch == u16::from(b'"') {
            in_quotes = !in_quotes;
        } else if !in_quotes && is_separator(ch) {
            break;
        }
        index += 1;
    }
    while index < command_line.len() && is_separator(command_line[index]) {
        index += 1;
    }
    &command_line[index..]
}

fn is_separator(ch: u16) -> bool {
    ch == u16::from(b' ') || ch == u16::from(b'\t')
}

/// 組出 `"<program>" <tail>`，不含結尾 NUL。
///
/// 程式路徑一定加引號，`C:\Program Files\...` 才不會被拆成兩個參數。路徑沒有
/// 結尾反斜線，因此不必處理「反斜線吃掉收尾引號」的 Windows 特例；安裝目錄
/// 下的檔名固定是 `jc-engine.exe`。
pub fn quoted_command_line(program: &[u16], tail: &[u16]) -> Vec<u16> {
    let mut line = Vec::with_capacity(program.len() + tail.len() + 4);
    line.push(u16::from(b'"'));
    line.extend_from_slice(program);
    line.push(u16::from(b'"'));
    if !tail.is_empty() {
        line.push(u16::from(b' '));
        line.extend_from_slice(tail);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn text(wide: &[u16]) -> String {
        String::from_utf16(wide).expect("測試字串都是合法 UTF-16")
    }

    #[test]
    fn forwards_quoted_paths_and_empty_arguments() {
        let line = utf16(
            r#""C:\Program Files\jc-ime\jc-engine-launcher.exe" --supervise --data "C:\ProgramData\jc-ime\data" --settings "" --personal-prefix personal"#,
        );
        assert_eq!(
            text(args_after_program(&line)),
            r#"--supervise --data "C:\ProgramData\jc-ime\data" --settings "" --personal-prefix personal"#
        );
    }

    #[test]
    fn handles_unquoted_program_and_no_arguments() {
        assert_eq!(
            text(args_after_program(&utf16(r"C:\jc-engine-launcher.exe"))),
            ""
        );
        assert_eq!(
            text(args_after_program(&utf16(
                "C:\\jc-ime\\jc-engine-launcher.exe\t--supervise"
            ))),
            "--supervise"
        );
    }

    #[test]
    fn keeps_whitespace_inside_quoted_arguments() {
        let line = utf16(r#""C:\launcher.exe" --data "C:\dir with space" "  ""#);
        assert_eq!(
            text(args_after_program(&line)),
            r#"--data "C:\dir with space" "  ""#
        );
    }

    #[test]
    fn quotes_program_path_without_changing_tail() {
        let program = utf16(r"C:\Program Files\jc-ime\jc-engine.exe");
        let tail = utf16(r#"--supervise --data "C:\ProgramData\jc-ime\data""#);
        assert_eq!(
            text(&quoted_command_line(&program, &tail)),
            r#""C:\Program Files\jc-ime\jc-engine.exe" --supervise --data "C:\ProgramData\jc-ime\data""#
        );
    }

    #[test]
    fn empty_tail_leaves_no_trailing_space() {
        let line = quoted_command_line(&utf16(r"C:\jc-ime\jc-engine.exe"), &[]);
        assert_eq!(text(&line), r#""C:\jc-ime\jc-engine.exe""#);
    }

    #[test]
    fn engine_lives_next_to_launcher() {
        assert_eq!(
            engine_path(Path::new("/opt/jc-ime/jc-engine-launcher.exe")),
            PathBuf::from("/opt/jc-ime/jc-engine.exe")
        );
        assert_eq!(
            engine_path(Path::new("jc-engine-launcher.exe")),
            PathBuf::from(ENGINE_EXE)
        );
    }

    #[cfg(windows)]
    #[test]
    fn engine_lives_next_to_launcher_in_program_files() {
        assert_eq!(
            engine_path(Path::new(r"C:\Program Files\jc-ime\jc-engine-launcher.exe")),
            PathBuf::from(r"C:\Program Files\jc-ime\jc-engine.exe")
        );
    }
}
