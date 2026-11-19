//! 評測與互動模式共用的資料載入。
//!
//! 注音用 `0c-testdata/golden/zh/data` 的固定 TSV（與前身同一份），日文用
//! `packs/` 的詞典 pack 與 `ja-src/connection.tsv`。路徑都可以用環境變數
//! 覆寫，找不到時回傳帶路徑的錯誤，不 panic。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jc_core::ja::decode::{JaDecoder, JaWordMatch, JaWordSource};
use jc_core::ja::lexicon::{JaEntry, SingleColumnCost};
use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::ScoreContext;

use crate::cli::CliError;

/// 沒有指定 `--testdata` 時使用的 `0c-testdata` 目錄。
pub const DEFAULT_TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata";

/// 注音評測資源。
#[derive(Debug, Clone)]
pub struct ZhResources {
    /// 固定詞典。
    pub lexicon: InMemoryLexicon,
    /// 成本設定。
    pub ctx: ScoreContext,
}

/// 解析 `0c-testdata` 目錄。
///
/// 順序：`--testdata`、`JC_TESTDATA` 環境變數、[`DEFAULT_TESTDATA`]。
/// 目錄必須存在。
pub fn testdata_dir(explicit: Option<PathBuf>) -> Result<PathBuf, CliError> {
    if let Some(dir) = explicit {
        return check_dir(dir);
    }
    if let Some(value) = std::env::var_os("JC_TESTDATA") {
        if !value.is_empty() {
            let base = PathBuf::from(value);
            let nested = base.join("0c-testdata");
            return if nested.is_dir() {
                Ok(nested)
            } else {
                check_dir(base)
            };
        }
    }
    check_dir(PathBuf::from(DEFAULT_TESTDATA))
}

/// 由 `0c-testdata` 目錄回推資料根目錄（`packs/`、`ja-src/` 所在處）。
///
/// `--testdata` 若直接指向 `0c-testdata` 的上層，也接受。
pub fn data_root(testdata: &Path) -> PathBuf {
    if testdata
        .file_name()
        .map(|name| name == "0c-testdata")
        .unwrap_or(false)
    {
        testdata
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| testdata.to_path_buf())
    } else {
        testdata.to_path_buf()
    }
}

fn check_dir(dir: PathBuf) -> Result<PathBuf, CliError> {
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(CliError::data(format!("找不到題庫目錄：{}", dir.display())))
    }
}

/// 載入注音固定詞典與成本設定。
///
/// `lexicon.tsv` 有詞面、讀音、P(r|w)；`lm.tsv` 有 unigram 機率。兩者都轉成
/// 以千分之一為單位的負對數成本，長詞與高機率詞優先。
pub fn load_zh_resources(data_dir: &Path) -> Result<ZhResources, CliError> {
    let lexicon_path = data_dir.join("lexicon.tsv");
    let lm_path = data_dir.join("lm.tsv");
    let lm = load_lm(&lm_path)?;
    let text = std::fs::read_to_string(&lexicon_path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", lexicon_path.display())))?;
    let mut lexicon = InMemoryLexicon::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 3 {
            return Err(CliError::data(format!(
                "{}:{}：需要詞面、讀音、P(r|w) 三欄",
                lexicon_path.display(),
                line_no + 1
            )));
        }
        let probability: f64 = fields[2].parse().map_err(|_| {
            CliError::data(format!(
                "{}:{}：機率 {:?} 不是數字",
                lexicon_path.display(),
                line_no + 1,
                fields[2]
            ))
        })?;
        let lm_probability = lm.get(fields[0]).copied().unwrap_or(1.0e-6);
        let cost = probability_cost(probability).saturating_add(probability_cost(lm_probability));
        lexicon.push(LexEntry::new(fields[0], fields[1], cost, 0));
    }
    if lexicon.is_empty() {
        return Err(CliError::data(format!(
            "{} 沒有任何詞條",
            lexicon_path.display()
        )));
    }
    let ctx = ScoreContext {
        unknown_cost: 30_000,
        ..ScoreContext::default()
    };
    Ok(ZhResources { lexicon, ctx })
}

fn load_lm(path: &Path) -> Result<HashMap<String, f64>, CliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))?;
    let mut lm = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 2 {
            // 三欄是 bigram；目前成本模型只有 unigram。
            continue;
        }
        if let Ok(probability) = fields[1].parse::<f64>() {
            if probability > 0.0 {
                lm.insert(fields[0].to_string(), probability);
            }
        }
    }
    Ok(lm)
}

/// 機率轉成整數成本；0 與極小值都給很大的成本。
fn probability_cost(probability: f64) -> i32 {
    if probability <= 0.0 || !probability.is_finite() {
        return 100_000;
    }
    (-probability.ln() * 1000.0).round().min(100_000.0) as i32
}

/// 預設的日文詞典 pack。
pub const DEFAULT_JA_PACK: &str = "packs/ja-lexicon-public-core.pack";

/// 記憶體日文詞典：由 curated `words.tsv` 建立。
///
/// `JaDecoder` 只要求 [`JaWordSource`]，不必一定要讀 pack；評測 baseline
/// 也是用同一份 curated 詞庫產生。
#[derive(Debug)]
pub struct TsvWordSource {
    entries: Vec<JaEntry>,
    index: HashMap<String, Vec<u32>>,
    longest: usize,
}

