//! Big5 → Unicode 查表。
//!
//! 給前導字元 `b` 用的碼位轉換。資料是 `scripts/gen-big5-table.py` 產生的
//! fixed-width 表，編譯時用 `include_bytes!` 帶進二進位：
//!
//! - 合法區間：lead `0xA1..=0xF9`、trail `0x40..=0x7E` 與 `0xA1..=0xFE`。
//! - 每列 157 格，每格 2 bytes（u16 小端），`0` 代表沒有對應。
//! - 索引是直接算出來的，查表 O(1)，不需要二分搜尋或排序。
//!
//! 資料來源與授權見 `docs/port/big5-table.md`。

/// 合法 lead 的下界。
const LEAD_MIN: u8 = 0xA1;
/// 合法 lead 的上界。
const LEAD_MAX: u8 = 0xF9;
/// 一列可用的 trail 數：`0x40..=0x7E` 共 63 個，`0xA1..=0xFE` 共 94 個。
const TRAIL_SLOTS: usize = 63 + 94;
/// lead 的列數。
const LEAD_COUNT: usize = (LEAD_MAX as usize) - (LEAD_MIN as usize) + 1;
/// 表格格數。
const SLOT_COUNT: usize = LEAD_COUNT * TRAIL_SLOTS;

const TABLE: &[u8; SLOT_COUNT * 2] = include_bytes!("../data/big5/big5.bin");

/// 把 Big5 雙位元組碼位轉成 Unicode 字元。
///
/// 只接受 Big5 合法區間內、且表中有對應的碼位；其他輸入回 [`None`]。
///
/// ```
/// assert_eq!(jc_core::big5_to_unicode(0xA440), Some('一'));
/// assert_eq!(jc_core::big5_to_unicode(0xA4A4), Some('中'));
/// assert_eq!(jc_core::big5_to_unicode(0x8140), None);
/// ```
pub fn big5_to_unicode(code: u16) -> Option<char> {
    let index = slot_index(code)?;
    let offset = index * 2;
    let scalar = u16::from_le_bytes([TABLE[offset], TABLE[offset + 1]]) as u32;
    if scalar == 0 {
        return None;
    }
    char::from_u32(scalar)
}

/// 把碼位換成表格索引；超出合法區間回 [`None`]。
fn slot_index(code: u16) -> Option<usize> {
    let lead = (code >> 8) as u8;
    let trail = code as u8;
    let lead = lead.checked_sub(LEAD_MIN)? as usize;
    if lead >= LEAD_COUNT {
        return None;
    }
    let column = match trail {
        0x40..=0x7E => (trail - 0x40) as usize,
        0xA1..=0xFE => 63 + (trail - 0xA1) as usize,
        _ => return None,
    };
    Some(lead * TRAIL_SLOTS + column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fmt::Write;

    /// 資料檔的 SHA-256；重跑 `scripts/gen-big5-table.py` 後若變了，這裡要一起更新。
    const TABLE_SHA256: &str = "6599b630ec39a573fbb21bd09d08535e373199c4a69d9b6313e1977832e102c2";
    /// `python3 scripts/gen-big5-table.py` 產生的映射數。
    const MAPPED_COUNT: usize = 13_710;

    #[test]
    fn known_codes() {
        assert_eq!(big5_to_unicode(0xA440), Some('一'));
        assert_eq!(big5_to_unicode(0xA4A4), Some('中'));
        assert_eq!(big5_to_unicode(0xA140), Some('\u{3000}'));
        assert_eq!(big5_to_unicode(0xA141), Some('，'));
    }

    #[test]
    fn last_mapped_code() {
        assert_eq!(big5_to_unicode(0xF9D5), Some('龘'));
        assert_eq!(big5_to_unicode(0xF9FE), None);
    }

    #[test]
    fn invalid_codes() {
        for code in [
            0x0000, 0x8140, 0xA100, 0xA13F, 0xA17F, 0xA1FF, 0xFA40, 0xFFFF,
        ] {
            assert_eq!(big5_to_unicode(code), None, "0x{code:04X} 不該有對應");
        }
    }

    #[test]
    fn table_layout_and_values_are_valid() {
        assert_eq!(SLOT_COUNT, 89 * 157);
        assert_eq!(TABLE.len(), SLOT_COUNT * 2);

        let mut mapped = 0;
        for (slot, pair) in TABLE.as_chunks::<2>().0.iter().enumerate() {
            let scalar = u16::from_le_bytes(*pair) as u32;
            if scalar == 0 {
                continue;
            }
            mapped += 1;
            assert!(
                char::from_u32(scalar).is_some(),
                "slot {slot} 的 U+{scalar:04X} 不是合法字元"
            );
        }
        assert_eq!(mapped, MAPPED_COUNT);
    }

    #[test]
    fn every_mapped_slot_is_reachable() {
        // 反向確認索引算法：每個非零格子都能由一組合法碼位查到。
        let mut reachable = 0;
        for lead in LEAD_MIN..=LEAD_MAX {
            for trail in (0x40..=0x7E).chain(0xA1..=0xFE) {
                let code = u16::from_be_bytes([lead, trail]);
                if big5_to_unicode(code).is_some() {
                    reachable += 1;
                }
            }
        }
        assert_eq!(reachable, MAPPED_COUNT);
    }

    #[test]
    fn table_checksum() {
        let mut hex = String::with_capacity(64);
        for byte in Sha256::digest(TABLE) {
            write!(&mut hex, "{byte:02x}").expect("寫入 String 不會失敗");
        }
        assert_eq!(hex, TABLE_SHA256);
    }
}
