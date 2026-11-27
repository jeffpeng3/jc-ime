# crate 切分與命名 review

## 結論摘要

- `jc-model` 應該消失。共用推論 primitive 改成薄的 `jc-infer`，日文模型搬到
  `jc-ja`，中文模型搬到 `jc-zh`。不建議只把 `jc-model` 改名後繼續放兩個語言
  的模型。
- `jc-windows` 改名 `jc-transport`。它是跨平台 local socket client 加 pipe
  名稱規則，不是 Windows 專屬 adapter。
- 平台 adapter 用平台命名：`jc-tsf`、`jc-fcitx5`、`jc-ibus`、`jc-macos`。
  刪掉沒有依賴者的 `jc-linux`、`jc-mac` 空殼。
- 中文目前沒有自己的 crate。注音執行期在 `jc-core::zh`、`jc-core::engine::zh`
  與 `jc-core::session` 的注音分支；`jc-jadic`、`jc-englishpack` 是資料建置
  工具，不是日文／英文的執行期 crate。
- `jc-core` 目前不是語言無關。它直接載入注音選字模型與日文 reranker，還直接
  match 注音與羅馬字事件。要做到語言 crate 對稱，必須先完成語言引擎的 P2
  接線，不能只搬資料夾。

## 範圍與依據

這份文件只盤點 workspace 的 crate 職責、相依方向與名稱，提出重組順序。程式與
建置設定不在這份變更內。

判斷依據：

- `cargo metadata --format-version 1 --no-deps` 的成員、target 與依賴種類。
- 各 `crates/*/Cargo.toml` 的 `[dependencies]`、`[dev-dependencies]` 與 target
  專屬依賴。
- 原始碼實際的 `use`，不只看文件描述。
- `cargo tree -p <crate> -i <crate>` 確認傳遞相依。
- 出貨清單以 `installer/build.ps1`、`installer/jc.iss` 與
  `scripts/gen-rust-notices.py` 的 `SHIPPED` 為準。

建置時間、DLL 大小與執行檔大小都沒有量測。本文提到可能變小時，一律標成
「待量測」，不當成已驗證的收益。

## 現況表

「被誰依賴」只列正常依賴。dev-dependency 另外在 `jc-testdata` 列說明。

| crate | 職責 | 主要依賴 | 被誰依賴 | 是否出貨 |
|---|---|---|---|---|
| `jc-core` | 注音、日文、Mixed 解碼，lattice／Viterbi、`Session`、候選、個人化；內含語言專屬實作 | `jc-data`、`jc-model`、`serde`、`unicode-*` | `jc-proto`、`jc-settings`、`jc-engine`、`jc-cli`、`jc-englishpack` | lib，隨 engine／cli／settings-ui 建置圖 |
| `jc-data` | `0CJADIC` pack、注音 `lexicon.bin`／`lm.bin` 的 mmap 讀取、manifest 驗證 | `memmap2`、`sha2`、`serde` | `jc-core`、`jc-engine`、`jc-cli`、`jc-settings-ui` | lib，隨上述執行檔 |
| `jc-model` | 共用名稱，實際同時有日文 reranker／Jinen、中文注音選字與 candle primitive | `candle-*`、`tokenizers`、`prost` | `jc-core`、`jc-engine`、`jc-cli` | lib，隨 engine／cli |
| `jc-proto` | v3 訊框、請求／回應、握手、錯誤碼、候選明細；直接轉出 `jc-core` 的 `Key`、`Mode`、`Snapshot` | `jc-core`、`serde_json` | `jc-engine`、`jc-windows`、`jc-tsf`、`jc-tsf-smoke`、`jc-linux`、`jc-mac` | lib，隨 engine／TSF／doctor |
| `jc-platform` | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 契約與測試宿主 | `serde` | 沒有正常依賴者；只有自己的 `tests/contract.rs` 使用 | 否 |
| `jc-eval` | 題庫 loader、top-1／top-5／邊界／延遲指標、`Report`、gate | `serde` | `jc-cli` | 否 |
| `jc-testdata` | 測試資料根目錄解析、`require_data!`、缺資料跳過 | 無 | `jc-cli` 的正常依賴；`jc-core`、`jc-data`、`jc-model`、`jc-engine`、`jc-eval`、`jc-englishpack` 的 dev 依賴 | 否；但會進 `jc-cli` 的建置圖與授權清單 |
| `jc-engine` | 引擎行程、請求迴圈、握手、資料與模型載入、設定、個人檔 flush、`--supervise` | `jc-core`、`jc-data`、`jc-model`、`jc-proto`、`jc-settings`、`jc-windows`、`interprocess` | `jc-cli` | `jc-engine.exe` |
| `jc-engine-launcher` | 登入自動啟動用的無視窗啟動器；轉交原始命令列給 `jc-engine.exe` | `windows-sys`（只有 Windows target） | 無 | `jc-engine-launcher.exe` |
| `jc-windows` | 跨平台 local socket client、握手、逾時與重連、pipe 名稱解析；Unix 也走同一份 `LocalSocket` | `jc-proto`、`interprocess`、`windows-sys` | `jc-engine`、`jc-tsf-smoke` | lib，隨 engine／doctor |
| `jc-linux` | Unix socket client 佔位，所有入口回 `Unsupported` | `jc-proto` | 無 | 否 |
| `jc-mac` | Unix socket／launchd client 佔位，所有入口回 `Unsupported` | `jc-proto` | 無 | 否 |
| `jc-cli` | 開發入口：`eval`、`convert`、`bench`、`tokenize-ja`、`data`，另有多個開發 bin | `jc-core`、`jc-data`、`jc-engine`、`jc-eval`、`jc-model`、`jc-testdata`、`ureq` | 無 | `jc-cli.exe`；其他 bin 不進安裝包 |
| `jc-jadic` | Mozc `dictionary_oss` 匯入、pack v2 與 manifest 寫出 | `serde`、`serde_json`、`sha2` | 無 | 否 |
| `jc-englishpack` | ESDB／SCOWL 與產品詞表轉成 `0cEN` 資料包 | `jc-core` | 無 | 否 |
| `jc-settings` | 設定與模型包的資料模型、驗證、原子寫檔；不含 UI | `jc-core`（只用模糊音與 inputstyle）、`sha2` | `jc-engine`、`jc-settings-ui` | lib，隨 engine／settings-ui |
| `jc-tsf` | Windows TSF 文字服務、COM 外殼、按鍵、組字、候選窗、自有 IPC | `jc-proto`、`jc-tsf-register`、`windows`、`windows-core` | 無 | `jc-tsf.dll`（x64／x86） |
| `jc-tsf-register` | 共用的 COM／TSF 註冊常式；只有 rlib，不提供 COM 匯出 | `windows`（只有 Windows target） | `jc-tsf` | lib，連結進 `jc-tsf.dll` |
| `jc-settings-ui` | GPUI Kit 設定程式，讀寫 `settings.json` | `jc-data`、`jc-settings`、`gpui-kit` | 無 | `jc-settings-ui.exe` |
| `jc-tsf-smoke` | 安裝診斷工具（doctor），輸出 `jc-ime-doctor/1` JSON，13 項檢查 | `jc-proto`、`jc-windows`、`windows` | 無 | `jc-tsf-smoke.exe` |

