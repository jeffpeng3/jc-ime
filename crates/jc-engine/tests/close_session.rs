//! T12d：多連線 session id 與 Close 的回归測試。
//!
//! 依 `common.md`，測試只寫只編譯，由整合階段統一跑 gate。
//!
//! Unix socket 與 Windows named pipe 的監聽端都走同一條路徑：
//! `Engine::accept()` → `Connection::into_engine()` → `serve()` → `Engine::handle()`。
//! 本檔直接用記憶體 buffer 驅動 `serve()`，所以兩邊的 Close 行為由同一組測試
//! 覆蓋；Windows 端另有 `pipe_listener` 的進入點測試。

use std::io::Cursor;

use jc_engine::{serve, Engine};
use jc_proto::{
    read_message, write_message, ErrorCode, Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION,
};

fn hello(client: &str) -> Request {
    Request::Hello {
        protocol: PROTOCOL_VERSION,
        client: client.into(),
    }
}

fn ready_session(response: &Response) -> u64 {
    match response {
        Response::Ready { session, .. } => *session,
        other => panic!("應是 Ready，得到 {other:?}"),
    }
}

fn sessions_alive(engine: &mut Engine) -> usize {
    let Response::Health { info } = engine.handle(Request::Health) else {
        panic!("Health 應回 Health");
    };
    info.sessions
}

/// 用與 listen 端 worker 相同的方式跑完整訊框往返。
fn serve_all(engine: &mut Engine, requests: &[Request]) -> Vec<Response> {
    let mut input = Vec::new();
    for request in requests {
        write_message(&mut input, request, DEFAULT_MAX_FRAME).unwrap();
    }
    let mut reader = Cursor::new(input);
    let mut output = Vec::new();
    serve(engine, &mut reader, &mut output, DEFAULT_MAX_FRAME).unwrap();

    let mut reader = Cursor::new(output);
    let mut responses = Vec::new();
    while let Some(response) = read_message::<_, Response>(&mut reader, DEFAULT_MAX_FRAME).unwrap()
    {
        responses.push(response);
    }
    responses
}

fn ownership_error() -> Response {
    Response::error(ErrorCode::Internal, "session 不存在或不屬於這個連線")
}

#[test]
fn two_connections_get_distinct_session_ids_from_ready() {
    let listener = Engine::new();
    let mut first = listener.accept().into_engine();
    let mut second = listener.accept().into_engine();

    let first_id = ready_session(&first.handle(hello("first")));
    let second_id = ready_session(&second.handle(hello("second")));

    assert_eq!(first_id, 1, "第一條連線沿用舊的 id=1");
    assert_eq!(second_id, 2, "第二條連線拿到自己的 id");
    assert_ne!(first_id, second_id);
    assert_eq!(first.session_ids(), vec![first_id]);
    assert_eq!(second.session_ids(), vec![second_id]);
}

#[test]
fn each_connection_closes_its_own_session() {
    let listener = Engine::new();
    let mut first = listener.accept().into_engine();
    let mut second = listener.accept().into_engine();
    let first_id = ready_session(&first.handle(hello("first")));
    let second_id = ready_session(&second.handle(hello("second")));
    assert_eq!(sessions_alive(&mut first), 2);

    assert_eq!(
        first.handle(Request::Close { session: first_id }),
        Response::Acknowledged { session: first_id }
    );
    assert_eq!(sessions_alive(&mut first), 1, "只移除自己的 session");
    assert!(first.session_ids().is_empty());
    assert_eq!(second.session_ids(), vec![second_id]);

    assert_eq!(
        second.handle(Request::Close { session: second_id }),
        Response::Acknowledged { session: second_id }
    );
    assert_eq!(sessions_alive(&mut second), 0);
    assert!(second.session_ids().is_empty());
}

#[test]
fn close_with_another_connections_id_is_rejected() {
    let listener = Engine::new();
    let mut first = listener.accept().into_engine();
    let mut second = listener.accept().into_engine();
    let first_id = ready_session(&first.handle(hello("first")));
    let second_id = ready_session(&second.handle(hello("second")));

    assert_eq!(
        first.handle(Request::Close { session: second_id }),
        ownership_error(),
        "跨連線 Close 必須被歸屬檢查擋下"
    );
    assert_eq!(sessions_alive(&mut first), 2, "誤關不該移除對方的 session");
    assert_eq!(
        second.handle(Request::Close { session: second_id }),
        Response::Acknowledged { session: second_id }
    );
    assert_eq!(sessions_alive(&mut second), 1);
    assert_eq!(first.session_ids(), vec![first_id]);
}

#[test]
fn closed_session_is_removed_from_registry() {
    let listener = Engine::new();
    let mut first = listener.accept().into_engine();
    let id = ready_session(&first.handle(hello("first")));

    assert_eq!(sessions_alive(&mut first), 1);
    assert_eq!(
        first.handle(Request::Close { session: id }),
        Response::Acknowledged { session: id }
    );
    assert_eq!(sessions_alive(&mut first), 0, "registry 不再算這個 session");
    assert!(first.session_ids().is_empty());
    assert_eq!(
        first.handle(Request::Close { session: id }),
        ownership_error(),
        "重複 Close 也要回錯誤"
    );
}

#[test]
fn ready_and_close_session_ids_travel_over_the_wire() {
    let listener = Engine::new();
    let mut first = listener.accept().into_engine();
    let mut second = listener.accept().into_engine();

    // 第一條連線的 Ready 帶 1；故意拿第二條連線才會配置的 id=2 去 Close。
    let first_responses = serve_all(&mut first, &[hello("first"), Request::Close { session: 2 }]);
    assert_eq!(first_responses.len(), 2);
    let first_id = ready_session(&first_responses[0]);
    assert_eq!(first_id, 1);
    assert_eq!(first_responses[1], ownership_error());

    let second_responses = serve_all(
        &mut second,
        &[hello("second"), Request::Close { session: 2 }],
    );
    assert_eq!(second_responses.len(), 2);
    let second_id = ready_session(&second_responses[0]);
    assert_eq!(second_id, 2);
    assert_ne!(first_id, second_id);
    assert_eq!(
        second_responses[1],
        Response::Acknowledged { session: second_id }
    );

    // 回到第一條連線，用自己的 id Close，訊框層也要回 Acknowledged。
    assert_eq!(
        serve_all(&mut first, &[Request::Close { session: first_id }]),
        vec![Response::Acknowledged { session: first_id }]
    );
    assert!(first.session_ids().is_empty());
    assert!(second.session_ids().is_empty());
}
