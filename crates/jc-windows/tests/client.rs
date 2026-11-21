//! `jc-windows` 的行為測試。
//!
//! 只用假串流，不連真的引擎，也不需要 Windows。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use jc_proto::{write_message, ErrorCode, Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION};
use jc_windows::{Client, Error};

/// 測試與假串流共用的觀察窗口。
#[derive(Clone)]
struct Shared {
    /// client 寫出的位元組。
    written: Arc<Mutex<Vec<u8>>>,
    /// client 從串流讀走的位元組數。
    consumed: Arc<AtomicUsize>,
    /// 串流是否已被丟掉。
    dropped: Arc<AtomicBool>,
    /// 讓阻塞中的讀取放行的訊號。
    release: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            written: Arc::new(Mutex::new(Vec::new())),
            consumed: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
            release: Arc::new(Mutex::new(None)),
        }
    }

    /// client 目前寫出的全部位元組。
    fn written(&self) -> Vec<u8> {
        match self.written.lock() {
            Ok(bytes) => bytes.clone(),
            Err(error) => panic!("寫入緩衝區鎖壞掉：{error}"),
        }
    }

    /// client 已讀走的位元組數。
    fn consumed(&self) -> usize {
        self.consumed.load(Ordering::SeqCst)
    }

    /// 放行卡在讀取的背景執行緒。
    fn release(&self) {
        if let Ok(mut sender) = self.release.lock() {
            sender.take();
        }
    }

    /// 等串流真的被丟掉；用來確認版本不符時有斷線。
    fn wait_dropped(&self, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if self.dropped.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.dropped.load(Ordering::SeqCst)
    }
}

/// 可腳本化的假串流：先吐出預錄的位元組，吐完後阻塞直到測試放行。
struct MockStream {
    /// 預錄的回應訊框。
    incoming: Vec<u8>,
    /// 已吐出的位置。
    cursor: usize,
    /// 吐完後用來阻塞的訊號。
    block: mpsc::Receiver<()>,
    /// 觀察窗口。
    shared: Shared,
}

impl MockStream {
    fn new(incoming: Vec<u8>) -> (Self, Shared) {
        let shared = Shared::new();
        let (release, block) = mpsc::channel();
        if let Ok(mut slot) = shared.release.lock() {
            *slot = Some(release);
        }
        let stream = Self {
            incoming,
            cursor: 0,
            block,
            shared: shared.clone(),
        };
        (stream, shared)
    }
}

impl Read for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cursor >= self.incoming.len() {
            // 模擬永不回應的引擎；測試呼叫 `release` 才會放行。
            let _ = self.block.recv();
            return Ok(0);
        }
        let remaining = self.incoming.len() - self.cursor;
        let count = remaining.min(buf.len());
        buf[..count].copy_from_slice(&self.incoming[self.cursor..self.cursor + count]);
        self.cursor += count;
        self.shared.consumed.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

impl Write for MockStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.shared.written.lock() {
            Ok(mut written) => written.extend_from_slice(buf),
            Err(error) => panic!("寫入緩衝區鎖壞掉：{error}"),
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for MockStream {
    fn drop(&mut self) {
        self.shared.dropped.store(true, Ordering::SeqCst);
    }
}

/// 把回應包成連續訊框。
fn encoded(responses: &[Response]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for response in responses {
        write_message(&mut bytes, response, DEFAULT_MAX_FRAME).unwrap();
    }
    bytes
}

/// 空的 `State` 回應。
fn empty_state() -> Response {
    Response::State {
        preedit: String::new(),
        cursor: 0,
        candidates: Vec::new(),
        details: Vec::new(),
        segments: Vec::new(),
        highlighted: 0,
        cursor_rune: 0,
        suggestion: None,
        label: None,
        navigating: false,
    }
}

/// 把 client 寫出的位元組解回請求序列。
fn requests(bytes: &[u8]) -> Vec<Request> {
    let mut cursor = io::Cursor::new(bytes);
    let mut requests = Vec::new();
    while let Some(request) = jc_proto::read_message(&mut cursor, DEFAULT_MAX_FRAME).unwrap() {
        requests.push(request);
    }
    requests
}

/// 握手用的 `Ready`。
fn ready() -> Response {
    Response::Ready {
        protocol: PROTOCOL_VERSION,
        engine: "jc-engine test".into(),
        session: 1,
    }
}

fn handshake(
    client_name: &str,
    responses: &[Response],
) -> (Result<Client<MockStream>, Error>, Shared) {
    let (stream, shared) = MockStream::new(encoded(responses));
    let result = Client::handshake(stream, client_name, Duration::from_secs(1));
    (result, shared)
}

#[test]
fn handshake_sends_hello_and_reads_exactly_one_frame() {
    let (result, shared) = handshake("jc-tsf/0.1.0", &[ready()]);
    let client = match result {
        Ok(client) => client,
        Err(error) => panic!("握手應該成功：{error}"),
    };

    assert!(!client.is_closed());
    assert_eq!(
        shared.consumed(),
        encoded(&[ready()]).len(),
        "只該讀走一個 Ready 訊框"
    );
    assert_eq!(
        requests(&shared.written()),
        vec![Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "jc-tsf/0.1.0".into(),
        }],
        "握手只送一個 Hello"
    );

