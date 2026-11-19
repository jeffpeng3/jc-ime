//! 上游匯出檔正規化成 `esdb.tsv`。
//!
//! 對應前身 `internal/englishpack/import.go`。只比對固定的 SHA-256，不執行
//! 上游程式也不連網；換來源版本要重新審查匯出、授權與詞形差異。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jc_core::mixed::sha256_hex;

use crate::compile::{lower_word, valid_size, SourceMetadata};
use crate::error::BuildError;
use crate::fsutil::{read_limited, write_atomic};

/// 匯出檔大小上限。
const MAX_EXPORT_BYTES: u64 = 16 << 20;

/// `source.json` 大小上限。
const MAX_METADATA_BYTES: u64 = 64 << 10;

/// 讀入七份上游匯出檔，寫回 `esdb.tsv`。
///
/// 每份檔案都要和 `source.json` 的雜湊相符；正規化後的詞數與 SHA-256 也要
/// 和固定值一致，否則不寫回任何內容。
pub fn import_lists(source_dir: &Path, exports_dir: &Path) -> Result<(), BuildError> {
    let meta_bytes = read_limited(&source_dir.join("source.json"), MAX_METADATA_BYTES)?;
    let meta: SourceMetadata = serde_json::from_slice(&meta_bytes)?;
    if meta.exports.len() != 7 {
        return Err(BuildError::Source("英文匯出清單缺少等級".to_string()));
    }
    let mut grades: BTreeMap<String, i64> = BTreeMap::new();
    let mut blocked: BTreeSet<String> = BTreeSet::new();
    let mut sizes: BTreeSet<u32> = BTreeSet::new();
    for export in &meta.exports {
        let want_file = format!("{}.txt", export.size);
        if !valid_size(i64::from(export.size))
            || !sizes.insert(export.size)
            || export.file != want_file
            || !jc_core::mixed::is_sha256_hex(&export.sha256)
        {
            return Err(BuildError::Source("英文匯出清單不合法".to_string()));
        }
        let data = read_limited(&exports_dir.join(&export.file), MAX_EXPORT_BYTES)?;
        if sha256_hex(&data) != export.sha256 {
            return Err(BuildError::Source(format!(
                "英文匯出來源雜湊不符：{}",
                export.file
            )));
        }
        let text = std::str::from_utf8(&data)
            .map_err(|_| BuildError::Source(format!("{} 不是 UTF-8", export.file)))?;
        for line in text.lines() {
            let word = line.strip_suffix('!').unwrap_or(line);
            if !lower_word(word) {
                continue;
            }
            let grade = i64::from(export.size);
            match grades.get(word) {
                Some(&old) if old <= grade => {}
                _ => {
                    grades.insert(word.to_string(), grade);
                }
            }
            if line != word {
                blocked.insert(word.to_string());
            }
        }
    }
    let mut out = String::new();
    for (word, grade) in &grades {
        let flag = u8::from(blocked.contains(word));
        out.push_str(word);
        out.push('\t');
        out.push_str(&grade.to_string());
        out.push('\t');
        out.push(char::from(b'0' + flag));
        out.push('\n');
    }
    if grades.len() != meta.words || sha256_hex(out.as_bytes()) != meta.normalized_sha256 {
        return Err(BuildError::Source(
            "英文正規化結果與固定來源不符".to_string(),
        ));
    }
    write_atomic(&source_dir.join("esdb.tsv"), out.as_bytes())
}
