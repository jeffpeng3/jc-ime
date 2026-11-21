//! 引擎行程的請求／回應迴圈。
//!
//! 這一層只處理協定與行程行為；組字與轉換全部交給 `jc_core::Session`。
//! 每次請求都回傳完整快照，wrapper 不需要自己保存狀態。
//!
//! 資料載入在 [`loader`]；`jc-cli` 共用同一份載入程式碼。

use std::io::{Read, Write};

use jc_core::session::{Session, SessionConfig};
use jc_core::{KeyCode, Mode, Snapshot};

pub mod config;
pub mod loader;
pub mod supervise;
use jc_proto::message::{read_message, ProtoError};
use jc_proto::{write_message, ErrorCode, Request, Response, PROTOCOL_VERSION};

/// 回報給 wrapper 的引擎識別字串。
pub fn engine_label() -> String {
    format!("jc-engine {}", env!("CARGO_PKG_VERSION"))
}

/// 協定引擎。組字狀態全部在 [`Session`] 裡。
pub struct Engine {
    hello: bool,
    session: Session,
    close_connection: bool,
    shutdown: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// 建立一個尚未握手、沒有額外資料的引擎。
    pub fn new() -> Self {
        Self::with_session(Session::new(SessionConfig::default()))
    }

    /// 用載入好的資料建立引擎；`mode` 決定啟動時的輸入語言。
    pub fn with_data(data: &loader::EngineData, mode: Mode) -> Self {
        Self::with_session(Session::new(data.session_config(mode)))
    }

    /// 用指定的工作階段建立引擎。
    pub fn with_session(session: Session) -> Self {
        Self {
            hello: false,
            session,
            close_connection: false,
            shutdown: false,
        }
    }

    /// 目前的組字字串。
    pub fn composition(&self) -> String {
        self.session.preedit()
    }

    /// 這一條連線是否該收掉（版本不符或收到 `Shutdown`）。
    ///
    /// `Shutdown` 會同時設定這個旗標；全行程的 shutdown 狀態另外由
    /// [`Engine::is_shutdown`] 保存，`reset_session` 只清連線狀態。
    pub fn is_closing(&self) -> bool {
        self.close_connection
    }

    /// 是否收到 `Shutdown`，整個行程要結束。
    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    /// 清除單一連線的狀態，讓 socket 伺服器服務下一個連線。
    ///
    /// `Shutdown` 是全行程的旗標，不會被清掉。
    pub fn reset_session(&mut self) {
        self.hello = false;
        self.session.reset();
        self.close_connection = false;
    }

    /// 處理一個請求。任何請求都會得到一個回應。
    ///
    /// 版本不符或收到 `Shutdown` 時會設定收線旗標，呼叫端寫完回應後結束連線。
    pub fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Hello { protocol, client } => {
                if protocol != PROTOCOL_VERSION {
                    self.close_connection = true;
                    return Response::error(
                        ErrorCode::VersionMismatch,
                        format!("協定版本不符：客戶端 {protocol}，引擎 {PROTOCOL_VERSION}"),
                    );
                }
                self.hello = true;
                log::info!("客戶端 {} 完成握手（協定 {protocol}）", client);
                Response::Ready {
                    protocol: PROTOCOL_VERSION,
                    engine: engine_label(),
                }
            }
            Request::Key { key } => {
                if !self.hello {
                    return Response::error(ErrorCode::Internal, "尚未完成握手");
                }
                let enters = key.code == KeyCode::Enter;
                let snapshot = self.session.press(key);
                if enters {
                    let text = self.session.commit();
                    if !text.is_empty() {
                        return Response::Committed { text };
                    }
                }
                state(snapshot)
            }
            Request::Toggle => {
                if !self.hello {
                    return Response::error(ErrorCode::Internal, "尚未完成握手");
                }
                // 單按左右 Shift：切到 En 或回到切換前的 Mixed／Ja。
                state(self.session.toggle_english())
            }
            Request::Snapshot => {
                if !self.hello {
                    return Response::error(ErrorCode::Internal, "尚未完成握手");
                }
                self.snapshot()
            }
            Request::Commit => {
                if !self.hello {
                    return Response::error(ErrorCode::Internal, "尚未完成握手");
                }
                self.commit()
            }
            Request::Shutdown => {
                self.shutdown = true;
                self.close_connection = true;
                self.snapshot()
            }
            Request::Unknown => Response::error(ErrorCode::UnknownOp, "未知的操作"),
            // T3 新增的 0c op 在 T4／T6 接手前先回明確的「不支援」，不要落入
            // UnknownOp，讓 adapter 能區分協定不認識與引擎尚未實作。
            other => Response::error(ErrorCode::Unsupported, format!("jc 尚未支援 {other:?}")),
        }
    }

    /// 目前的完整快照。
    fn snapshot(&self) -> Response {
        state(self.session.snapshot())
    }

    /// 取走目前的組字並回傳提交結果。
    fn commit(&mut self) -> Response {
        Response::Committed {
            text: self.session.commit(),
        }
    }
}