workspace 目前有 20 個成員。`docs/architecture.md` 寫 19 個，漏了
`jc-engine-launcher`；`README.md` 的 crate 表也漏了 `jc-engine-launcher` 與
`jc-tsf-register`。這是文件漂移，不是 Cargo 設定錯誤。

## 語言 → crate 對照表

目前的名稱無法讓人從 crate 看出哪一個是中文。這張表以實際模組與檔案為準。

| 語言 | 執行期程式 | 資料格式／讀取 | 模型 | 建置工具 | 名稱現況 |
|---|---|---|---|---|---|
| 中文（注音） | `jc-core::zh` 的 `syllable`、`keyboard`、`input`、`select`、`fuzzy`、`han`、`pinyin`；`jc-core::engine::zh`；`jc-core::session` 的注音分支；`jc-core::big5`、`jc-core::zhconv` | `jc-data::zh` 的 `lexicon.bin`、`lm.bin`、manifest | `jc-model::candle::zh` 與 `jc-core::zh::select` 的輸入組裝 | 沒有專屬 crate；`jc-cli` 的 `data` 子命令與 `scripts/*` 產生詞典／bigram；`candle-zh-check` 產生量化模型 | 沒有 `jc-zh`，名稱最不對稱 |
| 日文 | `jc-core::ja` 的 `input`、`lexicon`、`lm`、`decode`、`live`、`rerank`、`personal`、`rewrite`；`jc-core::engine::ja`；session 的日文分支 | `jc-data::ja` 的 `0CJADIC` pack | `jc-model::{rerank, layout, spm, tokenizer}`、`jc-model::candle::jinen` | `jc-jadic` 匯入 Mozc 詞典並寫 pack v2 | 沒有 `jc-ja`；`jc-jadic` 是資料匯入器，不是執行期 crate |
| 英文 | `jc-core::mixed_latin`、`jc-core::engine::ascii`、`jc-core::personal::english`、`jc-core::mixed` 的英文詞表 | `jc-core::mixed` 的 `words.bin` 讀取 | 無 | `jc-englishpack` 建置並打包英文詞表 | 沒有 `jc-en`；`jc-englishpack` 是建置器，不是執行期 crate |
| Mixed | `jc-core::mixed`、`jc-core::session::mixed`、`jc-core::engine::router` | 無獨立格式 | 無 | 無 | 沒有獨立 crate，全部在 core |

不對稱的地方：

1. 中文完全沒有 crate 邊界。注音在 `jc-core` 裡，連一個叫 `jc-zh` 的模組
   邊界都沒有；`session` 內也沒有獨立的 `zh.rs`，注音分支散在 `session/core.rs`
   與 `session/misc.rs`。
2. `jc-jadic` 與 `jc-englishpack` 是資料建置工具。把它們當成「日文 crate」與
   「英文 crate」會誤解現況，因為執行期日文在 `jc-core::ja`，英文在
   `jc-core::mixed_latin` 與 `mixed`。
3. 中文沒有 Rust 的 pack 建置器。日文有 `jc-jadic`，英文有
   `jc-englishpack`；中文目前靠 `jc-cli` 與 shell script。
4. `jc-model` 同時包含中文與日文模型，讓「每個語言有自己的 crate」更難看出來。
5. `jc-core` 名稱看起來語言無關，實際包含注音、日文與 Mixed。語言引擎介面
   `LanguageEngine` 目前是 P1 shadow，`feed()` 與 `candidates()` 還沒接手，所以
   不能只用名稱推論 core 已經語言無關。

## 問題清單

沒有 cargo 套件循環。問題集中在分層方向、命名、語言邊界，以及幾個已沒有消費
者的 crate。

