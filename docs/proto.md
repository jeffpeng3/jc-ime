# jc-proto 協定規格（v3，凍結）

本文件是 wrapper 與 `jc-engine` 之間的介面契約。**v3 已凍結**：新增欄位必須是
可忽略的（`Option` 或 `default`），不得改變既有欄位語意；不相容的變更要升到
版本 4，並同步更新本文件與 `PROTOCOL_VERSION`。

## 1. 傳輸

| 平台 | 傳輸 | 狀態 | 程式 |
|---|---|---|---|
| 全部 | stdio | 已實作 | `crates/jc-engine/src/main.rs` |
| Linux／macOS | Unix domain socket（`--socket <path>`） | 已實作 | `crates/jc-engine/src/main.rs` |
| Windows | named pipe（`--socket` 或 `--supervise`） | 已實作 | `crates/jc-engine/src/pipe_listener.rs` |

Windows 的 `--socket` 只取最後一段，掛到 `\\.\pipe\`；`--supervise` 沒給
`--socket` 時由 `jc_windows::resolve_pipe_name()` 依 SID 尾段決定名稱。

每條連線由一個 worker thread 服務，同連線內的請求序列處理；不同連線可以
並行。斷線後可重連，新連線會拿到新的 session id；需要保留組字狀態的外殼要
自行重播或接受空白狀態。

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
3. 相同則回 `Ready { protocol: 3, engine: "<name/version>", session: <id> }`

`Ready.session` 是這條連線的 session id，第一條連線從 1 開始；舊引擎沒有這個
欄位時，反序列化預設為 1。外殼後續送 `close` 時要帶自己的 id，不能共用別條
連線的 session。

`Ready` 只有 `protocol`、`engine` 與 `session`，沒有 `max_frame`、`features`
或能力清單。訊框上限仍由雙方的 `--max-frame` 決定；部分能力旗標要另外送
`health`，從 `HealthInfo.capabilities` 讀取，見 §4。

沒有握手就送其他請求 → `Error { code: "internal", message: "尚未完成握手" }`。
`Shutdown` 不檢查握手。

## 4. 訊息

### Request

```json
{ "op": "hello", "protocol": 3, "client": "jc-tsf/<version>" }
{ "op": "key", "key": { ... } }
{ "op": "toggle" }
{ "op": "snapshot" }
{ "op": "commit" }
{ "op": "ping" }
{ "op": "health" }
{ "op": "close", "session": 1 }
{ "op": "reset_personal" }
{ "op": "output_simplified", "on": true }
{ "op": "shutdown" }
```

`key` 的形狀見 `jc_core::Key`／`KeyCode`。`toggle` 是單按左／右 Shift：引擎在目前
語言（Mixed 或 Ja）與純英文 En 之間切換，並記住切換前的模式；再收到一次就回到
它。未定義的 `op` 由 `unknown` 接住並回 `UnknownOp`，不會斷線。

以下是 0c adapter 的擴充 op，沿用協定版本 3；每個都是獨立 variant，不改變
既有請求的 JSON 形狀。

| `op` | 狀態 | 回應 | 程式 |
|---|---|---|---|
| `ping` | 已實作 | `Acknowledged` | `crates/jc-engine/src/lib.rs` |
| `health` | 已實作，回連線數、session 數、pid 與能力旗標 | `Health` | 同上 |
| `close` | 已實作，只允許關自己的 session | `Acknowledged`／`Error` | 同上 |
| `reset_personal` | 已實作，清空共用學習紀錄並存檔 | `Acknowledged` | 同上 |
| `output_simplified` | 已實作，設目前 session 的輸出字形；沒有 tw2s 轉換器時不會生效 | `State` | 同上 |
| `drain` | 已實作，flush 後中斷所有連線 | `Health` | 同上 |
| `apply_settings` | **部分**：介面收下設定並回 `Health`，引擎沒有套用 | `Health` | 同上 |
| `vocab` | **未實作**，回 `unsupported` | `Error` | `crates/jc-proto/src/lib.rs` 的 `Request::Vocab` |
| `context`／`mode`／`caps_lock` | **未實作**，回 `unsupported` | `Error` | 同上的 `Request` 變體 |

`unsupported` 與 `unknown_op` 不同：前者代表引擎認得這個 op，但這個版本還沒
實作；後者是協定完全不認得。

### Response

```json
{ "op": "ready", "protocol": 3, "engine": "jc-engine/<version>", "session": 1 }
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
{ "op": "health", "info": { "connections": 1, "sessions": 1, "pid": 1234,
  "capabilities": { "personal_persist": true, "output_simplified_live": true } } }
{ "op": "acknowledged", "session": 1 }
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
| `pos` | string? | 詞性名稱；沒有詞性時省略 |

`cost` 是 MeCab 語意的整數路徑成本，可以是負數，**不是機率**，也不適合直接
相加比較不同長度的候選。只供除錯與排序說明；顯示順序已由核心決定。

`pos` 的來源是日文詞典 pack 的 POS 表，名稱例如 `名詞,普通名詞,一般,*`。
核心的候選只保存 POS id，名稱在映射線格式時查共用的名稱表，不會為每個候選
複製字串。注音詞典沒有詞性，**注音候選一律不帶 `pos`**；wrapper 不得用空的
`pos` 推測詞性。日文候選的 POS id 取第一個文節的第一個詞段（主要詞）。

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
| `unsupported` | 引擎認得這個 op，但目前版本還沒實作 |
| `internal` | 狀態不允許，或引擎內部錯誤 |

