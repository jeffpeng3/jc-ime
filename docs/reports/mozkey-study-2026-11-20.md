# Mozkey 的 Zenz 整合：可照抄的規格

- 日期：2026-11-20
- 研究對象：`koyasi777/mozkey`，shallow clone 到 `/mnt/work/jc/ref/mozkey`，HEAD `4892563d6ef01b472733026e4d199eeeafd7f6c5`（2026-11-19）
- 模型：`jc-ime-data/models/external/zenz-v3.2-xsmall/ggml-model-Q5_K_M.gguf`（Q5_K_M，20,970,304 bytes）
- 本文的 `檔案:行` 全部相對 `/mnt/work/jc/ref/mozkey`；`docs/reports/*` 相對本 worktree；另外標註的絕對路徑除外。
- 邊界：未修改 `jc-ime`，未把 Mozkey 程式碼複製進本專案。第 6 節的推論在 `/tmp/opencode` 的 llama.cpp 上執行。

## 0. 結論先講

1. prompt 是 Zenzai v3.2 的條件欄位格式，不是自然語言指令：
   `U+EE02 左文脈 [U+EE07 右文脈] [U+EE03 profile] [U+EE04 topic] [U+EE05 style] [U+EE06 settings] U+EE00 片假名讀音 U+EE01`
   （`src/session/zenz_prompt_builder.cc:11`、`:224`）
2. scorer 的 pipe 協定不是 JSON，是 native-endian packed struct 加原始 prompt bytes；JSON 只出現在 scorer 對 llama-server 的 HTTP 請求。
   （`src/zenz/zenz_wire_protocol.h:16`、`:25`、`src/zenz_scorer/main.cc:1196`）
3. 推論是確定性的：`temperature=0.0`、`top_k=1`、`top_p=1.0`、`n_predict=64`、`cache_prompt=true`，停止符是 U+EE00..U+EE0F 與 CR/LF。
   （`src/zenz_scorer/main.cc:1204`、`:1190`、`:1209`）
4. 五條採用條件的預設狀態不同：讀音保持預設 OFF，context echo、首尾截斷預設 ON，絵文字抑制預設 ON，自動封鎖預設 ON。
5. zenz-v3.2-xsmall 用這份 prompt 可以跑：AJIMEE 前 20 題 exact match 10/20，暖機後 p50 24.9 ms、最大 74.1 ms（GB10，4 threads）。
6. 最短可行情境：統計轉換先出結果，Zenz 只做整句候選，通過長度、截斷、echo、絵文字檢查才替換，失敗一律回統計結果。不需要 feedback 學習、不需要 strict reading、不需要 Mozkey 的 pipe 協定。

## 1. 架構圖

### 1.1 Windows

```text
┌──────────────────────┐   named pipe                    ┌─────────────────────────┐
│ mozc_server.exe      │   \\.\pipe\mozc_zenz_scorer     │ mozc_zenz_scorer.exe    │
│  (TSF / IME server)  │ ─────────────────────────────▶  │  pipe server + HTTP     │
│                      │   24B header + prompt           │  client                 │
│ Session / Validator  │ ◀─────────────────────────────  │                         │
└──────────┬───────────┘   28B header + value + debug    └───────────┬─────────────┘
           │                                                         │
           │ SessionCommand callback (Delay 700 ms)                  │ POST /completion
           │ generation / key 檢查                                    │ 127.0.0.1:<臨時 port>
           ▼                                                         ▼
      Mozc 統計轉換結果                                     llama-server.exe
      （永遠先算好，當 fallback）                            （--parallel 1 --cache-ram 0）
                                                                     │
                                                                     ▼
                                                          zenz-v3.2-small-Q5_K_M.gguf
```

- `mozc_server` 透過 named pipe 呼叫 scorer：`README.md:393`、`:1428`。
- pipe 名預設 `\\.\pipe\mozc_zenz_scorer`：`src/zenz_scorer/main.cc:57`、`src/protocol/config.proto:381`。
- 客戶端會驗證 pipe server 的 PID、session、SID 與執行檔路徑，失敗就不送 prompt：`src/session/zenz_named_pipe_client.cc:325`、`:665`。
- scorer 啟動 llama-server 的參數：`-m <model> -c 256 -t 4 --parallel 1 --cache-ram 0 --host 127.0.0.1 --port <ephemeral> --api-key <32 bytes>`（`src/zenz_scorer/main.cc:1065`、`:1077`、`:1082`）。
- scorer 不存在時由 client 自動啟動，2 秒內不重試：`src/session/zenz_named_pipe_client.cc:389`。

### 1.2 macOS

```text
MozcConverter ──unix socket──▶ mozc_zenz_scorer ──spawn──▶ mozc_zenz_guard ──▶ llama-server
               /tmp/mozc_zenz_<euid>/scorer.sock
```

- socket 路徑：`src/zenz/zenz_unix_socket_path.cc:15`、`:23`。
- client 依平台建立 Unix socket client，並負責啟動 scorer：`src/session/zenz_client_factory.cc:102`、`:105`。
- macOS 多一層 `mozc_zenz_guard` 監督 llama-server 生命週期：`src/zenz_scorer/main_posix.cc:72`、`src/zenz_scorer/llama_server_guard.cc:28`。
- 兩個平台的 scorer 共用同一份 wire protocol 與 prompt 格式，差別只在傳輸層：`src/zenz/zenz_wire_protocol.h:10`、`src/zenz_scorer/posix_server.cc:436`。

### 1.3 Linux

Mozkey 沒有 Linux 正式路徑。`CreateZenzClient()` 在非 Apple 平台回傳 Windows named pipe client，而該檔主體包在 `#if defined(_WIN32)`：`src/session/zenz_client_factory.cc:106`、`src/session/zenz_named_pipe_client.cc:770`。我們要移植時不能直接重用這層。

## 2. Prompt 規格（逐字）

### 2.1 特殊 token

