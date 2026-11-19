//! 提交時補標點的模型接線。
//!
//! [`ModelPunctuator`] 是工作階段與命令列共用的介面；[`OnnxPunctuator`] 在
//! `onnx` feature 下載入 `ja-punct`／`zh-punct` 兩個模型。模型缺席、建置
//! 沒有 `onnx`、逾時或推論失敗時一律回傳原文，呼叫端不需要處理錯誤。

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::lang::Mode;

/// 提交時補標點的模型介面。
///
/// 實作只能插入標點，不得改寫、刪除或重排使用者輸入的字。`mode` 決定用
/// 哪個語言的模型；不支援的模式回傳原文。
pub trait ModelPunctuator: Send + Sync + fmt::Debug {
    /// 在 `text` 插入標點。失敗時回傳原文。
    fn punctuate(&self, text: &str, mode: Mode) -> String;
}

/// 補標點推論的累計統計。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PunctLatencyStats {
    /// 呼叫模型的次數。
    pub calls: u64,
    /// 真的插入標點的次數。
    pub inserted: u64,
    /// 推論延遲中位數，毫秒。
    pub p50_ms: f64,
    /// 推論延遲第 95 百分位，毫秒。
    pub p95_ms: f64,
}

/// ONNX 標點模型接線。
///
/// 只載入呼叫端要求的語言；`Mixed` 會依文字是否含假名選模型。
pub struct OnnxPunctuator {
    #[cfg(feature = "onnx")]
    ja: Option<jc_model::PunctuationModel>,
    #[cfg(feature = "onnx")]
    zh: Option<jc_model::ZhPunctuationModel>,
    samples: Mutex<Vec<f64>>,
    calls: Mutex<u64>,
    inserted: Mutex<u64>,
}

impl fmt::Debug for OnnxPunctuator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("OnnxPunctuator");
        #[cfg(feature = "onnx")]
        {
            out.field("ja", &self.ja.is_some());
            out.field("zh", &self.zh.is_some());
        }
        out.finish_non_exhaustive()
    }
}

impl OnnxPunctuator {
    /// 從模型集合根目錄載入指定語言。
    ///
    /// `modes` 的 `Mixed` 會同時載入日文與注音模型。模型缺席、manifest
    /// 損壞或 ORT 載入失敗都回傳錯誤，由呼叫端決定是否只警告並停用。
    #[cfg(feature = "onnx")]
    pub fn load_from_models(root: &Path, modes: &[Mode]) -> Result<Self, String> {
        use jc_model::{Availability, ModelSet, PunctuationModel, Role, ZhPunctuationModel};

        let set = ModelSet::load(root).map_err(|err| format!("{}：{err}", root.display()))?;
        let want_ja = modes
            .iter()
            .any(|mode| matches!(mode, Mode::Ja | Mode::Mixed));
        let want_zh = modes
            .iter()
            .any(|mode| matches!(mode, Mode::Zh | Mode::Mixed));
        let load_ja = || -> Result<Option<PunctuationModel>, String> {
            if !want_ja {
                return Ok(None);
            }
            match set.availability(Role::JaPunct) {
                Availability::Loaded { .. } => {
                    let dir = set
                        .model_dir(Role::JaPunct)
                        .ok_or_else(|| "ja-punct 沒有解析出目錄".to_string())?;
                    let spec = set.spec(Role::JaPunct).cloned().unwrap_or_default();
                    PunctuationModel::load_with_spec(dir, &spec, None)
                        .map(Some)
                        .map_err(|err| format!("載入 ja-punct 失敗：{err}"))
                }
                Availability::Unavailable { reason, .. } => {
                    Err(format!("ja-punct 不可用：{reason}"))
                }
            }
        };
        let load_zh = || -> Result<Option<ZhPunctuationModel>, String> {
            if !want_zh {
                return Ok(None);
            }
            match set.availability(Role::ZhPunct) {
                Availability::Loaded { .. } => {
                    let dir = set
                        .model_dir(Role::ZhPunct)
                        .ok_or_else(|| "zh-punct 沒有解析出目錄".to_string())?;
                    let spec = set.spec(Role::ZhPunct).cloned().unwrap_or_default();
                    ZhPunctuationModel::load_with_spec(dir, &spec, None)
                        .map(Some)
                        .map_err(|err| format!("載入 zh-punct 失敗：{err}"))
                }
                Availability::Unavailable { reason, .. } => {
                    Err(format!("zh-punct 不可用：{reason}"))
                }
            }
        };
        Ok(Self {
            ja: load_ja()?,
            zh: load_zh()?,
            samples: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
            inserted: Mutex::new(0),
        })
    }

