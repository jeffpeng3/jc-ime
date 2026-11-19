//! 注音標點模型（需要 `onnx` feature）。
//!
//! 模型包是 `zuin-punct-runtime-v1`：`ids` 一個 int64 輸入，`probs` 輸出
//! `[seq][7]`。輸入格式是 `[BOS=1, TASK=7, 原字 id...]`，只取原字位置的
//! 機率。字元 id 是 `characters[i] + 8`，詞表外送 `[UNK]=4`。
//!
//! 插入規則照前身 `internal/neural/native/punct_plan.go` 與模型 README：
//! 只在漢字後、下一個字仍是漢字／空白時考慮，機率達門檻才插入，段落尾端
//! 永不插入。套用沿用 [`crate::punct::apply`] 的漢字邊界驗證。

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ort::session::{RunOptions, Session};
use ort::value::{Shape, Tensor};
use serde::Deserialize;

use crate::error::{ModelError, Result};
use crate::manifest::ModelSpec;
use crate::punct_plan::{apply, argmax, is_han, merge_window_rows, window_starts, Edit};
use crate::runtime::{build_session, init_runtime};
use crate::Role;

/// 模型 manifest 的 schema 名稱。
pub const SCHEMA: &str = "zuin-punct-runtime-v1";
/// 預設標點標籤；索引 0 是不插入。
pub const DEFAULT_LABELS: [&str; 7] = ["", "，", "。", "？", "！", "、", "；"];
/// 預設視窗大小。
pub const DEFAULT_WINDOW: usize = 128;
/// 預設視窗 stride。
pub const DEFAULT_STRIDE: usize = 64;
/// 預設機率門檻。
pub const DEFAULT_THRESHOLD: f32 = 0.63;
/// 單次要求的最大字數。
pub const DEFAULT_MAX_TEXT: usize = 256;
/// 詞表外的字元 id。
pub const UNK_ID: i64 = 4;

/// 模型 manifest 中本模組需要的欄位。
#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    threshold: Option<f32>,
    #[serde(default)]
    window: Option<usize>,
    #[serde(default)]
    stride: Option<usize>,
    #[serde(default)]
    max_text_runes: Option<usize>,
}

/// 注音標點模型的推論結果統計。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PunctZhStats {
    /// 呼叫次數。
    pub calls: u64,
    /// 真的插入標點的次數。
    pub inserted: u64,
}

/// 注音標點模型。
pub struct ZhPunctuationModel {
    session: Mutex<Session>,
    chars: HashMap<char, i64>,
    labels: Vec<String>,
    threshold: f32,
    window: usize,
    stride: usize,
    max_text: usize,
    timeout: Duration,
    stats: Mutex<PunctZhStats>,
}

impl std::fmt::Debug for ZhPunctuationModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZhPunctuationModel")
            .field("window", &self.window)
            .field("stride", &self.stride)
            .field("threshold", &self.threshold)
            .finish_non_exhaustive()
    }
}

