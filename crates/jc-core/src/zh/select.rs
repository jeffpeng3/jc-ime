//! 注音選字模型（`zuin-phonetic-v1`）的輸入組裝與候選重排。
//!
//! 這個模型不是 SentencePiece。`vocab.json` 直接給字元與注音 base 的索引：
//! 字元 `i` 的 id 是 `i + 8`，注音 base `i` 的 id 是 `i + 2`。特殊 token
//! `1` 是 `[BOS]`、`2` 是 `[SEP]`、`3` 是 `[OUT]`、`4` 是 `[UNK]`。
//!
//! 一次評分的序列由三部分組成：
//!
//! 1. `[BOS]` + 已確認前文（最多 64 字，超過保留最右側）+ `[SEP]`。
//! 2. 讀音的 base／聲調節點。前 `n-1` 個音節共用；最後一個音節是分支起點。
//! 3. 每個候選字串的共用前綴樹。候選第 `i` 個字放在位置 `S + n + i - 1`。
//!
//! 最後一個音節會多一個輕聲分支，讓句尾助詞打錯聲調時仍能被評分。分支
//! 分數取最大值，與前身 `core/rerank` 和 `internal/neural/native/input` 相同。
//!
//! 推論是執行期能力：模型檔（`model.safetensors` 或 `model.gguf`）缺席時只
//! 保留輸入組裝與純函式，呼叫端退回統計排序。

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// 送進模型的前文上限，與 manifest 的 `confirmed_left_64` 相同。
pub const MAX_CONTEXT_RUNES: usize = 64;
/// 一次評分最多幾個候選。
pub const MAX_CANDIDATES: usize = 16;
/// 模型支援的最少音節數。
pub const MIN_SYLLABLES: usize = 2;
/// 模型支援的最多音節數。
pub const MAX_SYLLABLES: usize = 32;
/// 校準後的融合權重預設值。
///
/// dev 集（fuzzy 注音題）上沒有任何「統計錯、模型救回」的組，權重只能由
/// 破壞量決定；0.5 是掃描網格中破壞最少的一端。
pub const DEFAULT_SELECT_WEIGHT: f64 = 0.5;
/// 校準後的覆寫門檻預設值，單位是融合後的 nat。
///
/// 3.0 是任務網格中破壞最少（2/161）的點。dev 的救回數是 0，因此這個值
/// 只用來限制誤覆寫，不代表模型已可啟用。
pub const DEFAULT_SELECT_MARGIN: f64 = 3.0;
/// 舊版的安全下限，保留給既有呼叫端。
///
/// 這個常數已不是最終門檻；重排改用 [`SelectConfig::default()`] 的
/// [`DEFAULT_SELECT_MARGIN`]。
pub const MIN_OVERRIDE_MARGIN: f64 = 1.0;

/// 本引擎的注音選字重排設定。
///
/// 上游 manifest 的 `weight = 2.0`、`margin = 0` 是 0c 引擎分數尺度的值，
/// 換引擎後不可直接沿用。預設值來自 `zh-calibrate` 的掃描結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectConfig {
    /// 模型分數的係數：`fused = log_score + weight × model_score`。
    pub weight: f64,
    /// 覆寫統計首選所需的最小融合領先。
    pub margin: f64,
}

impl Default for SelectConfig {
    fn default() -> Self {
        Self {
            weight: DEFAULT_SELECT_WEIGHT,
            margin: DEFAULT_SELECT_MARGIN,
        }
    }
}

impl SelectConfig {
    /// 是否為合法的融合設定。
    pub fn is_valid(&self) -> bool {
        self.weight.is_finite()
            && self.weight > 0.0
            && self.margin.is_finite()
            && self.margin >= 0.0
    }
}

/// 選字模型的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum SelectError {
    /// 讀寫檔案失敗。
    #[error("I/O 失敗：{0}")]
    Io(#[from] std::io::Error),
    /// JSON 格式或欄位不合法。
    #[error("選字模型設定錯誤：{0}")]
    Config(String),
    /// 讀音或候選不符合模型契約。
    #[error("選字模型輸入不支援：{0}")]
    Unsupported(String),
    /// 候選含詞表外的字元。
    #[error("候選含詞表外的字元：{0:?}")]
    UnknownCharacter(char),
    /// 推論失敗或逾時。
    #[error("選字模型推論失敗：{0}")]
    Inference(String),
}

/// `vocab.json` 的內容。
#[derive(Debug, Clone)]
pub struct Vocabulary {
    chars: HashMap<char, i64>,
    bases: HashMap<String, i64>,
    dictionary_sha256: String,
}

#[derive(Deserialize)]
struct VocabFile {
    schema: String,
    characters: Vec<String>,
    bases: Vec<String>,
    #[serde(default)]
    dictionary_sha256: String,
}

