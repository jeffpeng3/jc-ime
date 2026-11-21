//! 下載項（`shipped: false`）的查詢與下載。
//!
//! [`DownloadReport::inspect`] 只讀 manifest 與檔案系統；
//! [`download_missing`] 用 HTTP 把缺少的下載項抓到資料根目錄。兩者都不依賴 CLI，
//! 引擎可直接呼叫，CLI 的輸出格式化留在 `jc-cli`。
//!
//! 狀態分三類，對應安裝器與 `jc-cli data status` 要回答的問題：
//!
//! | 狀態 | 意義 | 下一步 |
//! |---|---|---|
//! | [`DownloadState::Present`] | 檔案存在、大小與 SHA-256 都相符 | 不用動作 |
//! | [`DownloadState::Missing`] | 檔案不存在 | 下載 |
//! | [`DownloadState::SizeMismatch`]／[`DownloadState::HashMismatch`] | 內容不對 | 刪掉重下 |
//!
//! 另外 [`DownloadStatus::partial_bytes`] 會回報 `.part` 暫存檔的大小，讓使用者
//! 知道中斷的下載還能不能續傳。查詢會實際算 SHA-256，因此對 130 MB 的模型
//! 需要讀完整個檔案；引擎啟動時的完整性檢查只比大小（見
//! [`crate::manifest::VerifyLevel::Startup`]）。

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use ureq::Agent;

use crate::manifest::{sha256_file, DataFile, DataManifest, DataRole};

/// 下載中的暫存檔後綴；完成後才改名成正式檔名。
pub const PART_SUFFIX: &str = ".part";

/// 單一下載項的狀態。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadState {
    /// 檔案存在且大小、SHA-256 都相符。
    Present,
    /// 檔案不存在，也沒有可續傳的暫存檔。
    Missing,
    /// 檔案存在但大小不符。
    SizeMismatch {
        /// manifest 記錄的大小。
        expected: u64,
        /// 實際大小。
        actual: u64,
    },
    /// 檔案存在、大小相符，但 SHA-256 不符。
    HashMismatch {
        /// manifest 記錄的雜湊。
        expected: String,
        /// 實際算出的雜湊。
        actual: String,
    },
    /// 檔案存在但讀不到。
    Unreadable {
        /// 錯誤訊息。
        message: String,
    },
}

impl DownloadState {
    /// 這個項目是否已經是可以用的完整檔案。
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present)
    }

    /// 這個項目是否完全不存在（沒有正式檔，也沒有暫存檔）。
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// 這個項目是否存在但內容不對。
    pub fn is_mismatch(&self) -> bool {
        matches!(
            self,
            Self::SizeMismatch { .. } | Self::HashMismatch { .. } | Self::Unreadable { .. }
        )
    }
}

/// 一個下載項的查詢結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadStatus {
    /// 相對於資料根目錄的路徑。
    pub path: String,
    /// 檔案角色。
    pub role: DataRole,
    /// 下載來源；manifest 沒寫時是 `None`，下載器會拒絕該項。
    pub url: Option<String>,
    /// manifest 記錄的大小。
    pub bytes: u64,
    /// manifest 記錄的 SHA-256。
    pub sha256: String,
    /// 目前狀態。
    pub state: DownloadState,
    /// `.part` 暫存檔的大小；沒有暫存檔時是 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_bytes: Option<u64>,
}

/// 整份 manifest 的下載項查詢結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadReport {
    /// 查詢的資料根目錄。
    pub root: PathBuf,
    /// manifest 的資料版本標籤。
    pub manifest_version: String,
    /// 逐項狀態，順序與 manifest 相同。
    pub items: Vec<DownloadStatus>,
}

impl DownloadReport {
    /// 查詢所有 `shipped: false` 的項目。
    ///
    /// 會實際算 SHA-256；檔案很大時這個函式也跟著變慢。
    pub fn inspect(root: impl AsRef<Path>, manifest: &DataManifest) -> Self {
        let root = root.as_ref();
        let items = manifest
            .download_files()
            .map(|file| inspect_file(root, file))
            .collect();
        Self {
            root: root.to_path_buf(),
            manifest_version: manifest.version.clone(),
            items,
        }
    }