| # | 問題 | 證據 | 影響 |
|---|---|---|---|
| 1 | `jc-proto` 依賴 `jc-core`，把整個推論堆疊帶進所有 client | `jc-proto/src/lib.rs` 直接 `pub use jc_core::{Key, KeyCode, Lang, Mode, Snapshot}`、`jc_core::candidate`；`cargo tree -p jc-windows -i candle-core` 與 `cargo tree -p jc-tsf --target x86_64-pc-windows-msvc -i candle-core` 都走 `jc-proto -> jc-core -> jc-model -> candle-core` | TSF DLL、wrapper client、doctor 都建置 candle／tokenizers；Rust linker 可能裁掉未使用的機器碼，但 cargo 仍會編譯整條圖。DLL 大小與 clean build 時間待量測 |
| 2 | `jc-settings` 為了兩組純規則依賴整個 `jc-core` | `jc-settings/src/settings.rs` 用 `jc_core::zh::fuzzy`；`profiles.rs`、`profile_rules.rs` 用 `jc_core::inputstyle` | `jc-settings-ui` 的 `cargo tree -p jc-settings-ui -i candle-core` 一樣命中 candle。設定程式不該需要推論後端 |
| 3 | `jc-windows` 的名稱與實際職責不符 | crate 內是 `LocalSocket`，Windows 用 named pipe、Unix 用 unix domain socket；`jc-engine` 還用它做 server 端的 pipe 名稱解析 | 文件與 README 把 client 說成 Windows 專用，Linux 讀者會以為沒有可用 client。名稱也讓 transport 與平台 adapter 混在一起 |
| 4 | 平台 adapter 用 OS 命名，沒有說出 adapter | `jc-linux`、`jc-mac` 是空殼；`jc-windows` 同時被當成 transport 與 Windows 端；沒有 `jc-fcitx5`、`jc-ibus`、`jc-macos` | Linux 可能同時有 fcitx5 與 ibus，一個 `jc-linux` 無法表達差異；讀者也看不出哪個 crate 是平台外殼 |
| 5 | `jc-model` 把兩個語言模型與共用推論混在一起 | `Role::{ZhSelect, JaRerank}`；`candle/zh.rs` 是中文注音選字，`candle/jinen.rs` 與 `rerank/layout/spm/tokenizer` 是日文；`jc-core` 的 `zh/select.rs`、`ja/rerank.rs` 直接載入模型 | core 直接綁定兩個語言模型；client 又透過 proto 拉進 candle；模型程式無法隨語言獨立演進 |
| 6 | `jc-core` 不是語言無關，名稱卻像語言無關 | `jc-core/src/mixed.rs` 同時 import `crate::zh` 與 `crate::ja`；`session/core.rs` 直接 match `ZhInputEvent` 與 `RomajiEvent`；`engine/zh.rs`、`engine/ja.rs` 仍把狀態留在 `Session` | 不能只把 `zh/`、`ja/` 搬出去就宣稱 core 語言無關。真正拆分需要先完成語言引擎 P2 |
| 7 | 中文沒有 runtime crate，名稱不對稱 | 中文在 `jc-core::zh`、`jc-core::engine::zh`、`jc-core::session` 與 `jc-data::zh`；沒有 `jc-zh`。日文有 `jc-jadic`，英文有 `jc-englishpack`，但兩者是建置工具 | 使用者無法從 crate 名稱辨識中文；文件也容易把資料建置器誤當成語言 runtime |
| 8 | pipe 名稱解析有兩份實作，行為已分叉 | `jc-windows/src/pipe_name.rs` 支援 `JC_IME_PIPE`、`HKCU\Software\jc-ime\EnginePipe`、SID 尾段；`jc-tsf/src/ipc.rs` 的 `server_pipe()` 只讀環境變數與 SID，沒有 registry 覆寫。`pipe_name.rs` 與 `ipc_transport.rs` 的註解卻說 TSF 呼叫 `resolve_pipe_name()` | 使用者只設 registry 覆寫時，引擎與 TSF 會算出不同 pipe 名稱，按鍵被放行成英文。這是行為問題，不只是重複程式碼 |
| 9 | `jc-linux`／`jc-mac` 是沒有依賴者的空殼 | `cargo metadata` 沒有任何正常依賴指向這兩個 crate；`jc-windows` 的 `LocalSocket` 已經支援 Unix domain socket | 兩個 crate 各自重複 `unix.rs`／`unsupported.rs`，增加 workspace 與文件維護成本，也讓現況看起來像「Linux 沒有 client」 |
| 10 | `jc-tsf-smoke` 名稱像測試，實際是出貨診斷工具 | `installer/jc.iss` 把它放進 `[Files]`；`installer/post-install-doctor.ps1` 預設執行它；CI 的安裝 job 用它做 13 項檢查；`scripts/gen-rust-notices.py` 的 `SHIPPED` 也列它 | 名稱會誤導讀者以為它只在測試跑。全 repo 近 40 個檔案提到這個名字，改名成本比一般 crate 高 |
| 11 | `jc-cli` 同時承擔出貨執行檔與開發工具 | `installer/jc.iss` 安裝 `jc-cli.exe`；`jc-cli` 另外有 `ja-eval`、`ja-cost`、`jinen-eval`、`jinen-gate`、`zh-calibrate`、`candle-zh-check` 六個 bin；`jc-testdata` 放在正常依賴 | `-p jc-cli` 會連開發 bin 一起建置，`installer/build.ps1` 必須用 `--bin` 逐一挑選。產品邊界與開發邊界不清楚 |
| 12 | `jc-platform` 是文件說在用、程式沒接的契約 crate | 只有 `crates/jc-platform/tests/contract.rs` 使用；`jc-core` 沒有 `jc_platform` 的 `use` | 讀者會以為核心透過 `InputHost` 與 wrapper 互動，實際 `Session` 回傳 `Snapshot`，由 wrapper 自行轉換 |
| 13 | `jc-core` 的「不碰檔案」描述與實作不一致 | `personal/mod.rs` 直接讀寫 `personal.json`；`zhconv.rs` 讀檔；`score.rs`、`ja/lexicon.rs` 直接使用 `jc-data` 的 mmap 型別 | 文件與責任表需要修正。這不代表要馬上拆 crate，但邊界描述要可信 |
| 14 | 文件的人數與 crate 清單已漂移 | `docs/architecture.md` 寫 19 個成員；`README.md` crate 表缺 `jc-engine-launcher`、`jc-tsf-register`；`docs/port/client-crates.md` 仍寫 `Client<NamedPipe>`，實際是 `Client<S>` 加 `LocalSocket` | 新讀者會用錯型別或漏看啟動器 |

### 分層方向的判讀

- `jc-proto -> jc-core` 是反向分層。協定 crate 應該是 wrapper 與引擎的最小共同
  語言，不該把 core 的解碼器、mmap 與 candle 一起拉進來。
- `jc-settings -> jc-core` 是上層資料模型依賴下層引擎。這裡沒有循環，因為
  core 不依賴 settings，但方向讓設定程式承擔推論建置成本。
- `jc-core -> jc-model` 不只是相依，還把語言模型載入與 forward 型別放進 core。
  這讓「core 語言無關」與「jc-model 是共用模型」兩個描述同時失效。
- `jc-core -> jc-data`、`jc-core -> jc-model` 是 core 內部的實作依賴。它可以
  運作，但讓 core 的責任變得過寬。
- 其餘正常依賴方向合理：`jc-engine -> core/data/model/proto/settings/windows`，
  `jc-tsf -> proto/tsf-register`，`jc-settings-ui -> data/settings`。

### 邊界判定

| 配對 | 判定 | 說明 |
|---|---|---|
| `jc-core` vs `jc-engine` | 大致合理，暫不拆 | engine 負責行程、IPC、設定與資料載入；core 負責 `Session` 與解碼。重疊在個人檔讀寫、神經 worker 與模型載入；模型部分由 `jc-model` 重組處理 |
| `jc-core` vs 語言模型 | 不合理，需調整 | core 直接建構 `ZhSelect`、`QuantZhSelect`、`Reranker`、`ModelSet`。core 應只留 `JaReranker`、`NeuralEngine` 與新的 zh selector trait，由 engine 注入 |
| `jc-model` vs `jc-zh`／`jc-ja` | 須拆分 | 共用 primitive 留薄 runtime；語言模型與 prompt／tokenizer／輸出解碼搬到語言 crate |
| `jc-proto` vs `jc-windows` | 協定／傳輸分界正確，依賴方向錯誤 | proto 只管訊框與訊息，transport 只管 socket。問題是 proto 依賴 core，不是兩者職責重疊 |
| `jc-proto` vs `jc-platform` | 不合併 | proto 是線格式與 JSON；platform 是宿主 trait。兩者可以都叫契約，但版本與消費者不同。合併會讓 platform 被 wire 版本綁住 |
| `jc-platform` vs 平台 adapter | 概念正確，接線未完成 | platform 提供 trait，adapter 提供 TSF／Fcitx5／IBus／macOS 實作。目前 adapter 只有 TSF，platform 沒有消費者 |
| `jc-windows` vs 平台 adapter | 須分離 | `jc-windows` 是 transport，不該同時代表 Windows adapter。Windows adapter 是 `jc-tsf` |
| `jc-data` vs `jc-testdata` | 合理，不合併 | data 是執行期 pack 讀取；testdata 是 repo 外的測試資料定位。壽命與出貨條件不同 |
| `jc-settings` vs `jc-settings-ui` | 合理，不動分界 | settings 是純資料模型與檔案格式；settings-ui 只做畫面。要修的是 settings 對 core 的依賴 |
| `jc-tsf` vs `jc-windows` | 傳輸實作分開是合理的 | TSF 需要 overlapped I/O、SDDL、spawn 與 1 秒分段逾時；`jc-windows` 用 interprocess 加背景執行緒。只該共用 pipe 名稱規則，不該強制共用 transport |
| `jc-engine-launcher` vs `jc-engine` | 合理，暫不合併 | 啟動器是 windows subsystem，只需要 `windows-sys`；併進 engine 會讓它連帶依賴整個引擎圖，也讓 subsystem 設定更容易被誤改 |
| `jc-eval` vs `jc-cli` | 目前合理 | `jc-eval` 沒有 core 依賴，只有 `jc-cli` 一個消費者；保留成 lib 讓評測測試與 CLI 分開 |
| `jc-jadic`／`jc-englishpack` vs 產品 | 合理，保持獨立 | 兩者是資料建置工具，不進安裝包。`jc-englishpack` 依賴 core 會拉進 candle，但只在建置資料時發生，優先度低 |