impl Vocabulary {
    /// 讀取 `vocab.json`。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SelectError> {
        let raw = std::fs::read(path)?;
        Self::from_json(&raw)
    }

    /// 由 JSON 位元組建立詞表。
    pub fn from_json(raw: &[u8]) -> Result<Self, SelectError> {
        let file: VocabFile = serde_json::from_slice(raw)
            .map_err(|err| SelectError::Config(format!("vocab.json：{err}")))?;
        if file.schema != "zuin-phonetic-v1" {
            return Err(SelectError::Config(format!(
                "vocab schema 應為 zuin-phonetic-v1，實際為 {}",
                file.schema
            )));
        }
        let mut chars = HashMap::with_capacity(file.characters.len());
        for (index, text) in file.characters.iter().enumerate() {
            let mut runes = text.chars();
            let Some(ch) = runes.next() else {
                continue;
            };
            if runes.next().is_none() {
                chars.insert(ch, index as i64 + 8);
            }
        }
        let mut bases = HashMap::with_capacity(file.bases.len());
        for (index, base) in file.bases.iter().enumerate() {
            bases.insert(base.clone(), index as i64 + 2);
        }
        Ok(Self {
            chars,
            bases,
            dictionary_sha256: file.dictionary_sha256,
        })
    }

    /// 字元對應的 id；詞表沒有時回傳 `None`。
    pub fn char_id(&self, ch: char) -> Option<i64> {
        self.chars.get(&ch).copied()
    }

    /// 注音 base 對應的 id。
    pub fn base_id(&self, base: &str) -> Option<i64> {
        self.bases.get(base).copied()
    }

    /// 詞表宣告的詞典雜湊。
    pub fn dictionary_sha256(&self) -> &str {
        &self.dictionary_sha256
    }

    /// 字元數。
    pub fn char_count(&self) -> usize {
        self.chars.len()
    }

    /// 注音 base 數。
    pub fn base_count(&self) -> usize {
        self.bases.len()
    }
}

/// 模型 manifest 中與重排有關的設定。
#[derive(Debug, Clone)]
pub struct SelectManifest {
    /// 融合權重；分數 = 統計分數 + weight × 模型分數。
    pub weight: f64,
    /// 覆寫首選所需的最小領先差距。
    pub margin: f64,
    /// 輕聲擴充候選的懲罰。
    pub neutral_expand_penalty: f64,
    /// 容許最後音節改輕聲的 base。
    pub neutral_tolerant_last: Vec<String>,
    /// 套件是否允許等長前綴候選。
    pub allow_partial: bool,
    /// 最少音節數。
    pub min_syllables: usize,
    /// 最多音節數。
    pub max_syllables: usize,
    /// 最多候選數。
    pub max_candidates: usize,
    /// 套件宣告的詞典雜湊。
    pub dictionary_sha256: String,
}

#[derive(Deserialize)]
struct ManifestFile {
    schema: String,
    context_policy: String,
    allow_partial: bool,
    weight: f64,
    margin: f64,
    #[serde(default)]
    neutral_expand_penalty: f64,
    #[serde(default)]
    neutral_tolerant_last: Vec<String>,
    #[serde(default)]
    limits: LimitsFile,
    #[serde(default)]
    dictionary_sha256: String,
}

#[derive(Deserialize, Default)]
struct LimitsFile {
    #[serde(default)]
    min_syllables: Option<usize>,
    #[serde(default)]
    max_syllables: Option<usize>,
    #[serde(default)]
    max_candidates: Option<usize>,
}

impl SelectManifest {
    /// 讀取模型目錄的 `manifest.json`。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SelectError> {
        let raw = std::fs::read(path)?;
        Self::from_json(&raw)
    }

    /// 由 JSON 位元組建立設定。
    pub fn from_json(raw: &[u8]) -> Result<Self, SelectError> {
        let file: ManifestFile = serde_json::from_slice(raw)
            .map_err(|err| SelectError::Config(format!("manifest.json：{err}")))?;
        if file.schema != "zuin-sequence-runtime-v2" {
            return Err(SelectError::Config(format!(
                "manifest schema 應為 zuin-sequence-runtime-v2，實際為 {}",
                file.schema
            )));
        }
        if file.context_policy != "confirmed_left_64" {
            return Err(SelectError::Config(format!(
                "不支援的 context_policy：{}",
                file.context_policy
            )));
        }
        if !file.weight.is_finite() || file.weight <= 0.0 {
            return Err(SelectError::Config("weight 必須是正有限值".into()));
        }
        if !file.margin.is_finite() || file.margin < 0.0 {
            return Err(SelectError::Config("margin 必須是非負有限值".into()));
        }
        if !file.neutral_expand_penalty.is_finite() || file.neutral_expand_penalty < 0.0 {
            return Err(SelectError::Config(
                "neutral_expand_penalty 必須是非負有限值".into(),
            ));
        }
        Ok(Self {
            weight: file.weight,
            margin: file.margin,
            neutral_expand_penalty: file.neutral_expand_penalty,
            neutral_tolerant_last: file.neutral_tolerant_last,
            allow_partial: file.allow_partial,
            min_syllables: file.limits.min_syllables.unwrap_or(MIN_SYLLABLES),
            max_syllables: file.limits.max_syllables.unwrap_or(MAX_SYLLABLES),
            max_candidates: file.limits.max_candidates.unwrap_or(MAX_CANDIDATES),
            dictionary_sha256: file.dictionary_sha256,
        })
    }

    /// 最後一個音節是否要加輕聲分支。
    fn neutral_last(&self, last: &str) -> bool {
        if self.neutral_expand_penalty <= 0.0 || last.is_empty() {
            return false;
        }
        let last_rune = last.chars().last().unwrap_or(' ');
        if last_rune == '˙' {
            return false;
        }
        let base = strip_tone(last);
        self.neutral_tolerant_last
            .iter()
            .any(|candidate| strip_tone(candidate) == base)
    }
}

