//! 訊框編解碼：4-byte 小端長度前綴加上內容。
//!
//! 這一層只管位元組，不碰 JSON。長度為 0 或超過上限的訊框一律拒絕。

/// 預設單一訊框上限：1 MiB。
pub const DEFAULT_MAX_FRAME: usize = 1 << 20;

/// 長度前綴的位元組數。
pub const HEADER_LEN: usize = 4;

/// 訊框格式錯誤。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// 長度前綴是 0；空訊框沒有意義。
    #[error("訊框長度為 0")]
    Empty,
    /// 長度超過呼叫端給的上限。
    #[error("訊框 {len} 位元組超過上限 {max}")]
    TooLarge {
        /// 長度前綴宣稱的內容長度。
        len: usize,
        /// 這個解碼器允許的上限。
        max: usize,
    },
}

/// 解碼結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded<'a> {
    /// 收到完整訊框；`consumed` 是含前綴的總位元組數。
    Complete {
        /// 訊框內容。
        payload: &'a [u8],
        /// 這個訊框佔用的位元組數。
        consumed: usize,
    },
    /// 資料還沒到齊，需要再讀。
    Incomplete,
}

/// 用預設上限包一個訊框。
pub fn encode(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    encode_with_limit(payload, DEFAULT_MAX_FRAME)
}

/// 用指定上限包一個訊框。空內容或超長都會失敗。
pub fn encode_with_limit(payload: &[u8], max: usize) -> Result<Vec<u8>, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::Empty);
    }
    if payload.len() > max {
        return Err(FrameError::TooLarge {
            len: payload.len(),
            max,
        });
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// 用預設上限解一個緩衝區裡的訊框。
pub fn decode(buf: &[u8]) -> Result<Decoded<'_>, FrameError> {
    decode_with_limit(buf, DEFAULT_MAX_FRAME)
}

/// 用指定上限解一個緩衝區裡的訊框。
///
/// 只要前 4 bytes 到齊就先檢查長度，超長或 0 長度不必等內容讀完就能拒絕。
pub fn decode_with_limit(buf: &[u8], max: usize) -> Result<Decoded<'_>, FrameError> {
    if buf.len() < HEADER_LEN {
        return Ok(Decoded::Incomplete);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    if buf.len() < HEADER_LEN + len {
        return Ok(Decoded::Incomplete);
    }
    Ok(Decoded::Complete {
        payload: &buf[HEADER_LEN..HEADER_LEN + len],
        consumed: HEADER_LEN + len,
    })
}
