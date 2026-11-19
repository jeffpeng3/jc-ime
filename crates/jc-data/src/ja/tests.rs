//! `ja` 模組的測試。pack builder 照 `pack-format.md` 的版面寫，
//! 讓讀取端與格式文件互相驗證。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::pack::{
    hex, is_hiragana, sha256, Entry, Pack, Split, FORMAT_VERSION, HEADER_SIZE, MAGIC, TRAILER_SIZE,
};
use super::{compare_entries, sort_surface_entries, Manifest, PackError};
use jc_testdata::require_data;

// ---------------------------------------------------------------------------
// 測試用 pack builder。
// ---------------------------------------------------------------------------

struct TestEntry {
    surface: &'static str,
    reading: &'static str,
    pos: &'static str,
    conj_type: &'static str,
    conj_form: &'static str,
    cost: i16,
    flags: u16,
}

struct EntryRec {
    surf_off: u32,
    surf_len: u16,
    read_off: u32,
    read_len: u16,
    pos_id: u16,
    type_id: u8,
    form_id: u8,
    cost: i16,
    flags: u16,
}

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_table_string(buf: &mut Vec<u8>, s: &str) {
    put_u16(buf, s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
}

fn table_bytes(strings: &[&str]) -> Vec<u8> {
    let mut buf = Vec::new();
    put_u16(&mut buf, strings.len() as u16);
    for s in strings {
        put_table_string(&mut buf, s);
    }
    buf
}

fn encode_test_pack(
    entries: &[TestEntry],
    pos: &[&str],
    types: &[&str],
    forms: &[&str],
) -> Vec<u8> {
    assert!(!entries.is_empty(), "測試 pack 至少需要一筆詞條");

    let mut pos: Vec<String> = if pos.is_empty() {
        vec!["名詞,普通名詞,一般,*".to_owned()]
    } else {
        pos.iter().map(|s| (*s).to_owned()).collect()
    };
    pos.sort();
    let mut types: Vec<String> = if types.is_empty() {
        vec!["*".to_owned()]
    } else {
        types.iter().map(|s| (*s).to_owned()).collect()
    };
    types.sort();
    let mut forms: Vec<String> = if forms.is_empty() {
        vec!["*".to_owned()]
    } else {
        forms.iter().map(|s| (*s).to_owned()).collect()
    };
    forms.sort();

    let pos_id: HashMap<&str, usize> = pos
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let type_id: HashMap<&str, usize> = types
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let form_id: HashMap<&str, usize> = forms
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    let mut sorted: Vec<&TestEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.surface.cmp(b.surface));

    let mut surface_blob = Vec::new();
    let mut surface_off: HashMap<&str, u32> = HashMap::new();
    let mut reading_blob = Vec::new();
    let mut recs = Vec::with_capacity(sorted.len());
    for e in &sorted {
        let off = *surface_off.entry(e.surface).or_insert_with(|| {
            let off = surface_blob.len() as u32;
            surface_blob.extend_from_slice(e.surface.as_bytes());
            off
        });
        let read_off = reading_blob.len() as u32;
        reading_blob.extend_from_slice(e.reading.as_bytes());
        let pos_idx = *pos_id
            .get(e.pos)
            .unwrap_or_else(|| panic!("測試詞條 {:?} 的 POS {:?} 不在 table", e.surface, e.pos));
        let type_idx = *type_id.get(e.conj_type).unwrap_or_else(|| {
            panic!(
                "測試詞條 {:?} 的活用型 {:?} 不在 table",
                e.surface, e.conj_type
            )
        });
        let form_idx = *form_id.get(e.conj_form).unwrap_or_else(|| {
            panic!(
                "測試詞條 {:?} 的活用形 {:?} 不在 table",
                e.surface, e.conj_form
            )
        });
        recs.push(EntryRec {
            surf_off: off,
            surf_len: e.surface.len() as u16,
            read_off,
            read_len: e.reading.len() as u16,
            pos_id: pos_idx as u16,
            type_id: type_idx as u8,
            form_id: form_idx as u8,
            cost: e.cost,
            flags: e.flags,
        });
    }

    let mut index = Vec::new();
    let mut seen: HashMap<&str, bool> = HashMap::new();
    for (i, e) in sorted.iter().enumerate() {
        if seen.insert(e.surface, true).is_some() {
            continue;
        }
        put_u32(&mut index, surface_off[e.surface]);
        put_u32(&mut index, i as u32);
    }

    let pos_table = {
        let refs: Vec<&str> = pos.iter().map(String::as_str).collect();
        table_bytes(&refs)
    };
    let conj_table = {
        let refs: Vec<&str> = types.iter().map(String::as_str).collect();
        let mut buf = table_bytes(&refs);
        let refs: Vec<&str> = forms.iter().map(String::as_str).collect();
        buf.extend_from_slice(&table_bytes(&refs));
        buf
    };

    let mut entry_blob = Vec::new();
    for r in &recs {
        put_u32(&mut entry_blob, r.surf_off);
        put_u16(&mut entry_blob, r.surf_len);
        put_u32(&mut entry_blob, r.read_off);
        put_u16(&mut entry_blob, r.read_len);
        put_u16(&mut entry_blob, r.pos_id);
        entry_blob.push(r.type_id);
        entry_blob.push(r.form_id);
        entry_blob.extend_from_slice(&r.cost.to_le_bytes());
        put_u16(&mut entry_blob, r.flags);
    }

    let mut offset = HEADER_SIZE;
    let mut header = [0u8; HEADER_SIZE];
    header[..MAGIC.len()].copy_from_slice(&MAGIC);
    header[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    header[16..20].copy_from_slice(&(recs.len() as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(seen.len() as u32).to_le_bytes());
    header[24..26].copy_from_slice(&(pos.len() as u16).to_le_bytes());
    header[26..28].copy_from_slice(&(types.len() as u16).to_le_bytes());
    header[28..30].copy_from_slice(&(forms.len() as u16).to_le_bytes());

    let mut set_section = |off_field: usize, len_field: usize, len: usize| {
        header[off_field..off_field + 4].copy_from_slice(&(offset as u32).to_le_bytes());
        header[len_field..len_field + 4].copy_from_slice(&(len as u32).to_le_bytes());
        offset += len;
    };
    set_section(32, 36, surface_blob.len());
    set_section(40, 44, reading_blob.len());
    set_section(48, 52, entry_blob.len());
    set_section(56, 60, index.len());
    set_section(64, 68, pos_table.len());
    set_section(72, 76, conj_table.len());

    let mut out = Vec::new();
    out.extend_from_slice(&header);
    out.extend_from_slice(&surface_blob);
    out.extend_from_slice(&reading_blob);
    out.extend_from_slice(&entry_blob);
    out.extend_from_slice(&index);
    out.extend_from_slice(&pos_table);
    out.extend_from_slice(&conj_table);
    let sum = sha256(&out);
    out.extend_from_slice(&sum);
    out
}

fn sample_pack() -> Vec<u8> {
    encode_test_pack(
        &[
            TestEntry {
                surface: "明日",
                reading: "あした",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 1000,
                flags: 0,
            },
            TestEntry {
                surface: "明日",
                reading: "あす",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 2500,
                flags: 0,
            },
            TestEntry {
                surface: "味",
                reading: "あじ",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 1200,
                flags: 0,
            },
            TestEntry {
                surface: "亜",
                reading: "あ",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 3000,
                flags: 1,
            },
            TestEntry {
                surface: "食べる",
                reading: "たべる",
                pos: "動詞,一般,*,*",
                conj_type: "五段",
                conj_form: "終止形-一般",
                cost: 800,
                flags: 0,
            },
        ],
        &["名詞,普通名詞,一般,*", "動詞,一般,*,*"],
        &["*", "五段"],
        &["*", "終止形-一般"],
    )
}

// ---------------------------------------------------------------------------
// 暫存檔輔助。
// ---------------------------------------------------------------------------

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_file(tag: &str, bytes: &[u8]) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("jc-data-ja-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建立暫存資料夾");
    let path = dir.join("test.pack");
    std::fs::write(&path, bytes).expect("寫入暫存 pack");
    path
}

fn open_bytes(tag: &str, bytes: &[u8]) -> Result<Pack, PackError> {
    Pack::open(temp_file(tag, bytes))
}

fn open_error(tag: &str, bytes: &[u8]) -> PackError {
    match open_bytes(tag, bytes) {
        Ok(_) => panic!("{tag}：預期解析失敗"),
        Err(err) => err,
    }
}

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn put_u16_at(mut data: Vec<u8>, off: usize, v: u16) -> Vec<u8> {
    data[off..off + 2].copy_from_slice(&v.to_le_bytes());
    data
}

fn put_u32_at(mut data: Vec<u8>, off: usize, v: u32) -> Vec<u8> {
    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
    data
}

fn toggle(mut data: Vec<u8>, off: usize) -> Vec<u8> {
    data[off] ^= 0xFF;
    data
}

fn mutate(mut data: Vec<u8>, off: usize, byte: u8) -> Vec<u8> {
    data[off] = byte;
    data
}

fn reseal(mut data: Vec<u8>) -> Vec<u8> {
    let payload_end = data.len() - TRAILER_SIZE;
    let sum = sha256(&data[..payload_end]);
    data[payload_end..].copy_from_slice(&sum);
    data
}

// ---------------------------------------------------------------------------
// pack 測試（來源：internal/jadic/pack_test.go）。
// ---------------------------------------------------------------------------

#[test]
fn test_parse_sample_pack() {
    let data = sample_pack();
    let pack = open_bytes("sample", &data).expect("解析 sample pack");

    assert_eq!(pack.entry_count(), 5);
    assert_eq!(pack.surface_count(), 4);
    let header = pack.header();
    assert_eq!(header.format_version, FORMAT_VERSION);
    assert_eq!(header.header_size, HEADER_SIZE as u16);
    assert_eq!(header.entry_count, 5);
    assert_eq!(pack.file_sha256(), hex(&sha256(&data)));

    let payload_end = data.len() - TRAILER_SIZE;
    assert_eq!(pack.payload_sha256(), hex(&sha256(&data[..payload_end])));

    let got = pack.lookup("明日");
    assert!(pack.has("明日"));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].reading.as_ref(), "あした");
    assert_eq!(got[1].reading.as_ref(), "あす");
    assert_eq!(pack.pos_str(got[0].pos_id), Some("名詞,普通名詞,一般,*"));
    assert_eq!(pack.conj_type(got[0].conj_type), Some("*"));
    assert_eq!(pack.conj_form(got[0].conj_form), Some("*"));

    let a = &pack.lookup("亜")[0];
    assert_eq!(a.split, Split::A);
    assert_eq!(a.flags, 1);

    assert!(pack.lookup("なし").is_empty());
    assert!(!pack.has("なし"));

    let prefixed: Vec<&Entry> = pack.prefix("明").collect();
    assert_eq!(prefixed.len(), 2);
    assert!(prefixed.iter().all(|e| e.surface.starts_with("明")));

    assert_eq!(pack.pos_list().len(), 2);
    assert_eq!(pack.conj_types().len(), 2);
    assert_eq!(pack.conj_forms().len(), 2);

    let stats = pack.stats();
    assert_eq!(stats.cost_min, 800);
    assert_eq!(stats.cost_max, 3000);
    assert_eq!(stats.max_readings, 2);
    assert_eq!(stats.max_reading_unit, 3);
    assert_eq!(stats.split_counts, [4, 1, 0]);

    assert_eq!(pack.entries().len(), 5);
    pack.verify().expect("驗證 sample pack");
}

