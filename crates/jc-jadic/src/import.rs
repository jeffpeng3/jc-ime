//! 匯入流程：讀 Mozc 來源、去重排序、寫 pack v2 與 manifest。

use std::collections::HashMap;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::ImportError;
use crate::mozc::{self, IdTable, RawRow};
use crate::pack::{self, PackData, PackEntry};
use crate::reading::normalize_reading;

/// 產出的 pack 檔名。
pub const PACK_FILE: &str = "ja-lexicon-mozc-v2.pack";
/// 產出的 manifest 檔名。
pub const MANIFEST_FILE: &str = "ja-lexicon-mozc-v2.manifest.json";
/// Mozc OSS 3.35.6354 的 context 數量。
pub const EXPECTED_CONTEXT_COUNT: u32 = 2672;
/// Mozc OSS 的 commit。
pub const MOZC_COMMIT: &str = "0136c35782536cb43d710575c311cd4185982738";
/// manifest schema。
pub const MANIFEST_SCHEMA: &str = "0c-lexicon-manifest/1";

/// 匯入選項。
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Mozc dictionary_oss 目錄。
    pub mozc_dir: PathBuf,
    /// pack 與 manifest 的輸出目錄。
    pub out_dir: PathBuf,
}

/// 匯入統計。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportStats {
    /// 來源列數。
    pub rows: usize,
    /// 成功轉成 pack 詞條的列數。
    pub read: usize,
    /// 去重後留下的詞條數。
    pub entries: usize,
    /// 唯一 surface 數。
    pub surfaces: usize,
    /// 同 identity 的重複列數。
    pub duplicates_dropped: usize,
    /// 讀音不是かな而跳過的列數。
    pub filtered_reading: usize,
}

/// 匯入結果。
#[derive(Debug, Clone)]
pub struct ImportReport {
    /// pack 檔路徑。
    pub pack_path: PathBuf,
    /// manifest 檔路徑。
    pub manifest_path: PathBuf,
    /// 匯入統計。
    pub stats: ImportStats,
    /// POS table 筆數。
    pub pos_count: usize,
    /// 活用型筆數。
    pub conj_types: usize,
    /// 活用形筆數。
    pub conj_forms: usize,
    /// context 數量。
    pub context_count: u16,
    /// pack 檔案大小。
    pub pack_bytes: u64,
    /// pack 整檔 SHA-256。
    pub pack_sha256: String,
    /// pack payload SHA-256。
    pub payload_sha256: String,
    /// manifest 整檔 SHA-256。
    pub manifest_sha256: String,
}

/// 執行 Mozc 匯入。
pub fn import(options: &ImportOptions) -> Result<ImportReport, ImportError> {
    let mozc_dir = &options.mozc_dir;
    let id_def_path = mozc_dir.join("id.def");
    let connection_path = mozc_dir.join("connection_single_column.txt");

    let id_table = read_id_def(&id_def_path)?;
    if id_table.context_count as u32 != EXPECTED_CONTEXT_COUNT {
        return Err(ImportError::ContextCount {
            actual: id_table.context_count as u32,
        });
    }
    let connection = read_connection(&connection_path, id_table.context_count)?;

    let dictionary_files = mozc::dictionary_files(mozc_dir)?;
    let mut rows: Vec<RawRow> = Vec::new();
    for path in &dictionary_files {
        let file = fs::File::open(path).map_err(|source| ImportError::Io {
            path: path.clone(),
            source,
        })?;
        let reader = BufReader::with_capacity(1 << 20, file);
        mozc::parse_dictionary(reader, path, id_table.context_count, |row| rows.push(row))?;
    }

    let (entries, stats) = collect_entries(&rows, &id_table);
    drop(rows);

    let data = PackData {
        entries,
        pos: id_table.pos.clone(),
        conj_types: id_table.conj_types.clone(),
        conj_forms: id_table.conj_forms.clone(),
        connection,
        context_count: id_table.context_count,
    };
    let encoded = pack::encode(&data)?;

    fs::create_dir_all(&options.out_dir).map_err(|source| ImportError::Write {
        path: options.out_dir.clone(),
        source,
    })?;
    let pack_path = options.out_dir.join(PACK_FILE);
    let manifest_path = options.out_dir.join(MANIFEST_FILE);
    write_atomic(&pack_path, &encoded.bytes)?;

    let pack_sha256 = sha256_hex(&encoded.bytes);
    let payload_sha256 = pack::hex(&encoded.payload_sha256);
    let manifest = build_manifest(
        mozc_dir,
        &id_table,
        &stats,
        encoded.bytes.len() as u64,
        &pack_sha256,
        &payload_sha256,
    )?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest 一定可以序列化");
    manifest_bytes.push(b'\n');
    write_atomic(&manifest_path, &manifest_bytes)?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);

    Ok(ImportReport {
        pack_path,
        manifest_path,
        stats,
        pos_count: id_table.pos.len(),
        conj_types: id_table.conj_types.len(),
        conj_forms: id_table.conj_forms.len(),
        context_count: id_table.context_count,
        pack_bytes: encoded.bytes.len() as u64,
        pack_sha256,
        payload_sha256,
        manifest_sha256,
    })
}

