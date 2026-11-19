//! 引擎與 `jc-cli` 共用的資料載入。
//!
//! 這個模組是資料檔到 [`jc_core::Session`] 的唯一入口，`jc-cli` 的
//! `data` 模組只做錯誤型別轉換與環境變數解析，不再自己重寫一份。
//!
//! 放在 `jc-engine` 而不是 `jc-data`：`jc-data` 是 `jc-core` 的依賴，
//! 反過來讓 `jc-data` 認識 `InMemoryLexicon`、`JaDecoder` 會形成循環依賴。
//! `jc-engine` 已經依賴 `jc-core`，且 `jc-cli` 可以依賴它的 lib。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use jc_core::ja::decode::{JaDecoder, JaDictionary, JaReranker, JaWordMatch, JaWordSource};
use jc_core::ja::lexicon::{JaEntry, JaLexicon, SingleColumnCost};
use jc_core::ja::rerank::NeuralReranker;
use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::output::OnnxPunctuator;
use jc_core::score::{BigramLm, ScoreContext, ZhLm};
use jc_core::session::SessionConfig;
use jc_core::Mode;

/// 注音詞典資料包目錄（相對於資料根目錄）。
pub const ZH_DIR: &str = "build-zh";

/// 注音 bigram 語言模型目錄（相對於資料根目錄）。
pub const ZH_BIGRAM_DIR: &str = "build-zh-bigram";

/// 預設的日文詞典 pack（含 Mozc context 連接矩陣的 v2）。
pub const DEFAULT_JA_PACK: &str = "packs/ja-lexicon-mozc-v2.pack";

/// 找不到 v2 時退回的 v1 pack（只有詞性連接成本）。
pub const FALLBACK_JA_PACK: &str = "packs/ja-lexicon-public-core.pack";

/// 預設的日文 tokenizer（相對於資料根目錄）。
pub const JA_TOKENIZER: &str = "models/ja-tokenizer/public_core_ja.model";

/// 資料載入錯誤。訊息一律帶上出錯的路徑。
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LoadError(String);

impl LoadError {
    /// 建立帶訊息的載入錯誤。
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn read(path: &Path, err: impl std::fmt::Display) -> Self {
        Self(format!("讀取 {} 失敗：{err}", path.display()))
    }
}

/// 注音資源：固定詞典、成本設定與 bigram 語言模型。
#[derive(Debug, Clone)]
pub struct ZhResources {
    /// 固定詞典。
    pub lexicon: InMemoryLexicon,
    /// 成本設定（含 bigram 內插權重）。
    pub ctx: ScoreContext,
    /// bigram 語言模型；`None` 表示只有詞典的 unigram 成本。
    pub lm: Option<Arc<ZhLm>>,
}

/// 載入注音固定詞典與成本設定（golden TSV）。
pub fn load_zh_resources(data_dir: &Path) -> Result<ZhResources, LoadError> {
    load_zh_tsv_resources(data_dir, None)
}

/// 載入 golden TSV 詞典，另外接上 `lm.bin` 的 bigram。
pub fn load_zh_resources_with_lm(
    data_dir: &Path,
    lm_path: &Path,
) -> Result<ZhResources, LoadError> {
    let lm = ZhLm::open(lm_path).map_err(|err| LoadError::read(lm_path, err))?;
    load_zh_tsv_resources(data_dir, Some(lm))
}

