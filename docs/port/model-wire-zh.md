# model-wire-zh：注音選字模型接進候選

把 `/mnt/work/jc/jc-ime-data/models/zh-select/` 的 D512 選字模型接進注音
N-best。預設關閉；`--features onnx` 加 `--select-model on` 才會載入模型。

## vocab 與模型輸入規則

`vocab.json` 的 schema 是 `zuin-phonetic-v1`，不是 SentencePiece：

| 項目 | 規則 |
|---|---|
| 字元 id | `characters[i]` → `i + 8` |
| 注音 base id | `bases[i]` → `i + 2` |
| 特殊 token | `1` `[BOS]`、`2` `[SEP]`、`3` `[OUT]`、`4` `[UNK]` |
| 詞表外的前文字元 | 送 `[UNK]`，不失敗 |
| 詞表外的候選字元 | 整個候選不送模型；分數保持中性 0（前身 `ZhAdapter` 行為） |
| 詞表外的注音 base | 整筆回退統計排序 |

一個候選的序列（`n` 是音節數，`S` 是音節區起點）：

```text
[BOS] + context（最多 64 字，超過保留最右側）+ [SEP]
      + sounds[0..n-1]                     # 前 n-1 個音節，共用
      + 分支節點：n-1 個 type=1 的 0
      + 最後一個音節：id=3、type=0          # [OUT]
      + 候選字 c[1..n-1] 的共用前綴樹       # 第一個字由 [OUT] 節點預測
```

- `bases`／`tones` 只填音節位置；聲調 `ˊ=2`、`ˇ=3`、`ˋ=4`、`˙=5`，
  沒有符號是一聲 `1`，`?` 是未知聲調 `0`。
- 候選第 `i` 個字（`i >= 1`）放在 `S + n + i - 1`，`positions` 使用相同值。
- `mask` 是 `[1, 1, L, L]` 的 bool：音節區是 causal；候選節點可看整段前文
  與自己的祖先鏈。
- `proj_pos` 是分支與前綴樹節點的序列索引；`target_node` 指向每個候選的
  前綴節點，`target_ids` 是 `char_id - 6`。
- 最後音節的 base 在 `neutral_tolerant_last` 內時多開一個輕聲分支；兩個
  分支的模型分數取最大值。
- 長度上限：2–32 音節、16 個候選、前文 64 字。單音節不送一般模式。

`prepare()` 的輸出已用前身
`internal/neural/native/input/testdata/golden.json` 的 basic 案例逐欄比對
（`ids`、`bases`、`tones`、`types`、`positions`、`proj_pos`、`target_node`、
`target_ids`）。

## 重排方式

- 統計分數換算：`log_score = -cost / 1000`，與前身 `LogScore` 同尺度。
- 融合：`fused = log_score + 2.0 × model_score`。
- 只有其他候選的 `fused` 超過原首選 `margin = 0` 時才把首選換掉；同分保留
  統計首選。其餘候選順序不動。
- 推論預算 100 ms；逾時、模型缺席、feature 關閉、候選不合契約都保留統計
  排序。
- 每次 `Decoder::nbest` 嘗試重排，前文目前固定空字串：工作階段只保留尚未
  提交的組字，已提交文字不回頭送模型。
- 未知詞邊的字面就是讀音本身（例如「我ㄒㄧㄤˇ」），與前身 `zhGroups` 一樣
  跳過；有效候選少於 2 個就不呼叫模型。

## 驗證數字

環境：GB10 CPU，onnxruntime 1.32.0，`intra_threads = 1`。

| 項目 | 關閉 | 開啟 |
|---|---|---|
| `eval zh-golden` | 41/45 | 40/45 |
| 推論次數 | 0 | 100 |
| 推論延遲 p50 | — | 11.66 ms |
| 推論延遲 p95 | — | 19.20 ms |

