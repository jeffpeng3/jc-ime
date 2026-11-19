//! 評測與互動模式共用的資料載入。
//!
//! 注音用 `0c-testdata/golden/zh/data` 的固定 TSV（與前身同一份），日文用
//! `packs/` 的詞典 pack 與 `ja-src/connection.tsv`。路徑都可以用環境變數
//! 覆寫，找不到時回傳帶路徑的錯誤，不 panic。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jc_core::ja::decode::{JaDecoder, JaDictionary, JaWordMatch, JaWordSource};
use jc_core::ja::lexicon::{JaEntry, JaLexicon, SingleColumnCost};
use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::{BigramLm, ScoreContext, ZhLm};

use crate::cli::CliError;

/// 沒有指定 `--testdata` 時使用的 `0c-testdata` 目錄。
pub const DEFAULT_TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata";

/// 注音評測資源。
#[derive(Debug, Clone)]
pub struct ZhResources {
    /// 固定詞典。
    pub lexicon: InMemoryLexicon,
    /// 成本設定（含 bigram 內插權重）。
    pub ctx: ScoreContext,
    /// bigram 語言模型；`None` 表示只有詞典的 unigram 成本。
    pub lm: Option<Arc<ZhLm>>,
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
    load_zh_tsv_resources(data_dir, None)
}

/// 載入 golden 的 TSV 詞典，另外接上 `lm.bin` 的 bigram。
///
/// 詞條的 unigram 部分改用 `lm.bin` 的數值，避免同一條路徑上混用兩份
/// 語言模型；`lm.tsv` 只在沒有指定 `lm.bin` 時使用。
pub fn load_zh_resources_with_lm(data_dir: &Path, lm_path: &Path) -> Result<ZhResources, CliError> {
    let lm = ZhLm::open(lm_path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", lm_path.display())))?;
    load_zh_tsv_resources(data_dir, Some(lm))
}

fn load_zh_tsv_resources(data_dir: &Path, bigram: Option<ZhLm>) -> Result<ZhResources, CliError> {
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
        let lm_probability = match &bigram {
            Some(lm) => lm.uni(fields[0]),
            None => lm.get(fields[0]).copied().unwrap_or(1.0e-6),
        };
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
    Ok(ZhResources {
        lexicon,
        ctx,
        lm: bigram.map(Arc::new),
    })
}

/// 載入注音二進位資料包（`lexicon.bin`、`lm.bin`、`manifest.json`）。
///
/// 詞條成本沿用 golden 的換算：P(r|w) 與 unigram 機率各自轉成千分之一的
/// 負對數成本後相加。`lm.bin` 的 bigram 交給 [`jc_core::score::LmScorer`]；
/// 沒有 bigram 的資料包完全退回 unigram 行為。
pub fn load_zh_bin_resources(data_dir: &Path) -> Result<ZhResources, CliError> {
    load_zh_bin_resources_with_lm(data_dir, None)
}

/// 載入注音二進位資料包，`lm_path` 可覆寫資料包內的 `lm.bin`。
pub fn load_zh_bin_resources_with_lm(
    data_dir: &Path,
    lm_path: Option<&Path>,
) -> Result<ZhResources, CliError> {
    let bundle = jc_data::zh::Bundle::open(data_dir)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", data_dir.display())))?;
    let lm = match lm_path {
        Some(path) => ZhLm::open(path)
            .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))?,
        None => ZhLm::from_lm(bundle.lm),
    };
    let mut lexicon = InMemoryLexicon::new();
    for view in bundle.lexicon.entries() {
        let unigram = lm.uni(view.surface);
        let cost = probability_cost(view.prw as f64).saturating_add(probability_cost(unigram));
        lexicon.push(LexEntry::new(
            view.surface,
            bundle.lexicon.reading(view.index),
            cost,
            0,
        ));
    }
    if lexicon.is_empty() {
        return Err(CliError::data(format!(
            "{} 沒有任何詞條",
            data_dir.join("lexicon.bin").display()
        )));
    }
    let ctx = ScoreContext {
        unknown_cost: 30_000,
        ..ScoreContext::default()
    };
    Ok(ZhResources {
        lexicon,
        ctx,
        lm: Some(Arc::new(lm)),
    })
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

/// 機率轉成整數成本；與 [`jc_core::score::probability_cost`] 相同。
fn probability_cost(probability: f64) -> i32 {
    jc_core::score::probability_cost(probability)
}

