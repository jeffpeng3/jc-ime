//! 輸入模式與文字語言標籤。

/// 輸入模式。
///
/// 收斂後核心只有混打：注音、日文羅馬字與英文都在 [`Mode::Mixed`] 內由
/// 分段器逐段判定語言。純注音、純日文與純英文不再各自是一種輸入模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 注音（台灣繁體中文）。
    Zh,
    /// 中英日混打。
    Mixed,
}

impl Mode {
    /// 從設定或命令列代號解析輸入模式。
    ///
    /// 舊版 `zh`、`ja`、`en` 都映射到 [`Mode::Mixed`]：純注音與純日文已併入
    /// 混打，英文由外殼直接放行，不再送進引擎。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mixed" | "en" | "english" => Some(Mode::Mixed),
            "zh" | "zhuyin" => Some(Mode::Zh),
            "ja" | "japanese" => Some(Mode::Mixed),
            _ => None,
        }
    }
}

/// 片段或輸出使用的語言標籤。
///
/// 這是文字本身的語言，不是輸入模式：同一次混打可以同時包含注音、日文與
/// 英文片段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// 注音（台灣繁體中文）。
    Zh,
    /// 日文（假名輸入）。
    Ja,
    /// 英文直出。
    En,
    /// 尚未判定或跨語言。
    Mixed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_is_copy_and_comparable() {
        let mode = Mode::Mixed;
        let copy = mode;
        assert_eq!(mode, copy);
        assert_ne!(Lang::Zh, Lang::Ja);
        assert_ne!(Lang::En, Lang::Mixed);
    }

    #[test]
    fn legacy_mode_names_all_map_to_mixed() {
        assert_eq!(Mode::parse("en"), Some(Mode::Mixed));
        assert_eq!(Mode::parse("zh"), Some(Mode::Zh));
        assert_eq!(Mode::parse("ja"), Some(Mode::Mixed));
        assert_eq!(Mode::parse("mixed"), Some(Mode::Mixed));
        assert_eq!(Mode::parse("klingon"), None);
    }
}