#[test]
fn test_capabilities_v1_has_no_context_costs() {
    let pack = open_bytes("capabilities", &sample_pack()).expect("解析 sample pack");
    let caps = pack.capabilities();
    assert!(!caps.context_ids);
    assert!(!caps.connection_matrix);
}

#[test]
fn test_parse_rejects_structural_corruption() {
    let base = sample_pack();

    type Check = fn(&PackError) -> bool;
    let cases: Vec<(&str, Vec<u8>, Check)> = vec![
        ("empty", Vec::new(), |e| {
            matches!(e, PackError::TooSmall { .. })
        }),
        ("short", base[..HEADER_SIZE].to_vec(), |e| {
            matches!(e, PackError::TooSmall { .. })
        }),
        ("magic", mutate(base.clone(), 0, b'X'), |e| {
            matches!(e, PackError::BadMagic)
        }),
        ("version", put_u16_at(base.clone(), 8, 2), |e| {
            matches!(e, PackError::UnsupportedVersion { actual: 2, .. })
        }),
        ("header_size", put_u16_at(base.clone(), 10, 79), |e| {
            matches!(e, PackError::BadHeaderSize { actual: 79, .. })
        }),
        ("reserved", put_u16_at(base.clone(), 30, 1), |e| {
            matches!(e, PackError::NonZeroReserved(1))
        }),
        ("flags", put_u32_at(base.clone(), 12, 1), |e| {
            matches!(e, PackError::UnsupportedFlags(1))
        }),
        (
            "entry_count",
            reseal(put_u32_at(base.clone(), 16, 4)),
            |e| {
                matches!(
                    e,
                    PackError::SectionLengthMismatch {
                        section: "entries",
                        ..
                    }
                )
            },
        ),
        (
            "surface_count",
            reseal(put_u32_at(base.clone(), 20, 3)),
            |e| {
                matches!(
                    e,
                    PackError::SectionLengthMismatch {
                        section: "surface index",
                        ..
                    }
                )
            },
        ),
        (
            "entries_offset",
            reseal(put_u32_at(base.clone(), 48, 0)),
            |e| {
                matches!(
                    e,
                    PackError::SectionInsideHeader {
                        section: "entries",
                        start: 0
                    }
                )
            },
        ),
        (
            "entries_overrun",
            reseal(put_u32_at(base.clone(), 52, 0xFF_FFFF)),
            |e| {
                matches!(
                    e,
                    PackError::SectionOutOfBounds {
                        section: "entries",
                        ..
                    }
                )
            },
        ),
        (
            "overlap",
            reseal(put_u32_at(base.clone(), 40, HEADER_SIZE as u32)),
            |e| matches!(e, PackError::SectionOverlap { .. }),
        ),
        ("pos_count", reseal(put_u16_at(base.clone(), 24, 1)), |e| {
            matches!(
                e,
                PackError::TableCountMismatch {
                    table: "POS table",
                    actual: 2,
                    expected: 1
                }
            )
        }),
        (
            "conj_type_count",
            reseal(put_u16_at(base.clone(), 26, 1)),
            |e| {
                matches!(
                    e,
                    PackError::TableCountMismatch {
                        table: "活用型",
                        actual: 2,
                        expected: 1
                    }
                )
            },
        ),
        (
            "conj_form_count",
            reseal(put_u16_at(base.clone(), 28, 1)),
            |e| {
                matches!(
                    e,
                    PackError::TableCountMismatch {
                        table: "活用形",
                        actual: 2,
                        expected: 1
                    }
                )
            },
        ),
        ("payload_byte", toggle(base.clone(), HEADER_SIZE + 3), |e| {
            matches!(e, PackError::PayloadHashMismatch { .. })
        }),
        ("trailer_byte", toggle(base.clone(), base.len() - 1), |e| {
            matches!(e, PackError::PayloadHashMismatch { .. })
        }),
    ];

    for (name, data, check) in cases {
        let err = open_error(name, &data);
        assert!(check(&err), "{name}：得到 {err:?}");
    }
}

