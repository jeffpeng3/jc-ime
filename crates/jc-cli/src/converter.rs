//! `jc-core` 轉換器的薄包裝，以及 `jc-eval::Converter` 的實作。
//!
//! 評測與 `convert` 子命令都走這裡，確保兩邊的 N-best 完全一致。

use std::io::Write;
use std::path::{Path, PathBuf};

use jc_core::candidate::Candidate;
use jc_core::decode::{Decoder, Locks};
use jc_core::ja::decode::{mean_candidates, JaContext, JaDecoder};
use jc_core::ja::personal::{
    learn_unified, UnifiedLearning, UNIFIED_MANUAL_WEIGHT, UNIFIED_PASSIVE_WEIGHT,
};
use jc_core::lattice::{InMemoryLexicon, Lattice};
use jc_core::personal::PersonalStore;
use jc_core::score::ScoreContext;
use jc_core::session::neural::{maybe_convert, NeuralConfig, NeuralDecision, NeuralInput};
use jc_core::zh::syllable::class_of;
use jc_eval::Converter;
use serde_json::json;

use crate::cli::CliError;
use crate::data::{self, ZhResources};

/// 注音轉換器。
#[derive(Debug, Clone)]
pub struct ZhConverter {
    resources: ZhResources,
    personal: PersonalStore,
}

/// 一次轉換的 bigram 查詢統計。
///
/// 用來證明 bigram 真的有被查，而不是只有輸出剛好不同。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LmQueryStats {
    /// 查詢次數。
    pub queries: u64,
    /// 查到 bigram 的次數。
    pub hits: u64,
}

impl ZhConverter {
    /// 用固定詞典建立轉換器。
    pub fn new(resources: ZhResources) -> Self {
        Self {
            resources,
            personal: PersonalStore::new(),
        }
    }

    /// 接上個人化學習資料；候選排序會套用單字手選與自訂詞。
    pub fn with_personal(mut self, personal: PersonalStore) -> Self {
        self.personal = personal;
        self
    }

    /// 設定 bigram 內插權重 μ；照前身 `personal.Params.Mu`。
    pub fn with_mu(mut self, mu: f64) -> Self {
        self.resources.ctx.bigram_mu = mu;
        self
    }

    /// 前 `n` 個候選；讀音是空白分隔的注音音節。
    pub fn nbest(&self, reading: &str, n: usize) -> Vec<Candidate> {
        self.nbest_with_stats(reading, n).0
    }

    /// 與 [`ZhConverter::nbest`] 相同，另回傳 bigram 查詢統計。
    pub fn nbest_with_stats(&self, reading: &str, n: usize) -> (Vec<Candidate>, LmQueryStats) {
        let syllables = jc_core::zh::syllable::parse_reading(reading);
        if syllables.is_empty() || n == 0 {
            return (Vec::new(), LmQueryStats::default());
        }
        let units: Vec<&str> = syllables.iter().map(|s| s.as_str()).collect();
        let Ok(lattice) = Lattice::build(&units, &self.resources.lexicon, &self.resources.ctx)
        else {
            return (Vec::new(), LmQueryStats::default());
        };
        let scorer = self
            .resources
            .lm
            .as_deref()
            .filter(|lm| lm.bigrams() > 0)
            .map(|lm| jc_core::score::LmScorer::new(&self.resources.ctx, lm));
        let (hypotheses, stats) = match scorer.as_ref() {
            Some(scorer) => {
                let mut decoder =
                    Decoder::new(&self.resources.lexicon, &self.resources.ctx).with_scorer(scorer);
                decoder.beam = jc_core::decode::DEFAULT_BEAM;
                let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, n);
                let stats = LmQueryStats {
                    queries: scorer.queries(),
                    hits: scorer.hits(),
                };
                (hypotheses, stats)
            }
            None => {
                let decoder = Decoder::new(&self.resources.lexicon, &self.resources.ctx);
                let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, n);
                (hypotheses, LmQueryStats::default())
            }
        };
        let mut ranked: Vec<Candidate> = hypotheses
            .into_iter()
            .map(|hypothesis| {
                Candidate::new(
                    hypothesis.text,
                    reading.to_string(),
                    hypothesis.cost,
                    jc_core::candidate::CandidateSource::Lexicon,
                )
            })
            .collect();
        self.personal.rank_candidates(&mut ranked, reading);
        (ranked, stats)
    }
}

