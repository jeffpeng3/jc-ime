# live-default：逐字変換、自動変換與片段選字（2026-11-20）

## 結論（2026-11-20 修訂）

**預設改為「延遲整句轉換」（方案 A）。** 打字期間只顯示假名，停止輸入後做一次
完整轉換。`modern-500` 的 top-1 是 **92.2%**，與整句模式完全相同；每題推論
0.21 次，每鍵 p95 0 ms，停頓到轉換完成 p50 2.6 ms／p95 50.4 ms，已顯示文字被
系統改掉的比率 0%。`--no-live` 可以回到原本的手動整句轉換，仍為 92.2%／93.6%。

三個方案在 `modern-500` 的對照：

| 方案 | top-1 | 推論次數／題 | 每鍵 p95 | 每題同步累積 p95 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|
| **A 延遲整句（新預設）** | **92.2%** | 0.21 | 0.00 ms | 0.01 ms | 0% |
| B prefix cache（統計） | 86.0% | 0 | 8.3–12.4 ms | 2.1–6.5 ms | 0%（keep 0） |
| C 邊界觸發＋片段鎖定 | 88.2% | 1.64 | 22.3 ms | 95.4 ms | 0% |

A 的停頓延遲：`modern-500` p50 2.60 ms／p95 50.36 ms；`ajimee-200` 因為句子較長
且標點較多，p50 20.44 ms／p95 147.85 ms；`hard-cases` p50 3.68 ms／p95 36.53 ms。

`ajimee-200` 的 top-1 是 69.0%，和整句模式相同，沒有退步；`hard-cases` 是
77.1%，也和整句模式相同。A 沒有用題庫或門檻調整分數。

A 的缺點是「停頓後已顯示的轉換，繼續打字再停頓重轉」時，`ajimee-200` 有
34.0% 的已穩定片段被系統改掉（整句模式沒有這個問題，因為只轉一次）。單句、
只在句尾停頓的 `modern-500` 與 `hard-cases` 是 0%。長文多停頓的情境仍需
「User 片段固定」與右文脈來壓低這個數字。

## 使用者手動選字的保留率

`User` 片段（手選漢字／假名、F6–F10、拆回讀音）會固定在 `user_segments`；
後續停頓只重轉 Auto 的連續區段，User 片段當固定文字傳給左右文脈。

| 指標 | 結果 |
|---|---:|
| 手動選字保留率（後續停頓重轉） | **100%（1/1）** |
| 指定情境輸出 | `今日はいいてんきですね`（`てんき` 維持平假名） |

指定情境「きょうはいいてんき → てんき 選成平假名 → 繼續打 ですね → 重轉」的
測試是 `test_ja_deferred_sentence_keeps_user_segment_on_reconvert`。語料沒有
手動選字段，這個指標只在單元測試驗證，沒有 corpus 數字。

## 量測環境

- GB10，`cargo build -p jc-cli --release`，Jinen v2 small Q5_K_M，8 threads。
- 題庫 `modern-500`，500 題、7,240 次按鍵轉換。
- 改動前 binary 保存在 `/tmp/opencode/jc-cli-baseline`；改動後的一般路徑仍是
  `auto 92.2%`、`quality 93.6%`，沒有退步。
- 時間是牆鐘時間。模型逾時次數對負載敏感，重跑會有波動。

## 指標定義

| 指標 | 定義 |
|---|---|
| 每鍵 p95 | 每次 `Controller::input()` 的牆鐘時間 |
| 每題同步累積 p95 | 一題所有按鍵 `input()`＋句尾 `flush()` 的總和；不含停頓後模型補正 |
| 模型呼叫／題 | Jinen 推論執行次數 ÷ 500 |
| 已穩定片段變動率 | 上一次已經顯示成非かな的片段，後續被改掉的比例；不含當下尾巴 |
| 全片段變動率 | 所有顯示片段（含かな尾巴）的變動比例，僅供對照 |

