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
use std::time::{Duration, Instant};

use jc_core::ja::decode::{JaDecoder, JaDictionary, JaReranker, JaWordMatch, JaWordSource};
use jc_core::ja::lexicon::{JaEntry, JaLexicon, SingleColumnCost};
use jc_core::ja::rerank::NeuralReranker;
use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::mixed::{English, ENGLISH_PACK_SUBDIR};
use jc_core::score::{BigramLm, ScoreContext, ZhLm};
use jc_core::session::neural::{NeuralEngine, NeuralError, NeuralRequest, NeuralRuntime};
use jc_core::session::SessionConfig;
use jc_core::Mode;
use jc_data::manifest::{CheckStatus, DataManifest, DataRole, VerifyLevel, MANIFEST_FILE};
use jc_model::candle::jinen::Jinen;

/// 注音詞典資料包目錄（相對於資料根目錄）。
pub const ZH_DIR: &str = "build-zh";

/// 注音 bigram 語言模型目錄（相對於資料根目錄）。
pub const ZH_BIGRAM_DIR: &str = "build-zh-bigram";

/// 預設的日文詞典 pack（含 Mozc context 連接矩陣的 v2）。
pub const DEFAULT_JA_PACK: &str = "packs/ja-lexicon-mozc-v2.pack";

/// 找不到 v2 時退回的 v1 pack（只有詞性連接成本）。
pub const FALLBACK_JA_PACK: &str = "packs/ja-lexicon-public-core.pack";

/// 預設的 Jinen small GGUF（相對於資料根目錄）。
pub const JA_NEURAL_GGUF: &str = "models/external/jinen-v2-small/jinen-v2-small-Q4_K_M.gguf";

/// 覆寫 Jinen GGUF 路徑的環境變數。
pub const JA_NEURAL_GGUF_ENV: &str = "JC_JINEN_GGUF";

/// 覆寫 Jinen tokenizer 路徑的環境變數。
pub const JA_NEURAL_TOKENIZER_ENV: &str = "JC_JINEN_TOKENIZER";

/// Jinen tokenizer 的候選路徑，依序嘗試。
pub const JA_NEURAL_TOKENIZERS: [&str; 3] = [
    "models/external/jinen-v2-small/tokenizer.json",
    "models/external/jinen-v2-xsmall/tokenizer.json",
    "models/quant/jinen-v2-xsmall/tokenizer.json",
];

/// Jinen 推論一次最多產生幾個 token，與離線評測相同。
pub const JA_NEURAL_MAX_NEW_TOKENS: usize = jc_model::candle::jinen::DEFAULT_MAX_NEW_TOKENS;

/// 關閉引擎預設神經轉換的環境變數；值為 `off` 時完全不載入模型。
pub const JA_NEURAL_ENV: &str = "JC_JINEN";

/// Jinen 推論的執行緒數。
///
/// 180 ms 的預算下 8 threads 是量測到的甜蜜點：4 threads 在 ajimee-200 有
/// 34／148 次逾時，20 threads 會因為 candle 的 barrier pool 過度競爭而
/// 全部逾時。宿主已設定環境變數時尊重宿主的選擇。
pub const JA_NEURAL_THREADS: usize = 8;

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
    /// 固定詞典；`Arc` 讓所有 session 共用同一份，不逐個複製。
    pub lexicon: Arc<InMemoryLexicon>,
    /// 成本設定（含 bigram 內插權重）。
    pub ctx: ScoreContext,
    /// bigram 語言模型；`None` 表示只有詞典（缺少 bigram 資料）。
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
        lexicon: Arc::new(lexicon),
        ctx,
        lm: bigram.map(Arc::new),
    })
}

/// 載入注音二進位資料包（`lexicon.bin`、`lm.bin`、`manifest.json`）。
///
/// `lm.bin` 必須包含 bigram；unigram-only 的舊資料包會被拒絕。
pub fn load_zh_bin_resources(data_dir: &Path) -> Result<ZhResources, LoadError> {
    load_zh_bin_resources_with_lm(data_dir, None)
}