### `jc-proto`、`jc-platform`、`jc-windows`、`jc-testdata` 的邊界

這四個 crate 看起來零碎，但不是全部都該合併。

| crate | 是否真邊界 | 判斷 |
|---|---|---|
| `jc-proto` | 是 | 訊框、JSON、握手與版本是 wrapper 與 engine 的共同契約。保留獨立 crate，但移除它對 `jc-core` 的依賴 |
| `jc-platform` | 概念上是，目前沒有消費者 | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 是平台 adapter 的共同 trait。現在 core 沒接，應決定接上或刪除，不該維持「文件說在用」 |
| `jc-windows` | 是，名稱錯 | transport 與 client 是獨立邊界，該留；名稱改成 `jc-transport`，不要讓它冒充 Windows adapter |
| `jc-testdata` | 是 | 測試資料定位必須留在 runtime 之外。併進 `jc-data` 會把測試路徑與缺資料跳過帶進正式資料層 |

結論：四個裡面只有 `jc-platform` 需要決定去留。`jc-proto`、`jc-transport`、
`jc-testdata` 是三個不同生命週期的真邊界，不建議合併。`jc-proto` 與
`jc-platform` 也不建議合成一個 `jc-contract`，因為線格式版本與宿主 trait 的
演進速度不同。

## 重組建議

### A. `jc-windows` 改名為 `jc-transport`（低風險改名，P1）

**動機**

`jc-windows` 已經是跨平台 transport 加 client，還包含引擎與 TSF 共用的 pipe
名稱規則。`jc-transport` 能同時涵蓋 named pipe、Unix domain socket、握手與
名稱解析。若比較偏好 client 視角，`jc-client` 也可接受，但 engine 端也在用
這個 crate，`jc-transport` 比較準。

**影響檔案與 CI／installer**

- `Cargo.toml` 的 workspace member 與 `crates/jc-windows/Cargo.toml` 的
  `package.name`。
- 10 個 `.rs` 檔、21 處 `jc_windows`，主要在 `jc-engine` 的 `main.rs`、
  `pipe_listener.rs`、`supervise.rs`、`tests/socket.rs` 與
  `jc-tsf-smoke/src/engine.rs`。`jc-tsf/src/ipc_transport.rs` 只有註解提到。
- `.github/workflows/ci.yml` 的 `-p jc-windows`。
- `scripts/verify-keybinds.sh` 的說明文字。
- `README.md`、`docs/architecture.md`、`docs/wrapper.md`、`docs/proto.md`、
  `docs/status.md`、`docs/release.md`、`docs/port/client-crates.md` 等文件。
- `Cargo.lock` 與 `legal/RUST-THIRD-PARTY-NOTICES.md` 需重新產生。
- installer 沒有直接寫 `jc-windows` 套件名，`installer/build.ps1` 用
  `-p jc-engine` 帶進來，所以安裝腳本本身不需要改檔名。

**遷移步驟**

1. `git mv crates/jc-windows crates/jc-transport`，改 package name 與 workspace
   member，所有 path 依賴改成 `../jc-transport`。
2. 將程式中的 `jc_windows` 改為 `jc_transport`。這是 Rust 路徑，不是 wire
   格式，不需要版本相容層。
3. 更新 CI 的 `-p` 清單、`scripts/verify-keybinds.sh` 與所有文件。
4. 執行 `cargo metadata`、`cargo check --workspace --all-targets`、
   `cargo test -p jc-transport`，再跑 `python3 scripts/gen-rust-notices.py`。
5. 確認 `jc-engine.exe`、`jc-tsf.dll`、pipe 名稱與 JSON 都沒有變。

**風險**

低。這是套件名稱與 Rust 路徑變更，不動執行檔名、pipe 名或協定。主要風險是
文件與 CI 漏改，以及 `Cargo.lock` 的大量名稱變動。建議單獨一個 commit，不要
和依賴拆分混在一起。

### B. `jc-tsf-smoke` 改名為 `jc-tsf-doctor`（低風險改名，P2）

**動機**

這支程式已經出貨，安裝器最後一步會執行它，輸出 `jc-ime-doctor/1`。`smoke`
讓人以為它只在 CI 跑，與產品的安裝診斷角色不符。`jc-tsf-doctor` 與現有 JSON
schema 和文件用語一致。

**影響檔案與 CI／installer**

- `installer/jc.iss` 的 `[Files]`、簽章清單與安裝後執行項目。
- `installer/build.ps1` 的 `-p`、`--bin` 與預期產物清單。
- `installer/post-install-doctor.ps1` 的 `$SmokeExe` 預設值。
- `.github/workflows/ci.yml` 的套件清單、Windows 安裝 job 與 doctor 步驟。
- `scripts/gen-rust-notices.py` 的 `SHIPPED` 與
  `legal/RUST-THIRD-PARTY-NOTICES.md`。
- `README.md`、`docs/architecture.md`、`docs/install.md`、`docs/status.md`、
  `docs/port/tsf-smoke.md` 等文件。

**遷移步驟**

1. 先改 package name 為 `jc-tsf-doctor`，bin 名稱先維持 `jc-tsf-smoke`，確認
   installer 與 CI 只依賴 bin 檔名時不會中斷。
2. 下一個變更再把 bin 改成 `jc-tsf-doctor.exe`，同步更新 installer、doctor
   腳本、CI、`SHIPPED` 與文件。
3. 重新產生授權清單，跑 `installer/validate-linux.sh` 與安裝 job。
4. 若要降低外部腳本中斷，可保留一個 `jc-tsf-smoke` 的過渡 bin，內容只印
   遷移訊息並轉呼叫新程式；下一個 release 再移除。

**風險**

中。這是對外執行的檔名，安裝器、簽章、CI 與文件都可能漏改。分兩階段做比
一次改 package 加 bin 安全。

### C. 平台 adapter 依平台命名，刪除 `jc-linux`／`jc-mac` 空殼（低風險改名，P1）

**動機**

平台 adapter 的名字要說出平台。`jc-tsf` 已經是清楚的 Windows TSF adapter；
`jc-linux`、`jc-mac` 是 OS 名稱，不是 adapter 名稱，而且沒有依賴者。
`jc-windows` 同時代表 transport 與 Windows 端，會讓 adapter 與 transport 的
邊界消失。

目標形狀：

```text
jc-platform（共用 trait／能力）
  ├─ jc-tsf       Windows TSF
  ├─ jc-fcitx5    Linux fcitx5
  ├─ jc-ibus      Linux ibus
  └─ jc-macos     macOS

jc-transport      共用的 local socket client，不是平台 adapter
```

