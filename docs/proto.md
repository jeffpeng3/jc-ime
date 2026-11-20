# jc-proto 協定規格（v3，凍結）

本文件是 wrapper 與 `jc-engine` 之間的介面契約。**v3 已凍結**：新增欄位必須是
可忽略的（`Option` 或 `default`），不得改變既有欄位語意；不兼容的變更要升到
版本 4，並同步更新本文件與 `PROTOCOL_VERSION`。

## 1. 傳輸

| 平台 | 傳輸 | 狀態 |
|---|---|---|
| 全部 | stdio | 已實作 |
| Linux／macOS | Unix domain socket（`--socket <path>`） | 已實作 |
| Windows | named pipe | **未實作** |

一個連線一次只服務一個請求，序列處理。斷線後可重連；重連等於新的 session。

## 2. 訊框

```text
[ u32 little-endian length ][ JSON payload ]
```

| 規則 | 值 |
|---|---|
| 長度前綴 | 4 bytes，小端，只算 payload |
| payload 編碼 | UTF-8 JSON（serde） |
| 長度上限 | 預設 1 MiB，可用 `--max-frame` 調整 |
| 0 長度 | 拒絕（`BadFrame`） |
| 超長 | 拒絕（`FrameTooLarge`），讀完前不配置緩衝 |

## 3. 版本

`PROTOCOL_VERSION = 3`。

握手規則：

1. wrapper 先送 `Hello { protocol: 3, client: "<name/version>" }`
2. 引擎比對版本，不同就回 `Error { code: "version_mismatch" }` 並關閉連線
3. 相同則回 `Ready { protocol: 3, engine: "<name/version>" }`

沒有握手就送其他請求 → `Error { code: "internal", message: "尚未完成握手" }`。

## 4. 訊息

### Request

```json
{ "op": "hello", "protocol": 3, "client": "jc-tsf/0.1.0" }
{ "op": "key", "key": { ... } }
{ "op": "snapshot" }
{ "op": "commit" }
{ "op": "shutdown" }
```

`key` 的形狀見 `jc_core::Key`／`KeyCode`。未定義的 `op` 由 `unknown` 接住並回
`UnknownOp`，不會斷線。

### Response

```json
{ "op": "ready", "protocol": 3, "engine": "jc-engine/0.1.0" }
{ "op": "state", "preedit": "ㄨㄛˇ", "cursor": 3, "candidates": ["我", "窩"],
  "details": [
    { "reading": "ㄨㄛˇ", "source": "lexicon", "cost": 12 },
    { "reading": "ㄨㄛ", "source": "personal", "cost": -3, "learned": true }
  ],
  "segments": [
    { "start": 0, "end": 3, "mode": "zhuyin", "script": "bopomofo" }
  ],
  "highlighted": 1,
  "cursor_rune": 3, "suggestion": null, "label": null, "navigating": false }
{ "op": "committed", "text": "我" }
{ "op": "error", "code": "bad_frame", "message": "..." }
```

`State` 的欄位：

| 欄位 | 型別 | 語意 |
|---|---|---|
| `preedit` | string | 目前組字字串 |
| `cursor` | usize | 游標位置（Unicode scalar 數）。**只為相容而留** |
| `candidates` | string[] | 候選字面，順序即顯示順序 |
| `details` | CandidateDetail[] | 與 `candidates` **等長、同序**；空時省略 |
| `segments` | SegmentInfo[] | 組字片段，依序涵蓋整段 `preedit`；空時省略 |
| `highlighted` | usize | 目前高亮的候選索引；0 時省略 |
| `cursor_rune` | usize | 游標的權威位置（Unicode scalar 數）；0 時省略 |
| `suggestion` | string? | Tab 會接受的補全尾綴 |
| `label` | string? | 目前高亮候選的顯示原因 |
| `navigating` | bool | 是否在明確的候選選取模式 |

**游標**：`cursor_rune` 是權威值，單位是 Unicode scalar value（rune）。`cursor`
沿用版本 1 的定義，值是 `preedit` 的字元數，只為舊 wrapper 相容而保留。需要
UTF-16 位移的外殼請自己從 `cursor_rune` 換算，不要假設 `cursor` 會反映真實
游標位置。

**候選對齊**：`details` 與 `candidates` 由同一個候選向量產生，長度與順序必然
一致；`details[i]` 描述的就是 `candidates[i]`。這是 wrapper 唯一的假設，wrapper
不應自行重排候選。

### CandidateDetail

| 欄位 | 型別 | 語意 |
|---|---|---|
| `reading` | string | 讀音。注音為ㄅㄆㄇ，日文為平假名 |
| `source` | string | `lexicon`、`personal`、`predicted` 或 `literal` |
| `cost` | i32 | 路徑成本，**越小越優先** |
| `learned` | bool | `source == personal` 時為 `true`；`false` 時省略 |

