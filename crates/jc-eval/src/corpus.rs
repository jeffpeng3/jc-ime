//! 注音 golden、日文歧義詞與模糊輸入題庫的讀取。
//!
//! 這些題庫都共用 `schema: 1` 的格式，loader 會驗證題數、內部一致性與
//! 題庫 pin，錯誤訊息帶檔名與題號。

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{parse_json, read_bytes, EvalError, Result};
use crate::metrics::EvalCase;
use crate::sha256::hex_digest;

/// 這些題庫支援的格式版本。
pub const CORPUS_SCHEMA: u32 = 1;

/// `eval/zhuyin-golden.json`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ZhuyinGolden {
    /// 格式版本。
    pub schema: u32,
    /// 題庫名稱。
    pub name: String,
    /// 授權。
    pub license: String,
    /// 來源。
    pub source: String,
    /// 說明。
    #[serde(default)]
    pub notes: String,
    /// 來源 golden 的 pin。
    #[serde(default)]
    pub source_pin: ZhuyinSourcePin,
    /// 題數。
    pub count: usize,
    /// 題目。
    pub items: Vec<ZhuyinGoldenItem>,
}

/// 注音 golden 的來源 pin。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZhuyinSourcePin {
    /// `cases.json` 雜湊。
    #[serde(default)]
    pub cases_sha256: String,
    /// `expected/` 雜湊。
    #[serde(default)]
    pub expected_sha256: String,
    /// 收錄的案例 id。
    #[serde(default)]
    pub case_ids: Vec<String>,
}

/// 一題注音 golden。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZhuyinGoldenItem {
    /// 題目 id。
    pub id: String,
    /// 來源案例名稱。
    #[serde(default)]
    pub case: String,
    /// 完整讀音，音節以空白分隔。
    pub reading: String,
    /// 參考輸出。
    pub surface: String,
    /// 參考詞切分。
    #[serde(default)]
    pub segments: Vec<ZhuyinSegment>,
}

impl ZhuyinGoldenItem {
    /// 轉成通用評測題目。
    pub fn eval_case(&self) -> EvalCase {
        EvalCase {
            id: self.id.clone(),
            reading: self.reading.clone(),
            reference: self.surface.clone(),
            accept: Vec::new(),
            segments: self.segments.iter().map(|s| s.surface.clone()).collect(),
        }
    }
}

/// 注音 golden 的一個詞。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZhuyinSegment {
    /// 這個詞的讀音。
    pub reading: String,
    /// 這個詞的輸出。
    pub surface: String,
}

/// 讀取 `zhuyin-golden.json`，回傳題庫與檔案雜湊。
pub fn load_zhuyin_golden(path: &Path) -> Result<(ZhuyinGolden, String)> {
    let raw = read_bytes(path)?;
    let corpus: ZhuyinGolden = parse_json(path, &raw)?;
    validate_zhuyin(path, &corpus)?;
    Ok((corpus, hex_digest(&raw)))
}

