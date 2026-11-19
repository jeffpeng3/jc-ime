//! 凍結題庫的評測工具：讀取題庫、跑轉換、輸出可比較的指標。
//!
//! 題庫格式沿用 `0c-ime` 的 `testdata/ja`、`testdata/eval`、注音 golden 與
//! 模糊輸入題庫，讓新舊引擎可以在同一批題目上逐句比對。
//!
//! * [`Converter`] 是唯一的轉換介面；這個 crate 不依賴 `jc-core`。
//! * [`evaluate`] 計算 top-1、top-5、詞邊界與延遲。
//! * [`Report`] 可以輸出成與 `ja/baseline.json` 相同形狀的 JSON 或表格。
//! * [`RuleSet::gate`] 依 `rules.json` 的容許量判定是否退步。

mod corpus;
mod error;
mod golden;
mod ja;
mod metrics;
mod report;
mod sha256;

pub use corpus::{
    load_fuzzy_input, load_ja_lexicon_ambiguous, load_zhuyin_golden, AmbiguousCandidate,
    FuzzyCorpus, FuzzyItem, FuzzyMutation, FuzzySourcePin, JaAmbiguousItem, JaLexiconAmbiguous,
    PackInfo, ZhuyinGolden, ZhuyinGoldenItem, ZhuyinSegment, ZhuyinSourcePin, CORPUS_SCHEMA,
    FUZZY_INPUT_KINDS, FUZZY_KINDS,
};
pub use error::{EvalError, Result};
pub use golden::{
    load_golden_mixed, load_golden_zh, GoldenAction, GoldenCase, GoldenMixedFile, GoldenMixedSuite,
    GoldenOptions, GoldenPunct, GoldenRerank, GoldenSeed, GoldenSuggest, GoldenVocab, GoldenZhFile,
    GoldenZhSuite, MixedAlternative, MixedCase, MixedLanguage, MixedPiece, MixedSegment, Span,
    GOLDEN_FORMAT, MIXED_GOLDEN_FORMAT,
};
pub use ja::{
    check_ja_corpus_provenance, load_baseline, load_ja_corpus, load_ja_corpus_set, load_rules,
    run_ja, CorpusRule, Gate, JaCase, JaCorpus, JaCorpusEntry, RuleSet, JA_SCHEMA,
};
pub use metrics::{
    boundary_counts, chunks, edit_distance, evaluate, harmonic, is_han, percentile, ratio,
    segment_boundaries, CaseResult, Conversion, Converter, CorpusEvaluation, CorpusMetrics,
    EvalCase, RunOptions,
};
pub use report::{CorpusRun, EngineInfo, GateResult, Report, Run, SCHEMA};