| 常數 | 碼位 | UTF-8 bytes | 用途 |
|---|---|---|---|
| `kZenzReadingBegin` | U+EE00 | `EE B8 80` | 讀音開始 |
| `kZenzOutputBegin` | U+EE01 | `EE B8 81` | 輸出開始，也是 prompt 結尾 |
| `kZenzContextBegin` | U+EE02 | `EE B8 82` | 左文脈開始 |
| `kZenzProfileBegin` | U+EE03 | `EE B8 83` | 寫手／用途 |
| `kZenzTopicBegin` | U+EE04 | `EE B8 84` | 話題 |
| `kZenzStyleBegin` | U+EE05 | `EE B8 85` | 文體 |
| `kZenzSettingsBegin` | U+EE06 | `EE B8 86` | 轉換方針 |
| `kZenzRightContextBegin` | U+EE07 | `EE B8 87` | 右文脈開始 |

出處：`src/session/zenz_prompt_builder.cc:11` 到 `:18`。

### 2.2 組裝順序

實作在 `src/session/zenz_prompt_builder.cc:190` 到 `:251`，順序固定：

```text
U+EE02  left_context
[U+EE07 right_context]      right 非空才加
[U+EE03 profile]            profile 非空才加
[U+EE04 topic]              topic 非空才加
[U+EE05 style]              style 非空才加
[U+EE06 settings]           settings 非空才加
U+EE00  katakana(reading)
U+EE01
```

重點：

- 左文脈一定有 `U+EE02`，即使內容是空字串。`:224` 到 `:225`。
- 右文脈插在左文脈之後、profile 之前，不是放在讀音後面。`:226` 到 `:229`。
- 結束符就是 `U+EE01`，沒有 BOS/EOS 字串：`:248`。
- `profile`、`topic`、`style`、`settings` 各自最多 64 個 Unicode 字元，超過截斷：`:20`、`:197` 到 `:204`。

### 2.3 讀音轉換

- 讀音是使用者當下的 composition key，通常是平假名，不是 Mozc 轉換後的読み：`src/session/zenz_live_corrector.h:23` 到 `:24`、`src/session/session.cc:7345`。
- 只把 U+3041..U+3096 的平假名加 0x60 轉成片假名。長音 `ー`、標點、英數保持原樣：`src/session/zenz_prompt_builder.cc:171` 到 `:174`。
- 例：`てんきがいいです` → `テンキガイイデス`。

### 2.4 文字消毒

`SanitizeConditionText()`（`src/session/zenz_prompt_builder.cc:116` 到 `:155`）：

- 丟棄 U+EE00..U+EE0F，避免使用者文字偽造控制 token（`:108`、`:134`）。
- 丟棄其他 C0 控制字元與 DEL；TAB/LF/CR 換成一個空白（`:112`、`:138` 到 `:144`）。
- 無效 UTF-8 直接停止解析（`:130` 到 `:132`）。

### 2.5 保護片段與 placeholder

- 使用者字典詞、ASCII 技術詞、數字，會在 prompt 的讀音側換成 `__MOZC_ZENZ_PROTECTED_<n>__`，模型回傳後再還原：`src/session/zenz_adoption_policy.cc:507`、`src/session/session.cc:7320`、`:7845`。
- 已驗證的姓名走另一條代理：把「見上愛」換成「木村」再還原，並要求代理只出現一次、不得自行加敬稱：`src/session/session.cc:7823` 到 `:7843`。
- 這是 Mozkey 為了保護表記的加值層，不是 zenz 模型本身的要求。最小移植可以不做。

### 2.6 大小上限

| 限制 | 值 | 出處 |
|---|---|---|
| prompt bytes | 8192 | `src/zenz_scorer/main.cc:77`、`src/session/zenz_named_pipe_client.cc:37` |
| 輸出字元 | 256 | `src/zenz_scorer/main.cc:78` |
| 左文脈 | 預設 24 字元，最多 128 | `src/protocol/config.proto:373`、`src/session/zenz_context_request.cc:15` |
| 右文脈 | 預設 6 字元，最多 128 | `src/protocol/config.proto:456`、`src/session/session.cc:277` |
| 讀音 key | 64 字元 | `src/session/session.cc:1226` |
| Mozc 候選值 | 128 字元 | `src/session/session.cc:1227` |

### 2.7 可貼進程式的樣板

```text
prompt = "\uEE02" + left_context
       + (right_context.empty() ? "" : "\uEE07" + right_context)
       + (profile.empty()       ? "" : "\uEE03" + profile)
       + (topic.empty()         ? "" : "\uEE04" + topic)
       + (style.empty()         ? "" : "\uEE05" + style)
       + (settings.empty()      ? "" : "\uEE06" + settings)
       + "\uEE00" + hiragana_to_katakana(reading)
       + "\uEE01";
```

### 2.8 左右文脈怎麼來

- 專用欄位 `zenz_preceding_text` / `zenz_following_text`，與 Mozc 一般 `preceding_text` / `following_text` 分離：`src/protocol/commands.proto:522`、`:525`、`:534` 到 `:535`。
- 客戶端只在 server 要求的方向與長度取字：`src/protocol/commands.proto:1536` 到 `:1537`、`src/session/zenz_context_request.cc:32` 到 `:45`。
- 專用欄位未設定時才退回一般欄位；明確設成空字串代表刻意不給，例如隱私邊界：`src/session/zenz_client_context.cc:10` 到 `:20`、`src/protocol/commands.proto:530`。
- 左文脈先切到「目前段落」再取尾端 24 字元：`src/session/zenz_context_selector.cc:246` 到 `:259`。
- 右文脈遇到換行、句末 `。！？!?` 或關閉括號就停止，不跨句：`src/session/zenz_context_selector.cc:164` 到 `:177`、`:268` 到 `:300`。
- 選完才做隱私與語言檢查；敏感樣式（URL、email、路徑、token、長數字）整段丟棄：`src/session/zenz_context_sanitizer.cc:52` 到 `:73`。
- 不含日文字元的文脈丟棄：`src/session/zenz_context_sanitizer.cc:88` 到 `:95`。

## 3. 協定規格