fn validate_zhuyin(path: &Path, corpus: &ZhuyinGolden) -> Result<()> {
    if corpus.schema != CORPUS_SCHEMA {
        return Err(EvalError::invalid(
            path,
            format!("schema {}，本版支援 {}", corpus.schema, CORPUS_SCHEMA),
        ));
    }
    if corpus.name.is_empty() || corpus.license.is_empty() || corpus.source.is_empty() {
        return Err(EvalError::invalid(path, "name、license、source 都是必填"));
    }
    if corpus.count != corpus.items.len() || corpus.items.is_empty() {
        return Err(EvalError::invalid(
            path,
            format!(
                "count {} 與 {} 筆 items 不符",
                corpus.count,
                corpus.items.len()
            ),
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, item) in corpus.items.iter().enumerate() {
        let where_ = format!("第 {} 題", index + 1);
        if item.id.is_empty() || item.reading.is_empty() || item.surface.is_empty() {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：id、reading、surface 不能是空的"),
            ));
        }
        if !seen.insert(item.id.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("題目 id 重複：{}", item.id),
            ));
        }
        if !is_han_text(&item.surface) {
            return Err(EvalError::invalid(
                path,
                format!("{}：surface 含非漢字", item.id),
            ));
        }
        let mut surfaces = Vec::new();
        let mut readings = Vec::new();
        for segment in &item.segments {
            if segment.surface.is_empty() || segment.reading.is_empty() {
                return Err(EvalError::invalid(
                    path,
                    format!("{}：有空白的 segment", item.id),
                ));
            }
            if !is_han_text(&segment.surface) {
                return Err(EvalError::invalid(
                    path,
                    format!("{}：segment {:?} 含非漢字", item.id, segment.surface),
                ));
            }
            let syllables = segment.reading.split_whitespace().count();
            let characters = segment.surface.chars().count();
            if syllables != characters {
                return Err(EvalError::invalid(
                    path,
                    format!(
                        "{}：segment {:?} 有 {} 個音節、{} 個字",
                        item.id, segment.surface, syllables, characters
                    ),
                ));
            }
            surfaces.push(segment.surface.clone());
            readings.extend(segment.reading.split_whitespace().map(|s| s.to_string()));
        }
        if surfaces.concat() != item.surface {
            return Err(EvalError::invalid(
                path,
                format!("{}：segments 串接後不是 surface", item.id),
            ));
        }
        if !readings.is_empty() && readings.join(" ") != item.reading {
            return Err(EvalError::invalid(
                path,
                format!("{}：segment 讀音串接後不是 item reading", item.id),
            ));
        }
    }
    Ok(())
}

/// `eval/ja-lexicon-ambiguous.json`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JaLexiconAmbiguous {
    /// 格式版本。
    pub schema: u32,
    /// 題庫名稱。
    pub name: String,
    /// 授權。
    pub license: String,
    /// 來源。
    pub source: String,
    /// 說明。
    #[serde(default)]
    pub notes: String,
    /// 取樣種子。
    #[serde(default)]
    pub seed: i64,
    /// 取樣數。
    #[serde(default)]
    pub sample_size: usize,
    /// holdout 數。
    #[serde(default)]
    pub holdout_size: usize,
    /// 每題候選上限。
    #[serde(default)]
    pub candidate_cap: usize,
    /// 讀音最短長度。
    #[serde(default)]
    pub reading_min: usize,
    /// 讀音最長長度。
    #[serde(default)]
    pub reading_max: usize,
    /// 參考答案規則。
    #[serde(default)]
    pub reference_rule: String,
    /// 來源 pack。
    #[serde(default)]
    pub pack: PackInfo,
    /// 題數。
    pub count: usize,
    /// 題目。
    pub items: Vec<JaAmbiguousItem>,
}

/// 來源 pack 的識別。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackInfo {
    /// pack 路徑。
    #[serde(default)]
    pub path: String,
    /// pack 雜湊。
    #[serde(default)]
    pub sha256: String,
    /// manifest 路徑。
    #[serde(default)]
    pub manifest_path: String,
    /// manifest 雜湊。
    #[serde(default)]
    pub manifest_sha256: String,
    /// 詞條數。
    #[serde(default)]
    pub entries: usize,
}

/// 一題日文歧義詞。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JaAmbiguousItem {
    /// 題目 id。
    pub id: String,
    /// 讀音。
    pub reading: String,
    /// 參考詞面，依定義是成本最低的候選。
    pub reference: String,
    /// 詞典候選，成本由小到大。
    #[serde(default)]
    pub candidates: Vec<AmbiguousCandidate>,
    /// 是否屬於 holdout 切片。
    #[serde(default)]
    pub holdout: bool,
}