「已穩定片段變動率」才是使用者說的「打完又變」。かな尾巴本來就會變成漢字，
不列入分母。

## 1. 現況

### 每一鍵做什麼

1. `Controller::input` 把完整讀音存進 `seq`，呼叫 `convert`。
2. `convert` 只轉 `seq[cut..]`，`cut` 之前是已鎖定前綴，不會重送。
3. 統計結果交給 `advance_lock`，除最後一個文節外鎖定。
4. 神經開啟時，`adopt_neural` 把**整段未鎖定尾巴**送模型；通過五條驗證才替換。

第 4 步有一個結構性問題：神經輸出沒有詞邊界，替換後整段尾巴變成一個文節，
`advance_lock` 的 `tail.len() <= 1` 直接返回，`cut` 永遠不前進。於是下一鍵又從
頭轉換整句、又送一次模型。這是 27 倍呼叫與累積延遲的主因。本輪已用
`NeuralScope::Clause` 修掉：統計文節邊界保留，模型只換最後一個片段。

### `modern-500 --live` 改動前後

| 設定 | top-1 | top-5 | 模型呼叫／題 | 每鍵 p50／p95 | 每題累積 p95 |
|---|---:|---:|---:|---:|---:|
| 一般路徑 `--neural auto` | 92.2% | 98.4% | 0.21 | — | 55.1 ms |
| 一般路徑 `--neural off` | 86.0% | 98.0% | 0 | — | — |
| `--live --neural off` | 48.8% | 97.8% | 0 | 0.14／0.61 ms | 12.9 ms |
| `--live`（現況全段尾巴） | 54.4% | 99.0% | 5.56 | 1.18／29.1 ms | 313 ms |

改動前後 `--live` 的 top-1、呼叫次數、每鍵延遲完全相同（54.4%、2,779 次、
1.18／29.1 ms），確認沒有動到舊路徑。舊路徑的已穩定片段變動率是 `0%`，
但代價是每一鍵重轉整句。

## 2. 三個優化

### A. 只轉新片段＋重用鎖定

`NeuralScope::Clause`：統計結果先決定文節邊界並鎖定前綴，模型只處理最後一個
尚未鎖定的文節。已鎖定片段不會重送，`seq` 不變，`locked_seq` 只用來檢查前綴
是否仍完整。

| 設定 | top-1 | 模型呼叫／題 | 每鍵 p95 | 每題累積 p95 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|
| 每鍵＋只補最後片段 | 48.8% | 3.31 | 26.9 ms | 161.6 ms | 0% |

呼叫次數從 5.56 降到 3.31，但 top-1 掉到和 `--neural off` 相同。原因是每一鍵
都鎖定最新片段，過早把還會被下一個かな改變的切法固定住。

### B. 統計優先，模型只處理歧義片段（主要路線）

自動変換先把讀音累積到邊界或長度門檻，統計轉換從頭部轉換、尾巴保持假名；
模型只對最後一個未鎖定片段問一次，門檻與五條驗證沿用 `neural::maybe_convert`。

| 設定 | top-1 | 模型呼叫／題 | 每鍵 p95 | 每題累積 p95 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|
| 邊界觸發＋只補最後片段 | **88.2%** | 1.85 | 22.3 ms | 95.4 ms | **0%** |
| 同上，`--neural off` | 88.2% | **0** | 0.62 ms | 4.2 ms | 0% |

兩列 top-1 相同，表示片段層模型在 `modern-500` 沒有帶來品質。它仍然值得保留
在設計裡，因為成本低（1.85 次／題、22 ms p95），但現階段不是品質來源。

### C. 非同步與延遲預算

`NeuralTiming::Deferred`：停頓後才把未鎖定尾巴排進補正。`Poll_neural` 會比對
generation，使用者一打字就丟棄；模型超過 `neural_budget` 也丟棄，画面維持統計
結果。補正階段用 `correction_gap_ratio` 把成本差門檻收緊（預設 0.5）。