/// 一次評分請求。
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// 已確認的前文。
    pub context: &'a str,
    /// 每個音節的注音字串（可含聲調）。
    pub reading: &'a [String],
    /// 候選字串，每個候選的字數必須等於音節數。
    pub candidates: &'a [&'a str],
}

/// 組裝後的模型輸入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrays {
    /// 字元 id 序列。
    pub ids: Vec<i64>,
    /// 每個位置的注音 base id。
    pub bases: Vec<i64>,
    /// 每個位置的聲調 id。
    pub tones: Vec<i64>,
    /// 每個位置的節點種類（0 輸出、1 音節、2 候選字）。
    pub types: Vec<i64>,
    /// 位置索引。
    pub positions: Vec<i64>,
    /// `[L, L]` 的注意力遮罩，列優先。
    pub mask: Vec<bool>,
    /// 投影位置的序列索引。
    pub proj_pos: Vec<i64>,
    /// 每列候選在 `proj_pos` 的索引。
    pub target_node: Vec<Vec<i64>>,
    /// 每列候選的目標字元 id（已扣掉 6）。
    pub target_ids: Vec<Vec<i64>>,
    /// 每個分支的候選列數。
    pub branches: usize,
}

impl Arrays {
    /// 序列長度。
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// 是否為空序列。
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// 候選列數（分支數 × 候選數）。
    pub fn rows(&self) -> usize {
        self.target_node.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sound {
    base: i64,
    tone: i64,
}

/// 把一個注音字串拆成 base 與聲調 id。
fn sound_of(vocab: &Vocabulary, syllable: &str) -> Result<Sound, SelectError> {
    if syllable.is_empty() {
        return Err(SelectError::Unsupported("空音節".into()));
    }
    let (base, tone) = match syllable.chars().last() {
        Some('ˊ') => (&syllable[..syllable.len() - 'ˊ'.len_utf8()], 2),
        Some('ˇ') => (&syllable[..syllable.len() - 'ˇ'.len_utf8()], 3),
        Some('ˋ') => (&syllable[..syllable.len() - 'ˋ'.len_utf8()], 4),
        Some('˙') => (&syllable[..syllable.len() - '˙'.len_utf8()], 5),
        Some('?') => (&syllable[..syllable.len() - 1], 0),
        _ => (syllable, 1),
    };
    let base = vocab
        .base_id(base)
        .ok_or_else(|| SelectError::Unsupported(format!("詞表沒有注音 base {base}")))?;
    Ok(Sound { base, tone })
}

fn strip_tone(syllable: &str) -> &str {
    match syllable.chars().last() {
        Some(mark @ ('ˊ' | 'ˇ' | 'ˋ' | '˙')) => &syllable[..syllable.len() - mark.len_utf8()],
        Some('?') => &syllable[..syllable.len() - 1],
        _ => syllable,
    }
}

/// 依 manifest 與詞表組裝一次模型輸入。
///
/// 候選必須是合法 UTF-8、字數等於音節數、字元都在詞表內，且不得重複。
/// 任何一項不符就整筆回傳錯誤，呼叫端保留統計排序。
pub fn prepare(
    vocab: &Vocabulary,
    manifest: &SelectManifest,
    request: &Request<'_>,
) -> Result<Arrays, SelectError> {
    let n = request.reading.len();
    if n < manifest.min_syllables || n > manifest.max_syllables {
        return Err(SelectError::Unsupported(format!(
            "音節數 {n} 不在 {}..={}",
            manifest.min_syllables, manifest.max_syllables
        )));
    }
    if request.candidates.len() < 2 || request.candidates.len() > manifest.max_candidates {
        return Err(SelectError::Unsupported(format!(
            "候選數 {} 不在 2..={}",
            request.candidates.len(),
            manifest.max_candidates
        )));
    }
    let mut seen = std::collections::HashSet::with_capacity(request.candidates.len());
    let mut encoded: Vec<Vec<i64>> = Vec::with_capacity(request.candidates.len());
    for candidate in request.candidates {
        if !seen.insert(*candidate) {
            return Err(SelectError::Unsupported(format!("重複候選 {candidate:?}")));
        }
        let chars: Vec<char> = candidate.chars().collect();
        if chars.len() != n {
            return Err(SelectError::Unsupported(format!(
                "候選 {candidate:?} 有 {} 字，讀音有 {n} 個音節",
                chars.len()
            )));
        }
        let mut ids = Vec::with_capacity(n);
        for ch in chars {
            let id = vocab.char_id(ch).ok_or(SelectError::UnknownCharacter(ch))?;
            ids.push(id);
        }
        encoded.push(ids);
    }

    let mut sounds = Vec::with_capacity(n);
    for syllable in request.reading {
        sounds.push(sound_of(vocab, syllable)?);
    }

    let zero = Sound { base: 0, tone: 0 };
    let mut branches: Vec<Vec<Sound>> = vec![vec![sounds[n - 1], zero]];
    if manifest.neutral_last(&request.reading[n - 1]) {
        branches.push(vec![
            Sound {
                base: sounds[n - 1].base,
                tone: 5,
            },
            zero,
        ]);
    }

    let context_runes: Vec<char> = request.context.chars().collect();
    let start = context_runes.len().saturating_sub(MAX_CONTEXT_RUNES);
    let mut ids: Vec<i64> = Vec::new();
    let mut bases: Vec<i64> = Vec::new();
    let mut tones: Vec<i64> = Vec::new();
    let mut types: Vec<i64> = Vec::new();

    ids.push(1); // [BOS]
    bases.push(0);
    tones.push(0);
    types.push(0);
    for ch in &context_runes[start..] {
        ids.push(vocab.char_id(*ch).unwrap_or(4)); // [UNK]
        bases.push(0);
        tones.push(0);
        types.push(0);
    }
    ids.push(2); // [SEP]
    bases.push(0);
    tones.push(0);
    types.push(0);

    let shared = &sounds[..n - 1];
    for sound in shared {
        ids.push(0);
        bases.push(sound.base);
        tones.push(sound.tone);
        types.push(1);
    }
    let mut positions: Vec<i64> = (0..ids.len() as i64).collect();
    let seq_start = ids.len();
    let mut parent: HashMap<usize, usize> = HashMap::new();
    let mut nodes: Vec<HashMap<Vec<i64>, usize>> = Vec::with_capacity(branches.len());

    for branch in &branches {
        let mut prev: Option<usize> = None;
        for (k, sound) in branch.iter().enumerate() {
            let index = ids.len();
            if let Some(previous) = prev {
                parent.insert(index, previous);
            }
            prev = Some(index);
            let last = k + 1 == branch.len();
            ids.push(if last { 3 } else { 0 });
            bases.push(sound.base);
            tones.push(sound.tone);
            types.push(if last { 0 } else { 1 });
            positions.push((seq_start + k) as i64);
        }
        let output_start = seq_start + branch.len();
        let mut node: HashMap<Vec<i64>, usize> = HashMap::new();
        node.insert(Vec::new(), prev.expect("分支至少一個節點"));
        for candidate in &encoded {
            for i in 1..n {
                let key = candidate[..i].to_vec();
                if node.contains_key(&key) {
                    continue;
                }
                let previous = node[&candidate[..i - 1].to_vec()];
                let index = ids.len();
                parent.insert(index, previous);
                node.insert(key, index);
                ids.push(candidate[i - 1]);
                bases.push(0);
                tones.push(0);
                types.push(2);
                positions.push((output_start + i - 1) as i64);
            }
        }
        nodes.push(node);
    }

    let length = ids.len();
    let mut mask = vec![false; length * length];
    for q in 0..seq_start {
        for a in 0..=q {
            mask[q * length + a] = true;
        }
    }
    for q in seq_start..length {
        for a in 0..seq_start {
            mask[q * length + a] = true;
        }
        let mut a = q;
        loop {
            mask[q * length + a] = true;
            match parent.get(&a) {
                Some(next) => a = *next,
                None => break,
            }
        }
    }

    let mut proj_set: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    for node in &nodes {
        for value in node.values() {
            proj_set.insert(*value as i64);
        }
    }
    let proj_pos: Vec<i64> = proj_set.into_iter().collect();
    let mut index = HashMap::with_capacity(proj_pos.len());
    for (position, value) in proj_pos.iter().enumerate() {
        index.insert(*value, position as i64);
    }

    let mut target_node = Vec::with_capacity(nodes.len() * encoded.len());
    let mut target_ids = Vec::with_capacity(nodes.len() * encoded.len());
    for node in &nodes {
        for candidate in &encoded {
            let mut row = Vec::with_capacity(n);
            for i in 0..n {
                let key = candidate[..i].to_vec();
                row.push(index[&(node[&key] as i64)]);
            }
            target_node.push(row);
            target_ids.push(candidate.iter().map(|id| id - 6).collect());
        }
    }

    Ok(Arrays {
        ids,
        bases,
        tones,
        types,
        positions,
        mask,
        proj_pos,
        target_node,
        target_ids,
        branches: branches.len(),
    })
}

/// 把詞格的成本換成與模型同尺度的對數分數。
///
/// 詞典成本以千分之一 nat 為單位，因此 `log_score = -cost / 1000`。這與
/// 前身 `decode.Hypothesis.LogScore` 的換算一致。
pub fn cost_to_log_score(cost: i32) -> f64 {
    -f64::from(cost) / 1000.0
}

/// 依前身 `rerank.Choose` 的規則選出首選索引。
///
/// 融合分數是 `engine + weight * neural`。只有其他候選的融合分數超過原首選
/// `margin` 時才會回傳它的索引；否則保留 0。長度不符、權重或分數不是有限值
/// 時也回傳 0，讓呼叫端整筆回退。
pub fn choose(engine: &[f64], neural: &[f64], weight: f64, margin: f64) -> usize {
    if engine.is_empty() || engine.len() != neural.len() {
        return 0;
    }
    if !weight.is_finite() || weight <= 0.0 || !margin.is_finite() || margin < 0.0 {
        return 0;
    }
    let mut values = Vec::with_capacity(engine.len());
    let mut best = 0usize;
    for (index, (&engine, &neural)) in engine.iter().zip(neural.iter()).enumerate() {
        let value = engine + weight * neural;
        if !engine.is_finite() || !neural.is_finite() || !value.is_finite() {
            return 0;
        }
        values.push(value);
        if value > values[best] {
            best = index;
        }
    }
    if values[best] - values[0] > margin {
        best
    } else {
        0
    }
}

mod model {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Instant;

    use super::{choose, cost_to_log_score, prepare, Arrays, Request, SelectError};
    use super::{SelectConfig, SelectManifest, Vocabulary};

    static MODEL: OnceLock<Mutex<Option<Arc<SelectModel>>>> = OnceLock::new();

    /// 一次推論的統計。
    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    pub struct LatencyStats {
        /// 成功推論次數。
        pub calls: usize,
        /// 推論延遲 p50，毫秒。
        pub p50_ms: f64,
        /// 推論延遲 p95，毫秒。
        pub p95_ms: f64,
    }

    /// forward 的實作；fp32 與量化只差這一個欄位。
    enum Engine {
        /// candle fp32（`model.safetensors`）。
        Candle(jc_model::candle::zh::ZhSelect),
        /// candle 量化（`model.gguf`）。
        CandleQuant(jc_model::candle::zh::QuantZhSelect),
    }

    /// 已載入的注音選字模型。
    pub struct SelectModel {
        engine: Engine,
        vocab: Vocabulary,
        manifest: SelectManifest,
        durations: Mutex<Vec<f64>>,
    }

    impl SelectModel {
        /// 從模型目錄載入權重、`vocab.json` 與 `manifest.json`。
        ///
        /// 權重由 [`jc_model::candle::zh::weights_path`] 找：`model.safetensors`
        /// 走 fp32，`model.gguf` 走量化。
        pub fn load(dir: impl AsRef<Path>) -> Result<Self, SelectError> {
            let dir = dir.as_ref();
            let vocab = Vocabulary::load(dir.join("vocab.json"))?;
            let manifest = SelectManifest::load(dir.join("manifest.json"))?;
            if !manifest.dictionary_sha256.is_empty()
                && !vocab.dictionary_sha256().is_empty()
                && manifest.dictionary_sha256 != vocab.dictionary_sha256()
            {
                return Err(SelectError::Config(format!(
                    "manifest 與 vocab 的 dictionary_sha256 不符：{} vs {}",
                    manifest.dictionary_sha256,
                    vocab.dictionary_sha256()
                )));
            }
            let path = jc_model::candle::zh::weights_path(dir).ok_or_else(|| {
                SelectError::Inference(format!(
                    "{} 沒有 candle 權重（model.safetensors 或 model.gguf）",
                    dir.display()
                ))
            })?;
            let engine = if jc_model::candle::quant::is_gguf(&path) {
                Engine::CandleQuant(
                    jc_model::candle::QuantZhSelect::load(&path)
                        .map_err(|err| SelectError::Inference(err.to_string()))?,
                )
            } else {
                Engine::Candle(
                    jc_model::candle::ZhSelect::load(&path)
                        .map_err(|err| SelectError::Inference(err.to_string()))?,
                )
            };
            Ok(Self {
                engine,
                vocab,
                manifest,
                durations: Mutex::new(Vec::new()),
            })
        }

        /// 詞表。
        pub fn vocab(&self) -> &Vocabulary {
            &self.vocab
        }

        /// manifest 設定。
        pub fn manifest(&self) -> &SelectManifest {
            &self.manifest
        }

        /// 一次評分，回傳每個候選的模型分數（越高越好）。
        ///
        /// 任何候選不符合契約或推論逾時都回傳錯誤，呼叫端保留統計排序。
        pub fn score(
            &self,
            context: &str,
            reading: &[String],
            candidates: &[&str],
        ) -> Result<Vec<f64>, SelectError> {
            let arrays = prepare(
                &self.vocab,
                &self.manifest,
                &Request {
                    context,
                    reading,
                    candidates,
                },
            )?;
            let raw = self.run(&arrays)?;
            let count = candidates.len();
            if raw.len() != arrays.rows() || count == 0 || raw.len() % count != 0 {
                return Err(SelectError::Inference(format!(
                    "scores 長度 {} 與候選數 {count}、列數 {} 不符",
                    raw.len(),
                    arrays.rows()
                )));
            }
            let mut out = vec![f64::NEG_INFINITY; count];
            for (row, value) in raw.iter().enumerate() {
                let index = row % count;
                let value = f64::from(*value);
                if !value.is_finite() {
                    return Err(SelectError::Inference("分數不是有限值".into()));
                }
                if value > out[index] {
                    out[index] = value;
                }
            }
            Ok(out)
        }

        fn run(&self, arrays: &Arrays) -> Result<Vec<f32>, SelectError> {
            match &self.engine {
                Engine::Candle(model) => self.run_candle(arrays, |input| model.scores(input)),
                Engine::CandleQuant(model) => self.run_candle(arrays, |input| model.scores(input)),
            }
        }

        /// candle 的推論路徑；forward 無法中途停止，只量延遲。
        fn run_candle(
            &self,
            arrays: &Arrays,
            run: impl FnOnce(&jc_model::candle::ZhInput<'_>) -> jc_model::Result<Vec<f32>>,
        ) -> Result<Vec<f32>, SelectError> {
            let cols = arrays.target_node.first().map(Vec::len).unwrap_or(0);
            let target_node: Vec<i64> = arrays.target_node.iter().flatten().copied().collect();
            let target_ids: Vec<i64> = arrays.target_ids.iter().flatten().copied().collect();
            let input = jc_model::candle::ZhInput {
                seq: arrays.len(),
                ids: &arrays.ids,
                bases: &arrays.bases,
                tones: &arrays.tones,
                types: &arrays.types,
                positions: &arrays.positions,
                mask: &arrays.mask,
                proj_pos: &arrays.proj_pos,
                target_node: &target_node,
                target_ids: &target_ids,
                rows: arrays.rows(),
                cols,
            };
            let started = Instant::now();
            let result = run(&input);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let scores = result.map_err(|err| SelectError::Inference(err.to_string()))?;
            if let Ok(mut durations) = self.durations.lock() {
                durations.push(elapsed);
            }
            Ok(scores)
        }

        /// 推論延遲統計；還沒有成功推論時回傳預設值。
        pub fn latency_stats(&self) -> LatencyStats {
            let Ok(durations) = self.durations.lock() else {
                return LatencyStats::default();
            };
            if durations.is_empty() {
                return LatencyStats::default();
            }
            let mut sorted = durations.clone();
            sorted.sort_by(f64::total_cmp);
            LatencyStats {
                calls: sorted.len(),
                p50_ms: percentile(&sorted, 50.0),
                p95_ms: percentile(&sorted, 95.0),
            }
        }

        /// 對一組統計候選重排；回傳新的首選索引，沒有改動時回傳 `None`。
        pub fn rerank(
            &self,
            context: &str,
            reading: &[String],
            texts: &[&str],
            costs: &[i32],
        ) -> Result<Option<usize>, SelectError> {
            if texts.len() != costs.len() {
                return Err(SelectError::Unsupported("候選與成本數量不符".into()));
            }
            let scores = self.score(context, reading, texts)?;
            let engine: Vec<f64> = costs.iter().map(|cost| cost_to_log_score(*cost)).collect();
            let config = SelectConfig::default();
            let winner = choose(&engine, &scores, config.weight, config.margin);
            Ok((winner > 0).then_some(winner))
        }
    }

    fn percentile(sorted: &[f64], value: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let rank = (value / 100.0 * sorted.len() as f64).ceil() as usize;
        let index = rank.saturating_sub(1).min(sorted.len() - 1);
        sorted[index]
    }

    fn slot() -> &'static Mutex<Option<Arc<SelectModel>>> {
        MODEL.get_or_init(|| Mutex::new(None))
    }

    /// 安裝全域選字模型。之後的 [`crate::decode::Decoder::nbest`] 都會嘗試
    /// 用它重排注音候選。
    pub fn install(model: SelectModel) {
        install_shared(Arc::new(model));
    }

    /// 安裝已共用所有權的選字模型。
    ///
    /// 預熱需要暫時移除模型、跑一次不含選字的解碼，再把同一個模型裝回去；
    /// 重新載入權重會多花上百毫秒，所以另外留這個入口。
    pub fn install_shared(model: Arc<SelectModel>) {
        if let Ok(mut slot) = slot().lock() {
            *slot = Some(model);
        }
    }

    /// 移除全域選字模型。
    pub fn clear() {
        if let Ok(mut slot) = slot().lock() {
            *slot = None;
        }
    }

    /// 目前安裝的模型。
    pub fn installed() -> Option<Arc<SelectModel>> {
        slot().lock().ok().and_then(|slot| slot.clone())
    }

    /// 目前模型的延遲統計。
    pub fn latency_stats() -> Option<LatencyStats> {
        installed().map(|model| model.latency_stats())
    }

    /// 用已安裝的模型重排 N-best；沒有模型或不符合契約時原樣保留。
    ///
    /// 呼叫端是 [`crate::decode::Decoder::nbest`]。前文目前固定為空字串：
    /// 這個工作階段只保留尚未提交的組字，已提交的文字不回頭送進模型。
    pub fn rerank_nbest(
        hypotheses: &mut Vec<crate::decode::Hypothesis>,
        reading: &[String],
    ) -> bool {
        let Some(model) = installed() else {
            return false;
        };
        let manifest = model.manifest();
        if reading.len() < manifest.min_syllables || reading.len() > manifest.max_syllables {
            return false;
        }
        // 未知詞邊的字面就是讀音本身（例如「我ㄒㄧㄤˇ」），模型沒有這種候選。
        // 與前身 `zhGroups` 一樣跳過，不能因為一個無效候選就放棄整句。
        let vocab = model.vocab();
        let valid: Vec<usize> = hypotheses
            .iter()
            .enumerate()
            .filter(|(_, hypothesis)| {
                let chars: Vec<char> = hypothesis.text.chars().collect();
                chars.len() == reading.len() && chars.iter().all(|ch| vocab.char_id(*ch).is_some())
            })
            .map(|(index, _)| index)
            .collect();
        if valid.len() < 2 || valid.len() > manifest.max_candidates {
            return false;
        }
        let texts: Vec<&str> = valid
            .iter()
            .map(|index| hypotheses[*index].text.as_str())
            .collect();
        let Ok(scores) = model.score("", reading, &texts) else {
            return false;
        };

        // 無效候選保留中性 0 分，與前身 `ZhAdapter` 相同；比較基準仍是原始首選。
        let mut neural = vec![0.0; hypotheses.len()];
        for (position, index) in valid.iter().enumerate() {
            neural[*index] = scores[position];
        }
        let engine: Vec<f64> = hypotheses
            .iter()
            .map(|hypothesis| cost_to_log_score(hypothesis.cost))
            .collect();
        let config = SelectConfig::default();
        let winner = choose(&engine, &neural, config.weight, config.margin);
        if winner > 0 && winner < hypotheses.len() {
            let promoted = hypotheses.remove(winner);
            hypotheses.insert(0, promoted);
            true
        } else {
            false
        }
    }

    /// 由集合 manifest 找出 `zh-select` 角色目錄。
    pub fn resolve_model_dir(models_root: impl AsRef<Path>) -> Result<PathBuf, SelectError> {
        let set = jc_model::ModelSet::load(models_root)
            .map_err(|err| SelectError::Config(err.to_string()))?;
        set.model_dir(jc_model::Role::ZhSelect)
            .map(Path::to_path_buf)
            .ok_or_else(|| SelectError::Config("manifest 沒有可用的 zh-select 模型".into()))
    }
}

pub use model::{
    clear, install, install_shared, installed, latency_stats, rerank_nbest, resolve_model_dir,
    LatencyStats, SelectModel,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        let raw = br#"{
            "schema": "zuin-phonetic-v1",
            "characters": ["\u4e00", "\u4e8c", "\u4e09", "\u56db", "\u4e94", "\u516d", "\u4e03", "\u516b", "\u4e5d", "\u5341", "\u3042", "\u3044"],
            "bases": ["a", "b"],
            "dictionary_sha256": "x"
        }"#;
        Vocabulary::from_json(raw).unwrap()
    }