impl JaAmbiguousItem {
    /// 轉成通用評測題目；參考答案就是 `reference`。
    pub fn eval_case(&self) -> EvalCase {
        EvalCase::new(
            self.id.clone(),
            self.reading.clone(),
            self.reference.clone(),
        )
    }
}

/// 一個詞典候選。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousCandidate {
    /// 詞面。
    pub surface: String,
    /// 詞典成本，越小越優先。
    pub cost: i64,
}

/// 讀取 `ja-lexicon-ambiguous.json`，回傳題庫與檔案雜湊。
pub fn load_ja_lexicon_ambiguous(path: &Path) -> Result<(JaLexiconAmbiguous, String)> {
    let raw = read_bytes(path)?;
    let corpus: JaLexiconAmbiguous = parse_json(path, &raw)?;
    validate_ambiguous(path, &corpus)?;
    Ok((corpus, hex_digest(&raw)))
}

fn validate_ambiguous(path: &Path, corpus: &JaLexiconAmbiguous) -> Result<()> {
    if corpus.schema != CORPUS_SCHEMA {
        return Err(EvalError::invalid(
            path,
            format!("schema {}，本版支援 {}", corpus.schema, CORPUS_SCHEMA),
        ));
    }
    if corpus.name.is_empty() || corpus.license.is_empty() || corpus.source.is_empty() {
        return Err(EvalError::invalid(path, "name、license、source 都是必填"));
    }
    if corpus.pack.sha256.len() != 64 || corpus.pack.path.is_empty() {
        return Err(EvalError::invalid(path, "pack 的 sha256 與 path 都是必填"));
    }
    if corpus.count != corpus.items.len() || corpus.count == 0 || corpus.count != corpus.sample_size
    {
        return Err(EvalError::invalid(
            path,
            format!(
                "count {}、sample_size {} 與 {} 筆 items 不一致",
                corpus.count,
                corpus.sample_size,
                corpus.items.len()
            ),
        ));
    }
    if corpus.holdout_size == 0 || corpus.holdout_size >= corpus.count {
        return Err(EvalError::invalid(
            path,
            format!("holdout_size {} 無效", corpus.holdout_size),
        ));
    }
    if corpus.candidate_cap < 2 {
        return Err(EvalError::invalid(path, "candidate_cap 太小"));
    }
    let mut seen = BTreeSet::new();
    let mut readings = BTreeSet::new();
    let mut holdout = 0usize;
    for (index, item) in corpus.items.iter().enumerate() {
        let where_ = format!("第 {} 題", index + 1);
        if item.id.is_empty() || item.reading.is_empty() || item.reference.is_empty() {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：id、reading、reference 不能是空的"),
            ));
        }
        if !seen.insert(item.id.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("題目 id 重複：{}", item.id),
            ));
        }
        if !readings.insert(item.reading.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("讀音重複：{}", item.reading),
            ));
        }
        let reading_len = item.reading.chars().count();
        if reading_len < corpus.reading_min || reading_len > corpus.reading_max {
            return Err(EvalError::invalid(
                path,
                format!(
                    "{}：讀音長度 {} 不在 [{},{}]",
                    item.id, reading_len, corpus.reading_min, corpus.reading_max
                ),
            ));
        }
        if item.candidates.len() < 2 || item.candidates.len() > corpus.candidate_cap {
            return Err(EvalError::invalid(
                path,
                format!(
                    "{}：{} 個候選，應在 2..{}",
                    item.id,
                    item.candidates.len(),
                    corpus.candidate_cap
                ),
            ));
        }
        if item.candidates[0].surface != item.reference {
            return Err(EvalError::invalid(
                path,
                format!("{}：reference 不是第一個候選", item.id),
            ));
        }
        let mut previous: Option<&AmbiguousCandidate> = None;
        for candidate in &item.candidates {
            if !is_kanji_surface(&candidate.surface) {
                return Err(EvalError::invalid(
                    path,
                    format!("{}：候選 {:?} 不是漢字", item.id, candidate.surface),
                ));
            }
            if let Some(prev) = previous {
                if candidate.cost < prev.cost
                    || (candidate.cost == prev.cost && candidate.surface < prev.surface)
                {
                    return Err(EvalError::invalid(
                        path,
                        format!("{}：候選沒有依詞典順序排列", item.id),
                    ));
                }
            }
            previous = Some(candidate);
        }
        if item.holdout {
            holdout += 1;
        }
    }
    if holdout != corpus.holdout_size {
        return Err(EvalError::invalid(
            path,
            format!("holdout 有 {} 題，宣告 {}", holdout, corpus.holdout_size),
        ));
    }
    Ok(())
}

