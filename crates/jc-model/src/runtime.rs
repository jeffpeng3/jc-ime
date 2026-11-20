//! ONNX Runtime 初始化、執行期探測與有上限的推論。
//!
//! `ort` 使用 `load-dynamic`：建置時不連結、不下載 ONNX Runtime，執行時才從
//! `ORT_DYLIB_PATH`、`--ort-lib` 或系統上的 `libonnxruntime` 載入。
//!
//! 沒有可用的動態庫時[`ModelRuntime::probe`]回傳
//! [`RuntimeStatus::Unavailable`]，模型集合全部標成
//! [`crate::Availability::Unavailable`]，呼叫端走統計路徑。原因只記錄一次，
//! 不重複洗版，也不把錯誤丟給使用者。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ort::environment::Environment;
use ort::session::{RunOptions, Session};
use ort::value::{Shape, Tensor};

use crate::error::{ModelError, Result};

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static STATUS: Mutex<Option<RuntimeStatus>> = Mutex::new(None);
static LOGGED: AtomicBool = AtomicBool::new(false);

/// ONNX Runtime 的執行期狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// 動態庫已載入，可以建立 session。
    Available,
    /// 找不到或載入失敗。
    Unavailable {
        /// 原因；不含使用者輸入的文字。
        reason: String,
    },
}

impl RuntimeStatus {
    /// 是否可推論。
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    /// 不可用的原因；可用時回傳 `None`。
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

/// ONNX Runtime 的執行期入口。
#[derive(Debug, Clone)]
pub struct ModelRuntime {
    status: RuntimeStatus,
}

impl ModelRuntime {
    /// 探測預設的 ONNX Runtime，結果快取。
    ///
    /// 依序嘗試 `ORT_DYLIB_PATH` 與系統動態庫；失敗時只記錄一次原因。
    pub fn probe() -> Self {
        Self { status: probe() }
    }

    /// 目前的執行期狀態。
    pub fn status(&self) -> &RuntimeStatus {
        &self.status
    }

    /// 是否可推論。
    pub fn is_available(&self) -> bool {
        self.status.is_available()
    }