### 3.1 mozc_server ↔ mozc_zenz_scorer（pipe / unix socket）

請求是 packed、native-endian 的標頭加 payload，沒有 JSON、沒有長度前綴以外的欄位：`src/zenz/zenz_wire_protocol.h:12` 到 `:16`。

| 方向 | 欄位 | 型別 | bytes | 說明 |
|---|---|---|---|---|
| 請求 | `magic` | u32 | 4 | `0x315A4E5A`，ASCII `ZNZ1` |
| 請求 | `version` | u16 | 2 | 1 |
| 請求 | `kind` | u16 | 2 | 1 = request |
| 請求 | `generation` | u32 | 4 | 每次輸入遞增，用來丟棄過期結果 |
| 請求 | `timeout_msec` | u32 | 4 | 推論預算，0 視為 5000 |
| 請求 | `max_output_chars` | u32 | 4 | 輸出上限，0 視為 256 |
| 請求 | `prompt_size` | u32 | 4 | 後接的 prompt byte 數 |
| 回應 | `magic` | u32 | 4 | 同上 |
| 回應 | `version` | u16 | 2 | 1 |
| 回應 | `kind` | u16 | 2 | 2 = response |
| 回應 | `generation` | u32 | 4 | 必須等於請求值 |
| 回應 | `status` | u32 | 4 | 0 ok、1 error、2 timeout |
| 回應 | `latency_msec` | u32 | 4 | scorer 端量測 |
| 回應 | `value_size` | u32 | 4 | 後接的 UTF-8 輸出 byte 數 |
| 回應 | `debug_size` | u32 | 4 | 後接的 debug byte 數 |

- 標頭大小由 static assert 固定成 24 / 28 bytes：`src/zenz/zenz_wire_protocol.h:48`、`:49`。
- 請求送法：先寫 24B 標頭，再寫 prompt bytes：`src/session/zenz_named_pipe_client.cc:671` 到 `:687`。
- 回應讀法：先讀 28B 標頭，驗證 magic/version/kind/generation，再讀 value 與 debug：`src/session/zenz_named_pipe_client.cc:694` 到 `:730`。
- 錯誤與逾時用 `status` 回傳，client 不解析文字：`src/zenz/zenz_wire_protocol.h:21` 到 `:23`。
- scorer 端把 timeout 夾在 50..5000 ms、輸出夾在 1..256 字元：`src/zenz_scorer/main.cc:1638` 到 `:1653`。
- 同一時間只有一個 pipe instance，遠端連線被拒：`src/zenz_scorer/main.cc:1745` 到 `:1750`。

### 3.2 mozc_zenz_scorer ↔ llama-server（HTTP）

`POST /completion`，JSON body 逐字如下（`src/zenz_scorer/main.cc:1196` 到 `:1229`；macOS 同格式 `src/zenz_scorer/llama_http_client.cc:692` 到 `:710`）：

| 欄位 | 值 | 出處 |
|---|---|---|
| `prompt` | 第 2 節的完整字串，JSON escape | `src/zenz_scorer/main.cc:1198` |
| `n_predict` | `max(4, min(64, max_output_chars))`，預設 64 | `src/zenz_scorer/main.cc:1190` 到 `:1194` |
| `temperature` | 0.0 | `src/zenz_scorer/main.cc:1204` |
| `top_k` | 1 | `src/zenz_scorer/main.cc:1205` |
| `top_p` | 1.0 | `src/zenz_scorer/main.cc:1206` |
| `stream` | false | `src/zenz_scorer/main.cc:1207` |
| `cache_prompt` | true | `src/zenz_scorer/main.cc:1208` |
| `stop` | U+EE00..U+EE0F、`\n`、`\r` | `src/zenz_scorer/main.cc:1209` 到 `:1228` |

- 回應只取 JSON 的 `content` 字串，再切掉停止符與 `<s>`、`</s>`、`<unk>`、`<|endoftext|>`，trim ASCII 空白，截到 256 字元：`src/zenz_scorer/main.cc:536` 到 `:577`、`:1309` 到 `:1318`。
- 空輸出視為失敗，回 error 給 Mozc：`src/zenz_scorer/main.cc:1316`。
- HTTP 標頭帶 `Authorization: Bearer <api-key>`，金鑰只為避免誤連本機其他服務：`src/zenz_scorer/main.cc:1231` 到 `:1234`、`:1085`。

### 3.3 生命週期與失敗狀態

| 狀態 | 行為 | 出處 |
|---|---|---|
| scorer 未啟動 | client 自動 spawn，2 秒節流 | `src/session/zenz_named_pipe_client.cc:389` |
| pipe 連不上 | 冷啟動重試間隔 50 ms，受 deadline 限制 | `src/session/zenz_named_pipe_client.cc:489` 到 `:500` |
| scorer busy | 重試 3 次、每次 20 ms | `src/session/zenz_named_pipe_client.cc:547` 到 `:550` |
| llama-server 未 ready | 每 500 ms 探測，最多 120 次；至少等 1500 ms | `src/zenz_scorer/main.cc:67`、`:68`、`:73` |
| llama-server 掛掉 | 回 `server_exited`，背景重啟，下一次請求再試 | `src/zenz_scorer/main.cc:1688` 到 `:1695` |
| 推論逾時 | 回 status=2，Mozc 顯示既有結果 | `src/zenz_scorer/main.cc:1668` |

## 4. 參數表

### 4.1 模型與 llama-server

