# 核心、引擎與 wrapper 的邊界

## 形狀

```text
wrapper（各平台獨立）              引擎行程
  TSF DLL ─┐                       jc-engine ── jc-proto
  候選窗   ├── 長度前綴 JSON ──────┘    │
  設定程式 ┘                            └── jc-core ── jc-data
                                             （注音／日文／Mixed／session）

jc-cli ── jc-core、jc-data、jc-eval、jc-testdata  （開發工具）
jc-jadic ── 產出 pack v2                     （資料匯入）
jc-model ── candle 載入與推論（reranker、注音選字、Jinen）
jc-platform ── wrapper 契約                  （尚未接進 jc-engine）
```

相依方向是單向的：`jc-core` 只依賴 `jc-data`；`jc-engine` 只依賴
`jc-core` 與 `jc-proto`；`jc-eval` 不依賴 `jc-core`，轉換器由呼叫端注入。

## 責任

| 元件 | 負責 | 不負責 |
|---|---|---|
| `jc-core` | 注音音節與鍵盤、lattice／Viterbi、注音解碼、日文輸入與解碼、Mixed 路由、session、輸出 | 檔案格式、IPC、任何 OS API |
| `jc-data` | 日文 `0CJADIC` pack v1／v2、注音 `lexicon.bin`／`lm.bin` 的 mmap 讀取、manifest 驗證、連接矩陣與 bigram 查詢 | 匯入與寫檔（屬 `jc-jadic`）、轉換決策 |
| `jc-jadic` | Mozc `dictionary_oss` 解析、去重排序、pack v2 與 manifest 寫出 | 執行期轉換 |
| `jc-model` | 模型 manifest、tokenizer、candle reranker／注音選字／Jinen 推論 | 解碼排序、個人化 |
| `jc-proto` | 訊框、請求／回應列舉、版本握手、錯誤碼 | 組字狀態 |
| `jc-platform` | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 的語意契約 | 任何視窗或 OS handle |
| `jc-engine` | 一條連線的請求迴圈、握手、把按鍵交給 `Session` | 轉換邏輯、詞典載入 |
| `jc-eval` | 題庫 loader、top-1／top-5／邊界／延遲指標、`Report`、gate | 引擎與資料載入 |
| `jc-testdata` | 測試資料路徑解析與缺資料跳過 | 正式執行期資料 |
| `jc-cli` | 評測、轉換、bench 的開發入口 | 產品執行期 |

## 為什麼分離

1. 核心可在 Linux 上開發與評測，不必每次回到 Windows 驗證。
2. wrapper 只做轉發，崩潰不會把核心一起帶走，宿主程式（explorer）也不承擔
   核心風險。
3. 之後的 Fcitx5／macOS 只需要新的 wrapper，核心與題庫不動。

## 協定

- 傳輸：4-byte 小端長度前綴 + JSON（serde）；預設訊框上限 1 MiB。
- 版本：`jc_proto::PROTOCOL_VERSION`（目前 `2`），握手不一致即回
  `VersionMismatch` 並收線。
- 請求：`Hello`、`Key`、`Snapshot`、`Commit`、`Shutdown`；狀態回應是完整
  快照，wrapper 不需要自己保存組字狀態。
- 狀態回應的 `State` 另有 `cursor_rune`、`suggestion`、`label` 與
  `navigating`；空值時省略。
- 錯誤：`VersionMismatch`、`BadFrame`、`FrameTooLarge`、`UnknownOp`、
  `Internal`。
- 傳輸方式：`jc-engine` 目前提供 stdio 與 Unix domain socket，一次服務一條
  連線；Windows named pipe 尚未實作。
- wrapper 作者規格見 [`docs/wrapper.md`](wrapper.md)，內容包含生命週期、
  pass-through、UTF-16 轉換與已知限制。

## 核心的請求流程

1. wrapper 把平台按鍵轉成 `jc_proto::Key`（語意鍵，不含 OS 鍵碼）。
2. `Engine::handle` 檢查握手後呼叫 `Session::press`。
3. `Session` 依模式走注音、日文或 Mixed 解碼，回傳含組字文字、游標與候選的
   `Snapshot`。
4. `Enter` 或 `Commit` 走 `Session::commit`，回 `Response::Committed`；核心
   保證送出的文字非空，且之後組字狀態已清空。

## wrapper 契約

完整內容見 [`docs/port/jc-platform.md`](port/jc-platform.md)。摘要：

- `InputHost`：`update_preedit(text, cursor_utf16)`、`clear_preedit()`、
  `show_candidates(items, highlight)`、`commit(text)`、`show_status(text)`，
  全部回傳 `Result<(), HostError>`，wrapper 不得 panic。
- `HostEvent`：`FocusLost`、`FocusGained`、`CandidateClicked`、`Scroll`、
  `Timer`；候選點選的索引以整個候選清單為準。
- `Clock`／`Scheduler`：核心只從注入的時間來源取得時間；測試用
  `ManualClock` 與 `FakeScheduler`。
- 游標以 UTF-16 code unit 計算；其餘字串一律 UTF-8 `&str`。
- wrapper 不得在 `InputHost` 方法內同步把事件餵回核心；事件排到下一個回合。
- 核心保證 `commit` 後接 `clear_preedit`，且候選 `highlight` 一定落在
  `items` 範圍內。

`HostCall` 與 `HostEvent` 的 JSON 標籤是契約的一部分；`HeadlessHost` 與
`RecordingHost` 讓測試直接比對呼叫順序。

## 目前的邊界狀況

- `jc-platform` 的契約與測試已完成，但 `jc-engine` 還沒有依賴它，也還沒有
  任何實際 wrapper。
- `jc-model` 已接進 `jc-core` 的注音選字模型；`jc-engine` 啟動時若資料根
  目錄有 `models/` 就安裝，缺檔只警告並維持統計排序。日文 reranker 預設
  關閉，標點尚未進入送出流程。
- `jc-engine` 正式啟動要求 `--data` 或 `JC_DATA`，並載入注音詞典與日文
  pack；`--allow-empty-data` 才使用空 session，供協定測試與除錯。
- 注音 `lexicon.bin`／`lm.bin` 的讀取接在 `jc-engine` 的 `loader` 與
  `jc-cli`；`eval zh-golden` 仍讀凍結 TSV。