`jc-tsf` 可以保留自己的 IPC，不需要改用 `jc-transport`。`jc-fcitx5`、
`jc-ibus`、`jc-macos` 還沒有實作，等真的有程式再建立 crate。名稱可以再討論，
重點是 adapter 名稱要說出平台，transport 不要冒充 adapter。若兩個 Linux
adapter 之後有共用程式，再建立 `jc-linux-common`，不要用一個 `jc-linux`
吃掉兩種 adapter。

**影響檔案與 CI／installer**

- 根 `Cargo.toml` 的 `jc-linux`、`jc-mac` members 與兩個目錄。
- `README.md`、`docs/architecture.md`、`docs/proto.md`、`docs/status.md`、
  `docs/wrapper.md`、`docs/port/client-crates.md` 的 adapter 說明。
- CI 沒有直接 `-p` 這兩個 crate，`--workspace` 會自動少掉。
- installer 沒有引用，不需要改安裝腳本。
- 未來的 `jc-fcitx5`／`jc-ibus`／`jc-macos` 目前不建立，沒有 CI 變更。

**遷移步驟**

1. 確認沒有程式依賴：`cargo metadata` 與 `rg "jc_linux|jc_mac"`。
2. 刪除 `crates/jc-linux/`、`crates/jc-mac/` 與 workspace members。
3. 文件改成「`jc-transport` 已支援 Unix domain socket；Linux／macOS adapter
   尚未實作」，並列出預定的 adapter 名稱。
4. 執行 `cargo metadata`、`cargo check --workspace --all-targets` 與
   `cargo test --workspace`。
5. 未來實作 Fcitx5 時建立 `jc-fcitx5`，不要復活 `jc-linux`。

**風險**

低。沒有依賴者，也沒有出貨檔。主要風險是名稱討論與未來 Linux 共用程式
的擺放位置。共用程式等真的出現再抽，不要先建空殼。

### D. 切斷 `jc-proto -> jc-core`，讓 wrapper 不再建置推論堆疊（合併／拆分，P1 決策，P2 實作）

**動機**

目前 `jc-tsf`、`jc-windows`、`jc-tsf-smoke` 都透過 `jc-proto` 拉進
`jc-core`、`jc-data`、`jc-model`、`candle-core`、`tokenizers`。client 只需要
wire 型別，不應該承擔解碼器與模型。這也是 `jc-settings-ui` 與 TSF DLL 建置圖
過大的共同來源之一。

**影響檔案與 CI／installer**

- `crates/jc-proto/Cargo.toml`、`crates/jc-proto/src/lib.rs`。
- `crates/jc-core/Cargo.toml`、`crates/jc-core/src/lib.rs`，以及
  `zh/select.rs`、`ja/rerank.rs` 與 `session` 內相關的型別轉換。
- `crates/jc-windows`、`crates/jc-tsf`、`crates/jc-tsf-smoke` 的 `use`。
- `Cargo.lock`、授權清單、CI 的 `cargo tree` 檢查。
- installer 不需改檔名，但 TSF 與 doctor 的建置圖會變。

**遷移步驟**

有兩個方案，先量測再選：

1. **短期 feature 方案**：在 `jc-core` 加 `model` feature，預設關閉，將
   `jc_model` 的使用與 rerank／注音選字後端用 `cfg` 或 no-op 介面隔開。
   `jc-proto` 與 `jc-settings` 以 `default-features = false` 依賴 core，
   `jc-engine` 與 `jc-cli` 開啟 feature。用
   `cargo tree -p jc-tsf --target x86_64-pc-windows-msvc -i candle-core`
   驗證不再命中。注意 workspace 一起建置時 Cargo 會做 feature 聯集，
   installer 的單次多 package 建置仍可能把 feature 打開，這種情境要另外量測。
2. **長期型別方案**：新增 leaf crate（暫名 `jc-wire`），把 `Key`、`KeyCode`、
   `Lang`、`Mode`、`Snapshot`、`Candidate`、`CandidateSource` 與候選版面型別
   搬過去。`jc-proto` 只依賴 leaf；`jc-core` 也依賴 leaf 並保留 re-export，
   降低既有程式改動。這會動到較多 `use`，但能徹底移除 proto 對 core 的依賴。

不論哪個方案，都要用 `crates/jc-proto/tests/fixtures.rs` 鎖住 JSON 形狀，並
比較改動前後的 `jc-tsf.dll` 大小與 `cargo clean` 後建置時間（兩者目前都是
待量測）。

**風險**

中到高。feature 方案容易出現 `cfg` 分叉與 feature 聯集死角；型別方案則是
大範圍搬移，可能讓 wire 與 core 的 `Snapshot` 逐漸分叉。若量測顯示 DLL 大小
與建置時間沒有明顯差異，就不要為了整齊做型別方案。

### E. 抽出 `jc-input-rules`，切斷 `jc-settings -> jc-core`（合併／拆分，P2）

**動機**

`jc-settings` 只為了 `zh::fuzzy` 與 `inputstyle` 兩個純規則模組依賴 core，
結果 `jc-settings-ui` 的建置圖出現 candle。把這兩個模組抽成沒有依賴的 leaf
crate，core 與 settings 都依賴它，設定程式就能脫離推論堆疊。

**影響檔案與 CI／installer**

- 新 crate `crates/jc-input-rules` 與根 `Cargo.toml`。
- `crates/jc-core/src/zh/fuzzy.rs`、`crates/jc-core/src/inputstyle.rs` 及其
  測試。
- `crates/jc-settings/Cargo.toml`、`settings.rs`、`profiles.rs`、
  `profile_rules.rs`。
- `Cargo.lock`、授權清單、`cargo tree` 檢查。
- installer 不改檔名。

**遷移步驟**

1. 建立 leaf crate，搬入 `fuzzy` 與 `inputstyle`，公開路徑保持穩定。
2. `jc-core` 改成 re-export，既有 `jc_core::zh::fuzzy` 與
   `jc_core::inputstyle` 先不改呼叫端。
3. `jc-settings` 改依賴 leaf，移除 `jc-core`。
4. 驗證 `cargo tree -p jc-settings-ui -i candle-core` 不再命中，跑 settings 與
   core 的測試。
5. 比較 `jc-settings-ui.exe` 大小與 clean build 時間，兩者待量測。

**風險**

中。規則表是行為來源，搬移時若漏掉 cfg 或測試資料會造成模糊音與輸入習慣
預設漂移。用 re-export 保留舊路徑可以降低風險，但不要無限期保留兩份定義。

### F. 語言 crate 對稱化：`jc-zh`／`jc-ja`／`jc-en`（合併／拆分，P3）

**動機**

目前中文沒有 crate 名稱，日文與英文容易被誤認成有 crate，因為
`jc-jadic`、`jc-englishpack` 是資料建置器。使用者要求能把「哪個 crate 是
中文」直接看出來。長期目標是讓 `jc-core` 只留 router、`CandidateSet`、
`CandidateAction`、`Composition` 與通用 session，語言各自成 crate。