`cost` 是 MeCab 語意的整數路徑成本，可以是負數，**不是機率**，也不適合直接
相加比較不同長度的候選。只供除錯與排序說明；顯示順序已由核心決定。

`source` 的線格式：

| 值 | 意義 |
|---|---|
| `lexicon` | 系統詞典 |
| `personal` | 使用者詞庫或學習結果 |
| `predicted` | 由前文預測 |
| `literal` | 原樣輸出的字面（英文、符號、數字） |

### SegmentInfo

| 欄位 | 型別 | 語意 |
|---|---|---|
| `start` | usize | 片段起點（含），**rune** 位移 |
| `end` | usize | 片段終點（不含），**rune** 位移 |
| `mode` | string | `zhuyin`、`japanese`、`english` 或 `mixed` |
| `script` | string | `bopomofo`、`kana`、`han`、`latin`、`digit`、`symbol`、`mixed` 或 `other` |

核心內部的片段邊界是 UTF-8 byte 位移，線上一律換成 rune，wrapper 拿到後可
直接對應 UTF-16 轉換後的索引。片段依序相接、不重疊，第一個 `start` 為 0，最後
一個 `end` 等於 `preedit` 的 rune 長度。

`mode` 沿用核心 `Mode` 的語意：`zhuyin` 是注音輸入、`japanese` 是日文羅馬字、
`english` 是英文直出、`mixed` 由分段器逐段判定。純 Latin 片段在核心屬於
`Mixed` 模式，因此會看到 `"mode": "mixed"` 搭配 `"script": "latin"`。

### ErrorCode

| 值 | 意義 |
|---|---|
| `version_mismatch` | 握手版本不同 |
| `bad_frame` | JSON 解碼失敗或缺少欄位 |
| `frame_too_large` | 超過長度上限 |
| `unknown_op` | 不認得的 `op` |
| `internal` | 狀態不允許，或引擎內部錯誤 |

## 5. 生命週期

```text
連線建立
  │
  ├─ Hello ──▶ Ready
  │
  ├─ Key ─────▶ State（每次按鍵都回完整快照）
  ├─ Snapshot ▶ State
  ├─ Commit ──▶ Committed
  │
  └─ Shutdown ▶ （無回應）行程結束；或直接關閉連線
```

規則：

- 每個 `Key` 都回一個 `State`，wrapper 不需要自己維護組字狀態。
- `Commit` 後組字區清空，回 `Committed { text }`。
- 取消（Esc）由核心處理，回應仍是 `State`，`preedit` 為空。
- 引擎不會主動推送訊息；wrapper 以請求／回應為單位。

## 6. 效能預算

| 項目 | 值 |
|---|---|
| 一般按鍵往返 | < 5 ms（不含神經模型） |
| 神經模型觸發時 | 目標 p50 < 60 ms、硬上限 180 ms |
| 逾時行為 | 回退統計結果，回應不延遲 |

## 7. 已知限制（v3）

1. Windows named pipe 未實作。
2. 沒有 client crate：wrapper 需自行實作 framing 與重連。
3. 沒有滑鼠事件（點選候選需由 wrapper 轉成 `Key` 序列）。
4. 沒有主動通知（學習完成、模型狀態等）。
5. 組字游標固定在前端（核心目前不支援中途插入）。

### 尚未提供

以下資訊目前不在協定內，wrapper 不應從現有欄位推測：

1. **詞性**：`CandidateDetail` 沒有 part-of-speech 欄位。
2. **學習證據權重**：只有 `learned` 布林值，沒有次數、時間、來源證據或權重。
3. **模型候選來源**：無法分辨候選是統計解碼或神經模型產生，`source` 只描述
   詞典／個人／預測／字面四類。

## 8. 變更流程

1. 先改本文件，標明是「新增」還是「不兼容」；改線格式前先更新
   `crates/jc-proto/tests/fixtures/` 的手寫 fixture。
2. 新增：加 `#[serde(default)]` 欄位，保持舊 wrapper 可解析。
3. 不兼容：`PROTOCOL_VERSION` 加一，並在文件的變更記錄寫下差異。
4. 同步更新 `crates/jc-proto/src/lib.rs` 的 doc comment 與測試 fixture。

## 9. 變更記錄

| 版本 | 日期 | 變更 |
|---|---|---|
| 1 | 2026-11-19 | 初版：Hello／Key／Snapshot／Commit、長度前綴 framing |
| 2 | 2026-11-20 | `State` 新增 `cursor_rune`、`suggestion`、`label`、`navigating`；新增 `Shutdown` 與 `Unknown`；錯誤碼改為列舉 |
| 3 | 2026-11-20 | `State` 新增 `details`（與 `candidates` 等長同序）、`segments`（rune 位移）、`highlighted`；`CandidateSource` 提供 snake_case 線格式；文件明訂 `cursor_rune` 為權威值 |
