//! 訊框與訊息讀寫的行為測試：往返、分次到達、長度限制、EOF。

use std::io::{self, Cursor, Read};

use jc_proto::frame::{self, Decoded, FrameError, DEFAULT_MAX_FRAME, HEADER_LEN};
use jc_proto::message::{read_message, write_message, ProtoError};
use jc_proto::{CandidateDetail, CandidateSource, Key, Request, Response, SegmentInfo};

/// 每次 `read` 最多回 1 byte，模擬資料一段一段到達。
struct OneByteReader<R>(R);

impl<R: Read> Read for OneByteReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut byte = [0u8; 1];
        match self.0.read(&mut byte)? {
            0 => Ok(0),
            _ => {
                buf[0] = byte[0];
                Ok(1)
            }
        }
    }
}

#[test]
fn frame_round_trip_keeps_empty_and_non_ascii_text() {
    let mut buf = Vec::new();
    for message in [
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
            rows: 0,
            columns: 0,
            handled: None,
        },
        Response::State {
            preedit: "ㄇㄚˊ ㄆㄛˊ".into(),
            cursor: 5,
            candidates: vec!["麻婆".into(), "痲婆".into()],
            details: vec![
                CandidateDetail {
                    reading: "ㄇㄚˊㄆㄛˊ".into(),
                    source: CandidateSource::Lexicon,
                    cost: 18,
                    learned: false,
                    pos: None,
                    key: None,
                    note: None,
                },
                CandidateDetail {
                    reading: "ㄇㄚˊㄆㄛˊ".into(),
                    source: CandidateSource::Personal,
                    cost: -2,
                    learned: true,
                    pos: None,
                    key: None,
                    note: None,
                },
            ],
            segments: vec![
                SegmentInfo {
                    start: 0,
                    end: 4,
                    mode: "zhuyin".into(),
                    script: "bopomofo".into(),
                },
                SegmentInfo {
                    start: 4,
                    end: 5,
                    mode: "mixed".into(),
                    script: "symbol".into(),
                },
            ],
            highlighted: 1,
            cursor_rune: 5,
            suggestion: Some("婆".into()),
            label: Some("common".into()),
            navigating: true,
            rows: 0,
            columns: 0,
            handled: None,
        },
    ] {
        write_message(&mut buf, &message, DEFAULT_MAX_FRAME).unwrap();
        let decoded: Response = read_message(&mut Cursor::new(&buf), DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        assert_eq!(decoded, message);
        buf.clear();
    }
}