**影響檔案與 CI／installer**

- `crates/jc-core/src/zh/`、`src/ja/`、`src/mixed.rs`、
  `src/mixed_latin.rs`、`src/engine/{zh,ja,ascii}.rs` 與
  `src/session/{core,mixed,misc}.rs` 的大量 `use`。
- `jc-core/Cargo.toml`、`jc-engine/Cargo.toml`、`jc-cli/Cargo.toml`。
- 新 crate `jc-zh`、`jc-ja`、`jc-en` 與 `Cargo.lock`。
- 若改名資料建置器，`jc-jadic`、`jc-englishpack` 的 package name、腳本、
  CI 與文件。
- installer 不直接引用這些 lib，不需要改檔名。

**遷移步驟**

1. 先完成語言引擎 P2：把 `ZhInput`、`RomajiInput`、`JaDecoder`、
   `LiveController` 與候選產生搬進各自的 `LanguageEngine` adapter，讓
   `Session` 不再直接 match `ZhInputEvent`／`RomajiEvent`，mixed 路由改成向
   registry 取得語言能力。
2. 先做 G 的第一階段，把模型搬進 `jc-zh`、`jc-ja`。這時兩個 crate 只放模型，
   不依賴 `jc-core`；`jc-core` 暫時依賴它們，避免循環。
3. 在 core 定義 zh selector trait，並把 `jc-zh`、`jc-ja` 的模型載入改成由
   `jc-engine` 注入。core 不再依賴語言 crate 後，才能把 runtime 搬進去。
4. 把 `jc-core::zh` 與 `jc-core::ja` 的純語言模組搬成 `jc-zh`、`jc-ja`。
   這時語言 crate 依賴 `jc-core` 的 `Key`、`Candidate`、`LanguageEngine` 與
   其他通用型別，方向是語言 crate -> core，core 不能再依賴它們。
5. 把 `jc-en` 從 `mixed_latin`、`engine::ascii`、`personal::english` 抽出來。
   英文目前沒有模型，這一步可以在 zh／ja 穩定後再做。
6. 可選的命名整理：把 `jc-jadic` 改成 `jc-ja-pack`、`jc-englishpack` 改成
   `jc-en-pack`，讓資料建置器與 runtime crate 一眼可分。中文目前沒有 Rust
   建置器，不要先建空的 `jc-zh-pack`。

語言 crate 若需要 `Key`、`Candidate`、`LanguageEngine` 等型別，依賴方向只能是
language -> core，core 不能再依賴 language。D 的 `jc-wire` 可以讓語言 crate
依賴 leaf 型別 crate，避開這個方向問題；沒有 `jc-wire` 時，runtime 搬移必須
等 core 移除依賴後再做。

**風險**

高。`session` 內有大量直接語言分支，mixed 路由同時依賴 zh 與 ja；語言引擎
目前只是 P1 shadow，`feed()`、`candidates()`、`apply()` 還沒接手。一次搬完
會很難定位行為回歸。建議照上面的階段做，每一步都保留統計解碼與 golden 測試。
建置時間可能因 crate 變多而增加，待量測。

### G. `jc-model` 消失，拆成 `jc-infer` 加語言模型（合併／拆分，P2）

**動機**

`jc-model` 的名字暗示它是一個語言無關的模型層，實際同時包含：

- 中文注音選字：`candle/zh.rs`、`jc-core::zh::select` 的輸入組裝。
- 日文 reranker：`rerank.rs`、`layout.rs`、`spm.rs`、`tokenizer.rs`、
  `candle/mod.rs` 的 `CandleReranker` 與 `Encoder`。
- 日文 Jinen：`candle/jinen.rs`。
- 共用 primitive：`ModelError`、`QuantDtype`、`LinearOp`、`Lookup`、GGUF
  讀取與量化矩陣乘法。
- 模型集合目錄：`Role::{ZhSelect, JaRerank}`、`ModelSet`、`ModelSpec`。

`jc-core` 直接依賴它，所以語言無關的 core 反而綁定兩個語言。`jc-proto` 又
依賴 core，讓 wrapper 也拉進 candle。這是目前最不清楚的邊界。

**影響檔案與 CI／installer**

- `crates/jc-model/` 全部檔案，特別是 `candle/mod.rs`、`candle/quant.rs`、
  `candle/zh.rs`、`candle/jinen.rs`、`rerank.rs`、`layout.rs`、`spm.rs`、
  `tokenizer.rs`、`manifest.rs`、`role.rs`、`bin/candle-quant.rs`。
- `crates/jc-core/src/zh/select.rs`、`crates/jc-core/src/ja/rerank.rs` 的模型
  載入，以及 `crates/jc-core/src/ja/decode.rs` 的 `JaReranker` 注入邊界。
- `crates/jc-engine/src/loader.rs` 的模型路徑、`ModelSet`、Jinen 與 zh-select
  載入。
- `crates/jc-cli/src/{cli.rs,eval_cmd.rs,tokenize.rs}` 與三個模型 bin。
- `Cargo.lock`、授權清單、`cargo tree` 檢查、CI 的 `-p jc-model` 或
  `cargo run -p jc-model` 指令。
- installer 不改檔名，但 TSF、doctor、settings-ui 的建置圖會變。

**遷移步驟**

建議的目標拆分：

| 現在 | 內容 | 目標 |
|---|---|---|
| `error.rs` | `ModelError`、`Result` | `jc-infer` |
| `candle/mod.rs` 的 `LinearOp`、`Lookup`、`inference`、張量 helper | 共用 primitive | `jc-infer` |
| `candle/quant.rs` 的 `QuantDtype`、`QuantLinear`、`QuantEmbedding`、GGUF 讀取、`ExportEntry`／`ExportReport` | 共用量化 runtime | `jc-infer` |
| `candle/quant.rs` 的 `QuantReranker`、`export_gguf`、reranker 權重載入 | 日文 reranker | `jc-ja` |
| `candle/mod.rs` 的 `CandleConfig`、`Encoder`、`CandleReranker`、`position_ids`、`score_packed_with` | 日文 reranker | `jc-ja` |
| `rerank.rs`、`layout.rs`、`spm.rs`、`tokenizer.rs` | 日文 reranker 與 tokenizer | `jc-ja` |
| `candle/jinen.rs` | 日文 Jinen | `jc-ja` |
| `candle/zh.rs` | 中文注音選字 forward | `jc-zh` |
| `jc-core::zh::select` 的模型載入與 `prepare` | 中文注音選字輸入與重排 | `jc-zh`，等 core 改成 trait 注入後再搬 |
| `manifest.rs` 的 `ModelSet`、`Availability`、角色目錄解析；`role.rs` 的 `Role` | 模型集合與角色 | `jc-engine` 的模型目錄層 |
| `manifest.rs` 的 `ModelSpec`、rerank 預設 seq／slot／context | 日文 reranker 佈局 | `jc-ja` |
| `bin/candle-quant.rs` | reranker 量化開發工具 | `jc-ja` 或 `jc-cli` 的開發 bin |

