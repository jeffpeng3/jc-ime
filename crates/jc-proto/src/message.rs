//! 從 `Read` 讀完整訊息、把訊息寫到 `Write`。
//!
//! 讀取端會先讀滿 4 bytes 前綴，再依長度讀滿內容，所以資料分次到達也能
//! 正確組回一個訊息。開頭就遇到 EOF 回傳 `Ok(None)`，代表對端乾淨結束。

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::frame::{self, FrameError, HEADER_LEN};
use crate::ErrorCode;

/// 協定層錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    /// 底層讀寫失敗。
    #[error("讀寫失敗：{0}")]
    Io(#[from] io::Error),
    /// 訊框長度為 0。
    #[error("訊框長度為 0")]
    EmptyFrame,
    /// 訊框超過上限。
    #[error("訊框 {len} 位元組超過上限 {max}")]
    FrameTooLarge {
        /// 內容長度。
        len: usize,
        /// 允許的上限。
        max: usize,
    },
    /// 資料流在訊框完成前就結束。
    #[error("訊框在完成前就結束")]
    Truncated,
    /// JSON 內容無法解碼。
    #[error("JSON 解碼失敗：{0}")]
    Json(#[from] serde_json::Error),
}

impl ProtoError {
    /// 這個錯誤對應的協定錯誤碼。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Io(_) | Self::Json(_) | Self::Truncated => ErrorCode::BadFrame,
            Self::EmptyFrame => ErrorCode::BadFrame,
            Self::FrameTooLarge { .. } => ErrorCode::FrameTooLarge,
        }
    }
}

impl From<FrameError> for ProtoError {
    fn from(err: FrameError) -> Self {
        match err {
            FrameError::Empty => Self::EmptyFrame,
            FrameError::TooLarge { len, max } => Self::FrameTooLarge { len, max },
        }
    }
}

/// 讀出下一個訊框的原始內容。
///
/// 回傳 `Ok(None)` 表示在訊框開頭遇到 EOF。前綴或內容只到一半就 EOF 會回
/// [`ProtoError::Truncated`]。
pub fn read_payload<R: Read>(
    reader: &mut R,
    max_frame: usize,
) -> Result<Option<Vec<u8>>, ProtoError> {
    let mut header = [0u8; HEADER_LEN];
    let got = read_into(reader, &mut header)?;
    if got == 0 {
        return Ok(None);
    }
    if got < HEADER_LEN {
        return Err(ProtoError::Truncated);
    }

    let len = u32::from_le_bytes(header) as usize;
    // 先用 4 bytes 前綴驗 0 長度與上限，避免為超長訊框配置記憶體。
    frame::decode_with_limit(&header, max_frame)?;

    let mut payload = vec![0u8; len];
    if read_into(reader, &mut payload)? < len {
        return Err(ProtoError::Truncated);
    }
    Ok(Some(payload))
}

/// 讀出下一個訊息並解成 `T`。
pub fn read_message<R: Read, T: DeserializeOwned>(
    reader: &mut R,
    max_frame: usize,
) -> Result<Option<T>, ProtoError> {
    match read_payload(reader, max_frame)? {
        None => Ok(None),
        Some(payload) => Ok(Some(serde_json::from_slice(&payload)?)),
    }
}

/// 把 `message` 序列化成一個訊框寫出，並 flush。
///
/// 立即 flush 是必要的：stdio 與 socket 的對端要看到回應才算完成一次往返。
pub fn write_message<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
    max_frame: usize,
) -> Result<(), ProtoError> {
    let payload = serde_json::to_vec(message)?;
    let bytes = frame::encode_with_limit(&payload, max_frame)?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// 讀到 `buf` 滿或 EOF，回傳實際讀到的位元組數。
fn read_into<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize, io::Error> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}
