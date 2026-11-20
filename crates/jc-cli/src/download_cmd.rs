//! `jc-cli data download`、`jc-cli data status`：manifest 下載項的下載與查詢。
//!
//! 下載流程刻意保守：
//!
//! 1. 先寫 `<檔名>.part`，大小與 SHA-256 都核對通過才 `rename` 成正式檔名，
//!    避免半個檔案被引擎當成完整模型。
//! 2. `.part` 已存在時送 `Range: bytes=N-`；伺服器回 `206` 就續傳，回 `200`
//!    代表不支援續傳，直接從 0 重下。`416` 會刪掉暫存檔重試一次。
//! 3. 邊下載邊算 SHA-256。大小或雜湊不符會刪掉 `.part` 並回報；網路中斷則
//!    保留 `.part`，下次執行續傳。
//! 4. 進度寫 stderr（百分比、速度），`--quiet` 關閉；結果摘要寫 stdout。

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jc_data::download::{part_path, DownloadReport, DownloadState};
use jc_data::manifest::{sha256_file, DataManifest, MANIFEST_FILE};
use sha2::{Digest, Sha256};
use ureq::Agent;

use crate::cli::CliError;

/// 每次讀取的大小。
const READ_BUFFER: usize = 256 * 1024;

/// 進度輸出的最小間隔。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 建立連線的時限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// 等回應標頭的時限；下載本體不設總時限，慢速網路也能下載完。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// 下載失敗的原因。
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// 伺服器回應非 200／206。
    #[error("HTTP {status}：{url}")]
    Http {
        /// HTTP 狀態碼。
        status: u16,
        /// 下載來源。
        url: String,
    },
    /// 連線或傳輸失敗。
    #[error("連線 {url} 失敗：{message}")]
    Network {
        /// 下載來源。
        url: String,
        /// 底層錯誤訊息。
        message: String,
    },
    /// 本機檔案讀寫失敗。
    #[error("{path}：{source}")]
    Io {
        /// 出問題的路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
    /// 大小與 manifest 不符。
    #[error("{path} 大小不符：manifest {expected} 位元組，實際 {actual} 位元組")]
    Size {
        /// 檔案路徑。
        path: String,
        /// manifest 記錄的大小。
        expected: u64,
        /// 實際大小。
        actual: u64,
    },
    /// SHA-256 與 manifest 不符。
    #[error("{path} SHA-256 不符：manifest {expected}，實際 {actual}")]
    Hash {
        /// 檔案路徑。
        path: String,
        /// manifest 記錄的雜湊。
        expected: String,
        /// 實際算出的雜湊。
        actual: String,
    },
    /// 下載項沒有來源 URL。
    #[error("manifest 的下載項沒有 URL：{0}")]
    MissingUrl(String),
    /// 伺服器的續傳回應看不出起點。
    #[error("伺服器對 {path} 的續傳回應不完整：{message}")]
    Range {
        /// 檔案路徑。
        path: String,
        /// 錯誤訊息。
        message: String,
    },
    /// 傳輸中斷；`.part` 保留，可續傳。
    #[error("{path} 下載中斷：{message}（保留 .part，可重新執行續傳）")]
    Interrupted {
        /// 檔案路徑。
        path: String,
        /// 錯誤訊息。
        message: String,
    },
}

impl DownloadError {
    fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

/// 一次下載的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// 從 0 下載完成。
    Fresh,
    /// 從既有的 `.part` 續傳完成。
    Resumed {
        /// 續傳起點，位元組。
        from: u64,
    },
}

/// `jc-cli data status`：列出每個下載項的狀態。
///
/// 缺少下載項不算錯誤（使用者可能選擇不安裝神經模型），因此一律回傳
/// `Ok(())`；需要擋下安裝的是 `data verify`。
pub fn status(
    root: &Path,
    manifest: Option<&Path>,
    json: bool,
    stream: &mut dyn Write,
) -> Result<(), CliError> {
    let manifest = load_manifest(root, manifest)?;
    let report = DownloadReport::inspect(root, &manifest);
    if json {
        let text =
            serde_json::to_string_pretty(&report).map_err(|err| CliError::data(err.to_string()))?;
        writeln!(stream, "{text}")?;
    } else {
        for item in &report.items {
            writeln!(stream, "{}", describe(item))?;
        }
        writeln!(stream, "下載項：{}", report.summary())?;
    }
    Ok(())
}