impl Converter for ZhConverter {
    fn convert(&self, reading: &str, top_n: usize) -> Vec<String> {
        self.nbest(reading, top_n)
            .into_iter()
            .map(|candidate| candidate.surface)
            .collect()
    }
}

/// 日文轉換器。
///
/// `rolling_left` 開啟時，每次轉換把上一次的輸出接到左文尾巴，模擬連續
/// 輸入的 IME。前身的 `lang.Context.Left` 由外殼提供；評測沒有外殼，所以
/// 這個模式只用來量測「有左文」與「沒有左文」的差異，預設關閉。
#[derive(Debug, Clone)]
pub struct JaConverter {
    decoder: JaDecoder,
    rolling_left: bool,
    left: std::cell::RefCell<String>,
    neural: NeuralConfig,
}

/// 左文保留的字元上限，與 `jc_core::session` 相同（前身 `MaxLeftContextUnits`）。
const MAX_LEFT_CONTEXT_UNITS: usize = 128;

impl JaConverter {
    /// 用已載入的解碼器建立轉換器。
    pub fn new(decoder: JaDecoder) -> Self {
        Self {
            decoder,
            rolling_left: false,
            left: std::cell::RefCell::new(String::new()),
            neural: NeuralConfig::default(),
        }
    }

    /// 接上日文神經轉換設定；沒有 runtime 時與統計路徑完全相同。
    pub fn with_neural(mut self, neural: NeuralConfig) -> Self {
        self.neural = neural;
        self
    }

    /// 神經轉換的累計統計；模型未載入時為 `None`。
    pub fn neural_stats(&self) -> Option<jc_core::session::NeuralStatsSnapshot> {
        self.neural.runtime.as_ref().map(|runtime| runtime.stats())
    }

    /// 開啟滾動左文：把每次的第一名接到下一次轉換的左文。
    pub fn with_rolling_left(mut self) -> Self {
        self.rolling_left = true;
        self
    }

    /// 這次轉換要用的上下文。
    fn context(&self) -> JaContext {
        if !self.rolling_left {
            return JaContext::default();
        }
        JaContext {
            left: self.left.borrow().clone(),
            right: String::new(),
        }
    }

    /// 把已送出的輸出接到左文尾巴。
    fn remember(&self, surface: &str) {
        if !self.rolling_left || surface.is_empty() {
            return;
        }
        let mut left = self.left.borrow_mut();
        left.push_str(surface);
        let len = left.chars().count();
        if len > MAX_LEFT_CONTEXT_UNITS {
            *left = left.chars().skip(len - MAX_LEFT_CONTEXT_UNITS).collect();
        }
    }

    /// 前 `n` 條轉換結果。
    ///
    /// 統計結果永遠保留；第一名只有在成本差命中門檻、且輸出通過五條
    /// 驗證時才換成 Jinen 的結果。
    pub fn nbest(&self, reading: &str, n: usize) -> Vec<jc_core::decode::Conversion> {
        if n == 0 {
            return Vec::new();
        }
        let ctx = self.context();
        let chars: Vec<char> = reading.chars().collect();
        // 一次解到至少兩條：候選清單與 gating 的 top-2 共用同一次搜尋。
        let (results, gap) = self.decoder.results_with_gap(&chars, &ctx, n.max(2));
        let Some(best) = results.first() else {
            return Vec::new();
        };
        let mut conversions: Vec<jc_core::decode::Conversion> = results
            .iter()
            .take(n)
            .map(|result| result.conversion())
            .collect();
        let baseline = best.surface();
        // 門檻只用統計轉換已經有的資訊；平均候選數在這裡一次算好，
        // 不在等候模型的路徑上重新解碼或查詞典。
        let needs_neural = self.neural.enabled && self.neural.runtime.is_some();
        let statistical_cost = best.conversion().cost;
        let (runner_up_cost, mean) = if needs_neural {
            (
                gap.map(|gap| statistical_cost.saturating_add(gap)),
                mean_candidates(best, self.decoder.source().as_ref()),
            )
        } else {
            (results.get(1).map(|second| second.conversion().cost), 1.0)
        };
        let decision = maybe_convert(
            &self.neural,
            &NeuralInput {
                reading,
                left: &ctx.left,
                right: &ctx.right,
                statistical: &baseline,
                statistical_cost,
                runner_up_cost,
                mean_candidates: mean,
            },
        );
        if let NeuralDecision::Accepted { surface } = decision {
            // 神經輸出沒有詞邊界；清空 segments，只留下整體字串。
            if let Some(first) = conversions.first_mut() {
                first.surface = surface;
                first.segments.clear();
            }
        }
        conversions
    }
}