    /// 已存在的項目數。
    pub fn present(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_present())
            .count()
    }

    /// 缺少的項目數。
    pub fn missing(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_missing())
            .count()
    }

    /// 存在但內容不對的項目數。
    pub fn mismatched(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_mismatch())
            .count()
    }

    /// 全部項目都可用時回傳 `true`。
    pub fn is_complete(&self) -> bool {
        self.items.iter().all(|item| item.state.is_present())
    }

    /// 給記錄用的一行摘要。
    pub fn summary(&self) -> String {
        format!(
            "已存在 {}/{}，缺少 {}，不符 {}",
            self.present(),
            self.items.len(),
            self.missing(),
            self.mismatched()
        )
    }
}

/// 查詢單一 manifest 記錄。
pub fn inspect_file(root: impl AsRef<Path>, file: &DataFile) -> DownloadStatus {
    let root = root.as_ref();
    let target = root.join(&file.path);
    let partial_bytes = fs::metadata(part_path(&target))
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .filter(|bytes| *bytes > 0);
    let state = state_of(&target, file);
    DownloadStatus {
        path: file.path.clone(),
        role: file.role,
        url: file.url.clone(),
        bytes: file.bytes,
        sha256: file.sha256.clone(),
        state,
        partial_bytes,
    }
}

/// 正式檔旁邊的 `.part` 路徑。
pub fn part_path(target: impl AsRef<Path>) -> PathBuf {
    let target = target.as_ref();
    let mut name = target.as_os_str().to_os_string();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

fn state_of(target: &Path, file: &DataFile) -> DownloadState {
    let meta = match fs::metadata(target) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return DownloadState::Missing,
        Err(err) => {
            return DownloadState::Unreadable {
                message: err.to_string(),
            }
        }
    };
    if !meta.is_file() {
        return DownloadState::Unreadable {
            message: "不是一般檔案".to_string(),
        };
    }
    let actual = meta.len();
    if actual != file.bytes {
        return DownloadState::SizeMismatch {
            expected: file.bytes,
            actual,
        };
    }
    match sha256_file(target) {
        Err(err) => DownloadState::Unreadable {
            message: err.to_string(),
        },
        Ok((_, actual)) => {
            if actual.eq_ignore_ascii_case(&file.sha256) {
                DownloadState::Present
            } else {
                DownloadState::HashMismatch {
                    expected: file.sha256.clone(),
                    actual,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 可複用的下載器：不依賴 CLI，引擎與 `jc-cli data download` 都能用。
// ---------------------------------------------------------------------------

/// 每次讀取的大小。
const READ_BUFFER: usize = 256 * 1024;

/// 進度事件的節流間隔。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 一次下載的 timeout 策略。
///
/// CLI 手動下載用 [`DownloadPolicy::default`]，讓慢速網路也能抓完；引擎背景
/// 下載用 [`DownloadPolicy::background`]，時間到就保留 `.part`，下次啟動續傳。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadPolicy {
    /// 建立連線的時限。
    pub connect_timeout: Duration,
    /// 等回應標頭的時限。
    pub response_timeout: Duration,
    /// 接收整段 body 的時限；`None` 表示不限。
    pub body_timeout: Option<Duration>,
    /// 單次請求的總時限；`None` 表示不限。
    pub global_timeout: Option<Duration>,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            response_timeout: Duration::from_secs(60),
            body_timeout: None,
            global_timeout: None,
        }
    }
}

impl DownloadPolicy {
    /// 引擎背景自動下載：短 timeout，抓多少算多少，中斷留 `.part` 下次續傳。
    pub fn background() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            response_timeout: Duration::from_secs(10),
            body_timeout: Some(Duration::from_secs(30)),
            global_timeout: Some(Duration::from_secs(45)),
        }
    }
}

/// 單一下載項的事件，供呼叫端記錄進度或警告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    /// 開始下載一個檔案。
    Started {
        /// manifest 記錄的相對路徑。
        path: String,
        /// 預期大小，位元組。
        bytes: u64,
    },
    /// 已收到部分資料；由 [`download_missing`] 節流後送出。
    Progress {
        /// manifest 記錄的相對路徑。
        path: String,
        /// 目前已收到的位元組數（含續傳的前綴）。
        done: u64,
        /// 檔案總大小。
        total: u64,
    },
    /// 單檔下載並驗證完成，正式檔已就位。
    Done {
        /// manifest 記錄的相對路徑。
        path: String,
        /// 是否從 `.part` 續傳完成。
        resumed: bool,
    },
    /// 單檔失敗；其他檔案仍會繼續處理。
    Failed {
        /// manifest 記錄的相對路徑。
        path: String,
        /// 失敗原因。
        reason: String,
        /// `true` 表示下載中斷且已保留 `.part`，下次可續傳。
        resumable: bool,
    },
}