/// 下載所有缺少或內容不符的下載項。
///
/// 單一檔案失敗不會擋下其他檔案；全部跑完後才回傳含失敗數的錯誤。
pub fn download(
    root: &Path,
    manifest: Option<&Path>,
    quiet: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let manifest = load_manifest(root, manifest)?;
    let report = DownloadReport::inspect(root, &manifest);
    if report.items.is_empty() {
        writeln!(out, "manifest 沒有下載項")?;
        return Ok(());
    }
    let mut progress = std::io::stderr();
    let mut failures: Vec<String> = Vec::new();
    let mut completed = 0usize;
    for item in &report.items {
        let target = root.join(&item.path);
        match &item.state {
            DownloadState::Present => {
                writeln!(out, "已存在      {}（{}）", item.path, item.role.as_str())?;
                completed += 1;
                continue;
            }
            DownloadState::Unreadable { message } => {
                writeln!(out, "讀取失敗    {}：{message}", item.path)?;
                failures.push(format!("{}：{message}", item.path));
                continue;
            }
            DownloadState::SizeMismatch { .. } | DownloadState::HashMismatch { .. } => {
                writeln!(out, "重新下載    {}（現有檔案內容不符，先刪除）", item.path)?;
                if let Err(err) = fs::remove_file(&target) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        writeln!(out, "刪除失敗    {}：{err}", item.path)?;
                        failures.push(format!("{}：{err}", item.path));
                        continue;
                    }
                }
            }
            DownloadState::Missing => {}
        }
        let Some(url) = item.url.as_deref() else {
            writeln!(
                out,
                "缺少 URL    {}（manifest 的下載項沒有 url）",
                item.path
            )?;
            failures.push(format!("{}：manifest 沒有 url", item.path));
            continue;
        };
        let expected = Expected {
            url,
            bytes: item.bytes,
            sha256: &item.sha256,
        };
        writeln!(
            out,
            "下載        {}（{}，{} 位元組）",
            item.path,
            item.role.as_str(),
            item.bytes
        )?;
        match fetch_file(&target, &expected, &item.path, quiet, &mut progress) {
            Ok(Fetched::Fresh) => {
                completed += 1;
                writeln!(out, "完成        {}", item.path)?;
            }
            Ok(Fetched::Resumed { from }) => {
                completed += 1;
                writeln!(
                    out,
                    "完成        {}（續傳自 {}）",
                    item.path,
                    human_bytes(from)
                )?;
            }
            Err(err) => {
                writeln!(out, "失敗        {}：{err}", item.path)?;
                failures.push(format!("{}：{err}", item.path));
            }
        }
    }
    writeln!(
        out,
        "下載結果：完成 {completed}/{}，失敗 {}",
        report.items.len(),
        failures.len()
    )?;
    if !failures.is_empty() {
        return Err(CliError::data(format!(
            "下載失敗 {} 個檔案：{}",
            failures.len(),
            failures.join("；")
        )));
    }
    Ok(())
}

/// 下載一個檔案需要的資訊。
struct Expected<'a> {
    url: &'a str,
    bytes: u64,
    sha256: &'a str,
}