#[test]
fn test_parse_rejects_entry_corruption() {
    let base = sample_pack();
    let entry_off = read_u32(&base, 48) as usize;
    let pos_count = read_u16(&base, 24);
    let conj_type_count = read_u16(&base, 26);
    let conj_form_count = read_u16(&base, 28);
    let reading_len = read_u32(&base, 44);
    let surface_len = read_u32(&base, 36);

    type Edit = fn(Vec<u8>, usize) -> Vec<u8>;
    type Check = fn(&PackError) -> bool;
    let cases: Vec<(&str, Edit, Check)> = vec![
        (
            "negative_cost_min",
            |mut b, off| {
                b[off + 16..off + 18].copy_from_slice(&0x8000u16.to_le_bytes());
                b
            },
            |e| {
                matches!(
                    e,
                    PackError::CostOutOfRange {
                        cost: -32768,
                        minimum: -32767,
                        ..
                    }
                )
            },
        ),
        (
            "pos_id",
            |mut b, off| {
                b[off + 12..off + 14].copy_from_slice(&4u16.to_le_bytes());
                b
            },
            |e| matches!(e, PackError::PosIdOutOfRange { id: 4, .. }),
        ),
        (
            "conj_type",
            |mut b, off| {
                b[off + 14] = 4;
                b
            },
            |e| matches!(e, PackError::ConjTypeIdOutOfRange { id: 4, .. }),
        ),
        (
            "conj_form",
            |mut b, off| {
                b[off + 15] = 4;
                b
            },
            |e| matches!(e, PackError::ConjFormIdOutOfRange { id: 4, .. }),
        ),
        (
            "reading_overrun",
            |mut b, off| {
                b[off + 6..off + 10].copy_from_slice(&u32::MAX.to_le_bytes());
                b
            },
            |e| matches!(e, PackError::ReadingOutOfBounds { .. }),
        ),
        (
            "surface_overrun",
            |mut b, off| {
                b[off..off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                b
            },
            |e| matches!(e, PackError::SurfaceOutOfBounds { .. }),
        ),
        (
            "split",
            |mut b, off| {
                b[off + 18..off + 20].copy_from_slice(&3u16.to_le_bytes());
                b
            },
            |e| matches!(e, PackError::InvalidSplit { split: 3, .. }),
        ),
        (
            "unknown_flags",
            |mut b, off| {
                b[off + 18..off + 20].copy_from_slice(&4u16.to_le_bytes());
                b
            },
            |e| matches!(e, PackError::UndefinedFlags { flags: 4, .. }),
        ),
    ];

    assert_eq!(pos_count, 2);
    assert_eq!(conj_type_count, 2);
    assert_eq!(conj_form_count, 2);
    assert!(reading_len > 0);
    assert!(surface_len > 0);

    for (name, edit, check) in cases {
        let data = reseal(edit(base.clone(), entry_off));
        let err = open_error(name, &data);
        assert!(check(&err), "{name}：得到 {err:?}");
    }

    // POS table 必須依字串嚴格排序；用等長字串才不會動到長度前綴。
    let unsorted = encode_test_pack(
        &[TestEntry {
            surface: "x",
            reading: "あ",
            pos: "aaa",
            conj_type: "*",
            conj_form: "*",
            cost: 1,
            flags: 0,
        }],
        &["aaa", "bbb"],
        &[],
        &[],
    );
    let pos_off = read_u32(&unsorted, 64) as usize;
    let mut broken = unsorted;
    broken[pos_off + 4..pos_off + 7].copy_from_slice(b"zzz");
    let err = open_error("pos_order", &reseal(broken));
    assert!(
        matches!(
            &err,
            PackError::TableEntry {
                table: "POS table",
                reason,
                ..
            } if reason.contains("排序")
        ),
        "POS 排序：得到 {err:?}"
    );
}

#[test]
fn test_parse_rejects_index_corruption() {
    let base = sample_pack();
    let index_off = read_u32(&base, 56) as usize;

    let mut bad = base.clone();
    bad[index_off..index_off + 4].copy_from_slice(&1u32.to_le_bytes());
    let err = open_error("index_first_offset", &reseal(bad));
    assert!(
        matches!(err, PackError::FirstSurfaceOffset(1)),
        "第一筆 offset：得到 {err:?}"
    );

    let mut bad = base.clone();
    bad[index_off + 8 + 4..index_off + 8 + 8].copy_from_slice(&0u32.to_le_bytes());
    let err = open_error("index_first_entry", &reseal(bad));
    assert!(
        matches!(
            err,
            PackError::FirstEntryNotIncreasing {
                index: 2,
                actual: 0,
                ..
            }
        ),
        "first_entry：得到 {err:?}"
    );

    let ascii = encode_test_pack(
        &[
            TestEntry {
                surface: "aa",
                reading: "あ",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 1,
                flags: 0,
            },
            TestEntry {
                surface: "bb",
                reading: "い",
                pos: "名詞,普通名詞,一般,*",
                conj_type: "*",
                conj_form: "*",
                cost: 2,
                flags: 0,
            },
        ],
        &[],
        &[],
        &[],
    );
    let ascii_index = read_u32(&ascii, 56) as usize;
    let mut bad = ascii;
    bad[ascii_index + 8..ascii_index + 12].copy_from_slice(&1u32.to_le_bytes());
    let err = open_error("index_points_elsewhere", &reseal(bad));
    assert!(
        matches!(err, PackError::SurfaceIndexPointsElsewhere { index: 2, .. }),
        "index 與詞條不一致：得到 {err:?}"
    );
}

#[test]
fn test_parse_rejects_katakana_reading() {
    let data = encode_test_pack(
        &[TestEntry {
            surface: "アニメ",
            reading: "アニメ",
            pos: "名詞,普通名詞,一般,*",
            conj_type: "*",
            conj_form: "*",
            cost: 1,
            flags: 0,
        }],
        &[],
        &[],
        &[],
    );
    let err = open_error("katakana", &data);
    assert!(
        matches!(err, PackError::KatakanaReading { ch: 'ア', .. }),
        "片假名讀音：得到 {err:?}"
    );
}

#[test]
fn test_parse_rejects_mixed_reading() {
    let data = encode_test_pack(
        &[TestEntry {
            surface: "あ似",
            reading: "あにメ",
            pos: "名詞,普通名詞,一般,*",
            conj_type: "*",
            conj_form: "*",
            cost: 1,
            flags: 0,
        }],
        &[],
        &[],
        &[],
    );
    let err = open_error("mixed", &data);
    assert!(
        matches!(err, PackError::KatakanaReading { ch: 'メ', .. }),
        "混用讀音：得到 {err:?}"
    );
}

#[test]
fn test_parse_rejects_non_kana_reading() {
    let data = encode_test_pack(
        &[TestEntry {
            surface: "abc",
            reading: "あb",
            pos: "名詞,普通名詞,一般,*",
            conj_type: "*",
            conj_form: "*",
            cost: 1,
            flags: 0,
        }],
        &[],
        &[],
        &[],
    );
    let err = open_error("non_kana", &data);
    assert!(
        matches!(err, PackError::NonKanaReading { ch: 'b', .. }),
        "非かな讀音：得到 {err:?}"
    );
}

#[test]
fn test_parse_accepts_prolonged_sound_mark() {
    let data = encode_test_pack(
        &[TestEntry {
            surface: "アート",
            reading: "あーと",
            pos: "名詞,普通名詞,一般,*",
            conj_type: "*",
            conj_form: "*",
            cost: 1,
            flags: 0,
        }],
        &[],
        &[],
        &[],
    );
    let pack = open_bytes("prolonged", &data).expect("長音符應被接受");
    assert_eq!(pack.lookup("アート")[0].reading.as_ref(), "あーと");
}

// ---------------------------------------------------------------------------
// 排序測試（來源：internal/jadic/order_test.go）。
// ---------------------------------------------------------------------------

fn entry(cost: i16, reading: &str, pos_id: u16, conj_type: u8, conj_form: u8) -> Entry {
    Entry {
        surface: "同".into(),
        reading: reading.into(),
        pos_id,
        conj_type,
        conj_form,
        cost,
        split: Split::C,
        flags: 0,
    }
}

#[test]
fn test_compare_entries() {
    let base = entry(100, "あ", 0, 0, 0);
    assert_eq!(compare_entries(&base, &base), std::cmp::Ordering::Equal);

    let cases: [(&str, Entry, Entry); 5] = [
        ("成本", entry(100, "あ", 0, 0, 0), entry(101, "あ", 0, 0, 0)),
        ("讀音", entry(100, "あ", 0, 0, 0), entry(100, "い", 0, 0, 0)),
        ("詞性", entry(100, "あ", 1, 0, 0), entry(100, "あ", 2, 0, 0)),
        (
            "活用型",
            entry(100, "あ", 0, 1, 0),
            entry(100, "あ", 0, 2, 0),
        ),
        (
            "活用形",
            entry(100, "あ", 0, 0, 1),
            entry(100, "あ", 0, 0, 2),
        ),
    ];
    for (name, a, b) in cases {
        assert!(
            compare_entries(&a, &b) == std::cmp::Ordering::Less,
            "{name}：預期負值"
        );
        assert!(
            compare_entries(&b, &a) == std::cmp::Ordering::Greater,
            "{name}：反向預期正值"
        );
    }
}

#[test]
fn test_sort_surface_entries_stable() {
    let mut entries = vec![
        Entry {
            cost: 500,
            reading: "あ".into(),
            pos_id: 0,
            conj_type: 0,
            conj_form: 2,
            surface: "同".into(),
            split: Split::C,
            flags: 1,
        },
        Entry {
            cost: 500,
            reading: "あ".into(),
            pos_id: 0,
            conj_type: 0,
            conj_form: 0,
            surface: "同".into(),
            split: Split::A,
            flags: 2,
        },
        Entry {
            cost: 500,
            reading: "あ".into(),
            pos_id: 0,
            conj_type: 0,
            conj_form: 0,
            surface: "同".into(),
            split: Split::B,
            flags: 3,
        },
        Entry {
            cost: 400,
            reading: "え".into(),
            pos_id: 0,
            conj_type: 0,
            conj_form: 0,
            surface: "同".into(),
            split: Split::C,
            flags: 4,
        },
    ];
    sort_surface_entries(&mut entries);
    let got: Vec<u16> = entries.iter().map(|e| e.flags).collect();
    assert_eq!(got, vec![4, 2, 3, 1]);
}

// ---------------------------------------------------------------------------
// manifest 測試（來源：internal/jadic/manifest_test.go）。
// ---------------------------------------------------------------------------

fn sample_manifest(pack: &Pack) -> Manifest {
    Manifest {
        schema: super::MANIFEST_SCHEMA.to_owned(),
        lexicon: "public-core".to_owned(),
        target: "public-core-pack".to_owned(),
        pack_format: super::PackFormat {
            name: "0c-jadic".to_owned(),
            version: FORMAT_VERSION as i64,
        },
        reading_script: "hiragana".to_owned(),
        counts: super::ManifestCounts {
            entries: pack.entry_count() as i64,
            surfaces: pack.surface_count() as i64,
            pos: pack.pos_list().len() as i64,
            conj_types: pack.conj_types().len() as i64,
            conj_forms: pack.conj_forms().len() as i64,
        },
        outputs: super::ManifestOutputs {
            pack: super::ManifestPack {
                path: "ja-lexicon-public-core.pack".to_owned(),
                bytes: pack.bytes() as i64,
                sha256: pack.file_sha256(),
                payload_sha256: pack.payload_sha256(),
            },
        },
        allowed_sources: Vec::new(),
        file_sha256: "deadbeef".to_owned(),
    }
}

#[test]
fn test_manifest_verify() {
    let pack = open_bytes("manifest_ok", &sample_pack()).expect("解析 sample pack");
    sample_manifest(&pack)
        .verify(&pack)
        .expect("manifest 應配對");

    let mut manifest = sample_manifest(&pack);
    manifest.reading_script = "katakana".to_owned();
    manifest.verify(&pack).expect("katakana 應接受");
}

#[test]
fn test_manifest_verify_rejects_mismatch() {
    let pack = open_bytes("manifest_bad", &sample_pack()).expect("解析 sample pack");

    type Edit = fn(&mut Manifest);
    type Check = fn(&PackError) -> bool;
    let cases: Vec<(&str, Edit, Check)> = vec![
        (
            "schema",
            |m| m.schema = "0c-lexicon-manifest/2".into(),
            |e| matches!(e, PackError::ManifestSchema { .. }),
        ),
        (
            "format_name",
            |m| m.pack_format.name = "0c-jadic-next".into(),
            |e| matches!(e, PackError::ManifestFormatName { .. }),
        ),
        (
            "format_version",
            |m| m.pack_format.version = 2,
            |e| matches!(e, PackError::ManifestFormatVersion { actual: 2, .. }),
        ),
        (
            "reading_script",
            |m| m.reading_script = "romaji".into(),
            |e| matches!(e, PackError::ManifestReadingScript { .. }),
        ),
        (
            "entries",
            |m| m.counts.entries += 1,
            |e| {
                matches!(
                    e,
                    PackError::ManifestCount {
                        field: "counts.entries",
                        ..
                    }
                )
            },
        ),
        (
            "surfaces",
            |m| m.counts.surfaces += 1,
            |e| {
                matches!(
                    e,
                    PackError::ManifestCount {
                        field: "counts.surfaces",
                        ..
                    }
                )
            },
        ),
        (
            "pos",
            |m| m.counts.pos += 1,
            |e| {
                matches!(
                    e,
                    PackError::ManifestCount {
                        field: "counts.pos",
                        ..
                    }
                )
            },
        ),
        (
            "conj_types",
            |m| m.counts.conj_types += 1,
            |e| {
                matches!(
                    e,
                    PackError::ManifestCount {
                        field: "counts.conj_types",
                        ..
                    }
                )
            },
        ),
        (
            "conj_forms",
            |m| m.counts.conj_forms += 1,
            |e| {
                matches!(
                    e,
                    PackError::ManifestCount {
                        field: "counts.conj_forms",
                        ..
                    }
                )
            },
        ),
        (
            "bytes",
            |m| m.outputs.pack.bytes += 1,
            |e| matches!(e, PackError::ManifestBytes { .. }),
        ),
        (
            "file_sha",
            |m| m.outputs.pack.sha256 = "0".repeat(64),
            |e| matches!(e, PackError::ManifestFileHash { .. }),
        ),
        (
            "payload_sha",
            |m| m.outputs.pack.payload_sha256 = "0".repeat(64),
            |e| matches!(e, PackError::ManifestPayloadHash { .. }),
        ),
    ];

    for (name, edit, check) in cases {
        let mut manifest = sample_manifest(&pack);
        edit(&mut manifest);
        let err = manifest.verify(&pack).expect_err("預期驗證失敗");
        assert!(check(&err), "{name}：得到 {err:?}");
    }
}

#[test]
fn test_load_manifest_records_file_hash() {
    let path = temp_file(
        "manifest_load",
        br#"{
          "schema": "0c-lexicon-manifest/1",
          "lexicon": "public-core",
          "target": "public-core-pack",
          "pack_format": {"name": "0c-jadic", "version": 1},
          "reading_script": "hiragana",
          "counts": {"entries": 5, "surfaces": 4, "pos": 2, "conj_types": 2, "conj_forms": 2},
          "outputs": {"pack": {"path": "x", "bytes": 1, "sha256": "aa", "payload_sha256": "bb"}}
        }"#,
    );
    let manifest = Manifest::load(&path).expect("載入 manifest");
    assert_eq!(manifest.file_sha256.len(), 64);
    assert_eq!(manifest.lexicon, "public-core");
    assert_eq!(manifest.counts.entries, 5);
    manifest.verify_file(&path).expect("manifest 雜湊應相符");

    std::fs::write(&path, b"{}").expect("改寫 manifest");
    let err = manifest.verify_file(&path).expect_err("改寫後應不符");
    assert!(
        matches!(err, PackError::ManifestFileHash { .. }),
        "得到 {err:?}"
    );

    let missing = path.with_file_name("missing.json");
    assert!(Manifest::load(&missing).is_err());
}