/// 一次 [`download_missing`] 的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DownloadSummary {
    /// 已存在而跳過的項目數。
    pub skipped: usize,
    /// 新下載或續傳完成的項目數。
    pub downloaded: usize,
    /// 失敗的項目數。
    pub failed: usize,
}

impl DownloadSummary {
    /// 沒有任何失敗時回傳 `true`。
    pub fn is_success(self) -> bool {
        self.failed == 0
    }
}

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

/// 下載一個檔案需要的資訊。
struct Expected<'a> {
    url: &'a str,
    bytes: u64,
    sha256: &'a str,
}

/// 一次下載的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fetched {
    /// 從 0 下載完成，或既有 `.part` 已完整且通過驗證。
    Fresh,
    /// 從既有的 `.part` 續傳完成。
    Resumed {
        /// 續傳起點，位元組。
        from: u64,
    },
}

/// 下載所有缺少或內容不符的下載項；`Present` 直接跳過。
///
/// 只處理 `shipped: false` 且有 URL 的項目。單一檔案失敗不會擋下其他檔案，
/// 不會 panic，也不保證呼叫端在主執行緒之外；引擎應自己開背景執行緒。
///
/// 每個階段呼叫一次 `on_progress`：[`DownloadEvent::Started`]、數次
/// [`DownloadEvent::Progress`]，最後是 [`DownloadEvent::Done`] 或
/// [`DownloadEvent::Failed`]。`Present` 的項目完全不發事件。
///
/// 使用 [`DownloadPolicy::default`]（寬鬆 timeout）。引擎的背景下載改用
/// [`download_missing_with`] 搭配 [`DownloadPolicy::background`]。
pub fn download_missing(
    root: &Path,
    manifest: &DataManifest,
    on_progress: impl Fn(&DownloadEvent),
) -> DownloadSummary {
    download_missing_with(root, manifest, on_progress, DownloadPolicy::default())
}

/// 與 [`download_missing`] 相同，但可指定 timeout 策略。
///
/// 引擎用 [`DownloadPolicy::background`] 限制背景下載時間；時間到時
/// [`DownloadEvent::Failed`] 的 `resumable` 為 `true`，`.part` 會保留。
pub fn download_missing_with(
    root: &Path,
    manifest: &DataManifest,
    on_progress: impl Fn(&DownloadEvent),
    policy: DownloadPolicy,
) -> DownloadSummary {
    let report = DownloadReport::inspect(root, manifest);
    let mut summary = DownloadSummary::default();
    for item in &report.items {
        let target = root.join(&item.path);
        match &item.state {
            DownloadState::Present => {
                summary.skipped += 1;
                continue;
            }
            DownloadState::Unreadable { message } => {
                on_progress(&DownloadEvent::Failed {
                    path: item.path.clone(),
                    reason: format!("讀取失敗：{message}"),
                    resumable: false,
                });
                summary.failed += 1;
                continue;
            }
            DownloadState::SizeMismatch { .. } | DownloadState::HashMismatch { .. } => {
                // 現有檔案內容不對：刪掉重下；`.part` 留著給續傳用。
                if let Err(err) = fs::remove_file(&target) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        on_progress(&DownloadEvent::Failed {
                            path: item.path.clone(),
                            reason: format!("刪除舊檔失敗：{err}"),
                            resumable: false,
                        });
                        summary.failed += 1;
                        continue;
                    }
                }
            }
            DownloadState::Missing => {}
        }
        let Some(url) = item.url.as_deref() else {
            on_progress(&DownloadEvent::Failed {
                path: item.path.clone(),
                reason: "manifest 沒有 url".to_string(),
                resumable: false,
            });
            summary.failed += 1;
            continue;
        };
        let expected = Expected {
            url,
            bytes: item.bytes,
            sha256: &item.sha256,
        };
        match fetch_file(&target, &expected, &item.path, &on_progress, policy) {
            Ok(Fetched::Fresh) | Ok(Fetched::Resumed { .. }) => summary.downloaded += 1,
            Err(err) => {
                let resumable = matches!(err, DownloadError::Interrupted { .. });
                on_progress(&DownloadEvent::Failed {
                    path: item.path.clone(),
                    reason: err.to_string(),
                    resumable,
                });
                summary.failed += 1;
            }
        }
    }
    summary
}