| 設定 | top-1 | 模型呼叫／題 | 每鍵 p95 | 每題同步 p95 | 補正嘗試 | 被替換 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 邊界＋停頓，保留 1 片段 | 88.0% | 0.21 | 1.70 ms | 13.1 ms | 0.02／題 | 0% | 0% |
| 邊界＋停頓，保留 2 片段 | 90.4% | 0.31 | 2.90 ms | 18.6 ms | 1.00／題 | 3.6% | 10.92% |

保留 1 片段時，未鎖定尾巴幾乎都是助詞或かな，半截讀音規則直接跳過，補正沒有
東西可改。保留 2 片段才補得到內容詞，但被補的片段是使用者已經看過的漢字，
穩定度馬上升到 10.9%。這是 3c 的核心矛盾：**可補的尾巴與已穩定的畫面是同一塊。**

停頓處理時間（含模型推論）p50 0 ms、p95 24.9 ms；35 ms 預算下有 10 次超预算
丟棄，其餘在預算內完成。感知延遲只算按鍵與句尾統計，模型補正不擋輸入。

## 3b. Microsoft IME 形狀的自動変換

長度門檻只控制「什麼時候開始轉」；邊界包含助詞、標點與詞典完整詞查詢
（`Converter::is_complete_word`，單次索引查詢）。停頓在句尾與標點後模擬。

| 觸發 | top-1 | 模型呼叫／題 | 每鍵 p95 | 每題累積 p95 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|
| 長度 8 | 78.8% | 0.53 | 1.8 ms | 36.3 ms | 0% |
| 長度 12 | 85.6% | 0.31 | 1.0 ms | 28.2 ms | 0% |
| 長度 16 | 75.8% | 0.21 | 0.0 ms | 4.7 ms | 0% |
| 長度 8＋邊界 | 88.2% | 1.85 | 21.7 ms | 92.6 ms | 0% |
| 長度 12＋邊界 | 88.0% | 1.85 | 22.1 ms | 97.1 ms | 0% |
| 長度 16＋邊界 | 88.2% | 1.85 | 21.4 ms | 94.1 ms | 0% |
| 只有邊界 | 88.2% | 1.85 | 22.3 ms | 95.4 ms | 0% |
| 只有停頓 | 86.0% | 0.21 | 0.0 ms | 4.0 ms | 0% |

- 長度單獨使用時，12 最好（85.6%）；8 太早鎖定，16 太晚才轉。
- 加上邊界後，長度門檻幾乎沒有影響：詞典完整詞與助詞會先觸發，三種長度都是
  88.0–88.2%。**邊界是主要訊號，長度只是備援。**
- 停頓單獨使用等於「打完才轉」，top-1 86.0%、幾乎沒有中間轉換，穩定度最好，
  但失去逐字変換的即時感。

### 整段尾巴問模型（對照）

| 觸發 | top-1 | 模型呼叫／題 | 每鍵 p95 | 每題累積 p95 | 已穩定片段變動 |
|---|---:|---:|---:|---:|---:|
| 邊界＋整段尾巴 | 90.8% | 4.56 | 45.8 ms | 258.4 ms | 0% |
| 長度 8＋邊界＋整段尾巴 | 91.0% | 4.58 | 48.8 ms | 301.9 ms | 0% |

這是唯一在不犧牲穩定度的情況下跨過 90% 的組合，但每題 4.56 次呼叫、累積 p95
258 ms，兩個門檻都超過。

## 3d／ja-script：片段選字與假名變換

