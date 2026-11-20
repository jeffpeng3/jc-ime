//! 背景往返執行緒。
//!
//! 串流由這條執行緒獨佔；呼叫端把 [`Request`] 丟進 channel，等 [`Response`]。
//! 這樣任何 `Read + Write` 串流都能有逾時，不必為每一種傳輸另外寫非同步 I/O。

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use jc_proto::message::ProtoError;
use jc_proto::{read_message, write_message, Request, Response, DEFAULT_MAX_FRAME};

use crate::Error;

/// 一個往返工作。
struct Job {
    /// 要送出的請求。
    request: Request,
    /// 回應放這裡。
    reply: Sender<Result<Option<Response>, ProtoError>>,
}

/// 背景往返執行緒的把手。
pub(crate) struct Worker {
    /// 工作佇列；`None` 代表已經關閉。
    jobs: Option<Sender<Job>>,
    /// 執行緒的原始 handle，Windows 上拿來取消阻塞中的讀取。
    thread_handle: usize,
    /// 保留 `JoinHandle` 才不會關掉上面那個 handle；不 join，丟掉時只是 detach。
    _thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// 開一條執行緒專心收送這條串流。
    pub(crate) fn spawn<S: Read + Write + Send + 'static>(stream: S) -> Result<Self, Error> {
        let (jobs, incoming) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("jc-windows-io".into())
            .spawn(move || run(stream, incoming))?;
        let thread_handle = raw_thread_handle(&handle);
        // 不 join：執行緒可能正卡在讀取，join 會把呼叫端一起卡住。
        Ok(Self {
            jobs: Some(jobs),
            thread_handle,
            _thread: Some(handle),
        })
    }

    /// 送出一個請求並等待回應。
    ///
    /// 逾時時會打斷背景執行緒上阻塞中的讀取，並回 [`Error::Timeout`]。呼叫端
    /// 必須把這個 client 丟掉：串流裡可能還有上一個請求的殘留位元組。
    pub(crate) fn round_trip(
        &self,
        request: Request,
        timeout: Duration,
    ) -> Result<Option<Response>, Error> {
        let (reply, replies) = mpsc::channel();
        let sender = self.jobs.as_ref().ok_or_else(connection_closed)?;
        if sender.send(Job { request, reply }).is_err() {
            return Err(connection_closed());
        }

        match replies.recv_timeout(timeout) {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(protocol_error(error)),
            Err(RecvTimeoutError::Timeout) => {
                self.cancel_blocking_read();
                Err(Error::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => Err(connection_closed()),
        }
    }

    /// 取消背景執行緒上阻塞中的 I/O。
    fn cancel_blocking_read(&self) {
        cancel_blocking_read(self.thread_handle);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // 先放掉工作佇列，讓停在 `recv` 的執行緒自然結束；再取消阻塞中的讀取，
        // 讓停在 `read` 的執行緒也能收工，串流（named pipe handle）才會關閉。
        self.jobs = None;
        self.cancel_blocking_read();
    }
}

/// 執行緒主體：一個請求配一個回應，序列處理。
fn run<S: Read + Write>(mut stream: S, jobs: Receiver<Job>) {
    while let Ok(job) = jobs.recv() {
        let outcome = write_message(&mut stream, &job.request, DEFAULT_MAX_FRAME)
            .and_then(|()| read_message::<_, Response>(&mut stream, DEFAULT_MAX_FRAME));
        // 呼叫端可能已經因為逾時而離開，送不出去就算了。
        let _ = job.reply.send(outcome);
    }
}

/// 把訊框層錯誤轉成公開的錯誤。
fn protocol_error(error: ProtoError) -> Error {
    match error {
        ProtoError::Io(error) => Error::Io(error),
        other => Error::BadFrame(other.to_string()),
    }
}

/// 工作佇列斷了，代表背景執行緒已經結束。
fn connection_closed() -> Error {
    Error::Io(io::Error::new(io::ErrorKind::BrokenPipe, "引擎連線已結束"))
}

/// 取得執行緒的原始 handle；非 Windows 平台不需要。
fn raw_thread_handle(handle: &JoinHandle<()>) -> usize {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        handle.as_raw_handle() as usize
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        0
    }
}

/// 取消指定執行緒上阻塞中的同步 I/O。
///
/// 只有 Windows 有這個機制；其他平台是 no-op，被阻塞的執行緒會留在背景，直到
/// 對端關閉串流。這只影響清理，不影響逾時判定。
fn cancel_blocking_read(thread_handle: usize) {
    #[cfg(windows)]
    {
        if thread_handle == 0 {
            return;
        }
        // SAFETY: handle 來自本 crate 建立的執行緒；失敗只代表當時沒有阻塞中的 I/O。
        unsafe {
            let _ = windows_sys::Win32::System::IO::CancelSynchronousIo(
                thread_handle as windows_sys::Win32::Foundation::HANDLE,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = thread_handle;
    }
}