    fn manifest() -> SelectManifest {
        SelectManifest {
            weight: 2.0,
            margin: 0.0,
            neutral_expand_penalty: 0.0,
            neutral_tolerant_last: Vec::new(),
            allow_partial: false,
            min_syllables: 2,
            max_syllables: 32,
            max_candidates: 16,
            dictionary_sha256: String::new(),
        }
    }

    #[test]
    fn test_prepare_matches_reference_arrays() {
        // 對照 ref/0c-ime internal/neural/native/input/testdata/golden.json
        // 的 basic 案例。
        let reading: Vec<String> = vec!["a".into(), "b".into()];
        let candidates = ["三四", "五あ"];
        let arrays = prepare(
            &vocab(),
            &manifest(),
            &Request {
                context: "一二",
                reading: &reading,
                candidates: &candidates,
            },
        )
        .unwrap();
        assert_eq!(arrays.ids, vec![1, 8, 9, 2, 0, 0, 3, 10, 12]);
        assert_eq!(arrays.bases, vec![0, 0, 0, 0, 2, 3, 0, 0, 0]);
        assert_eq!(arrays.tones, vec![0, 0, 0, 0, 1, 1, 0, 0, 0]);
        assert_eq!(arrays.types, vec![0, 0, 0, 0, 1, 1, 0, 2, 2]);
        assert_eq!(arrays.positions, vec![0, 1, 2, 3, 4, 5, 6, 7, 7]);
        assert_eq!(arrays.proj_pos, vec![6, 7, 8]);
        assert_eq!(arrays.target_node, vec![vec![0, 1], vec![0, 2]]);
        assert_eq!(arrays.target_ids, vec![vec![4, 5], vec![6, 12]]);
        assert_eq!(arrays.len(), 9);
        assert_eq!(arrays.rows(), 2);
        // 分支列：q=5 只看到自己以前的音節，q=6、7、8 另外看到整段前文。
        assert!(arrays.mask[5 * 9 + 5]);
        assert!(!arrays.mask[5 * 9 + 6]);
        assert!(arrays.mask[6 * 9]);
        assert!(arrays.mask[6 * 9 + 4]);
        assert!(arrays.mask[6 * 9 + 5]);
        assert!(arrays.mask[6 * 9 + 6]);
        assert!(!arrays.mask[6 * 9 + 7]);
        assert!(arrays.mask[7 * 9 + 5]);
        assert!(arrays.mask[7 * 9 + 7]);
    }