| 項目 | 狀態 | 實作位置 |
|---|---|---|
| 候選含完整替代 | 已完成 | `ja::live::Controller::candidates`、`session::Session::open_ja_candidates` |
| 候選含平假名／全角片假名／半角片假名 | 已完成 | `Controller::candidates` 接在漢字候選後，`JaCandidateSource::{Hiragana,Katakana,HalfwidthKana}` 去重 |
| 片段重選只改那一段 | 已完成 | `Controller::clause_spans`、`replace_clause`；`Session::apply_ja_candidate` |
| 選字後 locked 與 seq 正確 | 已完成 | `Controller::lock_through`；選字後把片段（含之前未鎖定文節）鎖定，讀音不變 |
| 游標移出後維持 | 已完成 | 選字先寫入 locked，後續輸入只轉尾巴 |
| Space 只對游標所在片段開候選 | 已完成 | `Session::space` 的 `Mode::Ja` 分支、`Session::open_ja_candidates` |
| 左右鍵移動組字游標 | 已完成 | `Session::ja_cursor`、`Session::move_ja_cursor`、`segments::cursor_byte` |
| F6 平假名 | 已完成 | `KeyCode::F(6)` → `Session::ja_transform(Hiragana)` |
| F7 全角片假名／平假互切 | 已完成，鍵位可設定 | `SessionConfig::ja_toggle_key` 預設 `F(7)`、`Session::ja_toggle_script`、`Controller::toggle_script` |
| F8 半角片假名 | 已完成 | `KeyCode::F(8)` → `ja_transform(HalfwidthKana)` |
| F9 全角英數 | 已完成 | `KeyCode::F(9)` → `ja_transform(FullwidthLatin)` |
| F10 半角英數 | 已完成 | `KeyCode::F(10)` → `ja_transform(HalfwidthLatin)` |
| 漢字拆回讀音 | 已完成 | `Controller::unconvert`、`Session::ja_unconvert` |
| Home／End 移到組字頭尾 | **缺** | `candidate_edge` 仍只處理注音；日文要另加 ja_cursor 頭尾 |
| 外殼鍵位映射 | **缺** | TSF／mac／linux wrapper 尚未把實體 F6–F10 轉成 `KeyCode::F` |
| 數字鍵直接選片段候選 | **缺** | 現有數字鍵路徑只給注音 `select_window_candidate` |

### 與逐字変換的搭配

- **長度／停頓觸發**：`unconvert` 會把 `cut` 退回目標片段起點，長度觸發從新的
  `cut` 重新累計；停頓補正只會補回退後、尚未鎖定的尾巴，不會碰已鎖定前綴。
- **統計優先**：使用者明確選字或按 F6–F10 後，`lock_through` 立刻把該片段鎖定，
  統計與模型都不再改它。只有未選字的尾巴會進 `maybe_convert`。
- **非同步補正**：補正請求帶 generation；`unconvert`／選字／下一個かな都會讓
  generation 前進，舊補正結果直接丟棄。

### 候選順序與去重

`Controller::candidates` 的順序是：轉換器候選（含漢字／神經結果）→ 目前詞面 →
平假名 → 全角片假名 → 半角片假名。`Session::open_ja_candidates` 把目前詞面移到
最前面，因此畫面是「目前詞面、其他漢字、かな」。同一詞面只留第一筆，讀音本身
是平假名時不會出現第二個「とうきょう」。

## 驗收門檻對照

| 指標 | 目標 | A 延遲整句（預設） | 判定 |
|---|---:|---:|---|
| 每鍵 p95 | ≤ 50 ms | 0.00 ms | 達標 |
| 每題累積 p95 | ≤ 150 ms | 0.01 ms（同步）；停頓轉換另計 p95 50.4 ms | 達標 |
| 每句模型呼叫 | ≤ 3 次 | 0.21 | 達標 |
| 已顯示片段變動率 | ≤ 5% | 0%（modern-500／hard-cases）；ajimee 多停頓 34.0% | 單句達標，多停頓未達標 |
| `modern-500` top-1 | ≥ 90% | **92.2%** | 達標 |

## 建議設計

### 目前預設（方案 A）

