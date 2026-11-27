# 核心、引擎與 wrapper 的邊界

## 形狀

```text
wrapper（各平台獨立）                  引擎行程
  TSF DLL ─┐                          jc-engine ── jc-proto ── jc-core
  候選窗   ├─ client crate ── proto ──┘    │
  設定程式 ┘   jc-windows                  ├── jc-core ── jc-data
               jc-linux（佔位）            │      │
               jc-mac（佔位）              │      └── jc-model
                                           │            │
                                           │            └── candle（Jinen、注音 Q8_0）
                                           │                ort（對照後端，執行期偵測）
                                           ├── jc-settings
                                           └── jc-windows（pipe 名稱規則）

jc-tsf ── jc-proto、jc-tsf-register
jc-tsf-smoke ── jc-proto、jc-windows
jc-settings-ui ── jc-data、jc-settings
jc-cli ── jc-core、jc-data、jc-model、jc-eval、jc-engine、jc-testdata（開發工具）
jc-jadic ── Mozc 匯入與 pack v2 寫出          （資料匯入）
jc-englishpack ── jc-core                      （英文詞表建置）
jc-model ── candle 載入與推論（reranker、注音選字、Jinen）
jc-settings ── jc-core 的模糊音與輸入習慣規則 （設定資料模型）
jc-platform ── wrapper 契約                    （引擎尚未依賴它）
```

相依方向是單向的：

- `jc-proto` → `jc-core`；線格式直接重用核心的 `Key`、`Mode` 與 `Snapshot`。
- `jc-windows`／`jc-linux`／`jc-mac` → `jc-proto`；client 不依賴核心。
- `jc-engine` → `jc-core`、`jc-data`、`jc-model`、`jc-proto`、`jc-settings`、`jc-windows`。
- `jc-core` → `jc-data`、`jc-model`；神經 runtime 的 worker 在 `jc-core`。
- `jc-eval` 不依賴 `jc-core`，轉換器由呼叫端注入。
- `jc-settings` → `jc-core`，只為了共用模糊音與輸入習慣規則表。
- `jc-tsf` → `jc-proto`、`jc-tsf-register`；`jc-tsf-smoke` → `jc-proto`、`jc-windows`。
- `jc-settings-ui` → `jc-data`、`jc-settings`，不經過 client crate。

workspace 目前 19 個成員：`jc-core`、`jc-testdata`、`jc-data`、`jc-model`、
`jc-proto`、`jc-platform`、`jc-eval`、`jc-engine`、`jc-windows`、`jc-linux`、
`jc-mac`、`jc-cli`、`jc-jadic`、`jc-englishpack`、`jc-settings`、`jc-tsf`、
`jc-tsf-register`、`jc-settings-ui`、`jc-tsf-smoke`。

## 責任

| 元件 | 負責 | 不負責 |
|---|---|---|
| `jc-core` | 注音音節與鍵盤、lattice／Viterbi、注音解碼、日文輸入與解碼、Mixed 路由、session、神經 runtime worker、輸出 | 檔案格式、IPC、任何 OS API |
| `jc-data` | 日文 `0CJADIC` pack v1／v2、注音 `lexicon.bin`／`lm.bin` 的 mmap 讀取、manifest 驗證、連接矩陣與 bigram 查詢 | 匯入與寫檔（屬 `jc-jadic`）、轉換決策 |
| `jc-jadic` | Mozc `dictionary_oss` 解析、去重排序、pack v2 與 manifest 寫出 | 執行期轉換 |
| `jc-model` | 模型 manifest、tokenizer、candle reranker／注音選字／Jinen 推論、逾時 | 解碼排序、個人化 |
| `jc-proto` | v3 訊框、請求／回應列舉、版本握手、錯誤碼、候選明細 | 組字狀態 |
| `jc-platform` | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 的語意契約 | 任何視窗或 OS handle |
| `jc-engine` | 每條連線的請求迴圈、握手、資料與模型載入、把按鍵交給 `Session`、個人學習檔載入與 flush | 轉換邏輯 |
| `jc-eval` | 題庫 loader、top-1／top-5／邊界／延遲指標、`Report`、gate | 引擎與資料載入 |
| `jc-testdata` | 測試資料路徑解析與缺資料跳過 | 正式執行期資料 |
| `jc-cli` | 評測、轉換、bench 的開發入口 | 產品執行期 |
| `jc-englishpack` | 英文詞表建置與打包 | 執行期轉換 |
| `jc-settings` | 設定與模型包的資料模型、驗證、原子寫檔 | UI |
| `jc-windows` | local socket client（Windows named pipe、Unix domain socket）、握手、逾時與重連、pipe 名稱規則 | 平台外殼 |
| `jc-linux`／`jc-mac` | Unix socket client 的佔位 | 目前全部回 `Unsupported` |
| `jc-tsf` | Windows TSF 文字服務、按鍵 sink、組字與候選窗、D2D／DComp 自繪、`ITfDisplayAttributeProvider` | 轉換決策 |
| `jc-tsf-register` | 共用的 COM／TSF 註冊常式 | COM 匯出（由 `jc-tsf` 提供） |
| `jc-tsf-smoke` | 安裝診斷（doctor）與 `jc-ime-doctor/1` JSON | 修改系統註冊狀態 |
| `jc-settings-ui` | 設定程式 GPUI 介面 | 引擎執行期設定 |