impl Converter for JaConverter {
    fn convert(&self, reading: &str, top_n: usize) -> Vec<String> {
        self.nbest(reading, top_n)
            .into_iter()
            .map(|conversion| conversion.surface)
            .collect()
    }

    fn convert_segmented(&self, reading: &str, top_n: usize) -> Vec<jc_eval::Conversion> {
        let conversions: Vec<jc_eval::Conversion> = self
            .nbest(reading, top_n)
            .into_iter()
            .map(|conversion| {
                let mut boundaries = Vec::new();
                for segment in conversion
                    .segments
                    .iter()
                    .take(conversion.segments.len().saturating_sub(1))
                {
                    boundaries.push(conversion.surface[..segment.end].chars().count());
                }
                jc_eval::Conversion::new(conversion.surface, boundaries)
            })
            .collect();
        // 統計第一名就是這一輪送出的文字，接到下一次的左文。
        if let Some(first) = conversions.first() {
            self.remember(&first.surface);
        }
        conversions
    }
}

/// 依讀音自動選注音或日文的轉換器。
#[derive(Debug, Clone)]
pub struct AutoConverter {
    /// 注音轉換器。
    pub zh: ZhConverter,
    /// 日文轉換器。
    pub ja: JaConverter,
}

impl AutoConverter {
    /// 建立兩語轉換器。
    pub fn new(zh: ZhConverter, ja: JaConverter) -> Self {
        Self { zh, ja }
    }
}

/// 讀音是否含注音符號（決定走注音或日文轉換）。
pub fn is_zhuyin(reading: &str) -> bool {
    reading
        .chars()
        .any(|ch| class_of(ch) != jc_core::zh::syllable::Class::None)
}

/// [`is_zhuyin`] 的別名，供其他模組以穩定名稱呼叫。
pub fn is_zhuyin_pub(reading: &str) -> bool {
    is_zhuyin(reading)
}

impl Converter for AutoConverter {
    fn convert(&self, reading: &str, top_n: usize) -> Vec<String> {
        if is_zhuyin(reading) {
            self.zh.convert(reading, top_n)
        } else {
            self.ja.convert(reading, top_n)
        }
    }

    fn convert_segmented(&self, reading: &str, top_n: usize) -> Vec<jc_eval::Conversion> {
        if is_zhuyin(reading) {
            self.zh.convert_segmented(reading, top_n)
        } else {
            self.ja.convert_segmented(reading, top_n)
        }
    }
}

/// 由 `0c-testdata` 目錄載入兩語轉換器。
pub fn load_auto(testdata: &Path) -> Result<AutoConverter, CliError> {
    let zh_dir = testdata.join("golden/zh/data");
    let resources = data::load_zh_resources(&zh_dir)?;
    let root = data::data_root(testdata);
    let decoder = data::load_ja_decoder(&root)?;
    Ok(AutoConverter::new(
        ZhConverter::new(resources),
        JaConverter::new(decoder),
    ))
}

/// `convert` 子命令的參數。
///
/// 集中成一個型別，避免轉換函式堆滿位置參數；欄位順序與用法都照
/// [`crate::cli::Command::Convert`]。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConvertOptions {
    /// `0c-testdata` 目錄。
    pub testdata: Option<PathBuf>,
    /// 注音二進位資料包目錄。
    pub data: Option<PathBuf>,
    /// 覆寫的 bigram 語言模型。
    pub lm: Option<PathBuf>,
    /// bigram 內插權重 μ。
    pub mu: Option<f64>,
    /// 印出 bigram 查詢統計。
    pub stats: bool,
    /// 取幾個候選。
    pub n: usize,
    /// 輸出 JSON。
    pub json: bool,
    /// 啟用注音選字模型重排。
    pub select_model: bool,
    /// 推論後端；`None` 時讀 `JC_RERANK_BACKEND` 與預設 `ort`。
    pub backend: Option<jc_model::Backend>,
    /// 個人化學習資料目錄；未指定時不載入。
    pub user_data: Option<PathBuf>,
    /// 指定轉換模式；目前只有 `en` 會走英文候選。
    pub mode: Option<jc_core::Mode>,
    /// 日文轉換後要把第幾個候選記進學習（1 起算）；未指定時記首選。
    pub pick: Option<usize>,
}