步驟：

1. 建立 `jc-infer`，先搬 `ModelError`、`LinearOp`、`Lookup`、`QuantDtype`、
   `QuantLinear`、`QuantEmbedding`、GGUF reader 與共用張量 helper。不要在
   `jc-infer` 保留 `Role`、prompt、tokenizer 或語言名稱。
2. 建立 `jc-ja` 與 `jc-zh`，把上表的語言模型搬進去。第一階段只放模型，兩個
   crate 都依賴 `jc-infer` 與 candle／tokenizers，不依賴 `jc-core`，所以
   `jc-core` 可以暫時依賴它們，不會形成循環。runtime 要等 core 移除依賴後
   才能搬入，見 F。
3. 把 `ModelSet`、`Role` 與模型路徑解析搬到 `jc-engine`；`ModelSpec` 與
   reranker 的 seq／slot／context 預設留在 `jc-ja`。engine 已經在做 Jinen 與
   zh-select 載入，這是組裝點。
4. 在 `jc-core` 留下 `JaReranker`（已存在）、`NeuralEngine`（已存在）與新的
   zh selector trait。移除 `jc-core` 對 `jc-model`、`jc-zh`、`jc-ja` 的正常
   依賴。`jc-engine` 載入模型後注入 core。到這一步，core 才真正做到不依賴
   任何語言模型。
5. 用 `cargo tree -p jc-tsf --target x86_64-pc-windows-msvc -i candle-core`
   與 `cargo tree -p jc-settings-ui -i candle-core` 驗證 client／UI 不再命中
   candle。再量 `jc-tsf.dll` 大小與 clean build 時間。

**`jc-model` 該消失、改名，還是保留成薄 runtime？**

**`jc-model` 應該消失。** 功能上留一個薄的 `jc-infer`，但這不是把
`jc-model` 改名後繼續放語言程式。`jc-infer` 只處理 GGUF／safetensors 載入、
量化矩陣乘法、張量與錯誤型別，不知道 `zh-select`、`ja-rerank`、Jinen prompt
或 SentencePiece 是哪個語言。若切割後 `jc-infer` 只剩兩個 trait 與型別別名，
可以讓 `jc-zh`、`jc-ja` 直接依賴 candle，不要為了對稱保留空層；但只要還有
共用的 GGUF loader 或量化 matmul，就應該留著，避免兩邊複製貼上。

「`jc-core` 不得依賴任何語言模型，由 `jc-engine` 組裝」這個方向正確，但現況
尚未成立：`jc-core` 的 `zh/select.rs` 直接建構 `ZhSelect`／`QuantZhSelect`，
`ja/rerank.rs` 直接建構 `Reranker` 與 `ModelSet`。只有 Jinen 透過
`NeuralEngine` 注入。完成 G 的第 4 步後，core 只留 trait，模型載入與模型集合
都回到 `jc-engine`。

**風險**

中到高。`candle/mod.rs` 與 `candle/quant.rs` 同時混有共用 primitive 與日文
reranker，切割時最容易把 trait 與具體型別一起搬錯。`jc-core` 目前直接載入
zh-select 與 ja-rerank，改成 trait 注入會動到 `Session` 初始化與
`jc-engine::loader`。先用現有 golden 與模型的 smoke test 鎖行為，再分段搬。
模型檔案格式與 manifest 不動，降低 wire 相容風險。

### H. `jc-cli` 的產品與開發 bin 分家（合併／拆分，P3）

**動機**

`jc-cli` 同時是安裝包裡的 `jc-cli.exe`，也是六個開發 bin 的家。`jc-testdata`
因此放在正常依賴，`-p jc-cli` 也會連開發 bin 一起建置。先確認 `jc-cli.exe`
是不是產品：目前 installer 只在建置期用它產生與驗證 manifest，安裝後沒有
程式呼叫它。

**影響檔案與 CI／installer**

- `crates/jc-cli/Cargo.toml`、`src/bin/*`、`src/lib.rs` 與 `cli.rs` 的子命令。
- 若停止出貨：`installer/jc.iss`、`installer/build.ps1`、簽章清單、
  `scripts/gen-rust-notices.py` 的 `SHIPPED`、`README.md` 與 installer 文件。
- 若保留出貨但拆開發 bin：新增 `jc-dev` 套件、CI 的 `-p` 清單、授權清單。
- `scripts/build-data-bundle.sh`、`scripts/verify-bundle.py` 與
  `scripts/ja-size/*` 會受影響。

**遷移步驟**

1. 先確認產品需求。若 `jc-cli.exe` 不是產品，從 installer 移除，保留
   `jc-cli` 為開發工具，建置期的 manifest／verify 改用 `cargo run -p jc-cli`。
2. 若必須出貨，將 `ja-eval`、`ja-cost`、`jinen-eval`、`jinen-gate`、
   `zh-calibrate`、`candle-zh-check` 與 `eval`／`convert`／`bench` 搬到
   `jc-dev`，`jc-cli` 只留 `data` 子命令。
3. `jc-testdata` 移到 `jc-dev` 或 dev-dependency；`jc-eval` 跟著搬到 `jc-dev`。
4. 更新 installer 的 `--bin` 清單與 CI，重新產生授權清單。

**風險**

中。installer 建置流程依賴 `jc-cli data manifest/verify`，搬動時要保留這條
路徑。`jc-cli` 的評測子命令與 `jc-eval` 綁得很緊，拆包後要重新確認匯入與
測試範圍。

## 暫不動

| 項目 | 判斷 | 理由 | 何時再看 |
|---|---|---|---|
| `jc-engine-launcher` 併入 `jc-engine` | 不合併 | 啟動器是 windows subsystem，只有 `windows-sys`；合併會讓它連帶依賴整個引擎圖，也增加 subsystem 誤改風險 | 只有當啟動器邏輯大幅增加，才重新評估 |
| `jc-core` 與 `jc-engine` 再拆 | 暫不拆 | `Session`、候選、個人化與解碼共用大量不變量；拆成多個 crate 會增加介面與建置單元，收益未量測 | 等語言與模型 crate 穩定後，再量測 core 的建置成本 |
| `jc-data` 與 `jc-testdata` 合併 | 不合併 | 一個是執行期格式，一個是測試資料定位；出貨與壽命不同 | 不考慮 |
| `jc-settings` 與 `jc-settings-ui` 合併 | 不合併 | 資料模型與畫面分離是正確邊界；問題在 settings 對 core 的依賴 | E 完成後即回到乾淨邊界 |
| `jc-tsf` 改用 `jc-transport` | 不合併 transport | TSF 需要 overlapped I/O、SDDL、spawn、1 秒分段逾時與宿主執行緒限制；`jc-transport` 的 interprocess 設計不適合直接取代 | 只共用 pipe 名稱規則，見下一列 |
| pipe 名稱規則共用 | 先修行為，暫不新增 crate | 目前只有 `jc-tsf` 與 `jc-transport` 兩份；先把 registry 覆寫補進 `jc-tsf`，加契約測試，再決定是否抽 leaf | 出現第三個獨立消費端時再抽 |
| `jc-platform` | 暫不動，但不可保持現狀 | 目前沒有依賴者，文件卻說核心透過它互動。要嘛接上 `Session` 與 wrapper，要嘛標成 experimental，下一次整理時刪除 | 下一個平台 adapter（Fcitx5／macOS）動工前決定 |
| `jc-eval` 併入 `jc-cli` | 暫不併 | 只有一個消費者，但它是沒有 core 依賴的乾淨 lib，測試隔離有用 | H 執行時一起評估 |
| `jc-jadic`／`jc-englishpack` | 保持獨立 | 兩者是資料建置工具，不進安裝包；`jc-englishpack` 的 core 依賴只在建置期 | 若資料建置時間成為瓶頸，再處理 core 依賴 |
| `jc-tsf-register` | 保持獨立 | 註冊邏輯與 COM 匯出分開，避免 DLL 符號重複；依賴只有 `windows` | 不考慮合併 |