    /// 不可用的原因；可用時回傳 `None`。
    pub fn reason(&self) -> Option<&str> {
        self.status.reason()
    }
}

/// 探測預設的 ONNX Runtime；結果快取，同一個行程只記錄一次原因。
pub fn probe() -> RuntimeStatus {
    let mut cached = STATUS.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(status) = cached.as_ref() {
        return status.clone();
    }
    let status = match environment(None) {
        Ok(_) => RuntimeStatus::Available,
        Err(err) => RuntimeStatus::Unavailable {
            reason: err.to_string(),
        },
    };
    *cached = Some(status.clone());
    drop(cached);
    log_once(&status);
    status
}

/// 初始化 ONNX Runtime；重複呼叫會沿用同一個 environment。
///
/// `ort_lib` 為 `None` 時依序使用 `ORT_DYLIB_PATH` 與平台預設檔名。
/// 失敗回傳 [`ModelError::Runtime`]，呼叫端應退回統計路徑。
pub fn init_runtime(ort_lib: Option<&Path>) -> Result<()> {
    let status = match environment(ort_lib) {
        Ok(_) => RuntimeStatus::Available,
        Err(err) => RuntimeStatus::Unavailable {
            reason: err.to_string(),
        },
    };
    let mut cached = STATUS.lock().unwrap_or_else(|poison| poison.into_inner());
    if status.is_available() || cached.is_none() {
        *cached = Some(status.clone());
    }
    drop(cached);
    log_once(&status);
    match status {
        RuntimeStatus::Available => Ok(()),
        RuntimeStatus::Unavailable { reason } => Err(ModelError::Runtime(reason)),
    }
}

/// 取得（必要時載入）全域 ONNX Runtime environment。
///
/// `ort_lib` 為 `None` 時依序使用 `ORT_DYLIB_PATH` 與平台預設檔名。
pub fn environment(ort_lib: Option<&Path>) -> Result<Environment> {
    if let Some(env) = ENVIRONMENT.get() {
        return Ok(env.clone());
    }
    let env = load_environment(ort_lib)?;
    match ENVIRONMENT.set(env.clone()) {
        Ok(()) => Ok(env),
        Err(_) => Ok(ENVIRONMENT
            .get()
            .expect("environment 已由另一個執行緒設定")
            .clone()),
    }
}

fn load_environment(ort_lib: Option<&Path>) -> Result<Environment> {
    let mut errors = Vec::new();
    for path in candidates(ort_lib) {
        match ort::init_from(&path) {
            Ok(builder) => match builder.build() {
                Ok(env) => return Ok(env),
                Err(err) => errors.push(format!("{}：{err}", path.display())),
            },
            Err(err) => errors.push(format!("{}：{err}", path.display())),
        }
    }
    Err(ModelError::Runtime(if errors.is_empty() {
        "找不到 ONNX Runtime 動態庫".to_string()
    } else {
        errors.join("；")
    }))
}

/// 依序要嘗試的動態庫路徑。
///
/// `ORT_DYLIB_PATH` 有值時只用它；否則試平台預設名稱與帶主版本的 soname。
fn candidates(ort_lib: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = ort_lib {
        return vec![path.to_path_buf()];
    }
    if let Some(value) = std::env::var_os("ORT_DYLIB_PATH") {
        if !value.is_empty() {
            return vec![PathBuf::from(value)];
        }
    }
    let mut paths = vec![default_dylib()];
    #[cfg(target_os = "linux")]
    paths.push(PathBuf::from("libonnxruntime.so.1"));
    paths
}

fn default_dylib() -> PathBuf {
    PathBuf::from(format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
}

fn log_once(status: &RuntimeStatus) {
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    match status {
        RuntimeStatus::Available => log::debug!("ONNX Runtime 已載入，推論路徑可用"),
        RuntimeStatus::Unavailable { reason } => {
            log::info!("ONNX Runtime 不可用，推論路徑改用統計結果：{reason}")
        }
    }
}

/// 建一個 session；執行緒數固定，避免推論搶滿 CPU。
pub(crate) fn build_session(path: &Path, threads: usize) -> Result<Session> {
    let env = environment(None)?;
    let builder = Session::builder(&env).map_err(|e| ModelError::Runtime(e.to_string()))?;
    let mut builder = builder
        .with_intra_threads(threads)
        .map_err(|e| ModelError::Runtime(e.to_string()))?
        .with_inter_threads(1)
        .map_err(|e| ModelError::Runtime(e.to_string()))?;
    builder
        .commit_from_file(path)
        .map_err(|e| ModelError::Inference(format!("{}：{e}", path.display())))
}

/// 跑一次 `input_ids`／`attention_mask` 推論，回傳展平的 float 輸出。
///
/// 超過 `timeout` 時用 `RunOptions::terminate` 中止並回
/// [`ModelError::Inference`]，呼叫端（reranker）再轉成 `None`。
pub(crate) fn run_timed(
    session: &mut Session,
    ids: Vec<i64>,
    mask: Vec<i64>,
    rows: usize,
    seq: usize,
    timeout: Duration,
    output: &str,
) -> Result<Vec<f32>> {
    let ids = Tensor::from_array((Shape::new([rows as i64, seq as i64]), ids))
        .map_err(|e| ModelError::Inference(e.to_string()))?;
    let mask = Tensor::from_array((Shape::new([rows as i64, seq as i64]), mask))
        .map_err(|e| ModelError::Inference(e.to_string()))?;
    let options = Arc::new(RunOptions::new().map_err(|e| ModelError::Inference(e.to_string()))?);
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let watchdog_options = Arc::clone(&options);
    let watchdog = std::thread::spawn(move || {
        if stop_rx.recv_timeout(timeout).is_err() {
            let _ = watchdog_options.terminate();
        }
    });
    let inputs = ort::inputs!["input_ids" => ids, "attention_mask" => mask];
    let result = session.run_with_options(inputs, &options);
    let _ = stop_tx.send(());
    let _ = watchdog.join();
    let outputs = result.map_err(|e| ModelError::Inference(e.to_string()))?;
    let value = outputs
        .get(output)
        .ok_or_else(|| ModelError::Inference(format!("模型沒有輸出 {output}")))?;
    let (_shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| ModelError::Inference(e.to_string()))?;
    if data.iter().any(|v| !v.is_finite()) {
        return Err(ModelError::Inference("分數不是有限值".into()));
    }
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dylib_name_has_platform_suffix() {
        let name = default_dylib();
        let name = name.to_string_lossy();
        assert!(name.contains("onnxruntime"));
        assert!(name.ends_with(std::env::consts::DLL_SUFFIX));
    }

    #[test]
    fn explicit_path_wins_over_environment() {
        let paths = candidates(Some(Path::new("/tmp/libonnxruntime.so")));
        assert_eq!(paths, vec![PathBuf::from("/tmp/libonnxruntime.so")]);
    }

    #[test]
    fn status_reports_reason_only_when_unavailable() {
        assert!(RuntimeStatus::Available.is_available());
        assert_eq!(RuntimeStatus::Available.reason(), None);
        let down = RuntimeStatus::Unavailable {
            reason: "沒有動態庫".into(),
        };
        assert!(!down.is_available());
        assert_eq!(down.reason(), Some("沒有動態庫"));
    }
}
