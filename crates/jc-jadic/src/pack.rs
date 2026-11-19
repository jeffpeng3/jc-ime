//! 0CJADIC pack v2 編碼。
//!
//! 版面照 `jc-ime-data/plan/jc-ime/pack-v2-spec.md`：96 位元組檔頭、
//! 24 位元組 entry、surface index、POS／活用 table、context 連接矩陣，
//! 檔尾是 payload 的 SHA-256。

use sha2::{Digest, Sha256};

use crate::error::ImportError;

/// pack magic。
pub const MAGIC: [u8; 8] = *b"0CJADIC\0";
/// v2 格式版本。
pub const FORMAT_VERSION: u16 = 2;
/// v2 固定檔頭大小。
pub const HEADER_SIZE: usize = 96;
/// v2 固定 entry 大小。
pub const ENTRY_SIZE: usize = 24;
/// surface index 每筆大小。
pub const INDEX_ENTRY_SIZE: usize = 8;
/// 檔尾 SHA-256 大小。
pub const TRAILER_SIZE: usize = 32;
/// 詞成本下限（-32768 保留）。
pub const MIN_COST: i16 = -32767;

/// 要寫進 pack 的一筆詞條。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    /// 詞面。
    pub surface: String,
    /// 平假名讀音。
    pub reading: String,
    /// POS table 索引。
    pub pos_id: u16,
    /// 活用型索引。
    pub conj_type_id: u8,
    /// 活用形索引。
    pub conj_form_id: u8,
    /// 詞成本。
    pub cost: i16,
    /// flags，bit0-1 是 split。
    pub flags: u16,
    /// 左 context id。
    pub left_id: u16,
    /// 右 context id。
    pub right_id: u16,
}

/// 一個待編碼的 pack。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackData {
    /// 已依 surface 排序的詞條。
    pub entries: Vec<PackEntry>,
    /// 排序後的 POS table。
    pub pos: Vec<String>,
    /// 排序後的活用型 table。
    pub conj_types: Vec<String>,
    /// 排序後的活用形 table。
    pub conj_forms: Vec<String>,
    /// row-major 的連接成本，長度必須是 `context_count²`。
    pub connection: Vec<i16>,
    /// context 數量。
    pub context_count: u16,
}

/// 編碼結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// 完整檔案位元組，含檔尾 SHA-256。
    pub bytes: Vec<u8>,
    /// payload 的 SHA-256。
    pub payload_sha256: [u8; 32],
}