/// `fuzzy/fuzzy-input.json`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyCorpus {
    /// 格式版本。
    pub schema: u32,
    /// 題庫名稱。
    pub name: String,
    /// 授權。
    pub license: String,
    /// 來源。
    pub source: String,
    /// 說明。
    #[serde(default)]
    pub notes: String,
    /// 固定種子。
    #[serde(default)]
    pub seed: i64,
    /// 產生器。
    #[serde(default)]
    pub generator: String,
    /// 來源題庫 pin。
    #[serde(default)]
    pub sources: Vec<FuzzySourcePin>,
    /// 題數。
    pub count: usize,
    /// 題目。
    pub items: Vec<FuzzyItem>,
}

/// 模糊題庫的來源 pin。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzySourcePin {
    /// 題庫名稱。
    #[serde(default)]
    pub name: String,
    /// 檔案路徑。
    #[serde(default)]
    pub path: String,
    /// 檔案雜湊。
    #[serde(default)]
    pub sha256: String,
    /// 題數。
    #[serde(default)]
    pub count: usize,
}

/// 一個模糊輸入案例。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuzzyItem {
    /// 題目 id。
    pub id: String,
    /// 語言。
    pub language: String,
    /// 擾動種類。
    pub kind: String,
    /// 來源題目 id。
    pub source_id: String,
    /// 來源讀音。
    pub reading: String,
    /// 期望輸出。
    pub surface: String,
    /// 未擾動的輸入。
    pub original_input: String,
    /// 實際輸入。
    pub input: String,
    /// 輸入表示法。
    pub input_kind: String,
    /// 擾動後的讀音。
    #[serde(default)]
    pub noisy_reading: String,
    /// 尚未完成的按鍵。
    #[serde(default)]
    pub pending: String,
    /// 擾動內容。
    #[serde(default)]
    pub mutation: FuzzyMutation,
}

impl FuzzyItem {
    /// 轉成通用評測題目。
    ///
    /// 轉換器吃讀音，因此優先使用 `noisy_reading`；沒有時退回原始 `reading`。
    pub fn eval_case(&self) -> EvalCase {
        let reading = if self.noisy_reading.is_empty() {
            self.reading.clone()
        } else {
            self.noisy_reading.clone()
        };
        EvalCase::new(self.id.clone(), reading, self.surface.clone())
    }
}

/// 擾動的位置與前後值。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzyMutation {
    /// 位置。
    #[serde(default)]
    pub at: usize,
    /// 原值。
    #[serde(default)]
    pub from: String,
    /// 新值。
    #[serde(default)]
    pub to: String,
}

/// 模糊題庫的擾動種類。
pub const FUZZY_KINDS: [&str; 4] = ["wrong-key", "missing-key", "doubled-key", "near-sound"];

/// 模糊題庫的輸入表示法。
pub const FUZZY_INPUT_KINDS: [&str; 3] = ["zhuyin-keys", "zhuyin-syllables", "romaji"];

/// 讀取 `fuzzy-input.json`，回傳題庫與檔案雜湊。
pub fn load_fuzzy_input(path: &Path) -> Result<(FuzzyCorpus, String)> {
    let raw = read_bytes(path)?;
    let corpus: FuzzyCorpus = parse_json(path, &raw)?;
    validate_fuzzy(path, &corpus)?;
    Ok((corpus, hex_digest(&raw)))
}