/// 載入注音二進位資料包，`lm_path` 可覆寫資料包內的 `lm.bin`。
///
/// 未指定 `lm_path` 時讀 `data_dir/lm.bin`；檔案不存在則只載入詞典
/// （`lm = None`）。存在的語言模型一定要有 bigram。
pub fn load_zh_bin_resources_with_lm(
    data_dir: &Path,
    lm_path: Option<&Path>,
) -> Result<ZhResources, LoadError> {
    let lm = match lm_path {
        Some(path) => Some(ZhLm::open(path).map_err(|err| LoadError::read(path, err))?),
        None => {
            let path = data_dir.join("lm.bin");
            if path.is_file() {
                Some(ZhLm::open(&path).map_err(|err| LoadError::read(&path, err))?)
            } else {
                None
            }
        }
    };
    load_zh_bin_lexicon(data_dir, lm)
}

/// 只載入注音二進位詞典，不接語言模型。
///
/// `build-zh-bigram/lm.bin` 缺席時，引擎走這條路徑維持詞典-only。
pub fn load_zh_bin_lexicon_resources(data_dir: &Path) -> Result<ZhResources, LoadError> {
    load_zh_bin_lexicon(data_dir, None)
}

fn load_zh_bin_lexicon(data_dir: &Path, lm: Option<ZhLm>) -> Result<ZhResources, LoadError> {
    let manifest =
        jc_data::zh::Manifest::load(data_dir).map_err(|err| LoadError::read(data_dir, err))?;
    let lexicon_info = manifest.file("lexicon.bin").ok_or_else(|| {
        LoadError::new(format!(
            "{} 的 manifest 缺少 lexicon.bin",
            data_dir.display()
        ))
    })?;
    let lexicon_bin = data_dir.join("lexicon.bin");
    let source = jc_data::zh::Lexicon::open_verified(&lexicon_bin, lexicon_info)
        .map_err(|err| LoadError::read(&lexicon_bin, err))?;
    let mut lexicon = InMemoryLexicon::new();
    for view in source.entries() {
        let unigram = lm.as_ref().map_or(1.0e-6, |lm| lm.uni(view.surface));
        let cost = probability_cost(view.prw as f64).saturating_add(probability_cost(unigram));
        lexicon.push(LexEntry::new(
            view.surface,
            source.reading(view.index),
            cost,
            0,
        ));
    }
    if lexicon.is_empty() {
        return Err(LoadError(format!("{} 沒有任何詞條", lexicon_bin.display())));
    }
    let ctx = ScoreContext {
        unknown_cost: 30_000,
        ..ScoreContext::default()
    };
    Ok(ZhResources {
        lexicon: Arc::new(lexicon),
        ctx,
        lm: lm.map(Arc::new),
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
    entries: Vec<JaEntry<'static>>,
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
                surface: fields[0].to_string().into(),
                reading: fields[1].to_string().into(),
                pos: fields[3].to_string().into(),
                conj_type: fields.get(5).copied().unwrap_or("-").to_string().into(),
                conj_form: "-".to_string().into(),
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
                        entry: self.entries[id as usize].clone(),
                        left_id: None,
                        right_id: None,
                    });
                }
            }
        }
    }

    fn exact<'a>(&'a self, reading: &[char]) -> Vec<JaEntry<'a>> {
        let key: String = reading.iter().collect();
        self.index
            .get(&key)
            .map(|ids| {
                ids.iter()
                    .map(|&id| self.entries[id as usize].clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prefixed<'a>(&'a self, reading: &[char]) -> Vec<JaEntry<'a>> {
        if reading.is_empty() {
            return self.entries.clone();
        }
        let prefix: String = reading.iter().collect();
        self.index
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .flat_map(|(_, ids)| ids.iter().map(|&id| self.entries[id as usize].clone()))
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
///
/// 助詞讀音交替一律開啟，`jc-engine` 與 `jc-cli` 共用同一條路徑。
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
    let decoder =
        JaDecoder::new(Arc::new(JaDictionary::new(Arc::new(lex)))).with_particle_rewrite(true);
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
///
/// 與 pack 路徑一樣開啟助詞讀音交替。
pub fn load_curated_decoder(words: &Path, connection: &Path) -> Result<JaDecoder, LoadError> {
    let source = TsvWordSource::load(words)?;
    let mut decoder = JaDecoder::new(Arc::new(source)).with_particle_rewrite(true);
    if connection.is_file() {
        let cost =
            SingleColumnCost::load(connection).map_err(|err| LoadError::read(connection, err))?;
        decoder = decoder.with_connection(Arc::new(cost));
    }
    Ok(decoder)
}

/// 解析 Jinen 的 GGUF 與 tokenizer。
///
/// GGUF 順序：`JC_JINEN_GGUF`、資料 manifest 的 `ja_neural`、
/// [`JA_NEURAL_GGUF`]。
/// tokenizer 順序：`JC_JINEN_TOKENIZER`、資料 manifest 的 `ja_tokenizer`、
/// GGUF 同目錄的 `tokenizer.json`、[`JA_NEURAL_TOKENIZERS`]。
/// 找不到時回傳帶所有候選路徑的錯誤，呼叫端據此降級成統計轉換。
pub fn resolve_ja_neural(root: &Path) -> Result<(PathBuf, PathBuf), LoadError> {
    let gguf = if let Some(value) = std::env::var_os(JA_NEURAL_GGUF_ENV) {
        PathBuf::from(value)
    } else {
        manifest_file(root, DataRole::JaNeural)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| root.join(JA_NEURAL_GGUF))
    };
    if !gguf.is_file() {
        return Err(LoadError(format!(
            "找不到 Jinen 模型：{}；可用 {JA_NEURAL_GGUF_ENV} 指定",
            gguf.display()
        )));
    }
    if let Some(value) = std::env::var_os(JA_NEURAL_TOKENIZER_ENV) {
        let tokenizer = PathBuf::from(value);
        if !tokenizer.is_file() {
            return Err(LoadError(format!(
                "找不到 Jinen tokenizer：{}",
                tokenizer.display()
            )));
        }
        return Ok((gguf, tokenizer));
    }
    let mut candidates = Vec::new();
    if let Some(path) = manifest_file(root, DataRole::JaTokenizer) {
        candidates.push(path);
    }
    if let Some(dir) = gguf.parent() {
        candidates.push(dir.join("tokenizer.json"));
    }
    candidates.extend(JA_NEURAL_TOKENIZERS.iter().map(|path| root.join(path)));
    for tokenizer in &candidates {
        if tokenizer.is_file() {
            return Ok((gguf, tokenizer.clone()));
        }
    }
    let listed: Vec<String> = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    Err(LoadError(format!(
        "找不到 Jinen tokenizer，試過：{}；可用 {JA_NEURAL_TOKENIZER_ENV} 指定",
        listed.join("、")
    )))
}

/// 從資料根目錄 manifest 找某個角色的第一個檔案。
///
/// manifest 缺席、格式錯誤或沒有該角色時回傳 `None`，呼叫端改用寫死的
/// 候選路徑。這裡只做路徑解析，不驗證檔案是否存在或雜湊。
fn manifest_file(root: &Path, role: DataRole) -> Option<PathBuf> {
    let manifest = DataManifest::load(root.join(MANIFEST_FILE)).ok()?;
    manifest
        .files
        .iter()
        .find(|file| file.role == role)
        .map(|file| root.join(&file.path))
}

/// 載入 Jinen 並啟動推論 worker。
///
/// 模型缺席或載入失敗時回傳錯誤；呼叫端記一次警告後維持統計轉換即可。
pub fn load_ja_neural(root: &Path) -> Result<Arc<NeuralRuntime>, LoadError> {
    let (gguf, tokenizer) = resolve_ja_neural(root)?;
    // candle 的執行緒數必須在任何張量運算之前決定。
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        std::env::set_var("RAYON_NUM_THREADS", JA_NEURAL_THREADS.to_string());
    }
    if std::env::var_os("CANDLE_NUM_THREADS").is_none() {
        std::env::set_var("CANDLE_NUM_THREADS", JA_NEURAL_THREADS.to_string());
    }
    let model = Jinen::load(&gguf, &tokenizer)
        .map_err(|err| LoadError(format!("載入 {} 失敗：{err}", gguf.display())))?;
    Ok(Arc::new(NeuralRuntime::spawn(Box::new(JinenEngine {
        model,
        max_new_tokens: JA_NEURAL_MAX_NEW_TOKENS,
    }))))
}