| 參數 | Mozkey 值 | 出處 |
|---|---|---|
| 模型 | `zenz-v3.2-small-Q5_K_M.gguf` | `src/zenz_scorer/main.cc:296`、`src/zenz_scorer/main_posix.cc:21` |
| context size | 256 | `src/zenz_scorer/main.cc:63`、`src/zenz_scorer/main_posix.cc:17` |
| threads | 4 | `src/zenz_scorer/main.cc:64`、`src/zenz_scorer/main_posix.cc:18` |
| `--parallel` | 1 | `src/zenz_scorer/main.cc:1077` |
| `--cache-ram` | 0，關閉全域 prompt 快取 | `src/zenz_scorer/main.cc:1082` |
| `cache_prompt` | true，保留 request 內前綴重用 | `src/zenz_scorer/main.cc:1208` |
| n_predict 預設／上限 | 64／256（環境可調 4..256） | `src/zenz_scorer/main.cc:65`、`:89`、`:311` |
| context 上限 | 1024（環境可調 64..1024） | `src/zenz_scorer/main.cc:87`、`:303` |
| threads 上限 | 16（環境可調 1..16） | `src/zenz_scorer/main.cc:88`、`:307` |
| llama-server 版本 | v0.7.0，commit `620d2225` | `src/zenz_runtime/llama_runtime.lock.json:24` 到 `:25`（Windows）、`:47` 到 `:48`（macOS） |
| tokenizer patch | 把 `gpt2-small-japanese-char` 映射到 GPT2 pre-tokenizer | `src/mac/installer/zenz_runtime/patches/llama-vocab-gpt2-small-japanese-char.patch:9` |

### 4.2 時間參數

| 參數 | 值 | 出處 | 備註 |
|---|---|---|---|
| 自動補正 debounce | 700 ms | `src/protocol/config.proto:349`、`src/session/session.cc:270` | 自動路徑才等；手動 `forced` 為 0（`src/session/session.cc:7374`） |
| 請求 timeout | 180 ms | `src/protocol/config.proto:366`、`src/session/session.cc:271` | 上限 1000 ms（`src/session/session.cc:279`） |
| 結果輪詢間隔 | 24 ms | `src/session/session.cc:272` | IME thread 不阻塞，callback 輪詢 |
| deferred 顯示上限 | 250 ms | `src/protocol/config.proto:362`、`src/gui/config_dialog/config_dialog.cc:1123`、`README.md:391` | 只算「已送出請求」之後 |
| deferred runtime 預設 | 300 ms | `src/session/session.cc:274` | 欄位未設定時的程式 fallback，與 UI/proto 的 250 不同，見 4.5 |
| deferred 可調範圍 | 50..3000 ms | `src/session/session.cc:275` 到 `:276` | |
| 非同步等待上限 | 3000 ms | `src/session/session.cc:286`、`:7746` | 涵蓋冷啟動；不是模型 timeout |
| debounce 上限 | 5000 ms | `src/session/session.cc:278` | |
| 最小觸發字數 | 2 字元 | `src/protocol/config.proto:369`、`src/session/session.cc:273` | 程式再夾成 >= 2（`src/session/session.cc:1155`）；手動可繞過（`src/session/session.cc:7286`） |
| feedback 最短自動套用 | 7 字元 | `src/protocol/config.proto:422`、`src/config/config_handler.cc:347` | 只影響 feedback 自動套用 |
| pipe connect 安全期限 | 1000 ms | `src/zenz_scorer/main.cc:80` | |
| pipe I/O 安全期限 | 1500 + 5000 + 1500 = 8000 ms | `src/zenz_scorer/main.cc:81` 到 `:82` | |
| pipe flush 期限 | 1000 ms | `src/zenz_scorer/main.cc:83` | |
| HTTP timeout | 預設 5000 ms，夾在 50..30000 | `src/zenz_scorer/llama_http_client.cc:33` 到 `:35` | scorer 事件迴圈用 |
| guard 輪詢 | 50 ms | `src/zenz_scorer/llama_server_guard.cc:34` | |
| guard SIGKILL 寬限 | 40 × 50 ms = 2 s | `src/zenz_scorer/llama_server_guard.cc:35` | |
| scorer 啟動節流 | Windows 2000 ms，macOS 2 s | `src/session/zenz_named_pipe_client.cc:389`、`src/session/zenz_client_factory.cc:62` | |

### 4.3 文脈

| 參數 | 值 | 出處 |
|---|---|---|
| 左文脈預設 | 24 字元 | `src/protocol/config.proto:373` |
| 右文脈預設 | 6 字元 | `src/protocol/config.proto:456` 到 `:457` |
| 右文脈開關 | 產品預設 ON | `src/config/config_handler.cc:353` 到 `:354` |
| 平台擷取上限 | 128 字元／方向 | `src/session/zenz_context_request.cc:15` |
| 隱私檢查 | URL、email、路徑、secret、長數字丟棄 | `src/session/zenz_context_sanitizer.cc:61` 到 `:73` |
| 語言檢查 | 必須有日文 script 訊號 | `src/session/zenz_context_sanitizer.cc:88` 到 `:95` |

### 4.4 驗證門檻

| 參數 | 值 | 出處 |
|---|---|---|
| 輸出長度上限 | `key_len * 3 + 8` 字元 | `src/session/zenz_output_validator.cc:957` |
| context echo 最小重疊 | 4 字元 | `src/session/zenz_output_validator.cc:466` |
| echo 判定 | 重疊 >= baseline×2+1，或 value >= echo+baseline | `src/session/zenz_output_validator.cc:487` 到 `:489` |
| 絵文字 | 有序 cluster 序列必須完全相同 | `src/session/zenz_output_validator.cc:884` 到 `:893` |
| 自動封鎖最低拒絕次數 | 1 | `src/protocol/config.proto:435` |
| 自動封鎖最低拒絕比例 | 50% | `src/protocol/config.proto:442` 到 `:443` |
| 自動封鎖預設 | ON | `src/config/config_handler.cc:350` 到 `:352` |

### 4.5 兩個預設值不一致，照抄時要選一個

- `config.proto` 與 GUI 說 deferred 上限 250 ms：`src/protocol/config.proto:362`、`src/gui/config_dialog/config_dialog.cc:1123`、`README.md:391`。
- 但 session 在欄位「未設定」時回傳 300 ms：`src/session/session.cc:274`、`:1140` 到 `:1149`。
- 也就是全新 profile 尚未存過設定時，runtime 實際上限是 300 ms；使用者按過確定後才寫入 250。移植時建議直接固定 250，並保留 50..3000 的可調範圍。

