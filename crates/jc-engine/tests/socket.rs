//! 用 unix socket 起 `jc-engine`：序列服務多條連線，`Shutdown` 後清掉 socket。
//!
//! 非 unix 平台沒有 unix domain socket，整份測試不編譯。

#![cfg(unix)]

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use jc_proto::{
    read_message, write_message, Key, Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION,
};

struct SocketEngine {
    child: Child,
    path: PathBuf,
}

impl SocketEngine {
    fn spawn(path: PathBuf) -> Self {
        // 這筆用單一注音鍵驗證 session 隔離；Mixed 模式會多出 literal 候選，
        // 明確用 Zh 模式執行，保留原本「未完成的注音沒有候選」的語意。
        let child = Command::new(env!("CARGO_BIN_EXE_jc-engine"))
            .arg("--allow-empty-data")
            .arg("--mode")
            .arg("zh")
            .arg("--socket")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("jc-engine 應該可以啟動");
        let engine = Self { child, path };
        engine.wait_until_listening();
        engine
    }

    /// 等 socket 可以被連上（最多 5 秒）。
    fn wait_until_listening(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if UnixStream::connect(&self.path).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("5 秒內沒等到 {} 開始監聽", self.path.display());
    }

    fn connect(&self) -> Client {
        let stream = UnixStream::connect(&self.path).expect("連上 socket");
        Client {
            reader: BufReader::new(stream.try_clone().expect("複製 socket")),
            writer: stream,
        }
    }

    fn wait(&mut self) -> i32 {
        let status = self.child.wait().expect("等待 jc-engine 結束");
        status.code().unwrap_or(-1)
    }
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn call(&mut self, request: &Request) -> Response {
        write_message(&mut self.writer, request, DEFAULT_MAX_FRAME).expect("寫請求");
        read_message(&mut self.reader, DEFAULT_MAX_FRAME)
            .expect("讀回應")
            .expect("應該有回應")
    }
}

fn state(preedit: &str) -> Response {
    Response::state(jc_core::Snapshot::from_preedit(preedit))
}

fn temp_socket(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系統時間")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("jc-engine-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建立暫存資料夾");
    dir.join(name)
}

impl Drop for SocketEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn socket_serves_connections_sequentially_and_shutdown_cleans_up() {
    let path = temp_socket("engine.sock");
    let mut engine = SocketEngine::spawn(path.clone());

    {
        let mut first = engine.connect();
        assert_eq!(
            first.call(&Request::Hello {
                protocol: PROTOCOL_VERSION,
                client: "first".into(),
            }),
            Response::Ready {
                protocol: PROTOCOL_VERSION,
                engine: format!("jc-engine {}", env!("CARGO_PKG_VERSION")),
            }
        );
        assert_eq!(
            first.call(&Request::Key {
                key: Key::char('ㄅ', false)
            }),
            state("ㄅ")
        );
        // 不送 Shutdown 直接關閉，下一條連線應該拿到乾淨的 session。
    }

    let mut second = engine.connect();
    second.call(&Request::Hello {
        protocol: PROTOCOL_VERSION,
        client: "second".into(),
    });
    assert_eq!(
        second.call(&Request::Snapshot),
        state(""),
        "新連線不該看到上一條連線的組字"
    );
    second.call(&Request::Shutdown);

    assert_eq!(engine.wait(), 0, "Shutdown 應以 0 結束");
    assert!(
        !Path::new(&path).exists(),
        "結束後應移除 socket 檔：{}",
        path.display()
    );
}