開啟後唯一的變化是 `rerank-off`：統計首選「程式」（成本 7600）被模型換成
「城市」（成本 7823）。模型分數分別是 `-0.9937908` 與 `-0.5779103`，
與前身 Python 參考實作逐位相同；這是模型偏好，不是輸入組裝錯誤。

4 個學習依賴案例在開啟後仍然失敗，且原因不變：

| 案例 | 結果 |
|---|---|
| `learning-select-preference` | 期望「井」，得到「警」 |
| `learning-forget-request` | 期望「井」，得到「警」 |
| `undo-learned-word` | 未支援 `ShiftDelete` |
| `undo-custom-word` | 未支援 `ShiftBackspace` |

這些案例都是單音節，模型一般模式要求 2 個音節以上，推論次數沒有增加；
失敗來自個人學習與按鍵尚未實作，不是模型剛好答對。

`convert "ㄨㄛˇ ㄞˋ"`（`build-zh` 詞庫）首選仍是「我愛」，模型開啟前後
不變；單次推論 6.47 ms。使用 golden 的小詞庫時「愛」不在詞表，首選是
「我ㄞˋ」，模型因候選字元不在 vocab 而回退。

## 執行

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so

cargo run -p jc-cli --release -- eval zh-golden
cargo run -p jc-cli --release --features onnx -- eval zh-golden --select-model on
cargo run -p jc-cli --release --features onnx -- convert "ㄨㄛˇ ㄞˋ" \
  --data /mnt/work/jc/jc-ime-data/build-zh --select-model on
```

模型目錄預設由 `JC_MODELS` 或 `--testdata` 上層的 `models/manifest.json`
決定；也可以直接設 `JC_MODELS=/mnt/work/jc/jc-ime-data/models`。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/rerank/rerank.go`（`Supported`、`Choose`、限制） | `crates/jc-core/src/zh/select.rs`（`prepare`、`choose`、`cost_to_log_score`） | rewritten：拿掉隱私與 async，保留長度、去重、融合與 margin |
| `core/rerank/tones.go` | `crates/jc-core/src/zh/select.rs`（`SelectManifest::from_json`） | rewritten：只驗 v2 + `zuin-tone-mask-v1` 需要的欄位 |
| `core/score/zh_adapter.go` | `crates/jc-core/src/zh/select.rs`（`rerank_nbest`） | rewritten：跳過詞表外候選、中性 0 分、整筆回退 |
| `core/score/ranker.go`（模型分數併入成本） | `crates/jc-core/src/zh/select.rs`（`choose`） | rewritten：成本換成 `-cost/1000`，權重 2.0 |
| `core/session/rerank.go`（`rerankSegment`、`expandNeutral`） | `crates/jc-core/src/zh/select.rs` + `decode/nbest.rs` | rewritten：只做 N-best 首選重排與輕聲分支；extendShown、veto、hint 未移植 |
| `internal/neural/native/input/input.go` | `crates/jc-core/src/zh/select.rs`（`prepare`） | ported：前綴樹、分支、遮罩、`proj_pos`、`target_*` 逐欄一致 |
| `internal/neural/native/backend_ort.go`（sequence 模式） | `crates/jc-core/src/zh/select.rs`（`SelectModel::score`／`run`） | rewritten：單一 9 輸入 session、100 ms watchdog、分支取最大值 |
| `internal/neural/native/manifest.go` | `crates/jc-core/src/zh/select.rs`（`SelectManifest`） | rewritten：只保留重排需要的欄位與驗證 |
| `internal/neural/native/input/neutral.go` | `crates/jc-core/src/zh/select.rs`（`SelectManifest::neutral_last`） | ported |
| `internal/neural/native/select_e2e_test.go` | — | n/a：前身測試需要完整 0c session；本 task 用 golden 與 CLI 驗證 |
| `core/session/async_rerank.go` | — | n/a：本 task 只有同步 N-best 路徑，沒有背景候選窗 |
| `core/session/rerank.go`（extendShown／veto／hint） | — | n/a：個人學習、使用者手選與提示屬其他任務 |