## 5. 五條採用條件的判定流程

### 5.1 整體順序

```text
輸入變更
  │
  ├─ 自動路徑：password、非日文訊號、key < 2 字 → 不送 Zenz
  │  （incognito 仍可推論，只關閉 feedback 讀寫：src/session/session.cc:1177 到 :1181）
  ├─ 等 700 ms debounce，統計轉換先算好並顯示
  ├─ 送出請求，generation = N
  │
  ▼
回應 generation != N → 丟棄
  │
  ├─ 1. 左文脈前綴歧義 → 回 Mozc
  ├─ 2. 基本驗證（空、等於 Mozc、過長、特殊 token、UTF-8、URL/secret）
  ├─ 3. context echo → 回 Mozc
  ├─ 4. 絵文字序列改變 → 回 Mozc
  ├─ 5. 保護片段與首尾截斷 → 回 Mozc
  ├─ 6. 嚴格讀音保持（預設 OFF）→ 回 Mozc
  └─ 7. feedback 自動封鎖 → 回 Mozc
  │
  ▼
顯示 Zenz 結果；任何一步失敗都顯示已算好的 Mozc 結果
```

主流程：`src/session/session.cc:7707` 到 `:8028`；fallback：`src/session/session.cc:7586` 到 `:7600`。

### 5.2 條件一：讀音保持（strict reading preservation）

- 預設 OFF：`src/protocol/config.proto:411`、`src/config/config_handler.cc:321` 到 `:323`。
- ON 時，把最終 Zenz 候選丟回 Mozc converter 做逆轉換，取得候選讀音：`src/session/session.cc:189` 到 `:228`。
- 判定：候選讀音等於完整輸入 key，或等於正規化後的 key 讀音，才接受；取不到讀音就失敗：`src/session/zenz_output_validator.cc:896` 到 `:911`。
- 呼叫點在採用的最後一步：`src/session/session.cc:8017` 到 `:8028`。
- 拒絕理由：`reading_mismatch`、`reading_unavailable`。
- 注意：這是 opt-in 的保守模式，Mozkey 自己承認可能誤殺正確候選，見 `README.md:421`。

### 5.3 條件二：context echo

- 永遠開啟，沒有設定開關。
- 取左文脈的最長尾端，與 Zenz 輸出的開頭比對：`src/session/zenz_output_validator.cc:429` 到 `:459`。
- 重疊 < 4 字元不算：`:463` 到 `:468`。
- 重疊 >= `max(1, max(key_len, mozc_value_len)) * 2 + 1`，或輸出長度 >= 重疊 + baseline，判為 echo：`:477` 到 `:489`。
- 拒絕理由 `left_context_echo`：`src/session/zenz_output_validator.cc:951` 到 `:953`。
- 另一層保護：輸出真的以左文脈開頭時，只有「剝掉後剛好等於 Mozc 結果」才允許剝；否則整筆回 Mozc：`src/session/zenz_output_validator.cc:622` 到 `:644`、`src/session/session.cc:7806` 到 `:7819`。
- 設計意圖：避免把使用者自己輸入的語句誤刪，也避免模型只回偷來的文脈。

### 5.4 條件三：首尾截斷

- 判定極簡：候選比 Mozc 結果短，且是 Mozc 結果的 prefix 或 suffix，就拒絕：`src/session/zenz_output_validator.cc:614` 到 `:620`。
- 在 adoption 最後、所有修補之後再檢查一次，避免修補把截斷帶回來：`src/session/zenz_adoption_policy.cc:1364` 到 `:1374`。
- 拒絕理由 `obvious_surface_truncation`。
- 同一條路徑也套用在 feedback 快取候選：`src/session/session.cc:7013` 到 `:7029`。
- 限制：只抓表記上明顯的頭尾缺失，不保證語意完整，`README.md:417` 有明講。

### 5.5 條件四：絵文字抑制

- 預設 ON，設定欄位是反向的 `allow_zenz_emoji_changes=false`：`src/protocol/config.proto:416`、`src/config/config_handler.cc:325` 到 `:327`。
- 把 Mozc baseline 與 Zenz 輸出的絵文字 cluster 依序抽出，序列必須完全相同：`src/session/zenz_output_validator.cc:821`、`:860`、`:884` 到 `:893`。
- cluster 處理 ZWJ、膚色修飾、variation selector、keycap，所以「同一個絵文字換膚色」也算變更：`:821` 到 `:860`。
- 只比絵文字序列，不比位置，所以絵文字前後的日文仍可被 Zenz 修正：`README.md:425`。
- 採用後再驗一次，避免 placeholder 還原或修補繞過：`src/session/session.cc:7976` 到 `:7983`。
- 拒絕理由 `emoji_sequence_changed`、`emoji_sequence_changed_after_adoption`。
- 不影響 Mozc 自己的 `EmojiRewriter` 候選，也不阻止使用者手動選絵文字：`README.md:427`。

### 5.6 條件五：自動封鎖

- 產品預設 ON：`src/config/config_handler.cc:350` 到 `:352`；開關本身在 `src/protocol/config.proto:428`。
- 只在 feedback reuse 開啟、非 incognito、非手動 hard reject 時生效：`src/session/session.cc:8033` 到 `:8055`。
- 統計桶是 exact `(key, context_class, value)`：`src/session/zenz_feedback_store.cc:700`。
- 條件同時成立才封鎖：
  1. `auto_block_rejected >= minimum_reject_count`（預設 1）
  2. `rejected * 100 >= minimum_reject_percentage * (accepted + rejected)`（預設 50%）
  出處：`src/session/zenz_feedback_store.cc:642` 到 `:659`。
- `accepted=1 / rejected=1` 的 50% 也在封鎖內，因為用的是 `>=`：同上。
- 封鎖時 action 變 `kReject`、reason `feedback_auto_blocked`：`src/session/zenz_feedback_store.cc:682` 到 `:698`。
- 被自動封鎖後仍做 shadow observation；若使用者最後確定的值與隱藏的 Zenz 結果相同，會補 accepted，比例降到門檻下就自動解封：`README.md:499`。
- 短讀音（< 7 字元）不參與自動封鎖，但記錄照樣寫：`src/protocol/config.proto:421` 到 `:422`、`README.md:487`。