/// 把核心快照轉成協定回應。
fn state(snapshot: Snapshot) -> Response {
    Response::state(snapshot)
}

/// 服務一條連線直到 EOF、`Shutdown` 或協定錯誤。
///
/// 讀取端乾淨 EOF 回傳 `Ok(())`。協定層錯誤（壞訊框、超長、JSON 壞掉）會先
/// 回一個 `Error` 回應再結束，因為資料流已經不可信。
pub fn serve<R: Read, W: Write>(
    engine: &mut Engine,
    input: &mut R,
    output: &mut W,
    max_frame: usize,
) -> Result<(), ServeError> {
    loop {
        match read_message::<_, Request>(input, max_frame) {
            Ok(None) => return Ok(()),
            Ok(Some(request)) => {
                let response = engine.handle(request);
                write_message(output, &response, max_frame)?;
                if engine.is_closing() {
                    return Ok(());
                }
            }
            Err(err) => {
                log::warn!("協定錯誤，結束連線：{err}");
                let response = Response::error(err.code(), err.to_string());
                write_message(output, &response, max_frame)?;
                return Ok(());
            }
        }
    }
}

/// 迴圈層級的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// 回應寫不出去。
    #[error("寫出回應失敗：{0}")]
    Write(#[from] ProtoError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_core::Key;
    use std::io::Cursor;

    fn hello(engine: &mut Engine) {
        let response = engine.handle(Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "test".into(),
        });
        assert!(matches!(response, Response::Ready { .. }));
    }

    /// 沒有候選的預期 State。
    fn state_of(preedit: &str) -> Response {
        Response::state(Snapshot::from_preedit(preedit))
    }

    #[test]
    fn version_mismatch_closes_after_error() {
        let mut engine = Engine::new();
        let response = engine.handle(Request::Hello {
            protocol: PROTOCOL_VERSION + 1,
            client: "old".into(),
        });
        assert_eq!(
            response,
            Response::error(
                ErrorCode::VersionMismatch,
                format!(
                    "協定版本不符：客戶端 {}，引擎 {}",
                    PROTOCOL_VERSION + 1,
                    PROTOCOL_VERSION
                )
            )
        );
        assert!(engine.is_closing());
        assert!(!engine.is_shutdown());
    }

    #[test]
    fn key_edits_composition_and_commit_takes_it() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Key {
            key: Key::char('ㄇ', false),
        });
        engine.handle(Request::Key {
            key: Key::char('ㄚ', false),
        });
        assert_eq!(engine.handle(Request::Snapshot), state_of("ㄇㄚ"));
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed {
                text: "ㄇㄚ".into()
            }
        );
        assert_eq!(engine.composition(), "");
        assert_eq!(engine.handle(Request::Snapshot), state_of(""));
    }

    #[test]
    fn backspace_removes_one_character() {
        let mut engine = Engine::new();
        hello(&mut engine);
        for ch in "注音".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        });
        assert_eq!(engine.composition(), "注");
    }

    #[test]
    fn requests_before_hello_are_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Commit),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
        assert_eq!(engine.composition(), "");
    }

    #[test]
    fn key_produces_converted_candidates() {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let config = SessionConfig {
            zh_lexicon: Some(lexicon),
            ..Default::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);

        for ch in "ji3".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        let snapshot = engine.handle(Request::Snapshot);
        let Response::State {
            preedit,
            cursor,
            cursor_rune,
            candidates,
            ..
        } = snapshot
        else {
            panic!("快照應是 State");
        };
        assert_eq!(preedit, "我");
        assert_eq!(cursor, 1);
        assert_eq!(cursor_rune, 1);
        assert_eq!(candidates.first().map(String::as_str), Some("我"));
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed { text: "我".into() }
        );
    }

    #[test]
    fn toggle_request_switches_english_and_restores_previous_mode() {
        let config = SessionConfig::new(jc_core::Mode::Mixed);
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);
        assert_eq!(engine.session.mode(), jc_core::Mode::Mixed);
        assert!(matches!(
            engine.handle(Request::Toggle),
            Response::State { .. }
        ));
        assert_eq!(engine.session.mode(), jc_core::Mode::En);
        engine.handle(Request::Toggle);
        assert_eq!(engine.session.mode(), jc_core::Mode::Mixed);
    }

    #[test]
    fn toggle_before_hello_is_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Toggle),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
    }

    #[test]
    fn unknown_op_reports_unknown_op() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Unknown),
            Response::error(ErrorCode::UnknownOp, "未知的操作")
        );
    }

    #[test]
    fn shutdown_marks_process_exit() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Shutdown);
        assert!(engine.is_shutdown());
        assert!(engine.is_closing());
        engine.reset_session();
        assert!(engine.is_shutdown(), "reset_session 不該清掉 Shutdown");
        assert!(!engine.is_closing());
    }

    #[test]
    fn serve_returns_after_clean_eof() {
        let mut engine = Engine::new();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut input,
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();
        assert!(output.is_empty(), "沒有請求就不該有回應");
    }

    #[test]
    fn serve_answers_version_mismatch_then_stops() {
        let mut engine = Engine::new();
        let mut input = Vec::new();
        write_message(
            &mut input,
            &Request::Hello {
                protocol: PROTOCOL_VERSION + 1,
                client: "test".into(),
            },
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();
        // Shutdown 排在後面，版本不符應該先斷線、不處理它。
        write_message(&mut input, &Request::Shutdown, jc_proto::DEFAULT_MAX_FRAME).unwrap();

        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut Cursor::new(input),
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();

        let mut replies = Cursor::new(output);
        let reply: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        assert!(matches!(
            reply,
            Response::Error {
                code: ErrorCode::VersionMismatch,
                ..
            }
        ));
        assert!(
            read_message::<_, Response>(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
                .unwrap()
                .is_none(),
            "只能有一個回應"
        );
    }

    #[test]
    fn serve_reports_bad_frame_and_closes() {
        let mut engine = Engine::new();
        hello(&mut engine);
        let mut input = Vec::new();
        write_message(&mut input, &Request::Snapshot, jc_proto::DEFAULT_MAX_FRAME).unwrap();
        input.extend_from_slice(&5u32.to_le_bytes());
        input.extend_from_slice(b"nope!");

        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut Cursor::new(input),
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();

        let mut replies = Cursor::new(output);
        let first: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        assert!(matches!(first, Response::State { .. }));
        let second: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        match second {
            Response::Error { code, .. } => assert_eq!(code, ErrorCode::BadFrame),
            other => panic!("壞訊框應回 BadFrame，收到 {other:?}"),
        }
    }
}
