//! 檢查結果的輸出格式。
//!
//! 每一項檢查固定輸出一行：`[標籤] 編號. 標題：說明`。標籤只有四種：
//!
//! - `[PASS]`：通過
//! - `[FAIL]`：失敗，會讓行程以非零碼結束
//! - `[SKIP]`：無法判定（例如引擎沒開、輸入法沒安裝），不算失敗
//! - `[INFO]`：輔助資訊，不影響結果
//!
//! 這個模組不碰 Win32，Linux 的 `cargo check` 也會編譯它。

/// 通過。
pub const PASS: &str = "[PASS]";
/// 失敗。
pub const FAIL: &str = "[FAIL]";
/// 無法判定。
pub const SKIP: &str = "[SKIP]";
/// 輔助資訊。
pub const INFO: &str = "[INFO]";

/// 組出一行輸出，格式固定為 `[標籤] 標題：說明`。
pub fn line(tag: &str, title: &str, detail: &str) -> String {
    format!("{tag} {title}：{detail}")
}

/// 把 32 位元值格式化成 `0x` 開頭的大寫十六進位。
pub fn hex(value: u32) -> String {
    format!("0x{value:08X}")
}

/// 累計各項結果並負責輸出。
#[derive(Debug, Default)]
pub struct Report {
    passed: usize,
    failed: usize,
    skipped: usize,
}

impl Report {
    /// 記一項通過。
    pub fn pass(&mut self, title: impl AsRef<str>, detail: impl AsRef<str>) {
        self.passed += 1;
        println!("{}", line(PASS, title.as_ref(), detail.as_ref()));
    }

    /// 記一項失敗。
    pub fn fail(&mut self, title: impl AsRef<str>, detail: impl AsRef<str>) {
        self.failed += 1;
        println!("{}", line(FAIL, title.as_ref(), detail.as_ref()));
    }

    /// 記一項無法判定。
    pub fn skip(&mut self, title: impl AsRef<str>, detail: impl AsRef<str>) {
        self.skipped += 1;
        println!("{}", line(SKIP, title.as_ref(), detail.as_ref()));
    }

    /// 印一行輔助資訊，不計入任何計數。
    pub fn info(&self, detail: impl AsRef<str>) {
        println!("{} {}", INFO, detail.as_ref());
    }

    /// 失敗項數；行程結束碼由它決定。
    pub fn failed(&self) -> usize {
        self.failed
    }

    /// 一行總結。
    pub fn summary(&self) -> String {
        format!(
            "{} 項通過、{} 項失敗、{} 項略過",
            self.passed, self.failed, self.skipped
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_format_is_stable() {
        assert_eq!(
            line(PASS, "1. LoadLibraryW", "S_OK"),
            "[PASS] 1. LoadLibraryW：S_OK"
        );
        assert_eq!(
            line(FAIL, "2. DllGetClassObject", "HRESULT 0x80040111"),
            "[FAIL] 2. DllGetClassObject：HRESULT 0x80040111"
        );
        assert_eq!(
            line(SKIP, "6. 引擎 pipe", "沒開"),
            "[SKIP] 6. 引擎 pipe：沒開"
        );
        assert_eq!(line(INFO, "tid", "1"), "[INFO] tid：1");
    }

    #[test]
    fn hex_pads_to_eight_digits() {
        assert_eq!(hex(0), "0x00000000");
        assert_eq!(hex(0x8000_4005), "0x80004005");
        assert_eq!(hex(0xffff_ffff), "0xFFFFFFFF");
    }

    #[test]
    fn counts_are_independent() {
        // 不經過 println 的計數驗證：直接檢查計數器欄位。
        let report = Report {
            passed: 2,
            failed: 0,
            skipped: 3,
        };
        assert_eq!(report.failed(), 0);
        assert_eq!(report.summary(), "2 項通過、0 項失敗、3 項略過");
    }
}