### 5.7 其他會擋掉結果的條件

| 條件 | 判定 | 出處 |
|---|---|---|
| password field | 不送、不採用 | `src/session/session.cc:7256` |
| 無日文訊號（純羅馬字） | 不送 | `src/session/session.cc:1313` 到 `:1315` |
| key 隱私樣式 | 不送 | `src/session/session.cc:1318` 到 `:1328` |
| 輸出等於 Mozc | 視為無效，不顯示 | `src/session/zenz_output_validator.cc:923` |
| 輸出含特殊 token | 拒絕 | `src/session/zenz_output_validator.cc:931`、`:557` |
| 輸出像 URL/email/secret | 拒絕 | `src/session/zenz_output_validator.cc:961`、`:965` |
| 保護片段消失或邊界改變 | 拒絕或區域修補 | `src/session/zenz_adoption_policy.cc:1260` 到 `:1282` |

## 6. 20 題 AJIMEE 實測

### 6.1 環境與方法

- 題庫：`/mnt/work/jc/jc-ime-data/0c-testdata/ja/ajimee_200.json`，取 `items[0..19]`。每題有 `reading`（平假名）與 `surface`（期望輸出）。
- prompt：完全照第 2 節，左文脈為空，右文脈與 profile/topic/style/settings 都不送。也就是 `U+EE02 U+EE00 片假名(reading) U+EE01`。
- 推論：llama.cpp 自建（master `c406f43`），套用 Mozkey 倉庫內那一行 tokenizer patch，`llama-server -c 256 -t 4 --parallel 1 --cache-ram 0`。沒有編譯或執行 Mozkey 的 scorer。
- 這個 patch 是必要的：未套用時 llama.cpp 直接拒絕載入，錯誤是 `unknown pre-tokenizer type: 'gpt2-small-japanese-char'`。原因是 GGUF metadata 的 `tokenizer.ggml.pre` 是 `gpt2-small-japanese-char`，patch 把它映射到 GPT2 pre-tokenizer。
- 請求參數照 Mozkey：`n_predict=64`、`temperature=0`、`top_k=1`、`top_p=1`、`stop=U+EE00..U+EE0F,\n,\r`、`cache_prompt=true`。
- exact match 定義：輸出字串與 `surface` 完全相同，不做正規化。
- 延遲：Python 端量測整個 HTTP round trip，含 JSON 解析。模型已載入、第一題之前沒有 warmup 請求。

### 6.2 逐字輸入與輸出

無文脈：

```text
prompt   : "\uee02\uee00\u30c6\u30f3\u30ad\u30ac\u30a4\u30a4\u30c7\u30b9\uee01"
可見字元 : [EE02][EE00]テンキガイイデス[EE01]
bytes    : EE B8 82 EE B8 80 E3 83 86 E3 83 B3 E3 82 AD E3 82 AC
           E3 82 A4 E3 82 A4 E3 83 87 E3 82 B9 EE B8 81   （33 bytes）
content  : "天気がいいです"
```

有左右文脈與 profile，證明欄位順序可運作：

```text
prompt   : "\uee02今日は\uee07です\uee00テンキガイイ\uee01"
content  : "天気がいい"
prompt   : "\uee02今日は\uee03丁寧な文章\uee00テンキガイイ\uee01"
content  : "天気がいい"
```

實際送出的 JSON（第一題）：

```json
{"prompt":"\uee02\uee00テンキガイイデス\uee01","n_predict":64,"temperature":0.0,"top_k":1,"top_p":1.0,"stream":false,"cache_prompt":true,"stop":["\uee00","\uee01","\uee02","\uee03","\uee04","\uee05","\uee06","\uee07","\uee08","\uee09","\uee0a","\uee0b","\uee0c","\uee0d","\uee0e","\uee0f","\n","\r"]}
```

llama-server 回傳的 timing（暖機後，同一 prompt 前綴可重用）：

```json
{"cache_n":6,"prompt_n":11,"prompt_ms":3.997,"predicted_n":8,"predicted_ms":6.286,"predicted_per_second":1113.6}
```

### 6.3 結果

