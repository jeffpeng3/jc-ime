//! 0CJADIC v1／v2 pack 與 manifest 的錯誤型別。
//!
//! 每個驗證項目各自一個 variant，呼叫端能直接知道是哪一項失敗，
//! 不需要解析錯誤訊息。

use std::path::PathBuf;

/// pack／manifest 讀取與驗證的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// 檔案無法開啟或讀取。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 出錯的檔案路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },

    /// 檔案連檔頭加 trailer 都放不下。
    #[error("pack 太小：{size} 位元組，至少需要 {minimum}")]
    TooSmall {
        /// 實際檔案大小。
        size: usize,
        /// 檔頭加 trailer 的最小大小。
        minimum: usize,
    },

    /// 檔頭前 8 位元組不是 `0CJADIC\0`。
    #[error("magic 不符，不是 0CJADIC pack")]
    BadMagic,

    /// 格式版本不是本版支援的版本。
    #[error("不支援的格式版本 {actual}，本版支援 1 與 2")]
    UnsupportedVersion {
        /// 檔頭宣告的版本。
        actual: u16,
        /// 最新支援的版本。
        expected: u16,
    },

    /// 檔頭大小與固定值不符。
    #[error("header size 為 {actual}，預期 {expected}")]
    BadHeaderSize {
        /// 檔頭宣告的大小。
        actual: u16,
        /// 預期的固定大小。
        expected: u16,
    },

    /// reserved 欄位在 v1 必須是 0。
    #[error("reserved 欄位為 {0}，v1 必須是 0")]
    NonZeroReserved(u16),

    /// v2 檔頭尾端的 reserved 欄位必須是 0。
    #[error("v2 的 reserved 欄位為 {0}，必須是 0")]
    NonZeroReserved32(u32),

    /// flags 在 v1 必須是 0。
    #[error("不支援的 flags {0:#x}，v1 必須是 0")]
    UnsupportedFlags(u32),

    /// 必要的 count 欄位為 0。
    #[error("{field} 為 0")]
    EmptyCount {
        /// 欄位名稱。
        field: &'static str,
    },

    /// trailer 的 payload 雜湊與實際內容不符。
    #[error("payload SHA-256 不符：trailer {trailer}，實際 {computed}")]
    PayloadHashMismatch {
        /// trailer 記錄的雜湊。
        trailer: String,
        /// 重新計算的雜湊。
        computed: String,
    },

    /// 重新計算的整檔雜湊與開啟時記錄的不同。
    #[error("整檔 SHA-256 不符：開啟時 {expected}，實際 {actual}")]
    FileHashMismatch {
        /// 開啟時記錄的雜湊。
        expected: String,
        /// 重新計算的雜湊。
        actual: String,
    },

    /// section 起點落在檔頭內。
    #[error("{section} 起點 {start} 落在檔頭內")]
    SectionInsideHeader {
        /// section 名稱。
        section: &'static str,
        /// section 起點。
        start: u64,
    },

    /// section 長度溢位或超出 payload。
    #[error("{section} 結束於 {end}，超出 payload 結尾 {payload_end}")]
    SectionOutOfBounds {
        /// section 名稱。
        section: &'static str,
        /// section 結束位置。
        end: u64,
        /// payload 結束位置。
        payload_end: u64,
    },

    /// 兩個 section 的範圍重疊。
    #[error("section 重疊：{first} [{first_start},{first_end}) 與 {second} [{second_start},{second_end})")]
    SectionOverlap {
        /// 前一個 section 名稱。
        first: &'static str,
        /// 前一個 section 起點。
        first_start: u64,
        /// 前一個 section 結束位置。
        first_end: u64,
        /// 後一個 section 名稱。
        second: &'static str,
        /// 後一個 section 起點。
        second_start: u64,
        /// 後一個 section 結束位置。
        second_end: u64,
    },

    /// section 長度與 count 推算值不符。
    #[error("{section} 長度 {actual} 與預期 {expected} 不符")]
    SectionLengthMismatch {
        /// section 名稱。
        section: &'static str,
        /// 檔頭宣告的長度。
        actual: u64,
        /// 由 count 推算的長度。
        expected: u64,
    },

    /// 字串 table 筆數與檔頭不符。
    #[error("{table} 宣告 {actual} 筆，檔頭為 {expected}")]
    TableCountMismatch {
        /// table 名稱。
        table: &'static str,
        /// section 內宣告的筆數。
        actual: usize,
        /// 檔頭宣告的筆數。
        expected: usize,
    },

    /// 字串 table 的單筆內容不合法。
    #[error("{table} 第 {index} 筆錯誤：{reason}")]
    TableEntry {
        /// table 名稱。
        table: &'static str,
        /// 1-based 筆數。
        index: usize,
        /// 失敗原因。
        reason: String,
    },

    /// surface index 第一筆 offset 不是 0。
    #[error("surface index 第一筆 offset 為 {0}，預期 0")]
    FirstSurfaceOffset(u32),

    /// surface index 的 offset 未嚴格遞增。
    #[error("surface index 第 {index} 筆 offset {actual} 未遞增（前一筆 {previous}）")]
    SurfaceOffsetNotIncreasing {
        /// 1-based 筆數。
        index: usize,
        /// 本筆 offset。
        actual: u32,
        /// 前一筆 offset。
        previous: u32,
    },

    /// surface index 的 first_entry 未嚴格遞增。
    #[error("surface index 第 {index} 筆 first_entry {actual} 未遞增（前一筆 {previous}）")]
    FirstEntryNotIncreasing {
        /// 1-based 筆數。
        index: usize,
        /// 本筆 first_entry。
        actual: u32,
        /// 前一筆 first_entry。
        previous: u32,
    },

    /// first_entry 超出詞條數。
    #[error("surface index 第 {index} 筆 first_entry {value} 超出詞條數 {entry_count}")]
    FirstEntryOutOfRange {
        /// 1-based 筆數。
        index: usize,
        /// first_entry 值。
        value: u32,
        /// 詞條總數。
        entry_count: u32,
    },

    /// surface index 指向 surface blob 之外。
    #[error("surface index 第 {index} 筆 offset {offset} 超出 surface blob {blob_len}")]
    SurfaceOffsetOutOfRange {
        /// 1-based 筆數。
        index: usize,
        /// offset 值。
        offset: u32,
        /// surface blob 長度。
        blob_len: u32,
    },

    /// surface index 對應不到任何詞條。
    #[error("surface index 第 {index} 筆沒有詞條")]
    EmptySurfaceRange {
        /// 1-based 筆數。
        index: usize,
    },

    /// surface 長度超出 u16。
    #[error("surface index 第 {index} 筆的 surface 長度 {length} 超出 u16")]
    SurfaceLengthOverflow {
        /// 1-based 筆數。
        index: usize,
        /// surface 位元組長度。
        length: usize,
    },

    /// surface index 記錄的長度與詞條實際長度不符。
    #[error("surface {surface:?} 的長度 {length} 與 index 記錄 {indexed} 不符")]
    SurfaceLengthMismatch {
        /// surface 字串。
        surface: String,
        /// 詞條上的長度。
        length: usize,
        /// index 記錄的長度。
        indexed: u16,
    },

    /// surface index 的 first_entry 與詞條位置不符。
    #[error("surface index 第 {index} 筆 first_entry 為 {expected}，實際是 {actual}")]
    SurfaceFirstEntryMismatch {
        /// 1-based 筆數。
        index: usize,
        /// index 記錄的 first_entry。
        expected: u32,
        /// 詞條實際位置。
        actual: u32,
    },

    /// surface index 指到的字串與詞條 surface 不符。
    #[error("surface index 第 {index} 筆指向 {indexed:?}，詞條卻是 {entry:?}")]
    SurfaceIndexPointsElsewhere {
        /// 1-based 筆數。
        index: usize,
        /// index 指到的字串。
        indexed: String,
        /// 詞條的 surface。
        entry: String,
    },

    /// 詞條的 surface 不在 index 內。
    #[error("詞條 {index} 的 surface {surface:?} 不在 surface index 內")]
    UnindexedSurface {
        /// 詞條索引。
        index: u32,
        /// 詞條 surface。
        surface: String,
    },

    /// surface blob 尾端有未被 index 覆蓋的位元組。
    #[error("surface blob 尾端有未索引的位元組")]
    SurfaceBlobTrailingBytes,

    /// surface index 筆數與詞條實際對應到的 surface 數不符。
    #[error("surface index 有 {index_count} 筆，詞條只對應到 {matched} 筆")]
    SurfaceIndexCountMismatch {
        /// index 筆數。
        index_count: usize,
        /// 詞條實際對應到的 surface 數。
        matched: usize,
    },

    /// 詞條的 surface 超出 surface blob。
    #[error("第 {index} 筆詞條的 surface offset {offset}+{length} 超出 blob {blob_len}")]
    SurfaceOutOfBounds {
        /// 詞條索引。
        index: u32,
        /// blob 相對 offset。
        offset: u32,
        /// 位元組長度。
        length: u16,
        /// blob 長度。
        blob_len: u32,
    },

    /// 詞條的 reading 超出 reading blob。
    #[error("第 {index} 筆詞條的 reading offset {offset}+{length} 超出 blob {blob_len}")]
    ReadingOutOfBounds {
        /// 詞條索引。
        index: u32,
        /// blob 相對 offset。
        offset: u32,
        /// 位元組長度。
        length: u16,
        /// blob 長度。
        blob_len: u32,
    },

    /// 字串不是合法 UTF-8。
    #[error("第 {index} 筆詞條的 {field} 不是合法 UTF-8")]
    InvalidUtf8 {
        /// 詞條索引。
        index: u32,
        /// 欄位名稱。
        field: &'static str,
    },

    /// 詞條 surface 是空字串。
    #[error("第 {index} 筆詞條的 surface 是空字串")]
    EmptySurface {
        /// 詞條索引。
        index: u32,
    },

    /// 詞條 reading 是空字串。
    #[error("第 {index} 筆詞條的 reading 是空字串")]
    EmptyReading {
        /// 詞條索引。
        index: u32,
    },

    /// POS id 超出 POS table。
    #[error("第 {index} 筆詞條的 POS id {id} 超出 POS table {count}")]
    PosIdOutOfRange {
        /// 詞條索引。
        index: u32,
        /// POS id。
        id: u16,
        /// table 筆數。
        count: usize,
    },

    /// 活用型 id 超出 table。
    #[error("第 {index} 筆詞條的活用型 id {id} 超出 table {count}")]
    ConjTypeIdOutOfRange {
        /// 詞條索引。
        index: u32,
        /// 活用型 id。
        id: u8,
        /// table 筆數。
        count: usize,
    },

    /// 活用形 id 超出 table。
    #[error("第 {index} 筆詞條的活用形 id {id} 超出 table {count}")]
    ConjFormIdOutOfRange {
        /// 詞條索引。
        index: u32,
        /// 活用形 id。
        id: u8,
        /// table 筆數。
        count: usize,
    },

    /// 詞條的 left／right context id 超出矩陣。
    #[error("第 {index} 筆詞條的 {side} {id} 超出 context 數 {context_count}")]
    EntryContextOutOfRange {
        /// 詞條索引。
        index: u32,
        /// 欄位名稱。
        side: &'static str,
        /// context id。
        id: u16,
        /// context 數。
        context_count: u16,
    },

    /// 詞成本低於合法下限。
    #[error("第 {index} 筆詞條的詞成本 {cost} 低於合法下限 {minimum}")]
    CostOutOfRange {
        /// 詞條索引。
        index: u32,
        /// 詞成本。
        cost: i16,
        /// 合法下限。
        minimum: i16,
    },

    /// flags 帶有 v1 未定義的位元。
    #[error("第 {index} 筆詞條帶有未定義的 flags {flags:#x}")]
    UndefinedFlags {
        /// 詞條索引。
        index: u32,
        /// 原始 flags。
        flags: u16,
    },

    /// split 值不是 0、1、2。
    #[error("第 {index} 筆詞條的 split 值 {split} 不合法")]
    InvalidSplit {
        /// 詞條索引。
        index: u32,
        /// split 原始值。
        split: u16,
    },

    /// 詞條 surface 未依 code point 排序。
    #[error("第 {index} 筆詞條 surface {surface:?} 未依 code point 排序（前一筆 {previous:?}）")]
    SurfaceOrder {
        /// 詞條索引。
        index: u32,
        /// 本筆 surface。
        surface: String,
        /// 前一筆 surface。
        previous: String,
    },

    /// 讀音含片假名。
    #[error(
        "第 {index} 筆詞條 {surface:?} 的讀音 {reading:?} 含有片假名 {ch:?}；ja 讀音必須是平假名"
    )]
    KatakanaReading {
        /// 詞條索引。
        index: u32,
        /// 詞條 surface。
        surface: String,
        /// 讀音。
        reading: String,
        /// 違規字元。
        ch: char,
    },

    /// 讀音含非平假名字元。
    #[error("第 {index} 筆詞條 {surface:?} 的讀音 {reading:?} 含有非平假名字元 {ch:?}；ja 讀音必須是平假名")]
    NonKanaReading {
        /// 詞條索引。
        index: u32,
        /// 詞條 surface。
        surface: String,
        /// 讀音。
        reading: String,
        /// 違規字元。
        ch: char,
    },

    /// manifest.json 不是合法 JSON。
    #[error("解析 {path} 失敗：{source}")]
    ManifestJson {
        /// manifest 路徑。
        path: PathBuf,
        /// JSON 解析錯誤。
        #[source]
        source: serde_json::Error,
    },

    /// manifest schema 不符。
    #[error("manifest schema 為 {actual:?}，預期 {expected:?}")]
    ManifestSchema {
        /// manifest 宣告的值。
        actual: String,
        /// 預期的值。
        expected: &'static str,
    },

    /// manifest 的 pack_format.name 不符。
    #[error("manifest 的 pack_format.name 為 {actual:?}，預期 {expected:?}")]
    ManifestFormatName {
        /// manifest 宣告的值。
        actual: String,
        /// 預期的值。
        expected: &'static str,
    },

    /// manifest 的 pack_format.version 與 pack 檔頭不符。
    #[error("manifest 的 pack_format.version 為 {actual}，pack 檔頭是 {expected}")]
    ManifestFormatVersion {
        /// manifest 宣告的版本。
        actual: i64,
        /// pack 檔頭的版本。
        expected: i64,
    },

    /// manifest 的 pack_format.entry_size 與版本要求不符。
    #[error("manifest 的 pack_format.entry_size 為 {actual:?}，version 2 必須是 {expected}")]
    ManifestEntrySize {
        /// manifest 宣告的 entry_size，缺少時是 `None`。
        actual: Option<i64>,
        /// 該版本要求的 entry 大小。
        expected: u16,
    },

    /// manifest 的 reading_script 不受支援。
    #[error("manifest 的 reading_script 為 {actual:?}，只支援 hiragana 或 katakana")]
    ManifestReadingScript {
        /// manifest 宣告的值。
        actual: String,
    },

    /// manifest 宣告的數量與 pack 不符。
    #[error("manifest {field}={manifest} 與 pack {pack_field}={pack} 不符")]
    ManifestCount {
        /// manifest 欄位名稱。
        field: &'static str,
        /// manifest 數值。
        manifest: i64,
        /// pack 欄位名稱。
        pack_field: &'static str,
        /// pack 數值。
        pack: i64,
    },

    /// manifest 宣告的檔案大小與實際檔案不符。
    #[error("manifest outputs.pack.bytes={manifest} 與實際檔案 {actual} 位元組不符")]
    ManifestBytes {
        /// manifest 數值。
        manifest: i64,
        /// 實際大小。
        actual: u64,
    },

    /// manifest 記錄的整檔雜湊與實際檔案不符。
    #[error("整檔 SHA-256 不符（manifest {manifest}，實際 {actual}）")]
    ManifestFileHash {
        /// manifest 記錄的雜湊。
        manifest: String,
        /// 實際計算的雜湊。
        actual: String,
    },

    /// manifest 記錄的 payload 雜湊與 pack trailer 不符。
    #[error("payload SHA-256 不符（manifest {manifest}，trailer {trailer}）")]
    ManifestPayloadHash {
        /// manifest 記錄的雜湊。
        manifest: String,
        /// pack trailer 的雜湊。
        trailer: String,
    },

    /// 尚未記錄 manifest 檔案雜湊就呼叫 `verify_file`。
    #[error("manifest 尚未記錄檔案雜湊，請先呼叫 Manifest::load")]
    ManifestHashMissing,
}
