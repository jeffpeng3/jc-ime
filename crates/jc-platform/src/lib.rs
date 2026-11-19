//! 平台抽象。核心只透過這些 trait 與外界互動，作業系統整合由 wrapper 實作。

/// wrapper 提供給核心的宿主介面。
pub trait InputHost {
    /// 更新組字區文字與游標（以 UTF-16 位移表示）。
    fn update_preedit(&mut self, text: &str, cursor_utf16: usize);
    /// 顯示候選清單。
    fn show_candidates(&mut self, items: &[String], highlight: usize);
    /// 送出確定的文字。
    fn commit(&mut self, text: &str);
}

/// 時間來源；測試可注入固定時鐘。
pub trait Clock {
    fn now_ms(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Host;
    impl InputHost for Host {
        fn update_preedit(&mut self, _text: &str, _cursor_utf16: usize) {}
        fn show_candidates(&mut self, _items: &[String], _highlight: usize) {}
        fn commit(&mut self, _text: &str) {}
    }

    #[test]
    fn a_host_can_be_constructed() {
        let _ = Host;
    }
}
