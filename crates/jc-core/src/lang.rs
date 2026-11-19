//! 輸入模式。

/// 輸入模式。
///
/// `Zh` 只接受注音，`Ja` 只接受日文羅馬字，`En` 是英文直出；
/// `Mixed` 同時接受注音、日文羅馬字與英文，由分段器逐段判定語言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 注音（台灣繁體中文）。
    Zh,
    /// 日文（羅馬字輸入）。
    Ja,
    /// 英文直出。
    En,
    /// 中英日混打。
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
        assert_ne!(Mode::Zh, Mode::Ja);
        assert_ne!(Mode::En, Mode::Mixed);
    }
}
