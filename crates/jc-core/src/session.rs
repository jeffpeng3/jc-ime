//! 輸入工作階段：狀態機與對外 API 的骨幹。

/// 這個 crate 的語意版本，供協定握手與診斷使用。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_matches_package_version() {
        assert_eq!(CORE_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(CORE_VERSION.starts_with("0.1.0"));
    }
}