/// 把 [`PackData`] 編成 pack v2 位元組。
pub fn encode(data: &PackData) -> Result<Encoded, ImportError> {
    if data.context_count == 0 {
        return Err(ImportError::parse(
            "pack",
            0,
            "v2 的 context_count 不可為 0",
        ));
    }
    let n = data.context_count as usize;
    if data.connection.len() != n * n {
        return Err(ImportError::parse(
            "pack",
            0,
            format!(
                "連接矩陣有 {} 筆，預期 {}×{} = {} 筆",
                data.connection.len(),
                n,
                n,
                n * n
            ),
        ));
    }
    if data.pos.len() > u16::MAX as usize {
        return Err(ImportError::CountOverflow {
            field: "pos_count",
            value: data.pos.len(),
            limit: u16::MAX as u64,
        });
    }
    if data.conj_types.len() > u8::MAX as usize {
        return Err(ImportError::CountOverflow {
            field: "conj_type_count",
            value: data.conj_types.len(),
            limit: u8::MAX as u64,
        });
    }
    if data.conj_forms.len() > u8::MAX as usize {
        return Err(ImportError::CountOverflow {
            field: "conj_form_count",
            value: data.conj_forms.len(),
            limit: u8::MAX as u64,
        });
    }
    if data.entries.len() > u32::MAX as usize {
        return Err(ImportError::CountOverflow {
            field: "entry_count",
            value: data.entries.len(),
            limit: u32::MAX as u64,
        });
    }

    let mut surface_blob: Vec<u8> = Vec::new();
    let mut reading_blob: Vec<u8> = Vec::new();
    let mut raw_entries: Vec<[u8; ENTRY_SIZE]> = Vec::with_capacity(data.entries.len());
    let mut surface_index: Vec<(u32, u32)> = Vec::new();
    let mut previous_surface: Option<&str> = None;
    let mut current_surface_offset: u32 = 0;

    for (index, entry) in data.entries.iter().enumerate() {
        if entry.surface.is_empty() {
            return Err(ImportError::parse("pack", index + 1, "surface 不可為空"));
        }
        if entry.reading.is_empty() {
            return Err(ImportError::parse("pack", index + 1, "reading 不可為空"));
        }
        if entry.cost < MIN_COST {
            return Err(ImportError::parse(
                "pack",
                index + 1,
                format!("詞成本 {} 低於 {MIN_COST}", entry.cost),
            ));
        }
        if entry.pos_id as usize >= data.pos.len() {
            return Err(ImportError::parse(
                "pack",
                index + 1,
                format!("POS id {} 超出 POS table", entry.pos_id),
            ));
        }
        if entry.conj_type_id as usize >= data.conj_types.len() {
            return Err(ImportError::parse(
                "pack",
                index + 1,
                format!("活用型 id {} 超出 table", entry.conj_type_id),
            ));
        }
        if entry.conj_form_id as usize >= data.conj_forms.len() {
            return Err(ImportError::parse(
                "pack",
                index + 1,
                format!("活用形 id {} 超出 table", entry.conj_form_id),
            ));
        }
        if entry.left_id >= data.context_count || entry.right_id >= data.context_count {
            return Err(ImportError::parse(
                "pack",
                index + 1,
                format!(
                    "context id ({}, {}) 超出 0..{}",
                    entry.left_id, entry.right_id, data.context_count
                ),
            ));
        }

        let surface_offset = surface_blob.len();
        if previous_surface != Some(entry.surface.as_str()) {
            if let Some(previous) = previous_surface {
                // entries 依 surface 排序；遞減代表呼叫端沒有排好。
                if previous > entry.surface.as_str() {
                    return Err(ImportError::parse(
                        "pack",
                        index + 1,
                        format!(
                            "surface 未依 code point 排序：{previous:?} 之後是 {:?}",
                            entry.surface
                        ),
                    ));
                }
            }
            current_surface_offset =
                u32::try_from(surface_offset).map_err(|_| ImportError::OffsetOverflow {
                    section: "surface blob",
                    offset: surface_offset,
                })?;
            surface_index.push((
                current_surface_offset,
                u32::try_from(index).map_err(|_| ImportError::CountOverflow {
                    field: "entry_count",
                    value: index,
                    limit: u32::MAX as u64,
                })?,
            ));
            surface_blob.extend_from_slice(entry.surface.as_bytes());
            previous_surface = Some(entry.surface.as_str());
        }
        let surface_len = u16::try_from(entry.surface.len()).map_err(|_| {
            ImportError::parse(
                "pack",
                index + 1,
                format!("surface 長度 {} 超過 u16", entry.surface.len()),
            )
        })?;
        let surface_offset = current_surface_offset;

        let reading_offset = reading_blob.len();
        reading_blob.extend_from_slice(entry.reading.as_bytes());
        let reading_len = u16::try_from(entry.reading.len()).map_err(|_| {
            ImportError::parse(
                "pack",
                index + 1,
                format!("reading 長度 {} 超過 u16", entry.reading.len()),
            )
        })?;
        let reading_offset =
            u32::try_from(reading_offset).map_err(|_| ImportError::OffsetOverflow {
                section: "reading blob",
                offset: reading_offset,
            })?;

        let mut raw = [0u8; ENTRY_SIZE];
        raw[0..4].copy_from_slice(&surface_offset.to_le_bytes());
        raw[4..6].copy_from_slice(&surface_len.to_le_bytes());
        raw[6..10].copy_from_slice(&reading_offset.to_le_bytes());
        raw[10..12].copy_from_slice(&reading_len.to_le_bytes());
        raw[12..14].copy_from_slice(&entry.pos_id.to_le_bytes());
        raw[14] = entry.conj_type_id;
        raw[15] = entry.conj_form_id;
        raw[16..18].copy_from_slice(&entry.cost.to_le_bytes());
        raw[18..20].copy_from_slice(&entry.flags.to_le_bytes());
        raw[20..22].copy_from_slice(&entry.left_id.to_le_bytes());
        raw[22..24].copy_from_slice(&entry.right_id.to_le_bytes());
        raw_entries.push(raw);
    }

    let pos_table = encode_pos_table(&data.pos);
    let conj_table = encode_conj_table(&data.conj_types, &data.conj_forms);

    let mut payload = Vec::new();
    let mut offsets = SectionOffsets::default();
    // header 的 offset 一律從檔案開頭算，所以加上 header 大小。
    offsets.surface_blob = HEADER_SIZE + payload.len();
    payload.extend_from_slice(&surface_blob);
    offsets.surface_blob_len = surface_blob.len();

    offsets.reading_blob = HEADER_SIZE + payload.len();
    payload.extend_from_slice(&reading_blob);
    offsets.reading_blob_len = reading_blob.len();

    offsets.entries = HEADER_SIZE + payload.len();
    for raw in &raw_entries {
        payload.extend_from_slice(raw);
    }
    offsets.entries_len = raw_entries.len() * ENTRY_SIZE;

    offsets.index = HEADER_SIZE + payload.len();
    for (offset, first) in &surface_index {
        payload.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(&first.to_le_bytes());
    }
    offsets.index_len = surface_index.len() * INDEX_ENTRY_SIZE;

    offsets.pos = HEADER_SIZE + payload.len();
    payload.extend_from_slice(&pos_table);
    offsets.pos_len = pos_table.len();

    offsets.conj = HEADER_SIZE + payload.len();
    payload.extend_from_slice(&conj_table);
    offsets.conj_len = conj_table.len();

    offsets.connection = HEADER_SIZE + payload.len();
    for cost in &data.connection {
        payload.extend_from_slice(&cost.to_le_bytes());
    }
    offsets.connection_len = data.connection.len() * 2;

    let header = encode_header(
        data.entries.len() as u32,
        surface_index.len() as u32,
        data.pos.len() as u16,
        data.conj_types.len() as u16,
        data.conj_forms.len() as u16,
        data.context_count,
        &offsets,
    )?;

    let mut bytes = Vec::with_capacity(HEADER_SIZE + payload.len() + TRAILER_SIZE);
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    let payload_sha256: [u8; 32] = Sha256::digest(&bytes).into();
    bytes.extend_from_slice(&payload_sha256);
    debug_assert_eq!(bytes.len(), HEADER_SIZE + payload.len() + TRAILER_SIZE);

    Ok(Encoded {
        bytes,
        payload_sha256,
    })
}