## 5. 生命週期

```text
連線建立
  │
  ├─ Hello ──▶ Ready
  │
  ├─ Key ─────▶ State（一般按鍵）
  │             └─ Enter 且提交文字非空 ──▶ Committed
  ├─ Toggle ──▶ State（在目前語言與 En 間切換，回完整快照）
  ├─ Snapshot ▶ State
  ├─ Commit ──▶ Committed（文字可能為空字串）
  │
  └─ Shutdown ▶ State（回完後行程結束）；或直接關閉連線
```

| 請求 | 回應 |
|---|---|
| `hello` | `ready`；版本不符時 `error` 後關線 |
| `key` | `state`；Enter 且有可提交的文字時 `committed` |
| `toggle` | `state` |
| `snapshot` | `state` |
| `commit` | `committed`，`text` 可為空字串 |
| `ping`／`close`／`reset_personal` | `acknowledged` |
| `health`／`apply_settings`／`drain` | `health` |
| `output_simplified` | `state` |
| `shutdown` | `state`，之後行程結束 |
| 未實作的擴充 op | `error`（`unsupported`） |
| 不認得的 `op` | `error`（`unknown_op`） |

規則：

- 一般 `Key` 與 `Toggle` 都回完整 `State`，wrapper 不需要自己維護組字狀態。
- Enter 且引擎提交了非空文字時回 `Committed`，不是 `State`；wrapper 要同時
  處理這兩種回應。
- `Commit` 後組字區清空，回 `Committed { text }`。
- 取消（Esc）由核心處理，回應仍是 `State`。候選窗開著時只關窗，暫存注音還在
  時只清暫存，最後才清整段組字；wrapper 要連送直到 `preedit` 變空。
- 引擎不會主動推送訊息；wrapper 以請求／回應為單位。`Shutdown` 目前會先回
  一個 `State` 再結束行程。

## 6. 效能預算

| 項目 | 值 |
|---|---|
| 一般按鍵往返 | < 5 ms（不含神經模型） |
| 神經模型觸發時 | 目標 p50 < 60 ms、硬上限 180 ms |
| 逾時行為 | 回退統計結果，回應不延遲 |

## 7. 已知限制（v3）

1. `Ready` 沒有 `max_frame` 與 `features`。訊框上限只能靠雙方 command line
   一致；能力旗標只能另外送 `health` 讀 `capabilities`。
2. 沒有 `undo`／`forget` 請求。`Session::undo_learning` 與
   `Session::forget_word` 已在核心實作，但協定沒有入口。
3. 沒有滑鼠事件（點選候選需由 wrapper 轉成 `Key` 序列或自行實作選取）。
4. 沒有主動通知（學習完成、模型狀態等）。
5. 沒有「候選窗是否開啟」旗標，只能用 `candidates` 是否為空推測。
6. `apply_settings` 只收下設定，沒有執行期套用；`vocab`、`context`、`mode`、
   `caps_lock` 回 `unsupported`。
7. 一條連線內的請求序列處理；多條連線已可並行，但組字狀態按 session 隔離，
   跨應用程式共用時仍要由 wrapper 決定焦點切換的行為。
8. 沒有 client crate 給 Linux／macOS：`jc-windows` 可用，`jc-linux`／`jc-mac`
   仍是佔位。

### 尚未提供

以下資訊目前不在協定內，wrapper 不應從現有欄位推測：

1. **學習證據權重**：只有 `learned` 布林值，沒有次數、時間、來源證據或權重。
2. **模型候選來源**：無法分辨候選是統計解碼或神經模型產生，`source` 只描述
   詞典／個人／預測／字面四類。
3. **注音詞性**：注音詞典沒有詞性，`pos` 一律省略；這個欄位只描述日文候選。

## 8. 變更流程

1. 先改本文件，標明是「新增」還是「不相容」；改線格式前先更新
   `crates/jc-proto/tests/fixtures/` 的手寫 fixture。
2. 新增：加 `#[serde(default)]` 欄位，保持舊 wrapper 可解析。
3. 不相容：`PROTOCOL_VERSION` 加一，並在文件的變更記錄寫下差異。
4. 同步更新 `crates/jc-proto/src/lib.rs` 的 doc comment 與測試 fixture。

## 9. 變更記錄

| 版本 | 日期 | 變更 |
|---|---|---|
| 1 | 2026-11-19 | 初版：Hello／Key／Snapshot／Commit、長度前綴 framing |
| 2 | 2026-11-20 | `State` 新增 `cursor_rune`、`suggestion`、`label`、`navigating`；新增 `Shutdown` 與 `Unknown`；錯誤碼改為列舉 |
| 3 | 2026-11-20 | `State` 新增 `details`（與 `candidates` 等長同序）、`segments`（rune 位移）、`highlighted`；`CandidateSource` 提供 snake_case 線格式；文件明訂 `cursor_rune` 為權威值。同日補充：`CandidateDetail` 新增選填 `pos`，日文候選帶 pack 的詞性名稱，注音一律省略 |
| 3 擴充 | 2026-11-25 | 新增 `ping`、`health`、`close`、`reset_personal`、`apply_settings`、`drain`、`vocab`、`context`、`mode`、`caps_lock`、`output_simplified`；`Ready` 新增 `session`，`ErrorCode` 新增 `unsupported`。既有欄位與 op 不變 |