/// `convert` 子命令：單句轉換並列出 N-best。
///
/// `--data` 指向注音二進位資料包時會自動載入同目錄的 `lm.bin`；`--lm`
/// 可單獨覆寫語言模型。`--mu` 照前身 `personal.Params.Mu` 調整 bigram 內插；
/// `--stats` 印出 bigram 查詢次數與命中次數。
pub fn convert(
    options: ConvertOptions,
    reading: &str,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let ConvertOptions {
        testdata,
        data: data_dir,
        lm,
        mu,
        stats,
        n,
        json,
        select_model,
        backend,
        user_data,
        mode,
        pick,
    } = options;
    // 後端在載入任何模型前設定；沒有指定時讓 jc-model 讀
    // `JC_RERANK_BACKEND`，預設仍是 ort。
    if let Some(backend) = backend {
        jc_model::set_default_backend(backend);
    }
    if select_model {
        if let Err(err) = data::install_select_model(testdata.as_deref()) {
            eprintln!("警告：--select-model on 無法載入選字模型，維持統計排序：{err}");
        }
    } else {
        data::disable_select_model();
    }
    let english = mode == Some(jc_core::Mode::En);
    if pick.is_some() && (english || is_zhuyin(reading)) {
        return Err(CliError::Usage("--pick 只支援日文轉換".to_string()));
    }
    let (candidates, lm_stats) = if english {
        // `convert --mode en` 只列原字與拼字建議；前綴補完屬互動路徑，
        // 由 session 的候選清單負責。
        let mut out = vec![Candidate::new(
            reading.to_string(),
            reading.to_string(),
            0,
            jc_core::candidate::CandidateSource::Literal,
        )];
        let mut seen = vec![reading.to_lowercase()];
        for candidate in jc_core::mixed::default_english().spelling_candidates(reading, n - 1) {
            let key = candidate.surface.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            out.push(candidate);
        }
        out.truncate(n);
        (out, LmQueryStats::default())
    } else if is_zhuyin(reading) {
        let mut resources = match &data_dir {
            Some(dir) => data::load_zh_bin_resources_with_lm(dir, lm.as_deref())?,
            None => {
                let dir = data::testdata_dir(testdata)?;
                let golden = dir.join("golden/zh/data");
                match &lm {
                    Some(path) => data::load_zh_resources_with_lm(&golden, path)?,
                    None => data::load_zh_resources(&golden)?,
                }
            }
        };
        if let Some(mu) = mu {
            resources.ctx.bigram_mu = mu;
        }
        let personal = match &user_data {
            Some(dir) => data::load_personal(dir)?,
            None => PersonalStore::new(),
        };
        resources.lexicon = personal.augment_lexicon(&resources.lexicon);
        ZhConverter::new(resources)
            .with_personal(personal)
            .nbest_with_stats(reading, n)
    } else {
        let dir = data::testdata_dir(testdata)?;
        let decoder = data::load_ja_decoder(&data::data_root(&dir))?;
        // 個人資料載入失敗時只警告並用空白資料，不覆寫可能還能救的檔案。
        let mut personal = match &user_data {
            Some(dir) => match data::load_personal(dir) {
                Ok(store) => Some(store),
                Err(err) => {
                    eprintln!("警告：--user-data 載入個人資料失敗，這次不學習：{err}");
                    None
                }
            },
            None => None,
        };
        let ctx = JaContext::default();
        let chars: Vec<char> = reading.chars().collect();
        let mut results = decoder.results(&chars, &ctx, n.max(1));
        if let Some(store) = &personal {
            UnifiedLearning::new(store).rank_results(&mut results, &ctx.left);
        }
        if let (Some(store), Some(dir)) = (personal.as_mut(), user_data.as_ref()) {
            let index = pick.unwrap_or(1).saturating_sub(1);
            if let Some(chosen) = results.get(index) {
                let weight = if pick.is_some() {
                    UNIFIED_MANUAL_WEIGHT
                } else {
                    UNIFIED_PASSIVE_WEIGHT
                };
                learn_unified(store, &chosen.clauses, &ctx.left, weight, now_seconds());
                if let Err(err) = data::save_personal(dir, store) {
                    eprintln!("警告：無法寫回個人資料：{err}");
                }
            }
        }
        let candidates = results
            .iter()
            .map(|result| {
                let conversion = result.conversion();
                Candidate::new(
                    conversion.surface,
                    reading.to_string(),
                    conversion.cost,
                    jc_core::candidate::CandidateSource::Lexicon,
                )
            })
            .collect();
        (candidates, LmQueryStats::default())
    };
    if json {
        let rows: Vec<_> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                json!({
                    "rank": index + 1,
                    "surface": candidate.surface,
                    "cost": candidate.cost,
                })
            })
            .collect();
        if stats {
            let value = json!({
                "candidates": rows,
                "bigram_queries": lm_stats.queries,
                "bigram_hits": lm_stats.hits,
            });
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&value)
                    .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
            )?;
            return Ok(());
        }
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&rows)
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
        return Ok(());
    }
    writeln!(out, "rank  surface  cost")?;
    for (index, candidate) in candidates.iter().enumerate() {
        writeln!(
            out,
            "{:>4}  {:<8} {:>5}",
            index + 1,
            candidate.surface,
            candidate.cost
        )?;
    }
    if candidates.is_empty() {
        writeln!(out, "（沒有候選：讀音 {reading:?}）")?;
    }
    if stats {
        writeln!(
            out,
            "bigram queries={} hits={}",
            lm_stats.queries, lm_stats.hits
        )?;
    }
    if select_model {
        write_select_stats(out)?;
    }
    Ok(())
}