    /// 沒有 `onnx` feature 時不能載入模型。
    #[cfg(not(feature = "onnx"))]
    pub fn load_from_models(_root: &Path, _modes: &[Mode]) -> Result<Self, String> {
        Err("這個建置沒有啟用 onnx feature，不能載入標點模型".to_string())
    }

    /// 累計統計；沒有推論時 percentile 為 0。
    pub fn stats(&self) -> PunctLatencyStats {
        let mut samples = self
            .samples
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let percentile = |q: f64| -> f64 {
            if samples.is_empty() {
                return 0.0;
            }
            let index = ((samples.len() - 1) as f64 * q).round() as usize;
            samples[index.min(samples.len() - 1)]
        };
        PunctLatencyStats {
            calls: *self
                .calls
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
            inserted: *self
                .inserted
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
            p50_ms: percentile(0.5),
            p95_ms: percentile(0.95),
        }
    }
}

impl ModelPunctuator for OnnxPunctuator {
    fn punctuate(&self, text: &str, mode: Mode) -> String {
        #[cfg(feature = "onnx")]
        {
            if text.is_empty() {
                return String::new();
            }
            let started = std::time::Instant::now();
            let output = match mode {
                Mode::Ja => self.ja.as_ref().map(|model| model.punctuate(text)),
                Mode::Zh => self.zh.as_ref().map(|model| model.punctuate(text)),
                Mode::Mixed => {
                    if contains_kana(text) {
                        self.ja.as_ref().map(|model| model.punctuate(text))
                    } else {
                        self.zh.as_ref().map(|model| model.punctuate(text))
                    }
                }
                Mode::En => None,
            }
            .unwrap_or_else(|| text.to_string());
            *self
                .calls
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) += 1;
            if output != text {
                *self
                    .inserted
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) += 1;
            }
            self.samples
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(started.elapsed().as_secs_f64() * 1000.0);
            output
        }
        #[cfg(not(feature = "onnx"))]
        {
            let _ = mode;
            text.to_string()
        }
    }
}

/// 文字是否含日文假名。
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
fn contains_kana(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{3040}'..='\u{30FF}').contains(&ch))
}

/// 共用同一個模型的 `Arc` 包裝；方便設定檔持有。
pub type SharedPunctuator = Arc<dyn ModelPunctuator>;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct InsertAtEnd;

    impl ModelPunctuator for InsertAtEnd {
        fn punctuate(&self, text: &str, mode: Mode) -> String {
            match mode {
                Mode::En => text.to_string(),
                _ if text.is_empty() => text.to_string(),
                _ => format!("{text}。"),
            }
        }
    }

    #[test]
    fn test_shared_trait_object_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedPunctuator>();
        let punctuator: SharedPunctuator = Arc::new(InsertAtEnd);
        assert_eq!(punctuator.punctuate("今日は寒い", Mode::Ja), "今日は寒い。");
        assert_eq!(punctuator.punctuate("Hello", Mode::En), "Hello");
        assert_eq!(punctuator.punctuate("", Mode::Zh), "");
    }

    #[test]
    fn test_contains_kana() {
        assert!(contains_kana("わたし"));
        assert!(contains_kana("テスト"));
        assert!(!contains_kana("我愛你"));
        assert!(!contains_kana("abc"));
    }

    #[cfg(not(feature = "onnx"))]
    #[test]
    fn test_without_onnx_returns_text_unchanged() {
        let err = OnnxPunctuator::load_from_models(Path::new("/nonexistent"), &[Mode::Ja]);
        assert!(err.is_err());
        // 沒有模型時仍然可以建立物件，只是原樣回傳。
        let punctuator = OnnxPunctuator {
            samples: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
            inserted: Mutex::new(0),
        };
        assert_eq!(punctuator.punctuate("今日は寒い", Mode::Ja), "今日は寒い");
        assert_eq!(punctuator.stats().calls, 0);
    }
}