/// 把 `target` 下載完成；已完成的 `.part` 直接升級成正式檔。
fn fetch_file(
    target: &Path,
    expected: &Expected<'_>,
    label: &str,
    on_progress: &impl Fn(&DownloadEvent),
    policy: DownloadPolicy,
) -> Result<Fetched, DownloadError> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| DownloadError::io(parent, err))?;
        }
    }
    on_progress(&DownloadEvent::Started {
        path: label.to_string(),
        bytes: expected.bytes,
    });

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
                on_progress(&DownloadEvent::Done {
                    path: label.to_string(),
                    resumed: false,
                });
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

    let agent = agent(policy);
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
                    if last.elapsed() >= PROGRESS_INTERVAL {
                        on_progress(&DownloadEvent::Progress {
                            path: label.to_string(),
                            done,
                            total: expected.bytes,
                        });
                        last = Instant::now();
                    }
                }
                Err(err) => {
                    // 中斷時保留 `.part`，下次從這裡續傳。先落盤，避免下次讀到舊資料。
                    let _ = file.flush();
                    let _ = file.sync_all();
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
        fs::rename(&part, target).map_err(|err| DownloadError::io(target, err))?;
        on_progress(&DownloadEvent::Done {
            path: label.to_string(),
            resumed: used_resume,
        });
        return Ok(if used_resume {
            Fetched::Resumed { from: resumed_from }
        } else {
            Fetched::Fresh
        });
    }
}

