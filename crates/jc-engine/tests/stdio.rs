//! 用真的行程做一次 stdio 往返：起 `jc-engine`、握手、按鍵、提交、關機。

use std::io::BufReader;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use jc_proto::{
    read_message, write_message, ErrorCode, Key, KeyCode, Request, Response, DEFAULT_MAX_FRAME,
    PROTOCOL_VERSION,
};

struct EngineProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl EngineProcess {
    fn spawn() -> Self {
        // 這些協定測試拿單一注音鍵當組字輸入；Mixed 模式會多出 literal 候選，
        // 明確用 Zh 模式執行，保留原本的 State 與提交語意。
        let mut child = Command::new(env!("CARGO_BIN_EXE_jc-engine"))
            .arg("--allow-empty-data")
            .arg("--mode")
            .arg("zh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("jc-engine 應該可以啟動");
        let stdin = child.stdin.take().expect("應該有 stdin");
        let stdout = child.stdout.take().expect("應該有 stdout");
        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        }
    }

    fn call(&mut self, request: &Request) -> Response {
        write_message(&mut self.stdin, request, DEFAULT_MAX_FRAME).expect("寫請求");
        read_message(&mut self.stdout, DEFAULT_MAX_FRAME)
            .expect("讀回應")
            .expect("應該有回應")
    }

    fn expect_eof(&mut self) {
        let next: Option<Response> =
            read_message(&mut self.stdout, DEFAULT_MAX_FRAME).expect("讀 EOF");
        assert!(next.is_none(), "Shutdown 之後不該再有回應");
    }

    fn wait(mut self) -> i32 {
        drop(self.stdin);
        let status = self.child.wait().expect("等待 jc-engine 結束");
        status.code().unwrap_or(-1)
    }
}

fn state(preedit: &str) -> Response {
    Response::state(jc_core::Snapshot::from_preedit(preedit))
}

#[test]
fn stdio_round_trip_and_clean_shutdown() {
    let mut engine = EngineProcess::spawn();

    assert_eq!(
        engine.call(&Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "integration".into(),
        }),
        Response::Ready {
            protocol: PROTOCOL_VERSION,
            engine: format!("jc-engine {}", env!("CARGO_PKG_VERSION")),
        }
    );

    assert_eq!(
        engine.call(&Request::Key {
            key: Key::char('ㄇ', false)
        }),
        state("ㄇ")
    );
    assert_eq!(
        engine.call(&Request::Key {
            key: Key::char('ㄚ', false)
        }),
        state("ㄇㄚ")
    );
    assert_eq!(engine.call(&Request::Snapshot), state("ㄇㄚ"));

    assert_eq!(
        engine.call(&Request::Key {
            key: Key::new(KeyCode::Backspace, None, false)
        }),
        state("ㄇ")
    );
    assert_eq!(
        engine.call(&Request::Commit),
        Response::Committed { text: "ㄇ".into() }
    );
    assert_eq!(engine.call(&Request::Snapshot), state(""));

    assert_eq!(engine.call(&Request::Shutdown), state(""));
    engine.expect_eof();
    assert_eq!(engine.wait(), 0, "Shutdown 應以 0 結束");
}

#[test]
fn version_mismatch_is_rejected_and_closed() {
    let mut engine = EngineProcess::spawn();
    let response = engine.call(&Request::Hello {
        protocol: PROTOCOL_VERSION + 1,
        client: "future".into(),
    });
    match response {
        Response::Error { code, .. } => assert_eq!(code, ErrorCode::VersionMismatch),
        other => panic!("應該回 VersionMismatch，收到 {other:?}"),
    }
    engine.expect_eof();
    assert_eq!(engine.wait(), 0, "版本不符只關連線，不是失敗");
}

#[test]
fn eof_without_requests_exits_cleanly() {
    let engine = EngineProcess::spawn();
    assert_eq!(engine.wait(), 0, "stdin 關閉應以 0 結束");
}