#[derive(Default)]
struct SectionOffsets {
    surface_blob: usize,
    surface_blob_len: usize,
    reading_blob: usize,
    reading_blob_len: usize,
    entries: usize,
    entries_len: usize,
    index: usize,
    index_len: usize,
    pos: usize,
    pos_len: usize,
    conj: usize,
    conj_len: usize,
    connection: usize,
    connection_len: usize,
}

fn encode_header(
    entry_count: u32,
    surface_count: u32,
    pos_count: u16,
    conj_type_count: u16,
    conj_form_count: u16,
    context_count: u16,
    offsets: &SectionOffsets,
) -> Result<[u8; HEADER_SIZE], ImportError> {
    let mut header = [0u8; HEADER_SIZE];
    header[0..8].copy_from_slice(&MAGIC);
    header[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    header[12..16].copy_from_slice(&0u32.to_le_bytes());
    header[16..20].copy_from_slice(&entry_count.to_le_bytes());
    header[20..24].copy_from_slice(&surface_count.to_le_bytes());
    header[24..26].copy_from_slice(&pos_count.to_le_bytes());
    header[26..28].copy_from_slice(&conj_type_count.to_le_bytes());
    header[28..30].copy_from_slice(&conj_form_count.to_le_bytes());
    header[30..32].copy_from_slice(&context_count.to_le_bytes());

    let fields: [(&'static str, usize); 14] = [
        ("surface blob", offsets.surface_blob),
        ("surface blob", offsets.surface_blob_len),
        ("reading blob", offsets.reading_blob),
        ("reading blob", offsets.reading_blob_len),
        ("entries", offsets.entries),
        ("entries", offsets.entries_len),
        ("surface index", offsets.index),
        ("surface index", offsets.index_len),
        ("POS table", offsets.pos),
        ("POS table", offsets.pos_len),
        ("conjugation table", offsets.conj),
        ("conjugation table", offsets.conj_len),
        ("connection", offsets.connection),
        ("connection", offsets.connection_len),
    ];
    for (index, (section, value)) in fields.iter().enumerate() {
        let value = u32::try_from(*value).map_err(|_| ImportError::OffsetOverflow {
            section,
            offset: *value,
        })?;
        let start = 32 + index * 4;
        header[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    // 88..96 是 reserved，保持 0。
    Ok(header)
}

fn encode_pos_table(pos: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(pos.len() as u16).to_le_bytes());
    for value in pos {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn encode_conj_table(types: &[String], forms: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(types.len() as u16).to_le_bytes());
    for value in types {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out.extend_from_slice(&(forms.len() as u16).to_le_bytes());
    for value in forms {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

/// 回傳小寫 hex。
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PackData {
        let entry = |surface: &str, reading: &str, pos_id: u16, cost: i16, left, right| PackEntry {
            surface: surface.to_owned(),
            reading: reading.to_owned(),
            pos_id,
            conj_type_id: 0,
            conj_form_id: 0,
            cost,
            flags: 0,
            left_id: left,
            right_id: right,
        };
        PackData {
            entries: vec![
                entry("あい", "あい", 0, 100, 0, 1),
                entry("あい", "あい", 1, 200, 1, 0),
                entry("恋", "こい", 0, 300, 2, 2),
            ],
            pos: vec!["名詞,一般,*,*".to_owned(), "動詞,自立,*,*".to_owned()],
            conj_types: vec!["*".to_owned()],
            conj_forms: vec!["*".to_owned()],
            connection: vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            context_count: 3,
        }
    }

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    #[test]
    fn header_fields_match_v2_layout() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        assert_eq!(&b[0..8], &MAGIC);
        assert_eq!(u16_at(b, 8), 2);
        assert_eq!(u16_at(b, 10), 96);
        assert_eq!(u32_at(b, 12), 0);
        assert_eq!(u32_at(b, 16), 3);
        assert_eq!(u32_at(b, 20), 2);
        assert_eq!(u16_at(b, 24), 2);
        assert_eq!(u16_at(b, 26), 1);
        assert_eq!(u16_at(b, 28), 1);
        assert_eq!(u16_at(b, 30), 3);
        assert_eq!(u32_at(b, 88), 0);
        assert_eq!(u32_at(b, 92), 0);

        // surface blob：兩個唯一 surface，依 code point 排序。
        let blob_off = u32_at(b, 32) as usize;
        let blob_len = u32_at(b, 36) as usize;
        assert_eq!(blob_off, HEADER_SIZE);
        assert_eq!(&b[blob_off..blob_off + blob_len], "あい恋".as_bytes());
        assert_eq!(u32_at(b, 40), (blob_off + blob_len) as u32);
        assert_eq!(
            u32_at(b, 48),
            (blob_off + blob_len + u32_at(b, 44) as usize) as u32
        );
    }

    #[test]
    fn sections_are_contiguous_and_lengths_match_counts() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        let sections = [
            (32usize, 36usize),
            (40, 44),
            (48, 52),
            (56, 60),
            (64, 68),
            (72, 76),
            (80, 84),
        ];
        let mut expected_start = HEADER_SIZE;
        for (off_field, len_field) in sections {
            assert_eq!(u32_at(b, off_field) as usize, expected_start);
            expected_start += u32_at(b, len_field) as usize;
        }
        assert_eq!(expected_start + TRAILER_SIZE, b.len());

        assert_eq!(u32_at(b, 52) as usize, 3 * ENTRY_SIZE);
        assert_eq!(u32_at(b, 60) as usize, 2 * INDEX_ENTRY_SIZE);
        assert_eq!(u32_at(b, 84) as usize, 3 * 3 * 2);
    }

    #[test]
    fn entries_share_surface_offsets_and_store_context_ids() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        let entries_off = u32_at(b, 48) as usize;
        let first = entries_off;
        let second = entries_off + ENTRY_SIZE;

        assert_eq!(u32_at(b, first), 0);
        assert_eq!(u16_at(b, first + 4), "あい".len() as u16);
        assert_eq!(u32_at(b, second), 0);
        assert_eq!(u16_at(b, second + 12), 1);
        assert_eq!(u16_at(b, first + 20), 0);
        assert_eq!(u16_at(b, first + 22), 1);
        assert_eq!(u16_at(b, second + 20), 1);
        assert_eq!(u16_at(b, second + 22), 0);

        // 第三筆的 surface offset 指向 blob 內的「恋」。
        let third = entries_off + 2 * ENTRY_SIZE;
        assert_eq!(u32_at(b, third), "あい".len() as u32);
        assert_eq!(u16_at(b, third + 20), 2);
        assert_eq!(u16_at(b, third + 22), 2);
    }

    #[test]
    fn surface_index_points_at_unique_surfaces() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        let index_off = u32_at(b, 56) as usize;
        assert_eq!(u32_at(b, index_off), 0);
        assert_eq!(u32_at(b, index_off + 4), 0);
        assert_eq!(u32_at(b, index_off + INDEX_ENTRY_SIZE), "あい".len() as u32);
        assert_eq!(u32_at(b, index_off + INDEX_ENTRY_SIZE + 4), 2);
    }

    #[test]
    fn connection_matrix_is_row_major_i16() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        let off = u32_at(b, 80) as usize;
        let value = |left: usize, right: usize| -> i16 {
            let at = off + (left * 3 + right) * 2;
            i16::from_le_bytes([b[at], b[at + 1]])
        };
        assert_eq!(value(0, 0), 0);
        assert_eq!(value(1, 2), 5);
        assert_eq!(value(2, 1), 7);
    }

    #[test]
    fn trailer_hashes_payload() {
        let encoded = encode(&fixture()).expect("編碼");
        let b = &encoded.bytes;
        let end = b.len() - TRAILER_SIZE;
        let digest: [u8; 32] = Sha256::digest(&b[..end]).into();
        assert_eq!(&b[end..], &digest);
        assert_eq!(encoded.payload_sha256, digest);
    }

    #[test]
    fn unsorted_surface_is_rejected() {
        let mut data = fixture();
        data.entries.swap(1, 2);
        let err = encode(&data).expect_err("排序錯誤應失敗");
        assert!(
            err.to_string().contains("surface 未依 code point 排序"),
            "{err}"
        );
    }
}