/// 把 `target` 下載完成；已完成的 `.part` 直接升級成正式檔。
fn fetch_file(
    target: &Path,
    expected: &Expected<'_>,
    label: &str,
    quiet: bool,
    progress: &mut dyn Write,
) -> Result<Fetched, DownloadError> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| DownloadError::io(parent, err))?;
        }
    }
    let part = part_path(target);
    let mut resumed_from = 0u64;
    match fs::metadata(&part) {
        Ok(meta) if meta.is_file() => resumed_from = meta.len(),
        Ok(_) => remove_quietly(&part),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(DownloadError::io(&part, err)),
    }
    if resumed_from > expected.bytes {
        remove_quietly(&part);
        resumed_from = 0;
    }
    if resumed_from == expected.bytes && resumed_from > 0 {
        // 上次可能已經下載完但還沒改名；內容對就直接放上去。
        match sha256_file(&part) {
            Ok((_, actual)) if actual.eq_ignore_ascii_case(expected.sha256) => {
                fs::rename(&part, target).map_err(|err| DownloadError::io(target, err))?;
                return Ok(Fetched::Fresh);
            }
            Ok(_) => {
                remove_quietly(&part);
                resumed_from = 0;
            }
            Err(err) => {
                return Err(DownloadError::Io {
                    path: part.clone(),
                    source: std::io::Error::other(err.to_string()),
                })
            }
        }
    }

    let agent = agent();
    let mut used_resume = false;
    let mut restarted = false;
    loop {
        let mut request = agent.get(expected.url);
        if resumed_from > 0 {
            request = request.header("Range", format!("bytes={resumed_from}-"));
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(err) => {
                return Err(DownloadError::Network {
                    url: expected.url.to_string(),
                    message: err.to_string(),
                })
            }
        };
        let status = response.status().as_u16();
        let mut append = resumed_from > 0;
        match status {
            206 => {
                let content_range = response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let Some(start) = content_range.as_deref().and_then(range_start) else {
                    return Err(DownloadError::Range {
                        path: label.to_string(),
                        message: "缺少 content-range".to_string(),
                    });
                };
                if start != resumed_from {
                    if restarted {
                        return Err(DownloadError::Range {
                            path: label.to_string(),
                            message: format!("要求從 {resumed_from} 續傳，伺服器回報 {start}"),
                        });
                    }
                    remove_quietly(&part);
                    resumed_from = 0;
                    restarted = true;
                    continue;
                }
            }
            200 => {
                // 伺服器忽略 Range，回整個檔案：從 0 重下。
                append = false;
            }
            416 if !restarted => {
                remove_quietly(&part);
                resumed_from = 0;
                restarted = true;
                continue;
            }
            other => {
                return Err(DownloadError::Http {
                    status: other,
                    url: expected.url.to_string(),
                })
            }
        }
        used_resume = used_resume || append;

        let expected_len = if append {
            expected.bytes - resumed_from
        } else {
            expected.bytes
        };
        if let Some(length) = content_length(&response) {
            if length != expected_len {
                remove_quietly(&part);
                return Err(DownloadError::Size {
                    path: label.to_string(),
                    expected: expected_len,
                    actual: length,
                });
            }
        }

        let mut file = if append {
            OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(|err| DownloadError::io(&part, err))?
        } else {
            File::create(&part).map_err(|err| DownloadError::io(&part, err))?
        };
        let mut hasher = Sha256::new();
        if append {
            hash_prefix(&part, &mut hasher)?;
        }
        let mut done = if append { resumed_from } else { 0 };
        let started = Instant::now();
        let mut last = started;
        let mut buffer = vec![0u8; READ_BUFFER];
        let mut reader = response.body_mut().as_reader();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    done += read as u64;
                    if done > expected.bytes {
                        drop(file);
                        remove_quietly(&part);
                        return Err(DownloadError::Size {
                            path: label.to_string(),
                            expected: expected.bytes,
                            actual: done,
                        });
                    }
                    hasher.update(&buffer[..read]);
                    file.write_all(&buffer[..read])
                        .map_err(|err| DownloadError::io(&part, err))?;
                    if !quiet && last.elapsed() >= PROGRESS_INTERVAL {
                        let _ = write!(
                            progress,
                            "\r{}",
                            format_progress(label, done, expected.bytes, started.elapsed())
                        );
                        let _ = progress.flush();
                        last = Instant::now();
                    }
                }
                Err(err) => {
                    // 中斷時保留 `.part`，下次從這裡續傳。
                    return Err(DownloadError::Interrupted {
                        path: label.to_string(),
                        message: err.to_string(),
                    });
                }
            }
        }
        file.flush().map_err(|err| DownloadError::io(&part, err))?;
        file.sync_all()
            .map_err(|err| DownloadError::io(&part, err))?;
        drop(file);
        if done != expected.bytes {
            // 連線提早結束：多半是網路斷線，保留 `.part` 續傳。
            return Err(DownloadError::Interrupted {
                path: label.to_string(),
                message: format!("收到 {done}／{} 位元組", expected.bytes),
            });
        }
        let actual = hex(&hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected.sha256) {
            remove_quietly(&part);
            return Err(DownloadError::Hash {
                path: label.to_string(),
                expected: expected.sha256.to_string(),
                actual,
            });
        }
        if !quiet {
            let _ = write!(
                progress,
                "\r{}\n",
                format_progress(label, done, expected.bytes, started.elapsed())
            );
            let _ = progress.flush();
        }
        fs::rename(&part, target).map_err(|err| DownloadError::io(target, err))?;
        return Ok(if used_resume {
            Fetched::Resumed { from: resumed_from }
        } else {
            Fetched::Fresh
        });
    }
}