| # | id | 讀音 | 期望 | 輸出 | exact（o/x） | ms |
|---|---|---|---|---|---|---|
| 1 | `ajimee-001` | せいねんしでかくことのりゆうとして、だんせいせいねんしにおけるだんせいゆういのせいいしき（ | 青年誌で描くことの理由として、男性青年誌における男性優位の性意識（ | 青年誌で書くことの理由として、男性青年誌における男性優位の正意式（ | x | 91.9 |
| 2 | `ajimee-002` | のいらいがくるように | の依頼が来るように | の依頼が来るように | o | 17.6 |
| 3 | `ajimee-003` | のだんせいかたのどぞうから、むなかたしこうさくひんとおもわれるはんがなど、こっとうひんさんびゃくてんがとうなん | の男性方の土蔵から、棟方志功作品と思われる版画など、骨董品３００点が盗難 | の男性方の土蔵から、宗像師工作品と思われる版画など、骨董品300点が盗難 | x | 50.1 |
| 4 | `ajimee-004` | たとえばちょっこうざひょうへいめんじょうのにんいのてんのげんてんから | 例えば直交座標平面上の任意の点の原点から | 例えば直交座標平面上の任意の点の原点から | o | 27.4 |
| 5 | `ajimee-005` | 、ほんさくもその | 、本作もその | 、本作もその | o | 10.0 |
| 6 | `ajimee-006` | た。」とこじんてきなかんそうをのこしたが、あいんしゅたいんはれきしをせんこうしておらず、このようなかんちがいをした | た。」と個人的な感想を残したが、アインシュタインは歴史を専攻しておらず、このような勘違いをした | た。」と個人的な感想を残したが、アインシュタインは歴史を先行しておらず、このような勘違いをした | x | 53.8 |
| 7 | `ajimee-007` | では、ぎいんのふたいほとっけん | では、議員の不逮捕特権 | では、議員の不逮捕特権 | o | 21.3 |
| 8 | `ajimee-008` | にへんこう、あるいはふくごうせいをせんたくするためにはそのようにこんいんまえにてつづきをおこなわ | に変更、あるいは複合姓を選択するためにはそのように婚姻前に手続きを行わ | に変更、あるいは複合性を選択するためにはそのように婚姻前に手続きを行わ | x | 49.7 |
| 9 | `ajimee-009` | はしんれきはちがつついたちもしくはつきおくれのくがつついたち | は新暦８月１日もしくは月遅れの９月１日 | は新暦8月1日もしくは月遅れの9月1日 | x | 30.8 |
| 10 | `ajimee-010` | みたせばへんしんべるとをてにしたものがだれでもかめんらいだーになれるという、ここまでのへいせいらいだーでつちかわれたいめーじのしゅうたいせいてきなさくひん | 満たせば変身ベルトを手にしたものが誰でも仮面ライダーになれるという、ここまでの平成ライダーで培われたイメージの集大成的な作品 | 満たせば変身ベルトを手にしたものが誰でも仮面ライダーになれるという、ここまでの平成ライダーで培われたイメージの集大成的な作品 | o | 74.1 |
| 11 | `ajimee-011` | たあと、そのままひがしほうこうにのび | た後、そのまま東方向に延び | た後、そのまま東方向に伸び | x | 21.1 |
| 12 | `ajimee-012` | さんねんくがつじゅうごにち、ゆうじんのでにーす・まくねーあがはくじんゆうえつろんしゃによるじゅうろくばんがいせいよはねきょうかいでのばくだんてろ | ３年９月１５日、友人のデニース・マクネーアが白人優越論者による１６番街聖ヨハネ教会での爆弾テロ | 3年9月15日、友人のデニース・マクネーアが白人優越論者による16番街聖ヨハネ教会での爆弾テロ | x | 58.5 |
| 13 | `ajimee-013` | くけいじょうにはいれつした | 矩形状に配列した | 矩形状に配列した | o | 12.6 |
| 14 | `ajimee-014` | のけんきゅうてーまであるたへんすうふくそかんすうろん | の研究テーマである多変数複素関数論 | の研究テーマである多変数複素関数論 | o | 23.1 |
| 15 | `ajimee-015` | をおさめたるーかすはそのしきんをえいぞう・おんきょうかんけいのでじたるかをちゅうしんとしたぎじゅつかいはつにとうしし、えいがかいにただい | を収めたルーカスはその資金を映像・音響関係のデジタル化を中心とした技術開発に投資し、映画界に多大 | を収めたルーカスはその資金を映像・音響関係のデジタル化を中心とした技術開発に投資し、映画界に多大 | o | 60.1 |
| 16 | `ajimee-016` | さぷりめんとのせっしゅなどによりけいぞくてきにかじょうせっしゅしたばあいにはいかのよう | サプリメントの摂取などにより継続的に過剰摂取した場合には以下のよう | サプリメントの摂取などにより継続的に過剰摂取した場合には以下のよう | o | 40.1 |
| 17 | `ajimee-017` | あり（こうじゅつ）、すもうべやのかずはげんしょうけいこう | あり（後述）、相撲部屋の数は減少傾向 | あり（後述）、相撲部屋の数は減少傾向 | o | 20.6 |
| 18 | `ajimee-018` | じょうどしんしゅうじてんが、どれにていしょくするんです | 浄土真宗事典が、どれに抵触するんです | 浄土真宗寺典が、どれに抵触するんです | x | 17.6 |
| 19 | `ajimee-019` | すうせつあるが、 | 数説あるが、 | 数節あるが、 | x | 10.6 |
| 20 | `ajimee-020` | かたいからをもつたまごをうむこと、たかいたいしゃりつ | 堅い殻を持つ卵を産むこと、高い代謝率 | 硬い殻を持つ卵を産むこと、高い代謝率 | x | 24.9 |

統計：

- exact match：10/20（50%）。
- 延遲：全部 mean 35.8 ms、p50 26.1 ms；排除第一題冷啟動後 mean 32.8 ms、p50 24.9 ms、min 10.0 ms、max 74.1 ms。
- 第一題 91.9 ms 是 llama-server 第一次 inference 的 warmup 成本，之後同前綴有 prompt cache。
- 20 題合計產生 524 個 token，讀音合計 708 字元。

錯誤型態：

- 10 題錯，全部都是局部字詞錯誤，沒有整句重寫；多數只差 1 到 2 字：`描/書`、`性意識/正意式`、`専攻/先行`、`姓/性`、`延/伸`、`事典/寺典`、`説/節`、`堅/硬`。
- 3 題是數字全角/半角：`３００` → `300`、`８月１日` → `8月1日`、`１５日` → `15日`。這在 Mozkey 會由 `RestoreUserVisibleSymbolStyle` 與數字保護修掉，純 prompt 不會。
- 沒有任何一題出現整句崩壞、重複、或拒絕停止，代表 prompt 格式與停止符正確。

解讀：

- xsmall 單獨做整句取代，50% exact 不夠。日文統計轉換在 `modern-500` 是 top-1 86.0%（`docs/status.md`）。
- 若要上線，Zenz 應只當第二階段候選或 rerank 訊號，並且一定要保留 Mozc 結果當 fallback。這與 `docs/reports/ja-ime-survey-2026-11-20.md:5` 的結論一致。
- 延遲遠低於 Mozkey 的 180 ms 預算，GB10 上就算用 xsmall 也有很大餘裕；small（74 MB）值得再測。

## 7. 授權結論

