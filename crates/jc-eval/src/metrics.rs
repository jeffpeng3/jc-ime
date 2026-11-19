//! 評測指標：top-1、top-5、詞邊界與延遲。
//!
//! 轉換器由外部提供，本模組只負責把候選結果換算成和 `0c-ime` 相同的指標。
//! 所有指標都是純函式計算，不讀取真實引擎。

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// 一個轉換結果與它的詞邊界。
///
/// `boundaries` 是候選字串內的字元 offset，代表詞與詞之間的位置；`0` 與字串
/// 結尾不算邊界。沒有邊界資訊時留空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// 轉換後的文字。
    pub surface: String,
    /// 詞邊界（以 `char` 計算）。
    pub boundaries: Vec<usize>,
}

impl Conversion {
    /// 建立帶邊界的轉換結果。
    pub fn new(surface: impl Into<String>, boundaries: Vec<usize>) -> Self {
        Self {
            surface: surface.into(),
            boundaries,
        }
    }

    /// 建立沒有邊界資訊的轉換結果。
    pub fn plain(surface: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            boundaries: Vec::new(),
        }
    }
}

/// 把一個讀音轉成候選字串。
///
/// 這個 crate 不實作注音或日文轉換；`jc-cli` 之後以 closure 或 wrapper 型別
/// 接上真正的引擎。
pub trait Converter {
    /// 回傳前 `top_n` 個候選，最佳者在前。
    fn convert(&self, reading: &str, top_n: usize) -> Vec<String>;

    /// 回傳候選與詞邊界。預設沿用 [`Converter::convert`]，不提供邊界。
    fn convert_segmented(&self, reading: &str, top_n: usize) -> Vec<Conversion> {
        self.convert(reading, top_n)
            .into_iter()
            .map(Conversion::plain)
            .collect()
    }
}

/// 一題評測輸入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalCase {
    /// 題目識別碼。
    pub id: String,
    /// 送進轉換器的讀音。
    pub reading: String,
    /// 主要參考答案。
    pub reference: String,
    /// 其他可接受的答案。
    pub accept: Vec<String>,
    /// 參考答案的詞切分；空表示這一題不計邊界。
    pub segments: Vec<String>,
}

impl EvalCase {
    /// 建立沒有可接受變體與詞切分的題目。
    pub fn new(
        id: impl Into<String>,
        reading: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            reading: reading.into(),
            reference: reference.into(),
            accept: Vec::new(),
            segments: Vec::new(),
        }
    }

    /// 加上可接受變體。
    pub fn with_accept(mut self, accept: Vec<String>) -> Self {
        self.accept = accept;
        self
    }

    /// 加上參考詞切分。
    pub fn with_segments(mut self, segments: Vec<String>) -> Self {
        self.segments = segments;
        self
    }

    /// 候選是否符合參考答案或任一可接受變體。
    pub fn matches(&self, candidate: &str) -> bool {
        candidate == self.reference || self.accept.iter().any(|x| x == candidate)
    }
}

/// 一次評測的參數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// 每次轉換取幾個候選。
    pub top_n: usize,
    /// 超過這個毫秒數就記為 timeout；只記錄，不取消轉換。
    pub timeout_ms: u64,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            top_n: 5,
            timeout_ms: 100,
        }
    }
}

impl RunOptions {
    /// 建立參數。
    pub fn new(top_n: usize, timeout_ms: u64) -> Self {
        Self {
            top_n: top_n.max(1),
            timeout_ms,
        }
    }
}

/// 單題結果，足以重現一次判斷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    /// 題目識別碼。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// 送進轉換器的讀音。
    pub reading: String,
    /// 參考答案。
    pub reference: String,
    /// 排名第一的輸出。
    pub output: String,
    /// 前 `top_n` 個候選。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top5: Vec<String>,
    /// 排名第一是否命中。
    pub top1: bool,
    /// 前五名是否命中。
    pub top5_hit: bool,
    /// 這一題是否計入邊界指標。
    pub boundary_scored: bool,
    /// 邊界集合是否完全相同。
    pub boundary_exact: bool,
    /// 邊界 true positive 數。
    pub boundary_tp: usize,
    /// 邊界 false positive 數。
    pub boundary_fp: usize,
    /// 邊界 false negative 數。
    pub boundary_fn: usize,
    /// 這次轉換花費的毫秒數。
    pub duration_ms: f64,
    /// 是否超過 timeout。
    pub timed_out: bool,
}