## 建議順序

| 順序 | 項目 | 級別 | 說明 |
|---|---|---|---|
| 1 | 量測現況 | P1，先做 | 記錄 `jc-tsf.dll`、`jc-settings-ui.exe` 大小、`cargo clean` 後 `-p jc-tsf` 與 `-p jc-settings-ui` 的建置時間，以及 `cargo tree` 的 candle 路徑。沒有這些數字就不做大型拆分 |
| 2 | 修 `jc-tsf` 的 pipe registry 覆寫 | P1 | 只動 `jc-tsf` 與測試，補上 `HKCU\Software\jc-ime\EnginePipe`，加一個與 `jc-transport` 規則對照的測試。這是目前會讓按鍵失效的行為問題 |
| 3 | A：`jc-windows` 改名 `jc-transport` | P1 | 機械式改名，單獨 commit，先讓名稱與文件一致 |
| 4 | C：平台 adapter 命名，刪除 `jc-linux`／`jc-mac` | P1 | 沒有依賴者，與改名一起做可減少文件來回；未來只建立有實作的 adapter |
| 5 | D：切斷 proto 對 core 的依賴 | P2 | 先做 feature 方案或先做 `jc-wire`，依第 1 步的量測決定。若沒有明顯收益就停在文件標註，不硬做 |
| 6 | E：抽出 `jc-input-rules` | P2 | 與 D 分開，避免兩個大相依變更同時進行 |
| 7 | G：`jc-model` 拆成 `jc-infer` 加語言模型 | P2 | 與 F 的第一階段一起排，但分 PR。先搬共用層與日文／中文模型，core 依賴暫時保留 |
| 8 | B：`jc-tsf-smoke` 改名 `jc-tsf-doctor` | P2 | 動到 installer、CI 與簽章，排在安裝流程穩定時做 |
| 9 | F：語言 crate 完整對稱化 | P3 | 先完成語言引擎 P2，再讓 core 移除對 `jc-zh`／`jc-ja` 的依賴；`jc-en` 最後 |
| 10 | H：`jc-cli` 產品與開發 bin 分家 | P3 | 先確認 `jc-cli.exe` 是否為產品；不確定就維持現狀，只補文件 |
| 11 | `jc-platform` 與文件同步 | P3 | 決定接上或標 experimental；順手修正 20 個成員、README 表與 `client-crates.md` 的 `NamedPipe` 描述 |

P1 的 1 到 4 可以分成三個 commit：量測紀錄、pipe 行為修正、改名加刪空殼。
P2 的 5 到 8 各自獨立，不要同一個 PR。P3 的 9 到 11 可以等下一次文件整理，
其中 F 的 runtime 搬移必須等 G 的 core 依賴移除完成。

## 不要做的事

- 不要保留 `jc-model` 當跨語言模型層。它的名字會讓下一個人再把中文或日文
  模型放回去；共用層改名 `jc-infer`，語言模型搬到語言 crate。
- 不要只把 `jc-model` 改名後原封不動留著。改名沒有解決 `Role::ZhSelect`、
  Jinen、注音選字與 reranker 混在一起的事實。
- 不要為了 crate 數量好看，把 `jc-core` 拆成 `zh`、`ja`、`mixed`、`neural`
  多個 crate 後就宣稱 core 語言無關。`Session` 與 `mixed` 還直接依賴語言
  事件，必須先完成語言引擎 P2。
- 不要建立空的 `jc-zh` 或 `jc-zh-pack` 只為了對稱。先搬真的程式，再建 crate。
- 不要強制把 `jc-tsf` 的 IPC 換成 `jc-transport`。overlapped I/O、SDDL、
  spawn 與分段逾時是 TSF 宿主需求，兩者只該共用名稱規則。
- 不要為了少一個 crate 把 `jc-engine-launcher` 併進 `jc-engine`。GUI
  subsystem 與相依隔離有實際用途。
- 不要合併 `jc-data` 與 `jc-testdata`。執行期格式與測試資料定位的壽命、
  出貨條件都不同。
- 不要把 `jc-proto` 與 `jc-platform` 合成一個 `jc-contract`。線格式版本與
  宿主 trait 的演進速度不同，合併會讓 wrapper 為了 host 變更重建 wire。
- 不要趁改名修改 wire JSON、pipe 名稱、執行檔名稱或 CLI 參數。套件改名與
  執行期契約要分開。
- 不要承諾未量測的建置時間或 DLL 大小。所有預期收益都要先記錄基準。
- 不要複製 GGUF loader 或量化矩陣乘法到 `jc-zh`、`jc-ja`。若共用層真的只剩
  型別別名，再讓語言 crate 直接依賴 candle；在那之前用 `jc-infer`。
- 不要在 wrapper 或 transport 引入 async runtime 來合併實作。目前同步模型
  已經能滿足 TSF 的逾時需求，換 runtime 會增加 DLL 與執行緒風險。

## 待量測清單

| 項目 | 用途 | 狀態 |
|---|---|---|
| `jc-tsf.dll` x64／x86 大小 | 判斷 D 與 G 是否值得 | 待量測 |
| `jc-settings-ui.exe` 大小 | 判斷 E 的收益 | 待量測 |
| `cargo clean` 後 `-p jc-tsf` clean build | 判斷 D 與 G 的建置收益 | 待量測 |
| `cargo clean` 後 `-p jc-settings-ui` clean build | 判斷 E 的建置收益 | 待量測 |
| `jc-infer` 切割後的行數與依賴 | 判斷薄 runtime 是否值得保留 | 待量測 |
| `jc-zh`／`jc-ja` 拆出後的 build 時間 | 判斷語言 crate 是否讓建置變慢 | 待量測 |
| installer 單次多 package 建置的 feature 聯集 | 驗證 D 的 feature 方案在 installer 是否仍會拉進 candle | 待量測 |
| `jc-cli.exe` 安裝後的使用情境 | 決定 H 是否停止出貨 | 待確認 |
