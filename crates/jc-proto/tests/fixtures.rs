//! 線格式凍結測試：用手寫 JSON fixture 鎖住 v2 協定的欄位名稱、型別與預設值。
//!
//! 每個 fixture 都先反序列化成對應型別，再序列化回去，與原 fixture 做語意比較
//! （`serde_json::Value` 相等，因此鍵順序與空白不計）。這樣欄位被改名、換型別
//! 或被拿掉預設值時，測試會直接失敗。
//!
//! fixture 一律手寫，不可用 `serde_json::to_string` 產生，否則等於拿實作測自己。

use std::fs;
use std::path::PathBuf;

use jc_proto::{ErrorCode, Key, KeyCode, Request, Response, PROTOCOL_VERSION};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// `docs/proto.md` 的內容，用來比對版本號與 `PROTOCOL_VERSION` 是否一致。
const PROTO_DOC: &str = include_str!("../../../docs/proto.md");

/// 讀取 fixture 的原始文字。
fn read_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("讀不到 {}: {err}", path.display()))
}

/// 把 fixture 解析成 `serde_json::Value`，供語意比較使用。
fn fixture_value(name: &str) -> Value {
    serde_json::from_str(&read_fixture(name))
        .unwrap_or_else(|err| panic!("{name} 不是合法 JSON: {err}"))
}

/// 反序列化 fixture、再序列化回去，並確認兩邊語意相等。
fn round_trip<T>(name: &str) -> T
where
    T: DeserializeOwned + Serialize,
{
    let text = read_fixture(name);
    let parsed: T = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{name} 無法解析成協定型別: {err}"));
    let again = serde_json::to_string(&parsed)
        .unwrap_or_else(|err| panic!("{name} 解析後無法序列化: {err}"));
    let serialized: Value = serde_json::from_str(&again)
        .unwrap_or_else(|err| panic!("{name} 序列化結果不是合法 JSON: {err}"));
    assert_eq!(
        fixture_value(name),
        serialized,
        "{name} 序列化後與 fixture 語意不同；線格式被改動了"
    );
    parsed
}

/// 從 `docs/proto.md` 取出宣告的 `PROTOCOL_VERSION`。
fn doc_declared_version() -> u32 {
    let line = PROTO_DOC
        .lines()
        .find(|line| line.starts_with("`PROTOCOL_VERSION = "))
        .expect("docs/proto.md 缺少 `PROTOCOL_VERSION = N` 宣告");
    let digits: String = line
        .trim_start_matches("`PROTOCOL_VERSION = ")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|err| panic!("docs/proto.md 的版本號不是整數: {err}"))
}

#[test]
fn request_hello_round_trips() {
    let request: Request = round_trip("request_hello.json");
    assert_eq!(
        request,
        Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "jc-tsf/0.1.0".into(),
        }
    );
}

#[test]
fn request_key_round_trips() {
    let request: Request = round_trip("request_key.json");
    assert_eq!(
        request,
        Request::Key {
            key: Key::new(KeyCode::Char('ㄇ'), Some('ㄇ'), false),
        }
    );
}

#[test]
fn request_snapshot_round_trips() {
    let request: Request = round_trip("request_snapshot.json");
    assert_eq!(request, Request::Snapshot);
}

#[test]
fn request_commit_round_trips() {
    let request: Request = round_trip("request_commit.json");
    assert_eq!(request, Request::Commit);
}

#[test]
fn request_shutdown_round_trips() {
    let request: Request = round_trip("request_shutdown.json");
    assert_eq!(request, Request::Shutdown);
}

#[test]
fn request_unknown_is_captured_and_stays_unknown() {
    // 未知的 `op` 由 serde 的 `other` 收下；它會遺失原始 op 名稱，所以不比較
    // 序列化結果，只確認轉回去仍然是 `Unknown`，引擎才能回 `UnknownOp`。
    let request: Request = round_trip_typed_only("request_unknown.json");
    assert_eq!(request, Request::Unknown);

    let again = serde_json::to_string(&Request::Unknown).expect("Unknown 必須可序列化");
    let reparsed: Request = serde_json::from_str(&again).expect("序列化結果必須可解析");
    assert_eq!(reparsed, Request::Unknown);
}

/// 只做反序列化，不比較序列化結果。
fn round_trip_typed_only<T>(name: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(&read_fixture(name))
        .unwrap_or_else(|err| panic!("{name} 無法解析成協定型別: {err}"))
}

#[test]
fn response_ready_round_trips() {
    let response: Response = round_trip("response_ready.json");
    assert_eq!(
        response,
        Response::Ready {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine/0.1.0".into(),
        }
    );
}

#[test]
fn response_state_minimal_accepts_version_one_shape() {
    let response: Response = round_trip("response_state_minimal.json");
    assert_eq!(
        response,
        Response::State {
            preedit: "我".into(),
            cursor: 1,
            candidates: vec!["我".into()],
            // v1 沒有的欄位在 v2 型別下必須拿到預設值。
            cursor_rune: 0,
            suggestion: None,
            label: None,
            navigating: false,
        }
    );
}

#[test]
fn response_state_v2_round_trips_all_new_fields() {
    let response: Response = round_trip("response_state_v2.json");
    assert_eq!(
        response,
        Response::State {
            preedit: "ㄨㄛˇ".into(),
            cursor: 3,
            candidates: vec!["我".into(), "窩".into()],
            cursor_rune: 3,
            suggestion: Some("ˇ".into()),
            label: Some("常用字".into()),
            navigating: true,
        }
    );
    match response {
        Response::State {
            cursor,
            cursor_rune,
            candidates,
            ..
        } => {
            // 文件承諾兩者同值，且候選目前只有字面。
            assert_eq!(cursor_rune, cursor);
            assert_eq!(candidates, ["我", "窩"]);
        }
        other => panic!("應是 State，得到 {other:?}"),
    }
}

#[test]
fn response_committed_round_trips() {
    let response: Response = round_trip("response_committed.json");
    assert_eq!(response, Response::Committed { text: "我".into() });
}

#[test]
fn response_error_round_trips() {
    let response: Response = round_trip("response_error.json");
    assert_eq!(
        response,
        Response::Error {
            code: ErrorCode::BadFrame,
            message: "JSON 解碼失敗".into(),
        }
    );
}

#[test]
fn fixtures_use_current_protocol_version() {
    let hello: Request = round_trip_typed_only("request_hello.json");
    match hello {
        Request::Hello { protocol, .. } => assert_eq!(protocol, PROTOCOL_VERSION),
        other => panic!("request_hello.json 應是 Hello，得到 {other:?}"),
    }

    let ready: Response = round_trip_typed_only("response_ready.json");
    match ready {
        Response::Ready { protocol, .. } => assert_eq!(protocol, PROTOCOL_VERSION),
        other => panic!("response_ready.json 應是 Ready，得到 {other:?}"),
    }
}

#[test]
fn protocol_version_matches_docs() {
    assert_eq!(
        PROTOCOL_VERSION,
        doc_declared_version(),
        "PROTOCOL_VERSION 與 docs/proto.md 的宣告不一致"
    );
    assert!(
        PROTO_DOC.contains(&format!("（v{PROTOCOL_VERSION}，凍結）")),
        "docs/proto.md 的標題版本號沒跟著 PROTOCOL_VERSION 更新"
    );
    assert!(
        PROTO_DOC.contains(&format!("| {PROTOCOL_VERSION} |")),
        "docs/proto.md 的變更記錄缺 v{PROTOCOL_VERSION} 這一列"
    );
}