#[test]
fn frame_layout_is_little_endian_length_prefix() {
    let encoded = frame::encode(br#"{"op":"snapshot"}"#).unwrap();
    assert_eq!(
        &encoded[..HEADER_LEN],
        &17u32.to_le_bytes(),
        "前綴是小端長度"
    );
    assert_eq!(&encoded[HEADER_LEN..], br#"{"op":"snapshot"}"#);
    assert_eq!(encoded.len(), HEADER_LEN + 17);
}

#[test]
fn decode_reports_incomplete_until_payload_arrives() {
    let encoded = frame::encode(b"abc").unwrap();
    assert_eq!(frame::decode(&encoded[..2]).unwrap(), Decoded::Incomplete);
    assert_eq!(frame::decode(&encoded[..5]).unwrap(), Decoded::Incomplete);
    assert_eq!(
        frame::decode(&encoded).unwrap(),
        Decoded::Complete {
            payload: b"abc",
            consumed: 7
        }
    );
}

#[test]
fn zero_length_frame_is_rejected() {
    assert_eq!(frame::encode(b""), Err(FrameError::Empty));
    let zero = 0u32.to_le_bytes();
    assert_eq!(frame::decode(&zero), Err(FrameError::Empty));
    let err = read_message::<_, Request>(&mut Cursor::new(zero.to_vec()), DEFAULT_MAX_FRAME)
        .expect_err("0 長度訊框應被拒絕");
    assert!(matches!(err, ProtoError::EmptyFrame));
}

#[test]
fn oversized_frame_is_rejected_before_payload_arrives() {
    let len = (DEFAULT_MAX_FRAME as u32 + 1).to_le_bytes();
    assert_eq!(
        frame::decode(&len),
        Err(FrameError::TooLarge {
            len: DEFAULT_MAX_FRAME + 1,
            max: DEFAULT_MAX_FRAME,
        })
    );
    assert_eq!(
        frame::encode_with_limit(&[b'x'; 8], 4),
        Err(FrameError::TooLarge { len: 8, max: 4 })
    );
    // 只有前綴、內容還沒到，也要先在長度檢查就拒絕。
    let err = read_message::<_, Request>(&mut Cursor::new(len.to_vec()), DEFAULT_MAX_FRAME)
        .expect_err("超長訊框應被拒絕");
    assert!(matches!(
        err,
        ProtoError::FrameTooLarge {
            len,
            max: DEFAULT_MAX_FRAME
        } if len == DEFAULT_MAX_FRAME + 1
    ));
}

#[test]
fn message_over_custom_limit_is_rejected() {
    let message = Response::State {
        preedit: "x".repeat(64),
        cursor: 64,
        candidates: Vec::new(),
        details: Vec::new(),
        segments: Vec::new(),
        highlighted: 0,
        cursor_rune: 64,
        suggestion: None,
        label: None,
        navigating: false,
        rows: 0,
        columns: 0,
        handled: None,
    };
    let err = write_message(&mut Vec::new(), &message, 32).expect_err("超過 32 bytes 應被拒絕");
    assert!(matches!(err, ProtoError::FrameTooLarge { max: 32, .. }));
}

#[test]
fn message_survives_one_byte_at_a_time_delivery() {
    let mut buf = Vec::new();
    let message = Request::Key {
        key: Key::char('ㄚ', false),
    };
    write_message(&mut buf, &message, DEFAULT_MAX_FRAME).unwrap();

    let mut reader = OneByteReader(Cursor::new(buf));
    let decoded: Request = read_message(&mut reader, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("應該讀到一個請求");
    assert_eq!(decoded, message);

    let mut reader = OneByteReader(Cursor::new(Vec::<u8>::new()));
    assert_eq!(
        read_message::<_, Request>(&mut reader, DEFAULT_MAX_FRAME).unwrap(),
        None,
        "開頭 EOF 是乾淨結束"
    );
}

#[test]
fn truncated_frame_is_an_error() {
    let header = 5u32.to_le_bytes();
    let mut short_header = header[..2].to_vec();
    let err = read_message::<_, Request>(&mut Cursor::new(short_header.clone()), DEFAULT_MAX_FRAME)
        .expect_err("前綴不足");
    assert!(matches!(err, ProtoError::Truncated));

    short_header.clear();
    short_header.extend_from_slice(&header);
    short_header.extend_from_slice(b"ab");
    let err = read_message::<_, Request>(&mut Cursor::new(short_header), DEFAULT_MAX_FRAME)
        .expect_err("內容不足");
    assert!(matches!(err, ProtoError::Truncated));
}

#[test]
fn invalid_json_maps_to_bad_frame() {
    let mut buf = Vec::new();
    let payload = b"not json";
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    let err = read_message::<_, Request>(&mut Cursor::new(buf), DEFAULT_MAX_FRAME)
        .expect_err("壞掉的 JSON 應被拒絕");
    assert_eq!(err.code(), jc_proto::ErrorCode::BadFrame);
}

#[test]
fn encoder_output_is_accepted_by_custom_limits() {
    let encoded = frame::encode_with_limit(b"1234", 4).unwrap();
    assert_eq!(
        frame::decode_with_limit(&encoded, 4).unwrap(),
        Decoded::Complete {
            payload: b"1234",
            consumed: 8
        }
    );
}