impl ZhPunctuationModel {
    /// 以預設佈局載入角色目錄：`model.onnx`、`vocab.json`、`manifest.json`。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_spec(dir, &ModelSpec::default(), None)
    }

    /// 以指定設定與 ORT 動態庫路徑載入。
    pub fn load_with_spec(
        dir: impl AsRef<Path>,
        spec: &ModelSpec,
        ort_lib: Option<&Path>,
    ) -> Result<Self> {
        if spec.timeout().is_zero() {
            return Err(ModelError::Manifest("timeout 必須大於 0".into()));
        }
        init_runtime(ort_lib)?;
        let dir = dir.as_ref();
        let manifest = read_manifest(dir)?;
        let labels = if manifest.labels.len() >= 2 {
            manifest.labels.clone()
        } else {
            DEFAULT_LABELS.iter().map(|s| s.to_string()).collect()
        };
        let threshold = manifest
            .threshold
            .filter(|v| *v > 0.0 && *v < 1.0)
            .unwrap_or(DEFAULT_THRESHOLD);
        let window = manifest
            .window
            .filter(|v| (8..=DEFAULT_MAX_TEXT).contains(v))
            .unwrap_or(DEFAULT_WINDOW);
        let stride = manifest
            .stride
            .filter(|v| *v > 0 && *v <= window)
            .unwrap_or(DEFAULT_STRIDE.min(window));
        let max_text = manifest
            .max_text_runes
            .filter(|v| *v >= window)
            .unwrap_or(DEFAULT_MAX_TEXT.max(window));
        let chars = load_vocab(&dir.join(spec.vocab_file()))?;
        let model_path = resolve_model(dir, spec.model_file())?;
        let session = build_session(&model_path, spec.threads())?;
        Ok(Self {
            session: Mutex::new(session),
            chars,
            labels,
            threshold,
            window,
            stride,
            max_text,
            timeout: spec.timeout(),
            stats: Mutex::new(PunctZhStats::default()),
        })
    }

    /// 模型角色。
    pub fn role() -> Role {
        Role::ZhPunct
    }

    /// 視窗大小。
    pub fn window(&self) -> usize {
        self.window
    }

    /// 標點標籤。
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// 累計統計。
    pub fn stats(&self) -> PunctZhStats {
        *self
            .stats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// 在文字中插入標點；逾時、模型失敗或沒有合適邊界時回傳原文。
    pub fn punctuate(&self, text: &str) -> String {
        match self.try_punctuate(text) {
            Some(out) => out,
            None => text.to_string(),
        }
    }

    fn try_punctuate(&self, text: &str) -> Option<String> {
        let runes: Vec<char> = text.chars().collect();
        if runes.is_empty() {
            return Some(text.to_string());
        }
        if runes.len() > self.max_text {
            return Some(text.to_string());
        }
        let allowed = eligible_boundaries(&runes);
        if !allowed
            .iter()
            .take(runes.len().saturating_sub(1))
            .any(|v| *v)
        {
            return Some(text.to_string());
        }
        let starts = window_starts(runes.len(), self.window, self.stride);
        let mut rows = Vec::with_capacity(starts.len());
        for &start in &starts {
            let end = (start + self.window).min(runes.len());
            rows.push(self.run_chars(&runes[start..end])?);
        }
        let merged = merge_window_rows(runes.len(), self.window, &starts, &rows);
        let marks = &self.labels;
        let mut edits = Vec::new();
        for i in 0..runes.len().saturating_sub(1) {
            if !allowed[i] || !is_han(runes[i + 1]) {
                continue;
            }
            let Some(row) = merged[i].as_ref() else {
                continue;
            };
            if row.len() < 2 {
                continue;
            }
            let label = argmax(&row[1..]) + 1;
            let Some(mark) = marks.get(label) else {
                continue;
            };
            if mark.is_empty() || row[label] < self.threshold {
                continue;
            }
            // 句首的單一列舉字後面不補頓號，照模型 README。
            if i == 0 && mark == "、" && is_enumerator(runes[0]) {
                continue;
            }
            edits.push(Edit::new(i + 1, mark.clone()));
        }
        if edits.is_empty() {
            return Some(text.to_string());
        }
        let (out, ok) = apply(text, &edits, false, true);
        let mut stats = self
            .stats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        stats.calls += 1;
        if ok {
            stats.inserted += 1;
            Some(out)
        } else {
            Some(text.to_string())
        }
    }

    /// 跑一個字元視窗，回傳每個字元的標籤機率。
    fn run_chars(&self, src: &[char]) -> Option<Vec<Vec<f32>>> {
        if src.is_empty() {
            return Some(Vec::new());
        }
        let mut ids = Vec::with_capacity(src.len() + 2);
        ids.push(1i64);
        ids.push(7i64);
        for ch in src {
            ids.push(self.chars.get(ch).copied().unwrap_or(UNK_ID));
        }
        let seq = ids.len();
        let input = Tensor::from_array((Shape::new([1i64, seq as i64]), ids)).ok()?;
        let options = Arc::new(RunOptions::new().ok()?);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let watchdog_options = Arc::clone(&options);
        let timeout = self.timeout;
        let watchdog = std::thread::spawn(move || {
            if stop_rx.recv_timeout(timeout).is_err() {
                let _ = watchdog_options.terminate();
            }
        });
        let inputs = ort::inputs!["ids" => input];
        let mut session = self.session.lock().ok()?;
        let result = session.run_with_options(inputs, &options);
        let _ = stop_tx.send(());
        let _ = watchdog.join();
        let outputs = result.ok()?;
        let value = outputs.get("probs")?;
        let (_shape, data) = value.try_extract_tensor::<f32>().ok()?;
        if data.iter().any(|v| !v.is_finite()) {
            return None;
        }
        if data.len() % seq != 0 {
            return None;
        }
        let classes = data.len() / seq;
        let rows = src.len();
        if classes == 0 {
            return None;
        }
        let mut out = Vec::with_capacity(rows);
        for i in 0..rows {
            let base = (2 + i) * classes;
            out.push(data[base..base + classes].to_vec());
        }
        Some(out)
    }
}