```rust
live::Config {
    enabled: true,
    deferred_sentence: true,
    idle_delay: Duration::from_millis(300), // 250／300／400 可調
    debounce: Duration::from_millis(120),
    neural_timing: NeuralTiming::Inline,
    ..Config::default()
}
```

流程：

1. 打字時 `input` 只更新 `seq`，回傳 preview（假名）。
2. `idle_delay` 到期時 `tick` → `idle` → `convert_sentence`。
3. `convert_sentence` 把讀音依 User 片段切成 Auto 區段；每個 Auto 區段做
   統計＋現有門檻＋神經，User 片段原樣插入。
4. 左文脈是目前已轉出的文字，右文脈是目前已知的 User 片段文字。
5. 任何輸入讓尚未固定的 Auto 轉換失效，下一次停頓重轉；User 片段不動。

`--no-live` 把 `ja_conversion` 設為 `sentence`，回到手動整句轉換，
`modern-500` 92.2%、`--neural quality` 93.6%。

### 與 B／C 的取捨

- A 的 top-1 與整句模式相同，推論次數只有 0.21 次／題，是所有方案最低。
- A 的停頓延遲是唯一的成本：`modern-500` p95 50 ms，`ajimee-200` p95 148 ms。
  這是整句推論時間，不是每鍵延遲。
- B prefix cache 的統計 top-1 是 86.0%，比 A 低 6.2 點，且 keep 1 時片段幾乎
  每鍵都會被改寫。不採用。
- C 的 88.2% 比 A 低 4 點，推論次數 1.64 次／題，優點是每鍵就有轉換；
  現在保留為 `--live-trigger boundary --live-scope clause --live-inline` 對照。

### 仍要補的一塊

多停頓長文的已穩定片段變動率（`ajimee-200` 34.0%）還沒有壓到 5%。原因是
每次停頓重轉會改掉前一次顯示的 Auto 片段。下一步應在重轉前比對畫面：
只有當新結果的漢字片段真的變好時才替換，或把前一次顯示的 Auto 片段降級成
User（先顯示かな再轉）並提供 undo。

## 實作拆解