    #[test]
    fn test_neutral_branch_doubles_rows() {
        let reading: Vec<String> = vec!["a".into(), "b".into()];
        let candidates = ["一二", "三四"];
        let mut manifest = manifest();
        manifest.neutral_expand_penalty = 4.0;
        manifest.neutral_tolerant_last = vec!["b".into()];
        let arrays = prepare(
            &vocab(),
            &manifest,
            &Request {
                context: "",
                reading: &reading,
                candidates: &candidates,
            },
        )
        .unwrap();
        assert_eq!(arrays.branches, 2);
        assert_eq!(arrays.rows(), 4);
        // 第二個分支的起點是輕聲（tone 5）。
        assert!(arrays.tones.contains(&5));
    }

    #[test]
    fn test_choose_requires_margin() {
        let engine = [-7.0, -7.5];
        let neural = [0.0, 0.4];
        // -7.0 vs -7.5 + 2*0.4 = -6.7，超過 margin 0。
        assert_eq!(choose(&engine, &neural, 2.0, 0.0), 1);
        // 同樣的差距在 margin 0.5 下不覆寫。
        assert_eq!(choose(&engine, &neural, 2.0, 0.5), 0);
    }

    #[test]
    fn test_choose_rejects_invalid_shapes() {
        assert_eq!(choose(&[], &[], 2.0, 0.0), 0);
        assert_eq!(choose(&[0.0], &[0.0, 1.0], 2.0, 0.0), 0);
        assert_eq!(choose(&[0.0, 0.0], &[0.0, f64::NAN], 2.0, 0.0), 0);
        assert_eq!(choose(&[0.0, 0.0], &[0.0, 1.0], 0.0, 0.0), 0);
        assert_eq!(choose(&[0.0, 0.0], &[0.0, 1.0], 2.0, -1.0), 0);
    }