// ---------------------------------------------------------------------------
// 真實 pack（來源：internal/jadic/real_pack_test.go）。
// ---------------------------------------------------------------------------

#[test]
fn test_reference_pack() {
    let path = require_data!("packs/ja-lexicon-public-core.pack");
    let pack = Pack::open(&path).expect("開啟參考 pack");

    assert_eq!(pack.entry_count(), 1_147_021);
    assert_eq!(pack.surface_count(), 870_298);
    assert_eq!(
        pack.file_sha256(),
        "8e57a18deb64d0fdc8147b53b5e234ecda412fa49feb0e8096addc4349582840"
    );
    assert_eq!(
        pack.payload_sha256(),
        "b52ec1fe4224e1ca68d91e6fb82e699d7a12e8a1a610e4e3e6611a7da15ef946"
    );

    let header = pack.header();
    assert_eq!(header.format_version, 1);
    assert_eq!(header.pos_count, 71);
    assert_eq!(header.conj_type_count, 140);
    assert_eq!(header.conj_form_count, 62);

    let caps = pack.capabilities();
    assert!(!caps.context_ids);
    assert!(!caps.connection_matrix);

    let stats = pack.stats();
    assert_eq!(stats.cost_min, -8313);
    assert_eq!(stats.cost_max, 23263);
    assert_eq!(stats.negative_costs, 114);
    assert!(stats.max_reading_unit <= 64);

    pack.verify().expect("參考 pack 重新驗證");

    let entries = pack.lookup("明日");
    assert!(!entries.is_empty(), "參考 pack 應包含 明日");
    for entry in entries {
        for ch in entry.reading.chars() {
            assert!(
                ch == 'ー' || is_hiragana(ch),
                "讀音 {:?} 含非平假名字元 {:?}",
                entry.reading,
                ch
            );
        }
    }

    let prefixed: Vec<&Entry> = pack.prefix("明").collect();
    assert!(!prefixed.is_empty());
    assert!(prefixed.len() >= entries.len());
    assert!(prefixed.iter().all(|e| e.surface.starts_with("明")));
    for pair in prefixed.windows(2) {
        assert!(
            pair[0].surface <= pair[1].surface,
            "prefix 結果應依 surface 排序"
        );
    }

    let manifest_path = require_data!("packs/ja-lexicon-public-core.manifest.json");
    let manifest = Manifest::load(&manifest_path).expect("載入參考 manifest");
    manifest.verify(&pack).expect("manifest 與 pack 配對");
    manifest
        .verify_file(&manifest_path)
        .expect("manifest 檔案雜湊");
}