fn read_id_def(path: &Path) -> Result<IdTable, ImportError> {
    let file = fs::File::open(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    mozc::parse_id_def(BufReader::new(file), path)
}

fn read_connection(path: &Path, context_count: u16) -> Result<Vec<i16>, ImportError> {
    let file = fs::File::open(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    mozc::parse_connection(BufReader::with_capacity(1 << 20, file), path, context_count)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Identity {
    surface: String,
    reading: String,
    pos_id: u16,
    conj_type_id: u8,
    conj_form_id: u8,
    left_id: u16,
    right_id: u16,
}

/// 把原始列轉成 pack 詞條：跳過不可用讀音、依 identity 去重、依 surface 排序。
pub fn collect_entries(rows: &[RawRow], table: &IdTable) -> (Vec<PackEntry>, ImportStats) {
    let mut stats = ImportStats {
        rows: rows.len(),
        ..ImportStats::default()
    };
    let mut entries: Vec<PackEntry> = Vec::with_capacity(rows.len());
    let mut seen: HashMap<Identity, usize> = HashMap::with_capacity(rows.len());
    for row in rows {
        let Ok(reading) = normalize_reading(&row.reading) else {
            stats.filtered_reading += 1;
            continue;
        };
        let info = table.ids[row.left_id as usize];
        let identity = Identity {
            surface: row.surface.clone(),
            reading: reading.clone(),
            pos_id: info.pos_id,
            conj_type_id: info.conj_type_id,
            conj_form_id: info.conj_form_id,
            left_id: row.left_id,
            right_id: row.right_id,
        };
        stats.read += 1;
        if let Some(&index) = seen.get(&identity) {
            stats.duplicates_dropped += 1;
            if row.cost < entries[index].cost {
                entries[index].cost = row.cost;
            }
            continue;
        }
        seen.insert(identity, entries.len());
        entries.push(PackEntry {
            surface: row.surface.clone(),
            reading,
            pos_id: info.pos_id,
            conj_type_id: info.conj_type_id,
            conj_form_id: info.conj_form_id,
            cost: row.cost,
            flags: 0,
            left_id: row.left_id,
            right_id: row.right_id,
        });
    }

    entries.sort_by(|a, b| {
        a.surface
            .cmp(&b.surface)
            .then_with(|| a.reading.cmp(&b.reading))
            .then_with(|| a.cost.cmp(&b.cost))
            .then_with(|| a.pos_id.cmp(&b.pos_id))
            .then_with(|| a.conj_type_id.cmp(&b.conj_type_id))
            .then_with(|| a.conj_form_id.cmp(&b.conj_form_id))
            .then_with(|| a.left_id.cmp(&b.left_id))
            .then_with(|| a.right_id.cmp(&b.right_id))
    });

    stats.entries = entries.len();
    stats.surfaces = entries
        .windows(2)
        .filter(|pair| pair[0].surface != pair[1].surface)
        .count()
        + usize::from(!entries.is_empty());
    (entries, stats)
}

#[derive(Debug, Serialize)]
struct Manifest {
    schema: &'static str,
    lexicon: &'static str,
    target: &'static str,
    pack_license: &'static str,
    license_policy: &'static str,
    reading_script: &'static str,
    pack_format: PackFormat,
    counts: Counts,
    allowed_sources: Vec<&'static str>,
    attribution: Vec<&'static str>,
    sources: Vec<Source>,
    outputs: Outputs,
}

#[derive(Debug, Serialize)]
struct PackFormat {
    name: &'static str,
    version: u16,
    entry_size: usize,
    context_count: u16,
}

#[derive(Debug, Serialize)]
struct Counts {
    rows: usize,
    read: usize,
    entries: usize,
    surfaces: usize,
    pos: usize,
    conj_types: usize,
    conj_forms: usize,
    duplicates_dropped: usize,
    filtered_reading: usize,
    context_count: u16,
}

#[derive(Debug, Serialize)]
struct Source {
    id: &'static str,
    name: &'static str,
    repository: &'static str,
    commit: &'static str,
    tag: &'static str,
    license: &'static str,
    license_class: &'static str,
    license_files: Vec<&'static str>,
    path: String,
    files: Vec<SourceFile>,
    entries_used: usize,
}

#[derive(Debug, Serialize)]
struct SourceFile {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct Outputs {
    pack: PackOutput,
}

#[derive(Debug, Serialize)]
struct PackOutput {
    path: String,
    bytes: u64,
    sha256: String,
    payload_sha256: String,
}

fn build_manifest(
    mozc_dir: &Path,
    table: &IdTable,
    stats: &ImportStats,
    pack_bytes: u64,
    pack_sha256: &str,
    payload_sha256: &str,
) -> Result<Manifest, ImportError> {
    let mut files = Vec::new();
    for path in mozc::dictionary_files(mozc_dir)?.into_iter().chain([
        mozc_dir.join("id.def"),
        mozc_dir.join("connection_single_column.txt"),
    ]) {
        files.push(describe_file(&path)?);
    }
    for name in ["LICENSE", "README.txt"] {
        let path = mozc_dir.join(name);
        if path.is_file() {
            files.push(describe_file(&path)?);
        }
    }
    let mozc_path = mozc_dir.to_string_lossy().into_owned();

    Ok(Manifest {
        schema: MANIFEST_SCHEMA,
        lexicon: "mozc",
        target: "mozc-pack",
        pack_license: "permissive-notice",
        license_policy: "mozc-bsd",
        reading_script: "hiragana",
        pack_format: PackFormat {
            name: "0c-jadic",
            version: pack::FORMAT_VERSION,
            entry_size: pack::ENTRY_SIZE,
            context_count: table.context_count,
        },
        counts: Counts {
            rows: stats.rows,
            read: stats.read,
            entries: stats.entries,
            surfaces: stats.surfaces,
            pos: table.pos.len(),
            conj_types: table.conj_types.len(),
            conj_forms: table.conj_forms.len(),
            duplicates_dropped: stats.duplicates_dropped,
            filtered_reading: stats.filtered_reading,
            context_count: table.context_count,
        },
        allowed_sources: vec!["ja-mozc"],
        attribution: vec![
            "Mozc OSS dictionary_oss 3.35.6354 (https://github.com/google/mozc, commit 0136c35782536cb43d710575c311cd4185982738): BSD-3-Clause (Copyright 2010-2018, Google Inc.); ship LICENSE and README.txt with the pack",
        ],
        sources: vec![Source {
            id: "ja-mozc",
            name: "Mozc OSS dictionary_oss 3.35.6354",
            repository: "https://github.com/google/mozc",
            commit: MOZC_COMMIT,
            tag: "3.35.6354",
            license: "BSD-3-Clause",
            license_class: "permissive",
            license_files: vec!["LICENSE", "README.txt"],
            path: mozc_path,
            files,
            entries_used: stats.entries,
        }],
        outputs: Outputs {
            pack: PackOutput {
                path: PACK_FILE.to_owned(),
                bytes: pack_bytes,
                sha256: pack_sha256.to_owned(),
                payload_sha256: payload_sha256.to_owned(),
            },
        },
    })
}

fn describe_file(path: &Path) -> Result<SourceFile, ImportError> {
    let bytes = fs::read(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(SourceFile {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    let mut tmp = path.to_path_buf();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    tmp.set_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, bytes).map_err(|source| ImportError::Write {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| ImportError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// 回傳小寫 hex 的 SHA-256。
pub fn sha256_hex(bytes: &[u8]) -> String {
    pack::hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const ID_DEF: &str = "\
0 BOS/EOS,*,*,*,*,*,*
1 名詞,一般,*,*,*,*,*
2 動詞,自立,*,*,五段・カ行,基本形,*
";

    fn table() -> IdTable {
        mozc::parse_id_def(Cursor::new(ID_DEF), Path::new("id.def")).expect("解析 id.def")
    }

    fn row(reading: &str, surface: &str, left: u16, right: u16, cost: i16) -> RawRow {
        RawRow {
            reading: reading.to_owned(),
            surface: surface.to_owned(),
            left_id: left,
            right_id: right,
            cost,
        }
    }

    #[test]
    fn duplicate_identity_keeps_lowest_cost() {
        let rows = vec![row("アイ", "愛", 1, 1, 100), row("アイ", "愛", 1, 1, 50)];
        let (entries, stats) = collect_entries(&rows, &table());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cost, 50);
        assert_eq!(entries[0].reading, "あい");
        assert_eq!(stats.duplicates_dropped, 1);
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.surfaces, 1);
    }

    #[test]
    fn different_context_ids_stay_separate() {
        let rows = vec![row("アイ", "愛", 1, 1, 100), row("アイ", "愛", 1, 2, 200)];
        let (entries, stats) = collect_entries(&rows, &table());
        assert_eq!(entries.len(), 2);
        assert_eq!(stats.duplicates_dropped, 0);
    }

    #[test]
    fn entries_sort_by_surface_code_point_and_stay_contiguous() {
        let rows = vec![
            row("イ", "い", 1, 1, 10),
            row("ア", "あ", 1, 1, 20),
            row("ア", "あ", 2, 2, 5),
            row("ン", "ん", 1, 1, 30),
        ];
        let (entries, stats) = collect_entries(&rows, &table());
        let surfaces: Vec<&str> = entries.iter().map(|e| e.surface.as_str()).collect();
        assert_eq!(surfaces, vec!["あ", "あ", "い", "ん"]);
        assert_eq!(entries[0].cost, 5);
        assert_eq!(entries[1].cost, 20);
        assert_eq!(stats.surfaces, 3);
    }

    #[test]
    fn non_kana_reading_is_filtered_and_counted() {
        let rows = vec![row("1", "一", 1, 1, 10), row("ア・イ", "愛", 1, 1, 20)];
        let (entries, stats) = collect_entries(&rows, &table());
        assert!(entries.is_empty());
        assert_eq!(stats.filtered_reading, 2);
        assert_eq!(stats.rows, 2);
        assert_eq!(stats.read, 0);
    }

    #[test]
    fn empty_input_yields_zero_surfaces() {
        let (entries, stats) = collect_entries(&[], &table());
        assert!(entries.is_empty());
        assert_eq!(stats.surfaces, 0);
    }
}