## 為什麼分離

1. 核心可在 Linux 上開發與評測，不必每次回到 Windows 驗證。
2. wrapper 只做轉發，崩潰不會把核心一起帶走，宿主程式（explorer）也不承擔
   核心風險。
3. 之後的 Fcitx5／macOS 只需要新的 wrapper，核心與題庫不動。
4. 神經模型用獨立的 worker 執行緒，輸入路徑不必等模型；逾時就回統計結果。

## 神經 runtime 的位置

模型載入與 worker 分在兩層：

| 工作 | 位置 |
|---|---|
| manifest、tokenizer、權重載入 | `jc-engine` 的 `loader.rs` |
| 模型物件與推論 | `jc-model`（candle；ORT 只在對照後端使用） |
| worker 執行緒、請求佇列、180 ms 逾時、舊請求丟棄 | `jc-core` 的 `session/neural.rs` |
| 門檻判斷、五條輸出驗證、回退統計 | `jc-core` 的 `Session` |

`SessionConfig` 只持有 `NeuralRuntime` 的 handle。引擎載入模型後注入
runtime；沒有模型時 handle 是 `None`，轉換完全走統計，不報錯也不阻塞。

```text
按鍵 ──▶ Session::press
           ├─ 統計 Viterbi（一定先算）
           ├─ gap < 460？──否──▶ 回統計結果
           └─ 是 ──▶ NeuralRuntime 佇列 ──▶ worker（candle）
                        │                     │
                        └─ 180 ms 逾時 ───────┴─▶ 回統計結果
                                               └─ 通過驗證 ──▶ 替換
```

## 協定

- 傳輸：4-byte 小端長度前綴 + JSON（serde）；預設訊框上限 1 MiB。
- 版本：`jc_proto::PROTOCOL_VERSION`（目前 `3`），握手不一致即回
  `VersionMismatch` 並收線。
- 請求：`Hello`、`Key`、`Toggle`、`Snapshot`、`Commit`、`Shutdown`，另有
  `Ping`、`Health`、`Close`、`ResetPersonal`、`OutputSimplified` 等 0c 擴充 op；
  狀態回應是完整快照，wrapper 不需要自己保存組字狀態。
- 狀態回應的 `State` 另有 `cursor_rune`、`suggestion`、`label`、
  `navigating`、`details`、`segments`、`highlighted`；空值時省略。
- 錯誤：`VersionMismatch`、`BadFrame`、`FrameTooLarge`、`UnknownOp`、
  `Unsupported`、`Internal`。
- 傳輸方式：stdio、Unix domain socket，Windows 上 `--socket` 或 `--supervise`
  走 named pipe；每條連線一個 worker thread，可並行。
- `Ready` 只有 `protocol`、`engine`、`session`，沒有 `max_frame` 或 `features`。
- wrapper 作者規格見 [`docs/wrapper.md`](wrapper.md)，契約本體見
  [`docs/proto.md`](proto.md)。

## 核心的請求流程

1. wrapper 把平台按鍵轉成 `jc_proto::Key`（語意鍵，不含 OS 鍵碼）。
2. `Engine::handle` 檢查握手後呼叫 `Session::press`。
3. `Session` 依模式走注音、日文或 Mixed 解碼；日文先算統計，必要時再問
   Jinen。回傳含組字文字、游標、候選、明細與片段的 `Snapshot`。
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

- `jc-platform` 的契約與測試已完成，但 `jc-engine` 還沒有依賴它。Windows TSF
  外殼已實作（`jc-tsf`、`jc-tsf-register`）。
- `jc-windows` 已是可用的 local socket client（Windows named pipe／Unix domain
  socket）；`jc-linux`／`jc-mac` 只回 `Unsupported`。
- `jc-model` 已接進 `jc-core` 的注音選字；`jc-engine` 啟動時若資料根目錄有
  `models/` 就安裝，缺檔只警告並維持統計排序。
- 日文預設走 Jinen small 的門檻混合；reranker 保留程式但預設關閉，標點模型
  已移除。
- `jc-engine` 正式啟動要求 `--data` 或 `JC_DATA`，並載入注音詞典與日文
  pack；`--allow-empty-data` 才使用空 session，供協定測試與除錯。
- 注音 `lexicon.bin` 與 `build-zh-bigram/lm.bin` 的讀取接在 `jc-engine` 的
  `loader` 與 `jc-cli`；舊 `build-zh/lm.bin` 已退役，`eval zh-golden` 仍讀
  凍結 TSV。
- `jc-settings` 由 `jc-engine` 的 `config` 讀取，啟動時轉成 `SessionConfig`；
  執行期修改仍要重啟引擎。
- 個人學習檔由 `jc-engine` 載入與 flush；預設 `%LOCALAPPDATA%\jc-ime\personal.json`，
  可用 `--personal-dir` 改。