fn validate_fuzzy(path: &Path, corpus: &FuzzyCorpus) -> Result<()> {
    if corpus.schema != CORPUS_SCHEMA {
        return Err(EvalError::invalid(
            path,
            format!("schema {}，本版支援 {}", corpus.schema, CORPUS_SCHEMA),
        ));
    }
    if corpus.name.is_empty()
        || corpus.license.is_empty()
        || corpus.source.is_empty()
        || corpus.generator.is_empty()
    {
        return Err(EvalError::invalid(
            path,
            "name、license、source、generator 都是必填",
        ));
    }
    if corpus.seed <= 0 {
        return Err(EvalError::invalid(path, "seed 必須大於 0"));
    }
    if corpus.count != corpus.items.len() || corpus.count == 0 {
        return Err(EvalError::invalid(
            path,
            format!(
                "count {} 與 {} 筆 items 不符",
                corpus.count,
                corpus.items.len()
            ),
        ));
    }
    if corpus.sources.len() != 2 {
        return Err(EvalError::invalid(
            path,
            format!("需要 2 個來源 pin，得到 {}", corpus.sources.len()),
        ));
    }
    for source in &corpus.sources {
        if source.name.is_empty()
            || source.path.is_empty()
            || source.sha256.len() != 64
            || source.count == 0
        {
            return Err(EvalError::invalid(
                path,
                format!("來源 pin {:?} 無效", source.name),
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for (index, item) in corpus.items.iter().enumerate() {
        let where_ = format!("第 {} 題", index + 1);
        if item.id.is_empty()
            || item.source_id.is_empty()
            || item.surface.is_empty()
            || item.reading.is_empty()
            || item.input.is_empty()
        {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：id、source、surface、reading、input 不能是空的"),
            ));
        }
        if !seen.insert(item.id.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("題目 id 重複：{}", item.id),
            ));
        }
        if item.language != "zh" && item.language != "ja" {
            return Err(EvalError::invalid(
                path,
                format!("{}：未知語言 {:?}", item.id, item.language),
            ));
        }
        if !FUZZY_KINDS.contains(&item.kind.as_str()) {
            return Err(EvalError::invalid(
                path,
                format!("{}：未知擾動種類 {:?}", item.id, item.kind),
            ));
        }
        if item.kind == "near-sound" && item.language != "zh" {
            return Err(EvalError::invalid(
                path,
                format!("{}：near-sound 只用於注音", item.id),
            ));
        }
        if !FUZZY_INPUT_KINDS.contains(&item.input_kind.as_str()) {
            return Err(EvalError::invalid(
                path,
                format!("{}：未知輸入表示法 {:?}", item.id, item.input_kind),
            ));
        }
        let zhuyin_input = item.input_kind.starts_with("zhuyin");
        if zhuyin_input && item.language != "zh" {
            return Err(EvalError::invalid(
                path,
                format!("{}：{} 題目不能使用注音輸入", item.id, item.language),
            ));
        }
        if item.input_kind == "romaji" && item.language != "ja" {
            return Err(EvalError::invalid(
                path,
                format!("{}：{} 題目不能使用羅馬字輸入", item.id, item.language),
            ));
        }
    }
    Ok(())
}

fn is_han_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(crate::metrics::is_han)
}