/// [`NeuralEngine`] 的 Jinen 實作；模型只活在 worker 執行緒裡。
struct JinenEngine {
    model: Jinen,
    max_new_tokens: usize,
}

impl NeuralEngine for JinenEngine {
    fn convert(&mut self, request: &NeuralRequest) -> Result<String, NeuralError> {
        self.model
            .generate(&request.reading, &request.left, self.max_new_tokens)
            .map(|generation| generation.text)
            .map_err(|err| NeuralError::Inference(err.to_string()))
    }
}

/// 載入日文 reranker；模型缺席或 ONNX Runtime 不可用時回傳錯誤。
pub fn load_ja_reranker(root: &Path) -> Result<Arc<NeuralReranker>, LoadError> {
    let models = ja_models_dir(root);
    NeuralReranker::load_from_models(&models)
        .map(Arc::new)
        .map_err(|err| LoadError(format!("{}：{err}", models.display())))
}

/// 取得 tw2s 轉換器。
///
/// 字典已經編進 `jc-core`（約 175KB，Apache-2.0），不再依賴資料根目錄的
/// `opencc/`：安裝版原本沒帶那幾個檔，導致「輸出簡體字」永遠是空殼。
/// `root` 只在要覆寫時使用——目前不覆寫，保留參數讓呼叫端不必改。
pub fn load_zhconv(_root: &Path) -> Result<Arc<jc_core::zhconv::Converter>, LoadError> {
    jc_core::zhconv::tw2s_embedded()
        .map_err(|err| LoadError(format!("內建 tw2s 字典無法載入：{err}")))
}