/// 一個題庫的聚合指標；欄位形狀與 `ja/baseline.json` 相同。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CorpusMetrics {
    /// 題數。
    pub n: usize,
    /// top-1 命中率。
    pub top1_rate: f64,
    /// top-5 命中率。
    pub top5_rate: f64,
    /// 計入邊界指標的題數。
    pub boundary_n: usize,
    /// 邊界集合完全相同的比例。
    pub boundary_exact_rate: f64,
    /// 邊界 micro-F1。
    pub boundary_f1: f64,
    /// 轉換延遲的 p50，僅供參考。
    #[serde(rename = "latency_p50_ms_informational")]
    pub latency_p50_ms: f64,
    /// 轉換延遲的 p95，僅供參考。
    #[serde(rename = "latency_p95_ms_informational")]
    pub latency_p95_ms: f64,
    /// timeout 比例，僅供參考。
    #[serde(rename = "timeout_rate_informational")]
    pub timeout_rate: f64,
}

/// 一個題庫的逐題結果與聚合指標。
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusEvaluation {
    /// 逐題結果，順序與題庫相同。
    pub items: Vec<CaseResult>,
    /// 聚合指標。
    pub metrics: CorpusMetrics,
}

/// 跑完一個題庫的所有題目。
pub fn evaluate(
    converter: &dyn Converter,
    cases: &[EvalCase],
    options: &RunOptions,
) -> CorpusEvaluation {
    let top_n = options.top_n.max(1);
    let timeout = Duration::from_millis(options.timeout_ms);
    let mut items = Vec::with_capacity(cases.len());
    let mut latencies = Vec::with_capacity(cases.len());

    let mut top1 = 0usize;
    let mut top5 = 0usize;
    let mut boundary_n = 0usize;
    let mut boundary_exact = 0usize;
    let mut tp = 0usize;
    let mut fp = 0usize;
    let mut fnn = 0usize;
    let mut timeouts = 0usize;

    for case in cases {
        let started = Instant::now();
        let conversions = converter.convert_segmented(&case.reading, top_n);
        let elapsed = started.elapsed();
        let duration_ms = round_ms(elapsed);
        latencies.push(duration_ms);
        let timed_out = elapsed > timeout;
        if timed_out {
            timeouts += 1;
        }

        let mut result = CaseResult {
            id: case.id.clone(),
            reading: case.reading.clone(),
            reference: case.reference.clone(),
            output: String::new(),
            top5: Vec::new(),
            top1: false,
            top5_hit: false,
            boundary_scored: false,
            boundary_exact: false,
            boundary_tp: 0,
            boundary_fp: 0,
            boundary_fn: 0,
            duration_ms,
            timed_out,
        };

        if let Some(first) = conversions.first() {
            result.output = first.surface.clone();
            result.top1 = case.matches(&first.surface);
            result.top5 = conversions
                .iter()
                .take(5)
                .map(|c| c.surface.clone())
                .collect();
            result.top5_hit = result.top5.iter().any(|c| case.matches(c));

            if !case.segments.is_empty() {
                result.boundary_scored = true;
                let got = predicted_boundaries(first);
                let want = segment_boundaries(&case.segments);
                let (case_tp, case_fp, case_fn) = boundary_counts(&got, &want);
                result.boundary_tp = case_tp;
                result.boundary_fp = case_fp;
                result.boundary_fn = case_fn;
                result.boundary_exact = case_fp == 0 && case_fn == 0;
            }
        }

        if result.top1 {
            top1 += 1;
        }
        if result.top5_hit {
            top5 += 1;
        }
        if result.boundary_scored {
            boundary_n += 1;
            if result.boundary_exact {
                boundary_exact += 1;
            }
            tp += result.boundary_tp;
            fp += result.boundary_fp;
            fnn += result.boundary_fn;
        }
        items.push(result);
    }

    let n = cases.len();
    let precision = ratio(tp, tp + fp);
    let recall = ratio(tp, tp + fnn);
    let metrics = CorpusMetrics {
        n,
        top1_rate: ratio(top1, n),
        top5_rate: ratio(top5, n),
        boundary_n,
        boundary_exact_rate: ratio(boundary_exact, boundary_n),
        boundary_f1: harmonic(precision, recall),
        latency_p50_ms: percentile(&latencies, 50.0),
        latency_p95_ms: percentile(&latencies, 95.0),
        timeout_rate: ratio(timeouts, n),
    };

    CorpusEvaluation { items, metrics }
}

