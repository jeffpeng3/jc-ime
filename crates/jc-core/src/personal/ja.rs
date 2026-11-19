//! 日文個人學習檔路徑（對照 `core/personal/ja.go`）。

use std::path::{Path, PathBuf};

/// 日文個人學習檔的固定檔名。
pub const JA_FILE_NAME: &str = "personal.ja.json";

/// 回傳與注音個人資料檔同目錄的日文學習檔路徑。
pub fn ja_path(personal_path: &str) -> PathBuf {
    Path::new(personal_path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(JA_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ja_path_uses_same_directory_and_separate_file() {
        assert_eq!(
            ja_path("/data/user.json"),
            PathBuf::from("/data/personal.ja.json")
        );
        assert_eq!(ja_path("personal.json"), PathBuf::from("personal.ja.json"));
    }
}