/// 載入並安裝注音選字模型。
///
/// 模型集合 manifest 缺席或仍指向未隨安裝包散布的 ONNX 時，退回固定的
/// `models/zh-select` 目錄。GGUF、`vocab.json` 與模型自己的 `manifest.json`
/// 都隨 MSI 安裝，因此這種情況仍可載入。
pub fn install_select_model(models: &Path) -> Result<(), LoadError> {
    let dir =
        jc_core::zh::select::resolve_model_dir(models).unwrap_or_else(|_| models.join("zh-select"));
    let model = jc_core::zh::select::SelectModel::load(&dir)
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
        zh_lexicon: Some(Arc::clone(&resources.lexicon)),
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
    /// 英文詞表；`None` 表示使用內建原型詞彙。
    pub english: Option<Arc<English>>,
    /// 日文解碼器；重排器已接上延遲載入包裝。
    pub ja: Arc<JaDecoder>,
    /// Jinen 推論 worker；模型缺席或 `JC_JINEN=off` 時為 `None`。
    pub ja_neural: Option<Arc<NeuralRuntime>>,
    /// OpenCC `tw2s` 轉換器；`opencc/` 缺席或壞掉時為 `None`。
    pub zhconv: Option<Arc<jc_core::zhconv::Converter>>,
}

impl std::fmt::Debug for EngineData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineData")
            .field("root", &self.root)
            .field("zh_entries", &self.zh.lexicon.len())
            .field(
                "english_words",
                &self.english.as_ref().map(|e| e.dictionary().len()),
            )
            .field(
                "has_bigram",
                &self.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0),
            )
            .field("ja_neural", &self.ja_neural.is_some())
            .field("output_simplified", &self.zhconv.is_some())
            .finish()
    }
}

impl EngineData {
    /// 為指定模式建立 session 設定。
    ///
    /// 日文預設接上 Jinen；模型缺席時 `runtime` 為 `None`，行為與純統計
    /// 完全相同。要完全關閉時讓 [`JA_NEURAL_ENV`] 為 `off`。
    pub fn session_config(&self, mode: Mode) -> SessionConfig {
        let mut config = zh_session_config(mode, &self.zh);
        config.english = self.english.clone();
        config.ja = Some(Arc::clone(&self.ja));
        config.ja_neural.runtime = self.ja_neural.clone();
        config.simplified = self.zhconv.clone();
        config
    }

