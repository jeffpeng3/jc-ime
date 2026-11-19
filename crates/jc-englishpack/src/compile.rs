//! `esdb.tsv` ＋ `technical.tsv` ＋ `source.json` → 0cEN v1 資料包。
//!
//! 對應前身 `internal/englishpack/compile.go`。行為差異只在資料結構：
//! 前身用 Go map，這裡用 [`std::collections::BTreeMap`]，輸出仍然逐位元組
//! 相同（JSON 的 key 依字母排序，和 `encoding/json` 的 map 排序一致）。

use std::collections::BTreeMap;
use std::path::Path;

use jc_core::mixed::{
    sha256_hex, Dictionary, EnglishManifest, EnglishSource, Word, MAX_WORD_BYTES,
};

use crate::error::BuildError;
use crate::fsutil::read_limited;
use crate::pack::{write, MAX_PACK_WORDS};

/// 單一輸入檔大小上限。
const MAX_SOURCE_BYTES: u64 = 16 << 20;

/// `source.json`：來源、雜湊與匯出清單。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SourceMetadata {
    /// 上游來源與授權。
    pub source: EnglishSource,
    /// 正規化 `esdb.tsv` 的 SHA-256。
    pub normalized_sha256: String,
    /// `LICENSE.txt` 的 SHA-256。
    pub license_sha256: String,
    /// 正規化後的詞數。
    pub words: usize,
    /// 上游各級匯出檔；匯入時用。
    #[serde(default)]
    pub exports: Vec<Export>,
}

/// 一份上游匯出檔。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Export {
    /// 等級（10／20／35／40／50／55／60）。
    pub size: u32,
    /// 檔名，必須是 `<size>.txt`。
    pub file: String,
    /// 檔案 SHA-256。
    pub sha256: String,
}

/// 建置資料包，回傳寫出的 manifest。
///
/// 輸入目錄要有 `source.json`、`esdb.tsv`、`technical.tsv` 與 `LICENSE.txt`；
/// 任何雜湊、等級、排序、權重或授權錯誤都會在寫檔前拒絕。
pub fn compile(source_dir: &Path, out_dir: &Path) -> Result<EnglishManifest, BuildError> {
    let mut inputs: BTreeMap<String, String> = BTreeMap::new();
    let mut files: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    for name in ["source.json", "esdb.tsv", "technical.tsv", "LICENSE.txt"] {
        let data = read_limited(&source_dir.join(name), MAX_SOURCE_BYTES)?;
        inputs.insert(name.to_string(), sha256_hex(&data));
        files.insert(name, data);
    }
    let meta: SourceMetadata = serde_json::from_slice(&files["source.json"])?;
    if inputs["esdb.tsv"] != meta.normalized_sha256 || inputs["LICENSE.txt"] != meta.license_sha256
    {
        return Err(BuildError::Source("來源詞表或授權檔雜湊不符".to_string()));
    }
    let license = &files["LICENSE.txt"];
    if String::from_utf8_lossy(license).trim().is_empty() {
        return Err(BuildError::Source("缺少授權文字".to_string()));
    }

    let esdb = text_of(&files["esdb.tsv"], "esdb.tsv")?;
    let mut words: Vec<Word> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();
    let mut previous = String::new();
    for (line_no, line) in esdb.lines().enumerate() {
        let fields: Vec<&str> = line.split('\t').collect();
        let word = fields.first().copied().unwrap_or_default();
        if fields.len() != 3 || word <= previous.as_str() || !lower_word(word) {
            return Err(BuildError::Source(format!(
                "esdb.tsv 第 {} 行詞條內容或排序不合法",
                line_no + 1
            )));
        }
        let grade: i64 = fields[1]
            .parse()
            .map_err(|_| BuildError::Source("esdb.tsv 等級不是數字".to_string()))?;
        if !valid_size(grade) || !matches!(fields[2], "0" | "1") {
            return Err(BuildError::Source(
                "esdb.tsv 等級或建議標記不合法".to_string(),
            ));
        }
        words.push(Word::new(word, (100 - grade) as f64));
        if fields[2] == "1" {
            blocked.push(word.to_string());
        }
        previous = word.to_string();
    }
    if words.len() != meta.words || words.is_empty() || words.len() > MAX_PACK_WORDS {
        return Err(BuildError::Source(format!(
            "英文來源詞數不符：{} != {}",
            words.len(),
            meta.words
        )));
    }

    let technical = text_of(&files["technical.tsv"], "technical.tsv")?;
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    for (line_no, line) in technical.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 2 || !lower_word(fields[0]) || seen.insert(fields[0], ()).is_some() {
            return Err(BuildError::Source(format!(
                "technical.tsv 第 {} 行詞條不合法或重複",
                line_no + 1
            )));
        }
        let weight: f64 = fields[1]
            .parse()
            .map_err(|_| BuildError::Source("technical.tsv 權重不是數字".to_string()))?;
        let valid_weight = weight.is_finite() && weight > 0.0 && weight <= 1000.0;
        if !valid_weight {
            return Err(BuildError::Source("technical.tsv 權重不合法".to_string()));
        }
        words.push(Word::new(fields[0], weight));
    }
    if words.len() > MAX_PACK_WORDS {
        return Err(BuildError::Source("英文詞數超過上限".to_string()));
    }

    let dict = Dictionary::new_filtered(words, blocked);
    let mut source = meta.source.clone();
    if source.commit.is_none() {
        source.commit = commit_from_version(&source.version);
    }
    let manifest = EnglishManifest {
        format: 1,
        words: dict.len(),
        ranking: "ESDB size prior (100-size), with authored product vocabulary priors; not corpus frequency"
            .to_string(),
        sources: vec![source],
        inputs,
        files: BTreeMap::new(),
    };
    let notice = notice_text(&manifest.sources[0], license);
    write(out_dir, &dict, manifest, notice.as_bytes())
}

/// 從版本字串取出 40 字元的上游 commit；沒有就回傳 `None`。
pub fn commit_from_version(version: &str) -> Option<String> {
    let bytes = version.as_bytes();
    for start in 0..bytes.len() {
        if bytes.len() < start + 40 {
            break;
        }
        let candidate = &version[start..start + 40];
        if candidate
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Some(candidate.to_string());
        }
    }
    None
}

fn notice_text(source: &EnglishSource, license: &[u8]) -> String {
    let mut text = format!(
        "jc-ime 英文詞表\n\n來源：{}\n版本：{}\n網址：{}\n來源 SHA256：{}\n\n\
選用 A/B/Z、size ≤ 60、variant ≤ 1；保留小寫 ASCII 字母及上游 no-suggest 標記。\n\
排序採 size 先驗及本專案自行編列的產品詞表初值，並非語料詞頻。\n\n",
        source.name, source.version, source.url, source.sha256
    );
    text.push_str(&String::from_utf8_lossy(license));
    text
}

fn text_of<'a>(data: &'a [u8], name: &str) -> Result<&'a str, BuildError> {
    std::str::from_utf8(data).map_err(|_| BuildError::Source(format!("{name} 不是 UTF-8")))
}

/// 詞面是否只有小寫 ASCII 字母且長度合法。
pub(crate) fn lower_word(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_WORD_BYTES && text.bytes().all(|b| b.is_ascii_lowercase())
}

/// 上游等級只能是這幾個值。
pub(crate) fn valid_size(n: i64) -> bool {
    matches!(n, 10 | 20 | 35 | 40 | 50 | 55 | 60)
}