fn agent() -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .build()
        .into()
}

fn load_manifest(root: &Path, manifest: Option<&Path>) -> Result<DataManifest, CliError> {
    if !root.is_dir() {
        return Err(CliError::data(format!(
            "資料根目錄不存在：{}",
            root.display()
        )));
    }
    let path: PathBuf = manifest
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(MANIFEST_FILE));
    DataManifest::load(&path).map_err(|err| CliError::data(err.to_string()))
}

fn describe(item: &jc_data::download::DownloadStatus) -> String {
    let role = item.role.as_str();
    match &item.state {
        DownloadState::Present => format!(
            "已存在      {}（{}，{} 位元組）",
            item.path, role, item.bytes
        ),
        DownloadState::Missing => {
            let partial = match item.partial_bytes {
                Some(bytes) => format!("，已下載 {} 可續傳", human_bytes(bytes)),
                None => String::new(),
            };
            format!(
                "缺少        {}（{}，預期 {} 位元組{partial}）",
                item.path, role, item.bytes
            )
        }
        DownloadState::SizeMismatch { expected, actual } => format!(
            "大小不符    {}（{}，manifest {expected} 位元組，實際 {actual} 位元組）",
            item.path, role
        ),
        DownloadState::HashMismatch { expected, actual } => format!(
            "雜湊不符    {}（{}，manifest {expected}，實際 {actual}）",
            item.path, role
        ),
        DownloadState::Unreadable { message } => {
            format!("讀取失敗    {}（{}）：{message}", item.path, role)
        }
    }
}