    /// 在開始接受連線前把第一次輸入會用到的路徑先跑一遍。
    ///
    /// 冷啟動的第一鍵慢在三件事：第一次解碼、第一次 zh-select 推論、mmap
    /// 分頁。這裡依序量測並預熱：
    ///
    /// 1. 暫時移除 select 模型，用 `ㄨㄛˇ ㄞˋ ㄋㄧˇ ㄨㄛˇ` 跑一次完整注音解碼。
    /// 2. 把同一個模型裝回去，直接跑一次固定候選的推論。
    /// 3. 走訪詞典全部詞條，並讀過 `lm.bin` 每個記憶體頁。
    /// 4. 把模型裝回去，用同一個輸入再跑一次完整路徑（解碼＋nbest＋重排）。
    ///
    /// 回傳各段耗時；結果本身丟掉。固定輸入只用內建詞表一定有的字，模型
    /// 檔案壞掉時 `score` 會失敗，但不影響啟動。
    pub fn prewarm(&self, config: &SessionConfig) -> PrewarmStats {
        let started = Instant::now();
        let mut config = config.clone();
        // 預熱固定跑注音路徑；設定是日文或英文時也一樣，第一鍵的注音成本
        // 不分模式都存在。
        config.mode = Mode::Mixed;

        let model = jc_core::zh::select::installed();
        jc_core::zh::select::clear();
        let decode_started = Instant::now();
        let mut session = jc_core::session::Session::new(config.clone());
        // 四個音節，跟下面完整路徑與第一次 select 推論用同一個輸入。
        for ch in "ji394su3ji3".chars() {
            session.press(jc_core::Key::char(ch, false));
        }
        // 選字模型缺席時 `commit` 只取走統計首選；有模型時才會再跑一次
        // 重排。這裡的重點是解碼與詞格建置。
        let _ = session.commit();
        let decode = decode_started.elapsed();
        if let Some(model) = model {
            jc_core::zh::select::install_shared(model);
        }

        let select = match jc_core::zh::select::installed() {
            Some(model) => {
                // 四個音節，跟下面完整路徑用同一個輸入，讓第一次推論的
                // 張量形狀就是實際第一個較長輸入會用到的形狀。
                let reading = vec![
                    "ㄨㄛˇ".to_string(),
                    "ㄞˋ".to_string(),
                    "ㄋㄧˇ".to_string(),
                    "ㄨㄛˇ".to_string(),
                ];
                let select_started = Instant::now();
                let result = model.score("", &reading, &["我愛你我", "我愛你窩"]);
                let elapsed = select_started.elapsed();
                // 推論結果沒有用途，但不要讓最佳化把整段拿掉。
                if let Ok(scores) = result {
                    std::hint::black_box(scores);
                }
                elapsed
            }
            None => Duration::ZERO,
        };

        let lexicon_started = Instant::now();
        let mut touched = 0usize;
        for entry in self.zh.lexicon.entries() {
            touched = touched.wrapping_add(entry.surface.len() + entry.reading.len());
        }
        if let Some(lm) = &self.zh.lm {
            touched = touched.wrapping_add(lm.touch_pages());
        }
        std::hint::black_box(touched);
        let lexicon = lexicon_started.elapsed();

        // 最後把整條路徑接起來跑一次：解碼、nbest、select 重排都在同一個
        // session 裡。前面分開量過的兩段都已經暖了，這一步補的是第一次
        // 把它們串起來時的張量配置與候選準備成本。
        let pipeline_started = Instant::now();
        let mut session = jc_core::session::Session::new(config);
        // `我愛你我` 剛好四個音節，讓詞格、nbest 與 select 的張量形狀都跟
        // 實機上第一個較長的輸入一致。
        for ch in "ji394su3ji3".chars() {
            session.press(jc_core::Key::char(ch, false));
        }
        let _ = session.commit();
        let pipeline = pipeline_started.elapsed();

        PrewarmStats {
            decode,
            select,
            lexicon,
            pipeline,
            total: started.elapsed(),
        }
    }
}

/// 啟動預熱的各段耗時。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrewarmStats {
    /// 第一次注音解碼（不含 select 模型）。
    pub decode: Duration,
    /// 第一次 select 模型推論。
    pub select: Duration,
    /// 走訪詞典與 `lm.bin` mmap 分頁。
    pub lexicon: Duration,
    /// 接上 select 後跑一次完整路徑（解碼＋nbest＋重排）。
    pub pipeline: Duration,
    /// 全部預熱工作的總時間。
    pub total: Duration,
}