/// 預設的日文詞典 pack（含 Mozc context 連接矩陣的 v2）。
pub const DEFAULT_JA_PACK: &str = "packs/ja-lexicon-mozc-v2.pack";

/// 找不到 v2 時退回的 v1 pack（只有詞性連接成本）。
pub const FALLBACK_JA_PACK: &str = "packs/ja-lexicon-public-core.pack";

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

/// 解析日文 pack 路徑。
///
/// 順序：`explicit`（`--pack`）、`JC_JA_PACK`、資料根目錄的
/// [`DEFAULT_JA_PACK`]、[`FALLBACK_JA_PACK`]。都沒有時回傳錯誤；不寫死
/// 絕對路徑，因此 `JC_TESTDATA` 指到其他資料根目錄時也找得到同一組 pack。
pub fn resolve_ja_pack(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, CliError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(value) = std::env::var_os("JC_JA_PACK") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    for relative in [DEFAULT_JA_PACK, FALLBACK_JA_PACK] {
        let candidate = root.join(relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(CliError::data(format!(
        "找不到日文 pack：{} 或 {}；可用 --pack 指定",
        root.join(DEFAULT_JA_PACK).display(),
        root.join(FALLBACK_JA_PACK).display()
    )))
}

/// 載入日文解碼器，使用預設 pack（見 [`resolve_ja_pack`]）。
pub fn load_ja_decoder(root: &Path) -> Result<JaDecoder, CliError> {
    load_ja_decoder_with_pack(root, None)
}

/// 模型集合根目錄：`JC_JA_MODELS`，否則資料根目錄下的 `models/`。
pub fn ja_models_dir(root: &Path) -> PathBuf {
    if let Some(value) = std::env::var_os("JC_JA_MODELS") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    root.join("models")
}

/// 載入日文 reranker（需要 `onnx` feature）。
///
/// 由模型集合的 `manifest.json` 找 `ja-rerank`；模型缺席回傳帶路徑的
/// 錯誤，呼叫端決定要中止或退回統計排序。
#[cfg(feature = "onnx")]
pub fn load_ja_reranker(root: &Path) -> Result<Arc<jc_core::ja::rerank::NeuralReranker>, CliError> {
    let models = ja_models_dir(root);
    jc_core::ja::rerank::NeuralReranker::load_from_models(&models)
        .map(Arc::new)
        .map_err(|err| CliError::data(format!("{}：{err}", models.display())))
}

/// 載入日文解碼器，`pack` 覆寫預設 pack。
///
/// v2 pack 會同時接上詞條的 left／right ID 與 context 連接矩陣；v1 pack
/// 沒有矩陣，退回 `ja-src/connection.tsv` 的詞性成本。兩條路徑都不會在
/// 有 pack 時偷用 curated `words.tsv`，避免同一個 pack 在不同入口得到不同
/// 分數。
pub fn load_ja_decoder_with_pack(root: &Path, pack: Option<&Path>) -> Result<JaDecoder, CliError> {
    match resolve_ja_pack(root, pack) {
        Ok(pack) => open_pack_decoder(&pack, &root.join("ja-src/connection.tsv")),
        Err(err) => {
            let words = root.join("ja-src/lexicon/words.tsv");
            if pack.is_none() && words.is_file() {
                load_curated_decoder(&words, &root.join("ja-src/connection.tsv"))
            } else {
                Err(err)
            }
        }
    }
}

/// 開啟 pack，並依 pack 能力接上 context 矩陣或詞性成本。
fn open_pack_decoder(pack: &Path, connection: &Path) -> Result<JaDecoder, CliError> {
    let lex = JaLexicon::open(pack)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", pack.display())))?;
    let context = lex.connection_cost();
    let has_matrix = context.has_matrix();
    let decoder = JaDecoder::new(Arc::new(JaDictionary::new(Arc::new(lex))));
    if has_matrix {
        return Ok(decoder.with_context_cost(Arc::new(context)));
    }
    if connection.is_file() {
        let cost = SingleColumnCost::load(connection)
            .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", connection.display())))?;
        return Ok(decoder.with_connection(Arc::new(cost)));
    }
    Ok(decoder)
}

/// 沒有 pack 時使用 curated `words.tsv` 的保底路徑。
fn load_curated_decoder(words: &Path, connection: &Path) -> Result<JaDecoder, CliError> {
    let source = TsvWordSource::load(words)?;
    let mut decoder = JaDecoder::new(Arc::new(source));
    if connection.is_file() {
        let cost = SingleColumnCost::load(connection)
            .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", connection.display())))?;
        decoder = decoder.with_connection(Arc::new(cost));
    }
    Ok(decoder)
}

/// 載入 OpenCC tw2s 字典；資料在資料根目錄的 `opencc/`。
pub fn load_zhconv(root: &Path) -> Result<jc_core::zhconv::Converter, CliError> {
    let dir = root.join("opencc");
    jc_core::zhconv::tw2s_from_dir(&dir)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", dir.display())))
}

/// 模型集合根目錄。
///
/// 依序使用 `JC_MODELS`、`--testdata` 的上層 `models/`，最後退回預設
/// `0c-testdata` 的上層。目錄不存在時仍回傳路徑，由載入端報錯。
pub fn models_root(testdata: Option<&Path>) -> PathBuf {
    if let Some(value) = std::env::var_os("JC_MODELS") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    match testdata {
        Some(dir) => data_root(dir).join("models"),
        None => data_root(Path::new(DEFAULT_TESTDATA)).join("models"),
    }
}

/// 載入並安裝注音選字模型。
///
/// 模型目錄由 [`models_root`] 的集合 manifest 決定。載入失敗時回傳錯誤，
/// 呼叫端不應默默退回統計排序。
#[cfg(feature = "onnx")]
pub fn install_select_model(testdata: Option<&Path>) -> Result<(), CliError> {
    let root = models_root(testdata);
    let dir = jc_core::zh::select::resolve_model_dir(&root)
        .map_err(|err| CliError::data(format!("找不到 zh-select 模型：{err}")))?;
    let model = jc_core::zh::select::SelectModel::load(&dir, None)
        .map_err(|err| CliError::data(format!("載入 zh-select 模型失敗：{err}")))?;
    jc_core::zh::select::install(model);
    eprintln!(
        "警告：zh-select 尚未完成本引擎的權重與 margin 校準，\
         zh-golden 沒有任何改善；正式使用請保持 --select-model off。"
    );
    Ok(())
}

/// 關閉注音選字模型。
#[cfg(feature = "onnx")]
pub fn disable_select_model() {
    jc_core::zh::select::clear();
}

/// 選字模型的推論延遲統計。
#[cfg(feature = "onnx")]
pub fn select_model_stats() -> Option<jc_core::zh::select::LatencyStats> {
    jc_core::zh::select::latency_stats()
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
        zh_lm: resources.lm.clone(),
        ..jc_core::session::SessionConfig::default()
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
    fn bin_resources_load_lm_and_bigrams() {
        let Some(plain) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = load_zh_bin_resources(&plain).unwrap();
        let bigram = load_zh_bin_resources(&bigram).unwrap();
        let plain_lm = plain.lm.expect("build-zh 仍有 lm.bin");
        let bigram_lm = bigram.lm.expect("build-zh-bigram 有 lm.bin");
        assert_eq!(plain_lm.bigrams(), 0);
        assert!(bigram_lm.bigrams() > 0);
        assert!((bigram_lm.bi("我", "愛") - 0.0044).abs() < 0.001);
        assert_eq!(bigram.ctx.bigram_mu, 0.7);
    }

    #[test]
    fn tsv_resources_can_attach_bigram_lm() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let golden = root.join("0c-testdata/golden/zh/data");
        let lm = jc_testdata::require_data!("build-zh-bigram/lm.bin");
        let resources = load_zh_resources_with_lm(&golden, &lm).unwrap();
        let zh_lm = resources.lm.expect("指定 --lm 時要接上模型");
        assert!(zh_lm.bigrams() > 0);
        // unigram 改用 lm.bin 的數值：我 的機率與 golden lm.tsv 不同。
        assert!((zh_lm.uni("我") - 0.0054).abs() < 0.0002);
    }

    #[test]
    fn default_pack_converts_a_sentence() {
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

    #[test]
    fn curated_ja_lexicon_converts_a_sentence() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let decoder = load_curated_decoder(
            &root.join("ja-src/lexicon/words.tsv"),
            &root.join("ja-src/connection.tsv"),
        )
        .unwrap();
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