    #[test]
    fn test_observed_bad_override_is_blocked_by_default_margin() {
        // 「ㄔㄥˊ ㄕˋ」的實際分數：成本差 0.223、模型分差 0.4159195。
        let engine = [-7.6, -7.823];
        let neural = [-0.9937908053398132, -0.577910304069519];
        let config = SelectConfig::default();
        assert_eq!(choose(&engine, &neural, 2.0, 0.0), 1);
        assert_eq!(choose(&engine, &neural, config.weight, config.margin), 0);
    }

    #[test]
    fn test_select_config_defaults_are_calibrated() {
        let config = SelectConfig::default();
        assert_eq!(config.weight, 0.5);
        assert_eq!(config.margin, 3.0);
        assert!(config.is_valid());
    }

    #[test]
    fn test_cost_to_log_score_matches_predecessor() {
        assert_eq!(cost_to_log_score(0), 0.0);
        assert_eq!(cost_to_log_score(1000), -1.0);
        assert_eq!(cost_to_log_score(2500), -2.5);
    }

    #[test]
    fn test_prepare_rejects_unknown_character_and_length() {
        let reading: Vec<String> = vec!["a".into(), "b".into()];
        let err = prepare(
            &vocab(),
            &manifest(),
            &Request {
                context: "",
                reading: &reading,
                candidates: &["一X", "二二"],
            },
        )
        .unwrap_err();
        assert!(matches!(err, SelectError::UnknownCharacter('X')));

        let err = prepare(
            &vocab(),
            &manifest(),
            &Request {
                context: "",
                reading: &reading,
                candidates: &["一", "二二"],
            },
        )
        .unwrap_err();
        assert!(matches!(err, SelectError::Unsupported(_)));
    }

    #[test]
    fn test_real_vocab_has_expected_shape() {
        let path = jc_testdata::require_data!("models/zh-select/vocab.json");
        let vocab = Vocabulary::load(path).unwrap();
        assert_eq!(vocab.char_count(), 27_857);
        assert_eq!(vocab.base_count(), 416);
        assert_eq!(vocab.char_id('我'), Some(11839));
    }

    #[test]
    fn test_real_manifest_operating_point() {
        let path = jc_testdata::require_data!("models/zh-select/manifest.json");
        let manifest = SelectManifest::load(path).unwrap();
        assert_eq!(manifest.weight, 2.0);
        assert_eq!(manifest.margin, 0.0);
        assert_eq!(manifest.neutral_expand_penalty, 4.0);
        assert!(manifest.neutral_tolerant_last.contains(&"ㄇㄚ".to_string()));
    }
}