impl PrewarmStats {
    /// 給啟動日誌用的一行摘要。
    pub fn log_line(&self) -> String {
        format!(
            "預熱完成：decode={:.1}ms select={:.1}ms lexicon={:.1}ms 合計={:.1}ms",
            self.decode.as_secs_f64() * 1000.0,
            self.select.as_secs_f64() * 1000.0,
            self.lexicon.as_secs_f64() * 1000.0,
            self.total.as_secs_f64() * 1000.0
        )
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
    /// 資料根目錄的 `manifest.json` 無效。
    #[error("資料 manifest 無效：{0}")]
    Manifest(#[from] jc_data::manifest::ManifestError),
    /// 資料根目錄的 `manifest.json` 逐檔檢查失敗。
    #[error("資料 manifest 檢查失敗：{0}")]
    ManifestFiles(String),
}

/// 啟動時的 manifest 檢查：只讀 manifest、比對存在與大小。
///
/// 沒有 `manifest.json` 只警告；manifest 格式錯誤、隨附檔缺少或大小不符
/// 則拒絕啟動。下載項（`shipped: false`）與 Jinen tokenizer（`ja_tokenizer`）
/// 缺席只警告：相關功能自然降級，不擋啟動。
fn check_data_manifest(root: &Path) -> Result<(), EngineDataError> {
    let path = root.join(MANIFEST_FILE);
    if !path.is_file() {
        log::warn!(
            "找不到資料 manifest {}，跳過完整性檢查；可用 jc-cli data manifest 產生",
            path.display()
        );
        return Ok(());
    }
    let manifest = DataManifest::load(&path)?;
    let report = manifest.verify(root, VerifyLevel::Startup);
    let mut fatal = 0;
    let mut degraded = 0;
    for check in &report.checks {
        match &check.status {
            CheckStatus::Ok => {}
            CheckStatus::Missing if !check.shipped => {
                degraded += 1;
                log::warn!(
                    "下載項缺席：{}（{}），相關功能維持停用",
                    check.path,
                    check.role.as_str()
                );
            }
            CheckStatus::Missing if check.role == DataRole::JaTokenizer => {
                degraded += 1;
                log::warn!(
                    "Jinen tokenizer 缺席：{}，日文神經轉換停用，維持統計轉換",
                    check.path
                );
            }
            status => {
                log::error!(
                    "資料檔檢查失敗：{}（{}）：{status:?}",
                    check.path,
                    check.role.as_str()
                );
                fatal += 1;
            }
        }
    }
    if fatal > 0 {
        return Err(EngineDataError::ManifestFiles(format!(
            "{fatal} 個致命錯誤，通過 {}/{}",
            report.passed(),
            report.checks.len()
        )));
    }
    log::info!(
        "資料 manifest 檢查通過（通過 {}/{}，降級 {}，資料版本 {}）",
        report.passed(),
        report.checks.len(),
        degraded,
        manifest.version
    );
    Ok(())
}

/// 依資料根目錄載入引擎所需資料。
///
/// 分級：
/// - 注音 `lexicon.bin`、日文 pack 缺失或壞掉 → 啟動失敗。
/// - `build-zh-bigram/lm.bin` 是必要資料：存在的語言模型一定要有 bigram；
///   缺席時只記 info，引擎以詞典-only 啟動（沒有 unigram-only 退回路徑）。
/// - `english/` 缺席只記 info；目錄存在但驗證失敗記警告，退回內建原型。
/// - zh-select 存在就安裝；載入失敗只警告一次，維持統計排序。
/// - reranker 缺席或壞掉 → 記一次警告後繼續。
///
/// 不再探測 `models/ja-tokenizer/public_core_ja.model`：那是 reranker 與已移除的
/// 標點模型共用的分詞器，`EngineData` 沒有使用它。Jinen 的 tokenizer 由
/// [`resolve_ja_neural`] 解析，走資料 manifest 的 `ja_tokenizer` 或 Jinen 隨附檔。
pub fn load_engine_data(root: &Path) -> Result<EngineData, EngineDataError> {
    if !root.is_dir() {
        return Err(EngineDataError::MissingRoot(root.to_path_buf()));
    }
    // 只驗路徑存在與大小；SHA-256 留給 `jc-cli data verify`，避免每次啟動
    // 重算上百 MB。沒有 manifest 時只警告，讓舊資料根目錄仍可啟動。
    check_data_manifest(root)?;
    let zh_dir = root.join(ZH_DIR);
    let lexicon_bin = zh_dir.join("lexicon.bin");
    if !lexicon_bin.is_file() {
        return Err(EngineDataError::MissingZhLexicon { path: lexicon_bin });
    }
    let bigram_lm = root.join(ZH_BIGRAM_DIR).join("lm.bin");
    let zh = if bigram_lm.is_file() {
        let resources = load_zh_bin_resources_with_lm(&zh_dir, Some(&bigram_lm))?;
        log::info!("已載入注音 bigram：{}", bigram_lm.display());
        resources
    } else {
        log::info!(
            "未安裝注音 bigram {}，引擎以詞典-only 啟動",
            bigram_lm.display()
        );
        load_zh_bin_lexicon_resources(&zh_dir)?
    };
    // 英文詞表跟著引擎的資料根目錄走；Windows 安裝版不必設 JC_IMEDATA。
    let english_dir = root.join(ENGLISH_PACK_SUBDIR);
    let english = English::load_optional(&english_dir).map(Arc::new);
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
    // 日文預設接上 Jinen；缺席只警告一次，維持統計轉換。
    let neural_off = std::env::var(JA_NEURAL_ENV)
        .map(|value| value.eq_ignore_ascii_case("off"))
        .unwrap_or(false);
    let ja_neural = if neural_off {
        None
    } else {
        match load_ja_neural(root) {
            Ok(runtime) => {
                log::info!(
                    "已載入 Jinen 日文轉換：{}",
                    root.join(JA_NEURAL_GGUF).display()
                );
                Some(runtime)
            }
            Err(err) => {
                log::warn!("Jinen 不可用，日文維持統計轉換：{err}");
                None
            }
        }
    };
    // 簡體輸出用編進 jc-core 的 tw2s 字典；解析失敗只停用這個開關，不擋啟動。
    let zhconv = match load_zhconv(root) {
        Ok(converter) => {
            log::info!("已啟用內建 OpenCC tw2s（tw2s，約 175KB 編進二進位）");
            Some(converter)
        }
        Err(err) => {
            log::warn!("OpenCC tw2s 不可用，輸出簡體停用：{err}");
            None
        }
    };
    Ok(EngineData {
        root: root.to_path_buf(),
        zh,
        english,
        ja: Arc::new(decoder),
        ja_neural,
        zhconv,
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
        let bigram = jc_testdata::require_data!(ZH_BIGRAM_DIR);
        let bigram = load_zh_bin_resources(&bigram).unwrap();
        let bigram_lm = bigram.lm.expect("build-zh-bigram 有 lm.bin");
        assert!(bigram_lm.bigrams() > 0);
        assert!((bigram_lm.bi("我", "愛") - 0.0044).abs() < 0.001);
        assert_eq!(bigram.ctx.bigram_mu, 0.7);
    }

    #[test]
    fn unigram_only_bin_pack_is_rejected() {
        let Some(plain) = jc_testdata::path(ZH_DIR) else {
            eprintln!("缺少測試資料：{ZH_DIR}");
            return;
        };
        let err = load_zh_bin_resources(&plain).unwrap_err();
        assert!(
            err.to_string().contains("bigram"),
            "unigram-only 資料包要回報必須有 bigram：{err}"
        );
        // 詞典本身仍可單獨載入，供 bigram 缺席時降級。
        let lexicon_only = load_zh_bin_lexicon_resources(&plain).unwrap();
        assert!(lexicon_only.lm.is_none());
        assert!(!lexicon_only.lexicon.is_empty());
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
    fn default_pack_enables_particle_rewrite() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let decoder = load_ja_decoder(&root).unwrap();
        let results = decoder.nbest("わたしわ", &jc_core::ja::decode::JaContext::default(), 1);
        assert_eq!(
            results.first().map(|result| result.surface.as_str()),
            Some("私は")
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
        assert!(data.zhconv.is_some(), "資料根目錄的 opencc/ 應載入 tw2s");
        assert!(data.session_config(Mode::Mixed).simplified.is_some());
        let english = data
            .session_config(Mode::Mixed)
            .english
            .expect("資料根目錄的 english/ 應載入詞表");
        assert!(
            english.dictionary().len() > 1_000,
            "英文詞數 {}",
            english.dictionary().len()
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
    fn mixed_session_keeps_long_literal_run() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let data = load_engine_data(&root).unwrap();
        let config = data.session_config(Mode::Mixed);
        assert!(config.ja_live.enabled, "引擎預設要開 ja_live");
        assert!(config.ja.is_some(), "引擎路徑要有日文解碼器");

        let preedit = |keys: &str| {
            let mut session = jc_core::session::Session::new(data.session_config(Mode::Mixed));
            for ch in keys.chars() {
                session.press(jc_core::key::Key::char(ch, false));
            }
            session.snapshot().preedit
        };

        assert_eq!(preedit(&"z".repeat(70)), "z".repeat(70));
        assert_eq!(preedit(&("z".repeat(70) + "a")), "z".repeat(70) + "a");
        assert_eq!(preedit(&("z".repeat(70) + "ab")), "z".repeat(70) + "ab");
        assert_eq!(preedit(&("z".repeat(70) + ".a")), "z".repeat(70) + ".a");

        // 既有混打切分不得回歸。
        let rishi = preedit("rishireej/");
        assert!(
            rishi == "りしれ供" || rishi == "りしれㄍㄨㄥ",
            "rishireej/ 回歸：{rishi}"
        );
        assert_eq!(preedit("w96j0 "), "台灣");
        assert_eq!(preedit("ji394su3"), "我愛你");
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

    #[test]
    fn invalid_manifest_refuses_startup() {
        let dir =
            std::env::temp_dir().join(format!("jc-engine-bad-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            br#"{"schema":"0c.data-pack/9","version":"v","generated_at":"t","files":[]}"#,
        )
        .unwrap();
        let err = load_engine_data(&dir).unwrap_err();
        assert!(
            err.to_string().contains("manifest"),
            "schema 不符應拒絕啟動：{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 寫一份只含指定角色路徑的 manifest；內容足以通過格式驗證。
    fn write_manifest(dir: &Path, entries: &[(DataRole, &str)]) {
        let files = entries
            .iter()
            .map(|(role, path)| jc_data::manifest::DataFile {
                path: (*path).to_string(),
                bytes: 1,
                sha256: "0".repeat(64),
                role: *role,
                license: vec!["L-21".to_string()],
                shipped: true,
                url: None,
            })
            .collect();
        let manifest = DataManifest {
            schema: jc_data::manifest::SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-20T00:00:00Z".to_string(),
            files,
        };
        std::fs::write(dir.join(MANIFEST_FILE), manifest.to_json_pretty().unwrap()).unwrap();
    }

    #[test]
    fn manifest_resolves_jinen_tokenizer_path() {
        let dir = std::env::temp_dir().join(format!(
            "jc-engine-manifest-tokenizer-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let gguf_rel = "models/external/jinen-v2-small/jinen-v2-small-Q4_K_M.gguf";
        let tokenizer_rel = "custom/jinen/tokenizer.json";
        for rel in [gguf_rel, tokenizer_rel] {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        write_manifest(
            &dir,
            &[
                (DataRole::JaNeural, gguf_rel),
                (DataRole::JaTokenizer, tokenizer_rel),
            ],
        );
        let (gguf, tokenizer) = resolve_ja_neural(&dir).unwrap();
        assert_eq!(gguf, dir.join(gguf_rel));
        assert_eq!(
            tokenizer,
            dir.join(tokenizer_rel),
            "tokenizer 路徑要從 manifest 解析，不走寫死的候選清單"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_manifest_tokenizer_degrades_without_failing_startup() {
        let dir = std::env::temp_dir().join(format!(
            "jc-engine-missing-tokenizer-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gguf_rel = "models/external/jinen-v2-small/jinen-v2-small-Q4_K_M.gguf";
        let tokenizer_rel = "models/quant/jinen-v2-xsmall/tokenizer.json";
        let gguf = dir.join(gguf_rel);
        std::fs::create_dir_all(gguf.parent().unwrap()).unwrap();
        std::fs::write(&gguf, b"x").unwrap();
        // manifest 收了 tokenizer，但檔案已被移除。
        write_manifest(
            &dir,
            &[
                (DataRole::JaNeural, gguf_rel),
                (DataRole::JaTokenizer, tokenizer_rel),
            ],
        );
        assert!(
            check_data_manifest(&dir).is_ok(),
            "tokenizer 缺席只警告，不擋引擎啟動"
        );
        let err = resolve_ja_neural(&dir).unwrap_err();
        assert!(
            err.to_string().contains("Jinen tokenizer"),
            "解析失敗要回報缺 tokenizer：{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
