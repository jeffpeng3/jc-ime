//! ONNX Runtime 初始化與有上限的推論。
//!
//! 使用 `ort` 的 `load-dynamic`：建置時不連結、不下載 ONNX Runtime，執行
//! 時從 `ORT_DYLIB_PATH`、`--ort-lib` 或目前目錄的 `libonnxruntime.so`
//! 載入。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ort::environment::Environment;
use ort::session::{RunOptions, Session};
use ort::value::{Shape, Tensor};

use crate::error::{ModelError, Result};

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();

/// 初始化 ONNX Runtime；重複呼叫會沿用同一個 environment。
///
/// `ort_lib` 為 `None` 時依序使用 `ORT_DYLIB_PATH` 與平台預設檔名。
pub fn init_runtime(ort_lib: Option<&Path>) -> Result<()> {
    environment(ort_lib).map(|_| ())
}

pub(crate) fn environment(ort_lib: Option<&Path>) -> Result<Environment> {
    if let Some(env) = ENVIRONMENT.get() {
        return Ok(env.clone());
    }
    let path = ort_lib
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(default_dylib);
    let env = ort::init_from(&path)
        .map_err(|e| ModelError::Runtime(format!("{}：{e}", path.display())))?
        .build()
        .map_err(|e| ModelError::Runtime(e.to_string()))?;
    match ENVIRONMENT.set(env.clone()) {
        Ok(()) => Ok(env),
        Err(_) => Ok(ENVIRONMENT
            .get()
            .expect("environment 已由另一個執行緒設定")
            .clone()),
    }
}

fn default_dylib() -> PathBuf {
    PathBuf::from(format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
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
/// [`ModelError::Inference`]，呼叫端（reranker／punctuator）再轉成 `None`。
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
}