fn hash_prefix(path: &Path, hasher: &mut Sha256) -> Result<(), DownloadError> {
    let file = File::open(path).map_err(|err| DownloadError::io(path, err))?;
    let mut reader = BufReader::with_capacity(READ_BUFFER, file);
    let mut buffer = vec![0u8; READ_BUFFER];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| DownloadError::io(path, err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn content_length(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

/// 從 `bytes 123-456/789` 取出起點 123。
fn range_start(value: &str) -> Option<u64> {
    let rest = value.strip_prefix("bytes ")?;
    let start = rest.split('-').next()?;
    start.parse().ok()
}

fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// 進度文字：百分比、已完成／總量、平均速度。
fn format_progress(label: &str, done: u64, total: u64, elapsed: Duration) -> String {
    let percent = if total == 0 {
        100.0
    } else {
        done as f64 * 100.0 / total as f64
    };
    let seconds = elapsed.as_secs_f64();
    let speed = if seconds > 0.0 {
        (done as f64 / seconds).round() as u64
    } else {
        0
    };
    format!(
        "{label} {percent:.1}%（{}/{}，{}/s）",
        human_bytes(done),
        human_bytes(total),
        human_bytes(speed)
    )
}

/// 以十進位單位顯示位元組，與文件中的 MB 一致。
fn human_bytes(bytes: u64) -> String {
    if bytes < 1_000 {
        format!("{bytes} B")
    } else if bytes < 1_000_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader as IoBufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-cli-download-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// 最小 HTTP 伺服器：`body` 是完整內容，`support_range` 決定是否處理 Range。
    fn serve(
        body: Vec<u8>,
        support_range: bool,
    ) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            // 每個測試只發一次請求，伺服器處理完就結束，join 不會卡住。
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = IoBufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                return;
            }
            let mut range = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(": ") {
                    if name.eq_ignore_ascii_case("range") {
                        range = Some(value.to_string());
                    }
                }
            }
            counter.fetch_add(1, Ordering::SeqCst);
            let start = range
                .as_deref()
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| value.strip_suffix('-'))
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|_| support_range)
                .unwrap_or(0);
            let payload = &body[start..];
            let status = if start > 0 {
                "206 Partial Content"
            } else {
                "200 OK"
            };
            let mut head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                payload.len()
            );
            if start > 0 {
                head.push_str(&format!(
                    "Content-Range: bytes {start}-{}/{}\r\n",
                    body.len() - 1,
                    body.len()
                ));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(payload);
            let _ = stream.flush();
        });
        (format!("http://{addr}/model.onnx"), requests, handle)
    }

    #[test]
    fn format_progress_reports_percent_and_speed() {
        assert_eq!(
            format_progress(
                "model.onnx",
                50_000_000,
                100_000_000,
                Duration::from_secs(10)
            ),
            "model.onnx 50.0%（50.0 MB/100.0 MB，5.0 MB/s）"
        );
        assert_eq!(
            format_progress("vocab.json", 312_708, 312_708, Duration::from_secs(1)),
            "vocab.json 100.0%（312.7 KB/312.7 KB，312.7 KB/s）"
        );
    }

    #[test]
    fn human_bytes_uses_decimal_units() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.5 KB");
        assert_eq!(human_bytes(130_723_137), "130.7 MB");
        assert_eq!(human_bytes(2_000_000_000), "2.00 GB");
    }

    fn manifest_json(path: &str, url: &str, body: &[u8]) -> String {
        let text = format!(
            r#"{{
  "schema": "0c.data-pack/1",
  "version": "test",
  "generated_at": "2026-11-20T00:00:00Z",
  "files": [
    {{
      "path": "{path}",
      "bytes": {},
      "sha256": "{}",
      "role": "zh_select",
      "license": ["L-11"],
      "shipped": false,
      "url": "{url}"
    }}
  ]
}}
"#,
            body.len(),
            sha256_hex(body)
        );
        text
    }

    #[test]
    fn status_reports_present_and_hash_mismatch() {
        let dir = temp_dir("status");
        let body = b"data".to_vec();
        let target = dir.join("models/zh-select/model.onnx");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &body).unwrap();
        fs::write(
            dir.join("manifest.json"),
            manifest_json(
                "models/zh-select/model.onnx",
                "https://example.invalid/m",
                &body,
            ),
        )
        .unwrap();

        let mut out = Vec::new();
        status(&dir, None, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("已存在      models/zh-select/model.onnx（zh_select，4 位元組）"),
            "{text}"
        );
        assert!(
            text.contains("下載項：已存在 1/1，缺少 0，不符 0"),
            "{text}"
        );

        fs::write(&target, b"datb").unwrap();
        let mut out = Vec::new();
        status(&dir, None, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("雜湊不符    models/zh-select/model.onnx"),
            "{text}"
        );
        assert!(
            text.contains("下載項：已存在 0/1，缺少 0，不符 1"),
            "{text}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn download_writes_manifest_target() {
        let dir = temp_dir("end-to-end");
        let body = b"hello from the test server".to_vec();
        let (url, _requests, handle) = serve(body.clone(), true);
        fs::write(
            dir.join("manifest.json"),
            manifest_json("models/zh-select/model.onnx", &url, &body),
        )
        .unwrap();
        let mut out = Vec::new();
        download(&dir, None, true, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("完成        models/zh-select/model.onnx"),
            "{text}"
        );
        assert!(text.contains("下載結果：完成 1/1，失敗 0"), "{text}");
        assert_eq!(
            fs::read(dir.join("models/zh-select/model.onnx")).unwrap(),
            body
        );
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn range_start_parses_content_range() {
        assert_eq!(range_start("bytes 0-99/100"), Some(0));
        assert_eq!(range_start("bytes 1234-9999/10000"), Some(1234));
        assert_eq!(range_start("items 1-2/3"), None);
    }

    #[test]
    fn downloads_fresh_file_and_renames_part() {
        let dir = temp_dir("fresh");
        let body = b"hello model".to_vec();
        let target = dir.join("models/zh-select/model.onnx");
        let (url, requests, handle) = serve(body.clone(), true);
        let expected = Expected {
            url: &url,
            bytes: body.len() as u64,
            sha256: &sha256_hex(&body),
        };
        let mut progress = Vec::new();
        let fetched = fetch_file(&target, &expected, "model.onnx", true, &mut progress).unwrap();
        assert_eq!(fetched, Fetched::Fresh);
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(!part_path(&target).exists(), "完成後不該留下 .part");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resumes_from_part_with_range() {
        let dir = temp_dir("resume");
        let body = b"0123456789".to_vec();
        let target = dir.join("model.bin");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(part_path(&target), &body[..4]).unwrap();
        let (url, requests, handle) = serve(body.clone(), true);
        let expected = Expected {
            url: &url,
            bytes: body.len() as u64,
            sha256: &sha256_hex(&body),
        };
        let mut progress = Vec::new();
        let fetched = fetch_file(&target, &expected, "model.bin", true, &mut progress).unwrap();
        assert_eq!(fetched, Fetched::Resumed { from: 4 });
        assert_eq!(fs::read(&target).unwrap(), body);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restarts_when_server_ignores_range() {
        let dir = temp_dir("no-range");
        let body = b"abcdefghij".to_vec();
        let target = dir.join("model.bin");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(part_path(&target), b"XXXX").unwrap();
        let (url, _requests, handle) = serve(body.clone(), false);
        let expected = Expected {
            url: &url,
            bytes: body.len() as u64,
            sha256: &sha256_hex(&body),
        };
        let mut progress = Vec::new();
        let fetched = fetch_file(&target, &expected, "model.bin", true, &mut progress).unwrap();
        assert_eq!(fetched, Fetched::Fresh, "忽略 Range 時視為重下");
        assert_eq!(fs::read(&target).unwrap(), body);
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_mismatch_deletes_part_and_reports() {
        let dir = temp_dir("bad-hash");
        let body = b"tampered".to_vec();
        let target = dir.join("model.bin");
        let (url, _requests, handle) = serve(body.clone(), true);
        let expected = Expected {
            url: &url,
            bytes: body.len() as u64,
            sha256: &"0".repeat(64),
        };
        let mut progress = Vec::new();
        let err = fetch_file(&target, &expected, "model.bin", true, &mut progress).unwrap_err();
        assert!(matches!(err, DownloadError::Hash { .. }), "{err}");
        assert!(!part_path(&target).exists(), "雜湊不符要刪掉 .part");
        assert!(!target.exists(), "不能留下半個檔案");
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn size_mismatch_is_rejected_before_writing() {
        let dir = temp_dir("bad-size");
        let body = b"short".to_vec();
        let target = dir.join("model.bin");
        let (url, _requests, handle) = serve(body.clone(), true);
        let expected = Expected {
            url: &url,
            bytes: 99,
            sha256: &sha256_hex(&body),
        };
        let mut progress = Vec::new();
        let err = fetch_file(&target, &expected, "model.bin", true, &mut progress).unwrap_err();
        assert!(matches!(err, DownloadError::Size { .. }), "{err}");
        assert!(!target.exists());
        handle.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }
}