impl TsvWordSource {
    /// 讀取 `詞面\t讀音\t成本\t詞性\t來源\t活用型` 格式。
    pub fn load(path: &Path) -> Result<Self, CliError> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))?;
        let mut entries = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 4 {
                return Err(CliError::data(format!(
                    "{}:{}：需要詞面、讀音、成本、詞性",
                    path.display(),
                    line_no + 1
                )));
            }
            let cost: f64 = fields[2].parse().map_err(|_| {
                CliError::data(format!(
                    "{}:{}：成本 {:?} 不是數字",
                    path.display(),
                    line_no + 1,
                    fields[2]
                ))
            })?;
            entries.push(JaEntry {
                surface: fields[0].into(),
                reading: fields[1].into(),
                pos: Arc::from(fields[3]),
                conj_type: Arc::from(fields.get(5).copied().unwrap_or("-")),
                conj_form: Arc::from("-"),
                cost: (cost * 1000.0).round() as i32,
                split: jc_core::ja::lexicon::Split::C,
            });
        }
        if entries.is_empty() {
            return Err(CliError::data(format!("{} 沒有任何詞條", path.display())));
        }
        let mut index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut longest = 0usize;
        for (i, entry) in entries.iter().enumerate() {
            index
                .entry(entry.reading.to_string())
                .or_default()
                .push(i as u32);
            longest = longest.max(entry.reading.chars().count());
        }
        Ok(Self {
            entries,
            index,
            longest,
        })
    }
}

impl JaWordSource for TsvWordSource {
    fn longest_reading_units(&self) -> usize {
        self.longest
    }

    fn matches<'a>(&'a self, reading: &[char], start: usize, out: &mut dyn FnMut(JaWordMatch<'a>)) {
        if start >= reading.len() {
            return;
        }
        let mut key = String::new();
        let last = (start + self.longest).min(reading.len());
        for end in start + 1..=last {
            key.push(reading[end - 1]);
            if let Some(ids) = self.index.get(&key) {
                for &id in ids {
                    out(JaWordMatch {
                        end,
                        entry: &self.entries[id as usize],
                        left_id: None,
                        right_id: None,
                    });
                }
            }
        }
    }

    fn exact(&self, reading: &[char]) -> Vec<&JaEntry> {
        let key: String = reading.iter().collect();
        self.index
            .get(&key)
            .map(|ids| ids.iter().map(|&id| &self.entries[id as usize]).collect())
            .unwrap_or_default()
    }

    fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry> {
        if reading.is_empty() {
            return self.entries.iter().collect();
        }
        let prefix: String = reading.iter().collect();
        self.index
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .flat_map(|(_, ids)| ids.iter().map(|&id| &self.entries[id as usize]))
            .collect()
    }
}

/// 載入日文解碼器。
///
/// 優先使用與 baseline 相同的 curated `ja-src/lexicon/words.tsv` 與
/// `ja-src/connection.tsv`；curated 詞庫不存在時才退回 pack：
/// `JC_JA_PACK`、`packs/ja-lexicon-public-core.pack`、
/// `packs/ja-lexicon-mozc-v2.pack`。
pub fn load_ja_decoder(root: &Path) -> Result<JaDecoder, CliError> {
    let words = root.join("ja-src/lexicon/words.tsv");
    let connection = root.join("ja-src/connection.tsv");
    if words.is_file() {
        let source = TsvWordSource::load(&words)?;
        let mut decoder = JaDecoder::new(Arc::new(source));
        if connection.is_file() {
            let cost = SingleColumnCost::load(&connection).map_err(|err| {
                CliError::data(format!("讀取 {} 失敗：{err}", connection.display()))
            })?;
            decoder = decoder.with_connection(Arc::new(cost));
        }
        return Ok(decoder);
    }
    let pack = match std::env::var_os("JC_JA_PACK") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            let mut found = None;
            for relative in [DEFAULT_JA_PACK, "packs/ja-lexicon-mozc-v2.pack"] {
                let candidate = root.join(relative);
                if candidate.is_file() {
                    found = Some(candidate);
                    break;
                }
            }
            found.ok_or_else(|| {
                CliError::data(format!(
                    "找不到日文詞典：{} 或 {}",
                    words.display(),
                    root.join(DEFAULT_JA_PACK).display()
                ))
            })?
        }
    };
    let mut decoder = JaDecoder::open(&pack)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", pack.display())))?;
    if connection.is_file() {
        let cost = SingleColumnCost::load(&connection)
            .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", connection.display())))?;
        decoder = decoder.with_connection(Arc::new(cost));
    }
    Ok(decoder)
}

/// 由注音資源建立 session 設定。
pub fn zh_session_config(
    mode: jc_core::Mode,
    resources: &ZhResources,
) -> jc_core::session::SessionConfig {
    jc_core::session::SessionConfig {
        mode,
        max_candidates: 5,
        zh_lexicon: Some(resources.lexicon.clone()),
        zh_ctx: Some(resources.ctx.clone()),
        ja: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_cost_orders_rare_words_later() {
        assert_eq!(probability_cost(1.0), 0);
        assert!(probability_cost(0.2) > probability_cost(1.0));
        assert_eq!(probability_cost(0.0), 100_000);
    }

    #[test]
    fn testdata_falls_back_to_missing_error() {
        let err = testdata_dir(Some(PathBuf::from("/no/such/jc-testdata"))).unwrap_err();
        assert!(err.to_string().contains("/no/such/jc-testdata"));
    }

    #[test]
    fn curated_ja_lexicon_converts_a_sentence() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let decoder = load_ja_decoder(&root).unwrap();
        let results = decoder.nbest(
            "わたしはほんをよむ。",
            &jc_core::ja::decode::JaContext::default(),
            1,
        );
        assert_eq!(
            results.first().map(|result| result.surface.as_str()),
            Some("私は本を読む。")
        );
    }
}
