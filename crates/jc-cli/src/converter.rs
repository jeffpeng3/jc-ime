//! `jc-core` 轉換器的薄包裝，以及 `jc-eval::Converter` 的實作。
//!
//! 評測與 `convert` 子命令都走這裡，確保兩邊的 N-best 完全一致。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jc_core::candidate::Candidate;
use jc_core::decode::{Decoder, Locks};
use jc_core::ja::decode::{JaContext, JaDecoder};
use jc_core::lattice::{InMemoryLexicon, Lattice};
use jc_core::score::ScoreContext;
use jc_core::zh::syllable::class_of;
use jc_eval::Converter;
use serde_json::json;

use crate::cli::CliError;
use crate::data::{self, ZhResources};

/// 注音轉換器。
#[derive(Debug, Clone)]
pub struct ZhConverter {
    resources: ZhResources,
}

impl ZhConverter {
    /// 用固定詞典建立轉換器。
    pub fn new(resources: ZhResources) -> Self {
        Self { resources }
    }

    /// 前 `n` 個候選；讀音是空白分隔的注音音節。
    pub fn nbest(&self, reading: &str, n: usize) -> Vec<Candidate> {
        let syllables = jc_core::zh::syllable::parse_reading(reading);
        if syllables.is_empty() || n == 0 {
            return Vec::new();
        }
        let units: Vec<&str> = syllables.iter().map(|s| s.as_str()).collect();
        let Ok(lattice) = Lattice::build(&units, &self.resources.lexicon, &self.resources.ctx)
        else {
            return Vec::new();
        };
        let decoder = Decoder::new(&self.resources.lexicon, &self.resources.ctx);
        let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, n);
        hypotheses
            .into_iter()
            .map(|hypothesis| {
                Candidate::new(
                    hypothesis.text,
                    reading.to_string(),
                    hypothesis.cost,
                    jc_core::candidate::CandidateSource::Lexicon,
                )
            })
            .collect()
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
#[derive(Debug, Clone)]
pub struct JaConverter {
    decoder: Arc<JaDecoder>,
}

impl JaConverter {
    /// 用已載入的解碼器建立轉換器。
    pub fn new(decoder: Arc<JaDecoder>) -> Self {
        Self { decoder }
    }

    /// 前 `n` 條轉換結果。
    pub fn nbest(&self, reading: &str, n: usize) -> Vec<jc_core::decode::Conversion> {
        self.decoder.nbest(reading, &JaContext::default(), n)
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
        self.nbest(reading, top_n)
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
            .collect()
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
        JaConverter::new(Arc::new(decoder)),
    ))
}

/// `convert` 子命令：單句轉換並列出 N-best。
pub fn convert(
    testdata: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    reading: &str,
    n: usize,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let candidates = if is_zhuyin(reading) {
        let resources = match &data_dir {
            Some(dir) => data::load_zh_bin_resources(dir)?,
            None => {
                let dir = data::testdata_dir(testdata)?;
                data::load_zh_resources(&dir.join("golden/zh/data"))?
            }
        };
        ZhConverter::new(resources).nbest(reading, n)
    } else {
        let dir = data::testdata_dir(testdata)?;
        let decoder = data::load_ja_decoder(&data::data_root(&dir))?;
        JaConverter::new(Arc::new(decoder))
            .nbest(reading, n)
            .into_iter()
            .map(|conversion| {
                Candidate::new(
                    conversion.surface,
                    reading.to_string(),
                    conversion.cost,
                    jc_core::candidate::CandidateSource::Lexicon,
                )
            })
            .collect()
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
    fn bigram_pack_changes_nothing_in_the_unigram_cost_model() {
        let Some(base) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = ZhConverter::new(data::load_zh_bin_resources(&base).unwrap());
        let with_bigram = ZhConverter::new(data::load_zh_bin_resources(&bigram).unwrap());
        let plain_best = plain.convert("ㄨㄛˇ ㄞˋ", 5);
        let bigram_best = with_bigram.convert("ㄨㄛˇ ㄞˋ", 5);
        // lexicon.bin 兩包相同，成本只用 unigram 與 P(r|w)，所以排序不變。
        assert_eq!(plain_best, bigram_best);
    }
}