| 元件 | 授權 | 出處 | 我們的義務 |
|---|---|---|---|
| Mozkey 程式（Mozc fork） | BSD-3-Clause | `LICENSE:1` 到 `:28` | 抄片段要保留 copyright、三條款與免責；不得用 Google/Mozkey 名義背書 |
| Mozc 詞典資料 | 混合授權 | `THIRD_PARTY_NOTICES.md:27` 到 `:34` | 只抄程式不抄詞典就沒有額外義務；要抄詞典得逐來源處理 |
| zenz-v3.2-xsmall / small 權重 | CC-BY-SA-4.0 | HF API `cardData.license`、`/mnt/work/jc/jc-ime-data/models/external/zenz-v3.2-xsmall/manifest.json:17` | 散布要給 attribution、授權連結、標示修改；改作或再散布權重要同授權 |
| llama.cpp / ggml | MIT | `src/mac/installer/zenz_runtime/licenses/THIRD_PARTY_NOTICES.md:17` 到 `:24` | 保留 MIT 聲明 |
| Zenzai 程式（非 Mozkey） | MIT | `docs/reports/ja-ime-survey-2026-11-20.md:11` | 若另抄 Zenzai 原始碼才需要 |

需要特別記錄的衝突：

- Mozkey 的 macOS notice 寫 zenz-v3.2-small 是 Apache-2.0：`src/mac/installer/zenz_runtime/licenses/zenz-v3.2-small-gguf.txt:6`、`src/mac/installer/zenz_runtime/licenses/THIRD_PARTY_NOTICES.md:11`。
- Hugging Face 上兩個 repo 的 `license` 欄位都是 `cc-by-sa-4.0`，xsmall 與 small 都一樣。
- 我們的 `manifest.json` 也記 CC-BY-SA-4.0，與上游一致。
- 因此不能照抄 Mozkey 的 Apache-2.0 宣告。若我們散布權重，要依 CC-BY-SA-4.0 處理，不能只放 Apache 檔。
- CC-BY-SA 的 ShareAlike 針對「改作」與「散布權重」。模型輸出是否構成改作，法律上沒有定論；保守做法是不要把輸出當成可任意授權的資產，也不要把權重與輸出混在同一個授權宣告裡。

若只抄規格、不抄程式碼、不散布權重：

- prompt 格式、token 順序、驗證規則這類事實與方法，沒有著作權義務。
- 但 Mozkey 的註解與程式結構有 BSD-3 著作權，貼程式片段就要附聲明。

## 8. 我們可以照抄與不該照抄的部分

### 8.1 可以直接照抄的規格

1. prompt token 順序與停止符：第 2 節，`src/session/zenz_prompt_builder.cc:224`。
2. 平假名轉片假名的範圍：`src/session/zenz_prompt_builder.cc:171`。
3. 確定性推論參數：`temperature=0`、`top_k=1`、`top_p=1`、`n_predict=64`：`src/zenz_scorer/main.cc:1190` 到 `:1208`。
4. 先算統計結果、Zenz 只當可替換候選的順序：`src/session/session.cc:7586`、`:7343`。
5. 三條便宜的驗證：等於 Mozc 就丟、首尾截斷就丟、context echo 就丟：`src/session/zenz_output_validator.cc:614`、`:923`、`:951`。
6. 絵文字序列保護：`src/session/zenz_output_validator.cc:884`。
7. 文脈先切段落／句子再取字：`src/session/zenz_context_selector.cc:232`、`:268`。
8. 180 ms 的互動預算與 fallback 優先：`src/protocol/config.proto:366`、`src/session/session.cc:7586`。

### 8.2 不該照抄的部分

| 項目 | 原因 | 出處 |
|---|---|---|
| Windows named pipe + WinHTTP 整套 | 我們沒有 Windows server；直接用 Unix socket 或 HTTP 更短 | `src/zenz_scorer/main.cc:57`、`:1136` |
| 24/28 byte packed wire protocol | 只有同機同平台才安全；跨平台除錯成本高 | `src/zenz/zenz_wire_protocol.h:12` |
| `mozc_zenz_guard` 與 scorer 雙程序 | 為 Windows/macOS 套件生命週期設計，jc-ime 尚無安裝器 | `src/zenz_scorer/llama_server_guard.cc:28` |
| feedback 學習、自動封鎖、TSV | 需要長期使用者資料，第一版沒有價值 | `src/session/zenz_feedback_store.cc:621` |
| strict reading 的逆轉換 | 預設 OFF，且會多一次轉換、可能誤殺 | `src/protocol/config.proto:411` |
| placeholder 保護與姓名代理 | 與 Mozc 的候選／詞典結構綁死，jc-ime 還沒有對應資料 | `src/session/zenz_adoption_policy.cc:507`、`src/session/session.cc:7823` |
| 700 ms debounce 與 deferred 顯示 | 那是 live conversion 的互動設計；jc-ime 目前是單次轉換 | `src/session/session.cc:270`、`:274` |
| Mozkey 的 Apache-2.0 模型宣告 | 與上游 CC-BY-SA-4.0 衝突 | `src/mac/installer/zenz_runtime/licenses/zenz-v3.2-small-gguf.txt:6` |

### 8.3 最短可行情境

前提：`jc-ime` 不改，先在 sidecar 或評測 harness 驗證。最小步子如下。

1. 一個常駐 `llama-server`，載 xsmall Q5，`-c 256 -t 4 --parallel 1 --cache-ram 0`。
2. 統計轉換先產生 top-1 與 top-5，這是 baseline，也是 fallback。
3. 用 top-1 的讀音與已確定的左文脈（最多 24 字元）組 prompt，`temperature=0`、`top_k=1`、`n_predict=64`、停止符 U+EE00..U+EE0F。
4. 只做四項驗證：空/等於 baseline、長度上限、首尾截斷、context echo。絵文字與 strict reading 第二版再加。
5. 通過才替換顯示；任何失敗、逾時、scorer 不在，都顯示 baseline。
6. 量測目標：p95 < 180 ms；目前 xsmall 在 GB10 的 p50 是 25 ms，空間足夠。

這樣不需要 Mozkey 的 pipe 協定、不需要 feedback、不需要 live correction，也能得到它的主要實際效果。真正的品質瓶頸在模型與驗證策略，不在傳輸層。