| 檔案 | 函式／型別 | 改動 |
|---|---|---|
| `crates/jc-core/src/ja/live.rs` | `Config::{deferred_sentence,idle_delay,prefix_cache,neural_flight,flight_append,cache_keep}` | A／B 與 single-flight 開關；預設 `deferred_sentence = true` |
| 同上 | `Controller::{convert_sentence,convert_range,invalidate_sentence,convert_now}` | 延遲整句轉換與 Enter 同步轉換 |
| 同上 | `UserSegment`、`Controller::{mark_user,user_segments}` | User 片段固定與重轉保留 |
| 同上 | `PrefixCache`、`Controller::{sync_cache,convert_cache,start_flight,apply_flight,pump_flight}` | prefix cache 與 single-flight 對照 |
| 同上 | `FlightResult`、`NeuralGate::{start,take,busy}` | 非同步推論介面；`LiveGate` 尚未實作 start/take |
| 同上 | `Config::{trigger,neural_scope,neural_timing,neural_budget,keep_tail}` | 邊界／停頓觸發與片段模型（C 對照） |
| 同上 | `Trigger`、`AutoTrigger`、`NeuralScope`、`NeuralTiming`、`GatePhase` | 新設定型別 |
| 同上 | `Controller::convert_auto` | 自動変換主路徑：統計先轉、保留文節邊界、可延後模型 |
| 同上 | `Controller::is_boundary`、`ends_with_particle`、`ends_with_punctuation`、`Converter::is_complete_word` | 邊界觸發 |
| 同上 | `Controller::apply_clause_neural`、`queue_neural`、`poll_neural`、`tail_request` | 片段模型與停頓補正 |
| 同上 | `Controller::flush`、`Controller::idle` | 句尾與停頓入口 |
| 同上 | `Controller::publish`、`record_stability`、`Stats` | 穩定度與量測 |
| 同上 | `Controller::{clause_spans,replace_clause,transform_clause,toggle_script,unconvert,lock_through}` | 片段選字、假名變換、拆回讀音 |
| 同上 | `KanaTransform`、`ClauseSpan` | 純字串轉換 API |
| `crates/jc-core/src/session/neural.rs` | `NeuralRuntime::start`、`NeuralStatsSnapshot::{inferences,busy_ms}`、`NeuralConfig::correction_gap_ratio` | 非阻塞送出與推論忙碌統計 |
| `crates/jc-settings/src/settings.rs` | `ja_conversion`、`effective_ja_conversion`、`ja_live_enabled` | 新設定欄位與舊 `ja_live_conversion` 遷移 |
| `crates/jc-engine/src/config.rs` | `ja_live` 映射 | 用 `settings.ja_live_enabled()` |
| `crates/jc-core/src/session/mod.rs` | `Session::{ja_transform,ja_toggle_script,ja_unconvert,open_ja_candidates,apply_ja_candidate,refresh_ja_live,move_ja_cursor}` | 外殼入口與片段游標 |
| 同上 | `SessionConfig::ja_toggle_key` | 預設 `F7`，可設定 |
| `crates/jc-core/src/session/core.rs` | `Session::navigate`、`Session::space`、`Session::finish_composition` | 左右鍵、Space、Enter 接日文片段 |
| `crates/jc-core/src/session/segments.rs` | `Session::cursor_byte` | 日文游標改用 `ja_cursor` |
| `crates/jc-core/src/key.rs` | `KeyCode::F(u8)` | F6–F10 |
| `crates/jc-core/src/ja/decode.rs` | `JaCandidateSource::{Hiragana,Katakana,HalfwidthKana}` | 候選來源標記 |
| `crates/jc-cli/src/cli.rs` | `--live-trigger`、`--live-scope`、`--live-deferred`、`--live-budget-ms`、`--live-correction-ratio`、`--live-keep-tail`、`--live-report` | 量測旗標 |
| `crates/jc-cli/src/converter.rs` | `JaConverter::live_nbest`、`LiveReport` | 逐鍵延遲、停頓模擬、穩定度聚合 |
| `crates/jc-cli/src/eval_cmd.rs` | `JaOptions`、`ja`、`live-report` 輸出 | 量測報告 |

## 可重現指令

```sh
# 現況（改動前後同一個 legacy 路徑）
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live --live-report

# A：只補最後片段，每鍵轉換
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-scope clause --live-report

# B：邊界觸發＋只補最後片段（建議的穩定設定）
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger boundary --live-report

# B 關閉模型（證明片段模型沒有品質貢獻）
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger boundary --neural off --live-report

# 3b：長度 8／12／16
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger length:12 --live-report

# 3b：長度＋邊界＋停頓
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger length:12,boundary,idle --live-report

# 3c：停頓後非同步補正，35 ms 預算
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger length:12,boundary,idle --live-deferred \
  --live-budget-ms 35 --live-report

# 對照：整段尾巴問模型，top-1 90.8%
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live \
  --live-trigger boundary --live-scope whole --live-report

# 一般路徑不得退步
cargo run -p jc-cli --release -- eval ja --corpus modern-500
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural quality
```

## 未完成

- 外殼（TSF／macOS／Linux）尚未把實體 F6–F10 轉成 `KeyCode::F`；核心 API 已可用。
- Home／End 尚未在日文組字游標上實作。
- 數字鍵直接選日文片段候選尚未接。
- 停頓補正沒有真正跨執行緒非同步；目前是「停頓時呼叫、超過預算丟棄」，感知
  延遲已排除模型時間，但 CPU 仍在同一執行緒使用。
- 右文脈仍固定空字串。
- 量測是單機牆鐘時間；Jinen 逾時次數對負載敏感。