/// 由參考詞切分算出邊界集合；最後一段的結尾不算邊界。
pub fn segment_boundaries(segments: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for (i, segment) in segments.iter().enumerate() {
        pos += segment.chars().count();
        if i + 1 < segments.len() {
            out.push(pos);
        }
    }
    out
}

/// 比較兩組邊界，回傳 `(tp, fp, fn)`。
pub fn boundary_counts(got: &[usize], want: &[usize]) -> (usize, usize, usize) {
    let tp = got.iter().filter(|p| want.contains(p)).count();
    let fp = got.len() - tp;
    let fnn = want.iter().filter(|p| !got.contains(p)).count();
    (tp, fp, fnn)
}

/// 最近排名分位數；空輸入回傳 0。
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

/// 調和平均；兩者皆 0 時回傳 0。
pub fn harmonic(a: f64, b: f64) -> f64 {
    if a + b == 0.0 {
        0.0
    } else {
        2.0 * a * b / (a + b)
    }
}

/// `a / b`；`b` 為 0 時回傳 0。
pub fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

/// 以字元為單位的編輯距離。
pub fn edit_distance(a: &str, b: &str) -> usize {
    let x: Vec<char> = a.chars().collect();
    let y: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=y.len()).collect();
    let mut cur = vec![0usize; y.len() + 1];
    for i in 1..=x.len() {
        cur[0] = i;
        for j in 1..=y.len() {
            let cost = if x[i - 1] == y[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[y.len()]
}

/// 把句子在非漢字處切開，只留長度至少 `min` 的純漢字片段。
pub fn chunks(text: &str, min: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for r in text.chars() {
        if is_han(r) {
            current.push(r);
        } else {
            flush_chunk(&mut out, &mut current, min);
        }
    }
    flush_chunk(&mut out, &mut current, min);
    out
}

/// 是否為漢字。
pub fn is_han(r: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&r) || ('\u{3400}'..='\u{4dbf}').contains(&r)
}

fn flush_chunk(out: &mut Vec<String>, current: &mut String, min: usize) {
    if current.chars().count() >= min && !current.is_empty() {
        out.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn predicted_boundaries(conversion: &Conversion) -> Vec<usize> {
    let surface: Vec<char> = conversion.surface.chars().collect();
    let mut out: Vec<usize> = conversion
        .boundaries
        .iter()
        .copied()
        .filter(|&b| b > 0 && b < surface.len() && !is_punct_boundary(surface[b]))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// 詞邊界落在標點前時不計分；標點不是文節。
fn is_punct_boundary(r: char) -> bool {
    matches!(
        r,
        '。' | '、'
            | '，'
            | '．'
            | '！'
            | '？'
            | '!'
            | '?'
            | ','
            | '.'
            | ';'
            | ':'
            | '：'
            | '；'
            | '「'
            | '」'
            | '『'
            | '』'
            | '（'
            | '）'
            | '('
            | ')'
    )
}

fn round_ms(duration: Duration) -> f64 {
    duration.as_micros() as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<&'static str>);

    impl Converter for Fixed {
        fn convert(&self, _reading: &str, top_n: usize) -> Vec<String> {
            self.0
                .iter()
                .take(top_n)
                .map(|s| (*s).to_string())
                .collect()
        }
    }

    struct Segmented(Vec<(&'static str, Vec<usize>)>);

    impl Converter for Segmented {
        fn convert(&self, _reading: &str, top_n: usize) -> Vec<String> {
            self.0
                .iter()
                .take(top_n)
                .map(|(s, _)| (*s).to_string())
                .collect()
        }

        fn convert_segmented(&self, _reading: &str, top_n: usize) -> Vec<Conversion> {
            self.0
                .iter()
                .take(top_n)
                .map(|(surface, boundaries)| Conversion::new(*surface, boundaries.clone()))
                .collect()
        }
    }

    #[test]
    fn top1_and_top5_use_rank_positions() {
        let cases = vec![EvalCase::new("a", "r", "A"), EvalCase::new("b", "r", "B")];
        let ev = evaluate(&Fixed(vec!["A", "B"]), &cases, &RunOptions::default());
        assert_eq!(ev.metrics.n, 2);
        assert_eq!(ev.metrics.top1_rate, 0.5);
        assert_eq!(ev.metrics.top5_rate, 1.0);
        assert!(ev.items[0].top1);
        assert!(!ev.items[1].top1);
        assert!(ev.items[1].top5_hit);
        assert_eq!(ev.items[1].top5, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn accepted_variant_counts_as_hit() {
        let case = EvalCase::new("a", "r", "今日").with_accept(vec!["きょう".to_string()]);
        let ev = evaluate(&Fixed(vec!["きょう"]), &[case], &RunOptions::default());
        assert_eq!(ev.metrics.top1_rate, 1.0);
        assert!(ev.items[0].top1);
    }

    #[test]
    fn boundary_exact_and_f1() {
        let case = EvalCase::new("a", "r", "私は本を読む。").with_segments(vec![
            "私は".to_string(),
            "本を".to_string(),
            "読む。".to_string(),
        ]);
        let exact = evaluate(
            &Segmented(vec![("私は本を読む。", vec![2, 4])]),
            std::slice::from_ref(&case),
            &RunOptions::default(),
        );
        assert_eq!(exact.metrics.boundary_n, 1);
        assert_eq!(exact.metrics.boundary_exact_rate, 1.0);
        assert_eq!(exact.metrics.boundary_f1, 1.0);

        let shifted = evaluate(
            &Segmented(vec![("私は本を読む。", vec![1, 2])]),
            std::slice::from_ref(&case),
            &RunOptions::default(),
        );
        assert_eq!(shifted.items[0].boundary_tp, 1);
        assert_eq!(shifted.items[0].boundary_fp, 1);
        assert_eq!(shifted.items[0].boundary_fn, 1);
        assert!(!shifted.items[0].boundary_exact);
        assert!((shifted.metrics.boundary_f1 - 0.5).abs() < 1e-12);
    }

    #[test]
    fn boundary_before_punctuation_is_ignored() {
        let conversion = Conversion::new("本を読む。", vec![1, 4]);
        let got = predicted_boundaries(&conversion);
        assert_eq!(got, vec![1]);
    }

    #[test]
    fn segment_boundaries_exclude_final_offset() {
        let segments = vec!["私わ".to_string(), "大雞巴".to_string(), "です".to_string()];
        assert_eq!(segment_boundaries(&segments), vec![2, 5]);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        assert_eq!(percentile(&values, 50.0), 50.0);
        assert_eq!(percentile(&values, 99.0), 99.0);
        assert_eq!(percentile(&values, 100.0), 100.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn edit_distance_matches_go() {
        assert_eq!(edit_distance("親子丼", "親子景"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("麻婆豆腐", "嘛婆鬥"), 3);
        assert_eq!(edit_distance("一樣", "一樣"), 0);
    }

    #[test]
    fn chunks_keep_han_runs() {
        let got = chunks("我想吃麻婆豆腐，但是API壞了。好", 2);
        assert_eq!(got, vec!["我想吃麻婆豆腐", "但是", "壞了"]);
    }

    #[test]
    fn empty_candidates_score_zero() {
        let case = EvalCase::new("a", "r", "A");
        let ev = evaluate(&Fixed(vec![]), &[case], &RunOptions::default());
        assert_eq!(ev.metrics.top1_rate, 0.0);
        assert_eq!(ev.items[0].output, "");
    }
}