    shared.release();
}

#[test]
fn version_mismatch_is_reported_and_closes_the_stream() {
    let (result, shared) = handshake(
        "old",
        &[Response::error(ErrorCode::VersionMismatch, "版本不符")],
    );

    match result {
        Ok(_) => panic!("版本不符不該成功"),
        Err(Error::VersionMismatch { engine, client }) => {
            assert_eq!(engine, 0, "引擎沒有回報自己的版本");
            assert_eq!(client, PROTOCOL_VERSION);
        }
        Err(other) => panic!("應是 VersionMismatch，得到 {other}"),
    }
    assert!(
        shared.wait_dropped(Duration::from_secs(1)),
        "版本不符要關閉串流"
    );
}

#[test]
fn overlong_frame_prefix_is_rejected_without_reading_the_payload() {
    // 只給 4-byte 前綴，長度遠超上限；不該為它配置緩衝，也不該繼續等內容。
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    let (stream, _shared) = MockStream::new(bytes);

    match Client::handshake(stream, "test", Duration::from_secs(1)) {
        Ok(_) => panic!("超長訊框不該成功"),
        Err(Error::BadFrame(message)) => {
            assert!(message.contains("超過上限"), "錯誤應說明超長：{message}");
        }
        Err(other) => panic!("應是 BadFrame，得到 {other}"),
    }
}

#[test]
fn timeout_marks_client_unusable() {
    let (result, shared) = handshake("test", &[ready()]);
    let mut client = match result {
        Ok(client) => client,
        Err(error) => panic!("握手應該成功：{error}"),
    };

    let first = client.request(&Request::Snapshot, Duration::from_millis(50));
    assert!(matches!(first, Err(Error::Timeout)), "第一個請求應逾時");

    let started = Instant::now();
    let second = client.request(&Request::Snapshot, Duration::from_secs(5));
    assert!(
        matches!(second, Err(Error::Timeout)),
        "逾時後應直接回 Timeout"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "不該再等一次逾時"
    );
    assert!(!client.is_closed(), "逾時不等於引擎要求關閉");

    shared.release();
}

#[test]
fn shutdown_closes_the_client() {
    let (result, shared) = handshake("test", &[ready(), empty_state()]);
    let mut client = match result {
        Ok(client) => client,
        Err(error) => panic!("握手應該成功：{error}"),
    };

    assert!(!client.is_closed());
    let response = client.request(&Request::Shutdown, Duration::from_secs(1));
    assert!(
        matches!(response, Ok(Response::State { .. })),
        "Shutdown 應回一個 State"
    );
    assert!(client.is_closed(), "送出 Shutdown 後應視為關閉");

    shared.release();
}

/// 統一 transport 後非 Windows 也能連 local socket；沒有監聽者時回 Io 錯誤。
#[cfg(not(windows))]
#[test]
fn connect_fails_without_a_listener_off_windows() {
    let name = format!("jc-ime-missing-{}", std::process::id());
    let err = Client::connect(&name, "test", Duration::from_millis(10)).err();
    assert!(matches!(err, Some(Error::Io(_))), "得到 {err:?}");
}

#[cfg(not(windows))]
#[test]
fn connect_with_retry_fails_without_a_listener_off_windows() {
    let name = format!("jc-ime-missing-retry-{}", std::process::id());
    let err =
        Client::connect_with_retry(&name, "test", Duration::from_millis(10), 3, Duration::ZERO)
            .err();
    assert!(matches!(err, Some(Error::Io(_))), "得到 {err:?}");
}