/// 目前 Unix 秒數；時鐘早於 epoch 時回傳 0。
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 印出選字模型的推論延遲。
fn write_select_stats(out: &mut dyn Write) -> Result<(), CliError> {
    let Some(stats) = data::select_model_stats() else {
        writeln!(out, "select-model：沒有推論")?;
        return Ok(());
    };
    writeln!(
        out,
        "select-model：推論 {} 次，p50 {:.2} ms，p95 {:.2} ms",
        stats.calls, stats.p50_ms, stats.p95_ms
    )?;
    Ok(())
}

/// 注音詞典的轉換器建構；供 golden 驅動直接使用。
pub fn zh_converter(testdata: &Path) -> Result<ZhConverter, CliError> {
    let resources = data::load_zh_resources(&testdata.join("golden/zh/data"))?;
    Ok(ZhConverter::new(resources))
}

/// 取用注音詞典，給 session 建立設定。
pub fn zh_resources(testdata: &Path) -> Result<ZhResources, CliError> {
    data::load_zh_resources(&testdata.join("golden/zh/data"))
}

/// 建立只含注音的 session 設定。
pub fn zh_config(
    testdata: &Path,
    mode: jc_core::Mode,
) -> Result<jc_core::session::SessionConfig, CliError> {
    let resources = zh_resources(testdata)?;
    Ok(data::zh_session_config(mode, &resources))
}

/// 轉換器使用的注音詞典型別（公開給測試建構）。
pub type Lexicon = InMemoryLexicon;

/// 轉換器使用的成本設定型別（公開給測試建構）。
pub type Costs = ScoreContext;

#[cfg(test)]
mod tests {
    use super::*;
    use jc_core::lattice::LexEntry;

    fn resources() -> ZhResources {
        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lexicon.push(LexEntry::new("想", "ㄒㄧㄤˇ", 0, 0));
        ZhResources {
            lexicon,
            ctx: ScoreContext::default(),
            lm: None,
        }
    }

    #[test]
    fn zh_converter_ranks_dictionary_words() {
        let converter = ZhConverter::new(resources());
        let got = converter.convert("ㄨㄛˇ ㄒㄧㄤˇ", 5);
        assert_eq!(got.first().map(String::as_str), Some("我想"));
    }