fn is_kanji_surface(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|r| crate::metrics::is_han(r) || matches!(r, '々' | '〆' | 'ヶ' | 'ヵ' | '〇'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-eval-corpus-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn zhuyin_segment_mismatch_is_rejected() {
        let dir = temp_dir("zh");
        let path = dir.join("corpus.json");
        std::fs::write(
            &path,
            r#"{"schema":1,"name":"x","license":"CC0","source":"test","count":1,
                "items":[{"id":"a","reading":"ㄨㄛˇ ㄒㄧㄤˇ","surface":"我想",
                          "segments":[{"reading":"ㄨㄛˇ","surface":"我"}]}]}"#,
        )
        .unwrap();
        let err = load_zhuyin_golden(&path).unwrap_err().to_string();
        assert!(err.contains("segments 串接"), "{err}");
    }

    #[test]
    fn ambiguous_reference_must_be_first_candidate() {
        let dir = temp_dir("ambig");
        let path = dir.join("corpus.json");
        std::fs::write(
            &path,
            r#"{"schema":1,"name":"x","license":"CC0","source":"test",
                "sample_size":2,"holdout_size":1,"candidate_cap":3,"reading_min":1,"reading_max":8,
                "pack":{"path":"p","sha256":"0000000000000000000000000000000000000000000000000000000000000000"},
                "count":2,"items":[
                  {"id":"a","reading":"あい","reference":"愛","holdout":true,
                   "candidates":[{"surface":"愛","cost":1},{"surface":"藍","cost":2}]},
                  {"id":"b","reading":"あお","reference":"青","holdout":false,
                   "candidates":[{"surface":"碧","cost":1},{"surface":"青","cost":2}]}
                ]}"#,
        )
        .unwrap();
        let err = load_ja_lexicon_ambiguous(&path).unwrap_err().to_string();
        assert!(err.contains("reference 不是第一個候選"), "{err}");
    }

    #[test]
    fn fuzzy_romaji_on_zh_is_rejected() {
        let dir = temp_dir("fuzzy");
        let path = dir.join("corpus.json");
        std::fs::write(
            &path,
            r#"{"schema":1,"name":"x","license":"CC0","source":"test","seed":1,"generator":"g",
                "sources":[
                  {"name":"a","path":"a","sha256":"0000000000000000000000000000000000000000000000000000000000000000","count":1},
                  {"name":"b","path":"b","sha256":"0000000000000000000000000000000000000000000000000000000000000000","count":1}
                ],
                "count":1,"items":[
                  {"id":"i","language":"zh","kind":"wrong-key","source_id":"s","reading":"ㄨㄛˇ",
                   "surface":"我","original_input":"a","input":"b","input_kind":"romaji"}
                ]}"#,
        )
        .unwrap();
        let err = load_fuzzy_input(&path).unwrap_err().to_string();
        assert!(err.contains("不能使用羅馬字輸入"), "{err}");
    }

    #[test]
    fn real_zhuyin_golden_loads() {
        let path = jc_testdata::require_data!("0c-testdata/eval/zhuyin-golden.json");
        let (corpus, hash) = load_zhuyin_golden(&path).unwrap();
        assert_eq!(corpus.count, 17);
        assert_eq!(corpus.items.len(), 17);
        assert_eq!(hash.len(), 64);
        assert_eq!(corpus.items[0].id, "single-char");
        assert_eq!(corpus.items[0].segments.len(), 1);
    }

    #[test]
    fn real_ambiguous_corpus_loads() {
        let path = jc_testdata::require_data!("0c-testdata/eval/ja-lexicon-ambiguous.json");
        let (corpus, _) = load_ja_lexicon_ambiguous(&path).unwrap();
        assert_eq!(corpus.count, 1200);
        assert_eq!(corpus.items.iter().filter(|i| i.holdout).count(), 400);
        assert_eq!(
            corpus.items[0].candidates[0].surface,
            corpus.items[0].reference
        );
    }

    #[test]
    fn real_fuzzy_corpus_loads() {
        let path = jc_testdata::require_data!("0c-testdata/fuzzy/fuzzy-input.json");
        let (corpus, _) = load_fuzzy_input(&path).unwrap();
        assert_eq!(corpus.count, 919);
        assert_eq!(corpus.sources.len(), 2);
        assert!(corpus.items.iter().any(|i| i.language == "ja"));
        assert!(corpus.items.iter().any(|i| i.language == "zh"));
    }
}