/// 找模型檔：優先用設定指定的名稱，找不到時退回 `model.onnx`／
/// `model.int8.onnx`。
fn resolve_model(dir: &Path, configured: &str) -> Result<std::path::PathBuf> {
    let candidates = [configured, "model.onnx", "model.int8.onnx"];
    for name in candidates {
        let path = dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(ModelError::manifest(format!(
        "找不到標點模型：{}",
        dir.join(configured).display()
    )))
}

fn read_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("manifest.json");
    match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw)
            .map_err(|e| ModelError::manifest(format!("{}：{e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest {
            labels: Vec::new(),
            threshold: None,
            window: None,
            stride: None,
            max_text_runes: None,
        }),
        Err(e) => Err(e.into()),
    }
}

#[derive(Debug, Deserialize)]
struct RawVocab {
    characters: Vec<String>,
}

fn load_vocab(path: &Path) -> Result<HashMap<char, i64>> {
    let raw = std::fs::read(path)?;
    let vocab: RawVocab = serde_json::from_slice(&raw)
        .map_err(|e| ModelError::tokenizer(format!("{}：{e}", path.display())))?;
    let mut chars = HashMap::with_capacity(vocab.characters.len());
    for (index, item) in vocab.characters.iter().enumerate() {
        let mut runes = item.chars();
        if let (Some(ch), None) = (runes.next(), runes.next()) {
            chars.insert(ch, index as i64 + 8);
        }
    }
    if chars.is_empty() {
        return Err(ModelError::tokenizer("vocab.json 的 characters 是空的"));
    }
    Ok(chars)
}

fn is_enumerator(ch: char) -> bool {
    "一二三四五六七八九十壹貳參肆伍陸柒捌玖拾甲乙丙丁戊己庚辛壬癸".contains(ch)
}

/// 可插入標點的邊界：漢字之後，且下一個字是漢字或空白；下一個可見字元
/// 必須是漢字或 ASCII 英數。網址內不插。
pub(crate) fn eligible_boundaries(runes: &[char]) -> Vec<bool> {
    let mut protected = vec![false; runes.len()];
    let mut i = 0;
    while i < runes.len() {
        if !starts_with_ignore_ascii_case(runes, i, "http://")
            && !starts_with_ignore_ascii_case(runes, i, "https://")
            && !starts_with_ignore_ascii_case(runes, i, "www.")
        {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < runes.len()
            && !" ，。！？、；「」『』（）【】".contains(runes[j])
            && !runes[j].is_whitespace()
        {
            j += 1;
        }
        for slot in protected.iter_mut().take(j).skip(i) {
            *slot = true;
        }
        if j > i {
            i = j;
        } else {
            i += 1;
        }
    }

    let mut allowed = vec![false; runes.len()];
    for i in 0..runes.len() {
        let next = runes.get(i + 1).copied();
        let next_visible = runes[i + 1..]
            .iter()
            .copied()
            .find(|ch| !ch.is_whitespace());
        allowed[i] = is_han(runes[i])
            && !protected[i]
            && next.is_none_or(|ch| is_han(ch) || ch.is_whitespace())
            && next_visible.is_none_or(|ch| is_han(ch) || ch.is_ascii_alphanumeric());
    }
    allowed
}

fn starts_with_ignore_ascii_case(runes: &[char], at: usize, prefix: &str) -> bool {
    let mut chars = runes[at..].iter();
    for want in prefix.chars() {
        match chars.next() {
            Some(ch) if ch.eq_ignore_ascii_case(&want) => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eligible_boundaries_only_inside_han_runs() {
        let text: Vec<char> = "好的我知道了明天見".chars().collect();
        let allowed = eligible_boundaries(&text);
        assert!(allowed.iter().all(|v| *v));
        assert_eq!(allowed.len(), text.len());

        let text: Vec<char> = "價格100元".chars().collect();
        let allowed = eligible_boundaries(&text);
        assert!(!allowed[1], "數字前不插");
        assert!(!allowed[2]);
        assert!(allowed.last() == Some(&false), "段尾不插");
    }

    #[test]
    fn test_eligible_boundaries_protects_urls() {
        let text: Vec<char> = "請看https://example.com好嗎".chars().collect();
        let allowed = eligible_boundaries(&text);
        let url_start = "請看".chars().count();
        for slot in allowed
            .iter()
            .take(url_start + "https://example.com".chars().count())
            .skip(url_start)
        {
            assert!(!slot, "網址內不得插標點");
        }
    }

    #[test]
    fn test_enumerator_set() {
        assert!(is_enumerator('一'));
        assert!(is_enumerator('甲'));
        assert!(!is_enumerator('我'));
    }

    #[test]
    fn test_starts_with_ignore_ascii_case() {
        let runes: Vec<char> = "HTTP://x".chars().collect();
        assert!(starts_with_ignore_ascii_case(&runes, 0, "http://"));
        assert!(!starts_with_ignore_ascii_case(&runes, 0, "https://"));
    }
}
