//! 漢字範圍，固定在 Unicode 15.0。
//!
//! 刻意不用工具鏈內建的漢字表：那張表跟著 Unicode 版本升級，同一份來源用不同
//! 版本的工具鏈匯入會得到不同的字表。要收新區段的字時改這張表，讓它成為資料
//! 上的決定，而不是換工具鏈的副作用。

/// Unicode 15.0 的漢字（Script=Han）區段，含範圍的頭尾。
const HAN_RANGES: &[(u32, u32)] = &[
    (0x2E80, 0x2E99),
    (0x2E9B, 0x2EF3),
    (0x2F00, 0x2FD5),
    (0x3005, 0x3005),
    (0x3007, 0x3007),
    (0x3021, 0x3029),
    (0x3038, 0x303B),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFA6D),
    (0xFA70, 0xFAD9),
    (0x16FE2, 0x16FE3),
    (0x16FF0, 0x16FF1),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B739),
    (0x2B740, 0x2B81D),
    (0x2B820, 0x2CEA1),
    (0x2CEB0, 0x2EBE0),
    (0x2F800, 0x2FA1D),
    (0x30000, 0x3134A),
    (0x31350, 0x323AF),
];

/// 回報是否為漢字。
pub fn is_han(ch: char) -> bool {
    let c = ch as u32;
    HAN_RANGES
        .iter()
        .any(|&(start, end)| (start..=end).contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_han_is_pinned() {
        let cases: &[(char, bool)] = &[
            ('丼', true),
            ('A', false),
            ('ㄅ', false),
            ('，', false),
            ('\u{323AF}', true),  // 擴充 H 區最後一個字（Unicode 15.0）
            ('\u{2B73A}', false), // Unicode 15.0 之後才補進擴充 C 區尾端
            ('\u{323B0}', false), // 擴充 J 區（Unicode 17.0）
        ];
        for &(ch, want) in cases {
            assert_eq!(is_han(ch), want, "is_han({ch:?})");
        }
    }
}