    #[test]
    fn zhuyin_detection_uses_bopomofo() {
        assert!(is_zhuyin("ㄨㄛˇ"));
        assert!(!is_zhuyin("わたし"));
        assert!(!is_zhuyin("ji3"));
    }

    #[test]
    fn real_zh_lexicon_converts_wo_ai() {
        let dir = jc_testdata::require_data!("build-zh");
        let resources = data::load_zh_bin_resources(&dir).unwrap();
        let converter = ZhConverter::new(resources);
        let got = converter.convert("ㄨㄛˇ ㄞˋ", 5);
        assert_eq!(got.first().map(String::as_str), Some("我愛"));
    }

    #[test]
    fn bigram_pack_boosts_common_word_pair() {
        let Some(base) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = ZhConverter::new(data::load_zh_bin_resources(&base).unwrap());
        let with_bigram = ZhConverter::new(data::load_zh_bin_resources(&bigram).unwrap());
        let plain_rows = plain.nbest("ㄨㄛˇ ㄞˋ", 5);
        let (bigram_rows, stats) = with_bigram.nbest_with_stats("ㄨㄛˇ ㄞˋ", 5);
        assert_eq!(plain_rows[0].surface, "我愛");
        assert_eq!(bigram_rows[0].surface, "我愛");
        // bigram 真的被查：至少查了 我→愛 與 句首→我 兩次，且 我→愛 命中。
        assert!(stats.queries >= 2, "{stats:?}");
        assert!(stats.hits >= 1, "{stats:?}");
        // 沒有 bigram 的資料包不會建立評分器，查詢次數是 0。
        assert_eq!(
            plain.nbest_with_stats("ㄨㄛˇ ㄞˋ", 5).1,
            LmQueryStats::default()
        );
        // 我→愛 的 bigram 把常用組合抬升，與第二名的差距變大。
        assert!(
            bigram_rows[0].cost < plain_rows[0].cost,
            "bigram 要把 我愛 抬升：{} vs {}",
            bigram_rows[0].cost,
            plain_rows[0].cost
        );
        let plain_gap = plain_rows[1].cost - plain_rows[0].cost;
        let bigram_gap = bigram_rows[1].cost - bigram_rows[0].cost;
        assert!(bigram_gap > plain_gap, "{bigram_gap} 應大於 {plain_gap}");
    }

    #[test]
    fn bigram_pack_reranks_homophones() {
        let Some(base) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = ZhConverter::new(data::load_zh_bin_resources(&base).unwrap());
        let with_bigram = ZhConverter::new(data::load_zh_bin_resources(&bigram).unwrap());
        let plain_rows = plain.nbest("ㄗㄞˋ ㄐㄧㄢˋ", 5);
        let bigram_rows = with_bigram.nbest("ㄗㄞˋ ㄐㄧㄢˋ", 5);
        assert_eq!(plain_rows[0].surface, "再見");
        assert_eq!(bigram_rows[0].surface, "再見");
        // 再→見 的 bigram 讓「在見」掉到「在建」後面。
        assert_eq!(plain_rows[1].surface, "在見");
        assert_eq!(bigram_rows[1].surface, "在建");
    }

    #[test]
    fn zero_mu_matches_unigram_costs() {
        let Some(base) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = ZhConverter::new(data::load_zh_bin_resources(&base).unwrap());
        let with_bigram =
            ZhConverter::new(data::load_zh_bin_resources(&bigram).unwrap()).with_mu(0.0);
        for reading in ["ㄨㄛˇ ㄞˋ", "ㄗㄞˋ ㄐㄧㄢˋ", "ㄋㄧˇ ㄏㄠˇ"] {
            let plain_rows = plain.nbest(reading, 5);
            let bigram_rows = with_bigram.nbest(reading, 5);
            let surfaces: Vec<&str> = plain_rows.iter().map(|row| row.surface.as_str()).collect();
            let other: Vec<&str> = bigram_rows.iter().map(|row| row.surface.as_str()).collect();
            let costs: Vec<i32> = plain_rows.iter().map(|row| row.cost).collect();
            let other_costs: Vec<i32> = bigram_rows.iter().map(|row| row.cost).collect();
            assert_eq!(surfaces, other, "{reading}");
            assert_eq!(costs, other_costs, "{reading}");
        }
    }
}
