# drop-punct：移除標點模型

2026-11-20 決定不再使用標點模型。模型資料檔保留在
`jc-ime-data/models/punctuation-public-core/` 與 `models/zh-punct/`，程式與
manifest 不再引用。`ctrl_punct`（`Ctrl+,` 全形逗號）與 `punct_mark`（送出前
插入單一標點）是規則，保留。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/punct/punct.go`（`Marks`、`Edit`、`Apply`） | `crates/jc-core/src/punct.rs` | ported：保留 `MARKS`、`Edit`、`apply`；刪除模型專用的 `Punctuator` trait、`PunctError`、`apply_japanese` |
| `core/punct/punct_test.go`（漢字邊界） | `crates/jc-core/src/punct.rs` | ported：保留 `test_apply_inserts_only_between_han`、`test_empty_edits_leave_text_untouched` |
| `core/output/punctuate.go`（半形轉全形） | `crates/jc-core/src/output/mod.rs` | ported：`LanguagePunctuator` 保留；`Options::punctuate` 改名 `punct_fullwidth`，避免與模型路徑混淆 |
| `internal/neural/native/punct_plan.go` | `crates/jc-model/src/punct_plan.rs` | 已移除：視窗、merge、decision 全部刪除 |
| `internal/neural/native/punct_ort.go`（日文） | `crates/jc-model/src/punctuation_model.rs` | 已移除：ORT 與 candle 推論一併刪除 |
| `internal/neural/punct_manifest.go`（注音） | `crates/jc-model/src/punct_zh.rs` | 已移除：注音字元模型與 manifest 角色刪除 |
| `oc_training` 標點 forward | `crates/jc-model/src/candle/punct.rs`、`candle/quant.rs` | 已移除：`CandlePunctuation`、`QuantPunctuation`、`export_punct_gguf` 刪除 |
| `core/session/session.go`（`punctuateWith`、`punctuateJapanese`） | `crates/jc-core/src/session/mod.rs`、`session/core.rs` | 已移除：`SessionConfig::punct`、`SharedPunctuator`、`model_punctuated` 刪除；commit 直接送 `finalize` 結果 |
| `cmd/0c-server`（模型包標點設定） | `crates/jc-cli/src/cli.rs`、`converter.rs`、`data.rs` | 已移除：`--punct` 參數與載入路徑刪除 |
| `cmd/0c-server/ja_models.go`（punct 載入） | `crates/jc-engine/src/loader.rs`、`crates/jc-cli/src/data.rs` | 已移除：`load_punctuator` 刪除 |
| — | `crates/jc-model/src/bin/probe.rs` | 已移除：探測程式只服務 reranker 與標點模型，改由 `eval ja --rerank on` 驗證 |
| — | `crates/jc-cli/src/bin/candle-punct-check.rs` | 已移除：只做標點模型逐組比對 |
| — | `crates/jc-model/src/bin/candle-quant.rs` | 修改：刪除 `punct` 子命令與 `--punct` 匯出分支 |

## 保留項目

- `ctrl_punct`：`crates/jc-core/src/session/input_style.rs`、`session/core.rs`；
  測試 `test_ctrl_comma_inserts_fullwidth_comma`。
- `punct_mark`：`crates/jc-core/src/session/core.rs` 的 `puncted`；
  測試 `test_punctuator_inserts_mark_before_last_han`。
- `crates/jc-core/src/output/mod.rs` 的半形轉全形規則。
- `legal/` 的授權紀錄與 `docs/models.md` 的歷史雜湊。

## 驗證

```sh
! grep -rn "punctuate\|SharedPunctuator\|--punct" crates/ | grep -v ctrl_punct
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p jc-cli --release -- eval zh-golden
cargo run -p jc-cli --release -- eval ja --corpus modern-500
```