fn agent(policy: DownloadPolicy) -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(policy.connect_timeout))
        .timeout_recv_response(Some(policy.response_timeout))
        .timeout_recv_body(policy.body_timeout)
        .timeout_global(policy.global_timeout)
        .build()
        .into()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-data-download-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn manifest(path: &str, content: &[u8]) -> DataManifest {
        DataManifest {
            schema: crate::manifest::SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-20T00:00:00Z".to_string(),
            files: vec![DataFile {
                path: path.to_string(),
                bytes: content.len() as u64,
                sha256: sha256_hex(content),
                role: DataRole::ZhSelect,
                license: vec!["L-11".to_string()],
                shipped: false,
                url: Some("https://example.invalid/model.onnx".to_string()),
            }],
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn present_requires_size_and_hash() {
        let dir = temp_dir("present");
        let content = b"data";
        write(&dir, "models/zh-select/model.onnx", content);
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert_eq!(report.present(), 1);
        assert!(report.is_complete());
        assert_eq!(report.summary(), "已存在 1/1，缺少 0，不符 0");
        assert_eq!(report.items[0].state, DownloadState::Present);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_and_partial_are_distinguished() {
        let dir = temp_dir("missing");
        let manifest = manifest("models/zh-select/model.onnx", b"data");
        let report = DownloadReport::inspect(&dir, &manifest);
        assert_eq!(report.missing(), 1);
        assert_eq!(report.items[0].state, DownloadState::Missing);
        assert_eq!(report.items[0].partial_bytes, None);

        // 中斷的下載留下 2 位元組的暫存檔，狀態仍是缺少，但要能看出可續傳。
        write(&dir, "models/zh-select/model.onnx.part", b"da");
        let report = DownloadReport::inspect(&dir, &manifest);
        assert_eq!(report.items[0].state, DownloadState::Missing);
        assert_eq!(report.items[0].partial_bytes, Some(2));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_size_with_different_bytes_is_hash_mismatch() {
        let dir = temp_dir("hash");
        let content = b"data";
        // 大小相同、內容不同：分類要走到雜湊，不是大小。
        write(&dir, "models/zh-select/model.onnx", b"datb");
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert_eq!(report.mismatched(), 1);
        assert_eq!(report.present(), 0);
        assert!(matches!(
            report.items[0].state,
            DownloadState::HashMismatch { .. }
        ));
        assert_eq!(report.summary(), "已存在 0/1，缺少 0，不符 1");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn different_size_is_size_mismatch() {
        let dir = temp_dir("size");
        let content = b"data";
        write(&dir, "models/zh-select/model.onnx", b"data!");
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert!(matches!(
            report.items[0].state,
            DownloadState::SizeMismatch {
                expected: 4,
                actual: 5
            }
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path("/data/models/zh-select/model.onnx"),
            PathBuf::from("/data/models/zh-select/model.onnx.part")
        );
    }

    #[test]
    fn real_root_reports_download_items() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        // 用現行 catalog 重新產生，避免讀到舊 manifest 裡已移除的角色。
        let manifest = DataManifest::generate_now(&root).unwrap();
        let report = DownloadReport::inspect(&root, &manifest);
        assert_eq!(report.items.len(), 2);
        assert!(
            report.items.iter().all(|item| item.url.is_some()),
            "下載項要帶 URL"
        );
    }

    // - --------------------------------------------------------------------
    // download_missing 的行為測試：用本地 HTTP server，不需要外部網路。
    // - --------------------------------------------------------------------

    use std::io::{BufRead, BufReader as IoBufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// 本地 HTTP server；Drop 時停止執行緒。慢速模式用來模擬逾時中斷。
    struct TestServer {
        url: String,
        stop: Arc<AtomicBool>,
        requests: Arc<AtomicUsize>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(body: Vec<u8>, support_range: bool) -> Self {
            Self::new(body, support_range, 0, Duration::ZERO)
        }

        /// 第一次請求只送 `slow_after` 位元組就停 `slow_for`，之後的請求正常回應。
        fn start_slow(
            body: Vec<u8>,
            support_range: bool,
            slow_after: usize,
            slow_for: Duration,
        ) -> Self {
            Self::new(body, support_range, slow_after, slow_for)
        }

        fn new(body: Vec<u8>, support_range: bool, slow_after: usize, slow_for: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let counter = Arc::clone(&requests);
            let thread_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(_) => break,
                    };
                    let mut reader = IoBufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                        continue;
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
                    let index = counter.fetch_add(1, Ordering::SeqCst);
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
                    if stream.write_all(head.as_bytes()).is_err() {
                        continue;
                    }
                    if index == 0 && slow_for > Duration::ZERO && !payload.is_empty() {
                        let first = slow_after.min(payload.len());
                        if stream.write_all(&payload[..first]).is_err() {
                            continue;
                        }
                        let _ = stream.flush();
                        thread::sleep(slow_for);
                        if stream.write_all(&payload[first..]).is_err() {
                            continue;
                        }
                    } else if stream.write_all(payload).is_err() {
                        continue;
                    }
                    let _ = stream.flush();
                }
            });
            Self {
                url: format!("http://{addr}/model.onnx"),
                stop,
                requests,
                handle: Some(handle),
            }
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn download_manifest(path: &str, url: &str, body: &[u8]) -> DataManifest {
        DataManifest {
            schema: crate::manifest::SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-21T00:00:00Z".to_string(),
            files: vec![DataFile {
                path: path.to_string(),
                bytes: body.len() as u64,
                sha256: sha256_hex(body),
                role: DataRole::ZhSelect,
                license: vec!["L-11".to_string()],
                shipped: false,
                url: Some(url.to_string()),
            }],
        }
    }

    #[test]
    fn download_missing_fetches_and_reports_events() {
        let dir = temp_dir("api-fresh");
        let body = b"hello from api".to_vec();
        let server = TestServer::start(body.clone(), true);
        let manifest = download_manifest("models/zh-select/model.onnx", &server.url, &body);
        let events = Mutex::new(Vec::new());
        let summary = download_missing(&dir, &manifest, |event| {
            events.lock().unwrap().push(event.clone());
        });

        assert_eq!(
            summary,
            DownloadSummary {
                skipped: 0,
                downloaded: 1,
                failed: 0
            }
        );
        let target = dir.join("models/zh-select/model.onnx");
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(!part_path(&target).exists());
        assert_eq!(server.requests(), 1);
        let events = events.into_inner().unwrap();
        assert_eq!(
            events.first(),
            Some(&DownloadEvent::Started {
                path: "models/zh-select/model.onnx".to_string(),
                bytes: body.len() as u64,
            })
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, DownloadEvent::Done { resumed: false, .. })),
            "完成事件要標記非續傳：{events:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn download_missing_skips_present_without_request() {
        let dir = temp_dir("api-present");
        let body = b"data".to_vec();
        write(&dir, "models/zh-select/model.onnx", &body);
        let server = TestServer::start(body.clone(), true);
        let manifest = download_manifest("models/zh-select/model.onnx", &server.url, &body);
        let calls = AtomicUsize::new(0);
        let summary = download_missing(&dir, &manifest, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.downloaded, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "Present 不發事件");
        assert_eq!(server.requests(), 0, "Present 不發 HTTP 請求");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn download_missing_hash_mismatch_leaves_no_bad_file() {
        let dir = temp_dir("api-hash");
        let body = b"tampered".to_vec();
        let server = TestServer::start(body.clone(), true);
        let mut manifest = download_manifest("models/zh-select/model.onnx", &server.url, &body);
        manifest.files[0].sha256 = "0".repeat(64);
        let events = Mutex::new(Vec::new());
        let summary = download_missing(&dir, &manifest, |event| {
            events.lock().unwrap().push(event.clone());
        });

        assert_eq!(summary.failed, 1);
        assert!(!summary.is_success());
        let target = dir.join("models/zh-select/model.onnx");
        assert!(!target.exists(), "不能留下內容不對的正式檔");
        assert!(!part_path(&target).exists(), "SHA 不符要刪掉 .part");
        let events = events.into_inner().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                DownloadEvent::Failed { reason, .. } if reason.contains("SHA-256 不符")
            )),
            "要回報 SHA-256 不符：{events:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn download_missing_resumes_part() {
        let dir = temp_dir("api-resume");
        let body = b"0123456789".to_vec();
        let target = dir.join("models/zh-select/model.onnx");
        write(&dir, "models/zh-select/model.onnx.part", &body[..4]);
        let server = TestServer::start(body.clone(), true);
        let manifest = download_manifest("models/zh-select/model.onnx", &server.url, &body);
        let events = Mutex::new(Vec::new());
        let summary = download_missing(&dir, &manifest, |event| {
            events.lock().unwrap().push(event.clone());
        });

        assert_eq!(summary.downloaded, 1);
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(
            events
                .into_inner()
                .unwrap()
                .iter()
                .any(|event| matches!(event, DownloadEvent::Done { resumed: true, .. })),
            "續傳完成要標記 resumed"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn background_policy_keeps_part_on_timeout_and_resumes_next_run() {
        let dir = temp_dir("api-timeout-resume");
        let body = b"0123456789abcdefghij".to_vec();
        let server = TestServer::start_slow(body.clone(), true, 6, Duration::from_millis(150));
        let manifest = download_manifest("models/zh-select/model.onnx", &server.url, &body);
        let target = dir.join("models/zh-select/model.onnx");
        let part = part_path(&target);
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_secs(2),
            response_timeout: Duration::from_secs(2),
            body_timeout: Some(Duration::from_millis(20)),
            global_timeout: None,
        };

        let first = Mutex::new(Vec::new());
        let first_summary = download_missing_with(
            &dir,
            &manifest,
            |event| {
                first.lock().unwrap().push(event.clone());
            },
            policy,
        );
        assert_eq!(first_summary.failed, 1, "短 timeout 應中斷");
        assert!(!target.exists(), "中斷不能留下正式檔");
        assert_eq!(
            fs::read(&part).unwrap(),
            &body[..6],
            "中斷要保留已抓到的 .part"
        );
        let events = first.into_inner().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                DownloadEvent::Failed {
                    resumable: true,
                    ..
                }
            )),
            "逾時中斷應標記可續傳：{events:?}"
        );

        // 下一次啟動從 .part 續傳；這次伺服器正常回應，完成後驗證 SHA 並改名。
        let second = Mutex::new(Vec::new());
        let second_summary = download_missing_with(
            &dir,
            &manifest,
            |event| {
                second.lock().unwrap().push(event.clone());
            },
            policy,
        );
        assert_eq!(second_summary.downloaded, 1);
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(!part.exists());
        assert!(
            second
                .into_inner()
                .unwrap()
                .iter()
                .any(|event| matches!(event, DownloadEvent::Done { resumed: true, .. })),
            "續傳完成要標記 resumed"
        );
        assert_eq!(server.requests(), 2);
        fs::remove_dir_all(&dir).ok();
    }
}