fn load_zh_tsv_resources(data_dir: &Path, bigram: Option<ZhLm>) -> Result<ZhResources, LoadError> {
    let lexicon_path = data_dir.join("lexicon.tsv");
    let lm_path = data_dir.join("lm.tsv");
    let lm = load_lm(&lm_path)?;
    let text = std::fs::read_to_string(&lexicon_path)
        .map_err(|err| LoadError::read(&lexicon_path, err))?;
    let mut lexicon = InMemoryLexicon::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 3 {
            return Err(LoadError(format!(
                "{}:{}：需要詞面、讀音、P(r|w) 三欄",
                lexicon_path.display(),
                line_no + 1
            )));
        }
        let probability: f64 = fields[2].parse().map_err(|_| {
            LoadError(format!(
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
        return Err(LoadError(format!(
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
pub fn load_zh_bin_resources(data_dir: &Path) -> Result<ZhResources, LoadError> {
    load_zh_bin_resources_with_lm(data_dir, None)
}

/// 載入注音二進位資料包，`lm_path` 可覆寫資料包內的 `lm.bin`。
pub fn load_zh_bin_resources_with_lm(
    data_dir: &Path,
    lm_path: Option<&Path>,
) -> Result<ZhResources, LoadError> {
    let bundle =
        jc_data::zh::Bundle::open(data_dir).map_err(|err| LoadError::read(data_dir, err))?;
    let lm = match lm_path {
        Some(path) => ZhLm::open(path).map_err(|err| LoadError::read(path, err))?,
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
        return Err(LoadError(format!(
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

fn load_lm(path: &Path) -> Result<HashMap<String, f64>, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|err| LoadError::read(path, err))?;
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
pub fn probability_cost(probability: f64) -> i32 {
    jc_core::score::probability_cost(probability)
}

/// 記憶體日文詞典：由 curated `words.tsv` 建立。
#[derive(Debug)]
pub struct TsvWordSource {
    entries: Vec<JaEntry>,
    index: HashMap<String, Vec<u32>>,
    longest: usize,
}

impl TsvWordSource {
    /// 讀取 `詞面\t讀音\t成本\t詞性\t來源\t活用型` 格式。
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(|err| LoadError::read(path, err))?;
        let mut entries = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 4 {
                return Err(LoadError(format!(
                    "{}:{}：需要詞面、讀音、成本、詞性",
                    path.display(),
                    line_no + 1
                )));
            }
            let cost: f64 = fields[2].parse().map_err(|_| {
                LoadError(format!(
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
            return Err(LoadError(format!("{} 沒有任何詞條", path.display())));
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
/// 順序：`explicit`、`JC_JA_PACK`、資料根目錄的 [`DEFAULT_JA_PACK`]、
/// [`FALLBACK_JA_PACK`]。都沒有時回傳帶路徑的錯誤。
pub fn resolve_ja_pack(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, LoadError> {
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
    Err(LoadError(format!(
        "找不到日文 pack：{} 或 {}；可用 --pack 指定",
        root.join(DEFAULT_JA_PACK).display(),
        root.join(FALLBACK_JA_PACK).display()
    )))
}

/// 模型集合根目錄：`JC_MODELS`，否則 `root/models`。
pub fn models_dir(root: &Path) -> PathBuf {
    if let Some(value) = std::env::var_os("JC_MODELS") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    root.join("models")
}

/// 日文模型根目錄：`JC_JA_MODELS`，否則 [`models_dir`]。
pub fn ja_models_dir(root: &Path) -> PathBuf {
    if let Some(value) = std::env::var_os("JC_JA_MODELS") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    models_dir(root)
}

/// 載入日文解碼器，使用預設 pack（見 [`resolve_ja_pack`]）。
pub fn load_ja_decoder(root: &Path) -> Result<JaDecoder, LoadError> {
    load_ja_decoder_with_pack(root, None)
}

/// 載入日文解碼器，`pack` 覆寫預設 pack。
pub fn load_ja_decoder_with_pack(root: &Path, pack: Option<&Path>) -> Result<JaDecoder, LoadError> {
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
fn open_pack_decoder(pack: &Path, connection: &Path) -> Result<JaDecoder, LoadError> {
    let lex = JaLexicon::open(pack).map_err(|err| LoadError::read(pack, err))?;
    let context = lex.connection_cost();
    let has_matrix = context.has_matrix();
    let decoder = JaDecoder::new(Arc::new(JaDictionary::new(Arc::new(lex))));
    if has_matrix {
        return Ok(decoder.with_context_cost(Arc::new(context)));
    }
    if connection.is_file() {
        let cost =
            SingleColumnCost::load(connection).map_err(|err| LoadError::read(connection, err))?;
        return Ok(decoder.with_connection(Arc::new(cost)));
    }
    Ok(decoder)
}

/// 沒有 pack 時使用 curated `words.tsv` 的保底路徑。
pub fn load_curated_decoder(words: &Path, connection: &Path) -> Result<JaDecoder, LoadError> {
    let source = TsvWordSource::load(words)?;
    let mut decoder = JaDecoder::new(Arc::new(source));
    if connection.is_file() {
        let cost =
            SingleColumnCost::load(connection).map_err(|err| LoadError::read(connection, err))?;
        decoder = decoder.with_connection(Arc::new(cost));
    }
    Ok(decoder)
}

/// 載入日文 reranker；模型缺席或 ONNX Runtime 不可用時回傳錯誤。
pub fn load_ja_reranker(root: &Path) -> Result<Arc<NeuralReranker>, LoadError> {
    let models = ja_models_dir(root);
    NeuralReranker::load_from_models(&models)
        .map(Arc::new)
        .map_err(|err| LoadError(format!("{}：{err}", models.display())))
}

/// 載入 OpenCC tw2s 字典；資料在資料根目錄的 `opencc/`。
pub fn load_zhconv(root: &Path) -> Result<jc_core::zhconv::Converter, LoadError> {
    let dir = root.join("opencc");
    jc_core::zhconv::tw2s_from_dir(&dir).map_err(|err| LoadError::read(&dir, err))
}

/// 載入提交時補標點的模型。
pub fn load_punctuator(models: &Path, modes: &[Mode]) -> Result<OnnxPunctuator, LoadError> {
    OnnxPunctuator::load_from_models(models, modes)
        .map_err(|err| LoadError(format!("{}：{err}", models.display())))
}

/// 載入並安裝注音選字模型。
pub fn install_select_model(models: &Path) -> Result<(), LoadError> {
    let dir = jc_core::zh::select::resolve_model_dir(models)
        .map_err(|err| LoadError(format!("找不到 zh-select 模型：{err}")))?;
    let model = jc_core::zh::select::SelectModel::load(&dir, None)
        .map_err(|err| LoadError(format!("載入 zh-select 模型失敗：{err}")))?;
    jc_core::zh::select::install(model);
    Ok(())
}

/// 關閉注音選字模型。
pub fn disable_select_model() {
    jc_core::zh::select::clear();
}

/// 選字模型的推論延遲統計。
pub fn select_model_stats() -> Option<jc_core::zh::select::LatencyStats> {
    jc_core::zh::select::latency_stats()
}

/// 由注音資源建立 session 設定。
pub fn zh_session_config(mode: Mode, resources: &ZhResources) -> SessionConfig {
    SessionConfig {
        mode,
        max_candidates: 5,
        zh_lexicon: Some(resources.lexicon.clone()),
        zh_ctx: Some(resources.ctx.clone()),
        zh_lm: resources.lm.clone(),
        ..SessionConfig::default()
    }
}

/// 延遲載入的日文重排器。
///
/// 啟動時只記住模型集合路徑，第一次日文轉換才載入 ONNX。載入失敗只記一次
/// 警告，之後永久退回統計排序。
pub struct LazyReranker {
    models: PathBuf,
    inner: OnceLock<Option<Arc<NeuralReranker>>>,
}

impl LazyReranker {
    /// 指定模型集合根目錄。
    pub fn new(models: impl Into<PathBuf>) -> Self {
        Self {
            models: models.into(),
            inner: OnceLock::new(),
        }
    }

    fn get(&self) -> Option<&Arc<NeuralReranker>> {
        self.inner
            .get_or_init(|| match NeuralReranker::load_from_models(&self.models) {
                Ok(reranker) => {
                    log::info!("已載入日文重排模型：{}", self.models.display());
                    Some(Arc::new(reranker))
                }
                Err(err) => {
                    log::warn!(
                        "日文重排模型不可用，沿用統計排序：{}：{err}",
                        self.models.display()
                    );
                    None
                }
            })
            .as_ref()
    }
}

impl std::fmt::Debug for LazyReranker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyReranker")
            .field("models", &self.models)
            .field("loaded", &self.inner.get().is_some())
            .finish()
    }
}

impl JaReranker for LazyReranker {
    fn score_words(&self, left: &str, right: &str, candidates: &[&str]) -> Option<Vec<f32>> {
        self.get()?.score_words(left, right, candidates)
    }

    fn begin(&self) {
        if let Some(reranker) = self.get() {
            reranker.begin();
        }
    }

    fn end(&self, timed_out: bool) {
        if let Some(reranker) = self.get() {
            reranker.end(timed_out);
        }
    }

    fn note_changed(&self) {
        if let Some(reranker) = self.get() {
            reranker.note_changed();
        }
    }

    fn timeout(&self) -> Duration {
        match self.inner.get() {
            Some(Some(reranker)) => reranker.timeout(),
            _ => Duration::from_millis(100),
        }
    }
}

/// 引擎啟動需要的資料。
pub struct EngineData {
    /// 資料根目錄。
    pub root: PathBuf,
    /// 注音詞典與語言模型。
    pub zh: ZhResources,
    /// 日文解碼器；重排器已接上延遲載入包裝。
    pub ja: Arc<JaDecoder>,
    /// 日文 tokenizer；模型缺席時為 `None`。
    pub ja_tokenizer: Option<Arc<jc_model::Tokenizer>>,
}

impl std::fmt::Debug for EngineData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineData")
            .field("root", &self.root)
            .field("zh_entries", &self.zh.lexicon.len())
            .field(
                "has_bigram",
                &self.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0),
            )
            .field("ja_tokenizer", &self.ja_tokenizer.is_some())
            .finish()
    }
}

impl EngineData {
    /// 為指定模式建立 session 設定。
    pub fn session_config(&self, mode: Mode) -> SessionConfig {
        let mut config = zh_session_config(mode, &self.zh);
        config.ja = Some(Arc::clone(&self.ja));
        config
    }
}

/// 引擎載入的致命錯誤。
#[derive(Debug, thiserror::Error)]
pub enum EngineDataError {
    /// 資料根目錄不存在。
    #[error("資料根目錄不存在：{}", .0.display())]
    MissingRoot(PathBuf),
    /// 注音詞典檔不存在。
    #[error("缺少注音詞典：{}", path.display())]
    MissingZhLexicon {
        /// 預期的 `lexicon.bin` 路徑。
        path: PathBuf,
    },
    /// 日文詞典 pack 不存在。
    #[error("缺少日文詞典：{message}")]
    MissingJaPack {
        /// [`resolve_ja_pack`] 的錯誤訊息，含兩個候選路徑。
        message: String,
    },
    /// 檔案存在但讀取失敗。
    #[error(transparent)]
    Load(#[from] LoadError),
}

/// 依資料根目錄載入引擎所需資料。
///
/// 分級：
/// - 注音 `lexicon.bin`、日文 pack 缺失或壞掉 → 啟動失敗。
/// - `build-zh-bigram/lm.bin` 缺席 → 警告，退回 `build-zh/lm.bin`。
/// - zh-select 存在就安裝；載入失敗只警告一次，維持統計排序。
/// - 其餘模型（日文 tokenizer、reranker）缺席或壞掉 → 各記一次警告後繼續。
pub fn load_engine_data(root: &Path) -> Result<EngineData, EngineDataError> {
    if !root.is_dir() {
        return Err(EngineDataError::MissingRoot(root.to_path_buf()));
    }
    let zh_dir = root.join(ZH_DIR);
    let lexicon_bin = zh_dir.join("lexicon.bin");
    if !lexicon_bin.is_file() {
        return Err(EngineDataError::MissingZhLexicon { path: lexicon_bin });
    }
    let bigram_lm = root.join(ZH_BIGRAM_DIR).join("lm.bin");
    let zh = if bigram_lm.is_file() {
        load_zh_bin_resources_with_lm(&zh_dir, Some(&bigram_lm))?
    } else {
        log::warn!(
            "找不到 bigram 語言模型 {}，改用 {}/lm.bin",
            bigram_lm.display(),
            ZH_DIR
        );
        load_zh_bin_resources(&zh_dir)?
    };
    let pack = resolve_ja_pack(root, None).map_err(|err| EngineDataError::MissingJaPack {
        message: err.to_string(),
    })?;
    let models = ja_models_dir(root);
    let mut decoder = load_ja_decoder_with_pack(root, Some(&pack))?;
    if models.is_dir() {
        decoder = decoder.with_reranker(Arc::new(LazyReranker::new(&models)));
        // 選字模型在啟動時安裝；缺檔或 ONNX 不可用只警告一次，維持統計排序。
        match install_select_model(&models) {
            Ok(()) => log::info!("已安裝 zh-select 模型：{}", models.display()),
            Err(err) => log::warn!("zh-select 不可用，維持統計排序：{err}"),
        }
    } else {
        log::warn!(
            "找不到模型目錄 {}，日文重排與 zh-select 停用",
            models.display()
        );
    }
    let tokenizer_path = root.join(JA_TOKENIZER);
    let ja_tokenizer = if tokenizer_path.is_file() {
        match jc_model::Tokenizer::load(&tokenizer_path) {
            Ok(tokenizer) => Some(Arc::new(tokenizer)),
            Err(err) => {
                log::warn!("讀取 {} 失敗：{err}", tokenizer_path.display());
                None
            }
        }
    } else {
        log::warn!("找不到日文 tokenizer：{}", tokenizer_path.display());
        None
    };
    Ok(EngineData {
        root: root.to_path_buf(),
        zh,
        ja: Arc::new(decoder),
        ja_tokenizer,
    })
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
    fn bin_resources_load_lm_and_bigrams() {
        let Some(plain) = jc_testdata::path(ZH_DIR) else {
            eprintln!("缺少測試資料：{ZH_DIR}");
            return;
        };
        let bigram = jc_testdata::require_data!(ZH_BIGRAM_DIR);
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

    #[test]
    fn engine_data_degrades_without_models() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let data = load_engine_data(&root).unwrap();
        assert!(!data.zh.lexicon.is_empty());
        assert!(
            data.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0),
            "有 build-zh-bigram/lm.bin 時應接上 bigram"
        );
        let results = data.ja.nbest(
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
    fn missing_root_and_lexicon_report_paths() {
        let err = load_engine_data(Path::new("/no/such/jc-data-root")).unwrap_err();
        assert!(err.to_string().contains("/no/such/jc-data-root"));

        let dir =
            std::env::temp_dir().join(format!("jc-engine-data-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = load_engine_data(&dir).unwrap_err();
        assert!(
            err.to_string().contains("lexicon.bin"),
            "錯誤應指出缺少的詞典：{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
