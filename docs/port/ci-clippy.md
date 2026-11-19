# ci-clippy：清掉 workspace 的 clippy 與 fmt 問題

本任務不做移植，只修 lint。下表列出每一條修掉的規則、位置與改法。
Go 檔案欄位對應「無」（n/a）。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `crates/jc-eval/src/sha256.rs` | rewritten：`chunks_exact` 改 `as_chunks::<64>()` 與 `as_chunks::<4>()` |
| n/a | `crates/jc-jadic/src/mozc.rs` | rewritten：`while let` 改 `for`、移除多餘 `mut`、`1 * 3 + 2` 改 `3 + 2` |
| n/a | `crates/jc-jadic/src/pack.rs` | rewritten：`SectionOffsets` 改用 struct update 初始化 |
| n/a | `crates/jc-model/src/layout.rs` | rewritten：`[0 + 1 + 60 - 1]` 改 `[1 + 60 - 1]`、`ModelSpec` 改用 struct update |
| n/a | `crates/jc-model/src/punctuation_model.rs` | rewritten：常數斷言改 `const { assert!(...) }` |
| n/a | `crates/jc-model/src/rerank.rs` | rewritten：`ModelSpec` 改用 struct update |
| n/a | `crates/jc-data/src/ja/pack.rs` | rewritten：`Entry::for_test` 的 `conj_type`／`conj_form` 合成 tuple，參數降到 7 個 |
| n/a | `crates/jc-data/src/ja/tests.rs` | rewritten：`1 * 4 + 2` 改 `4 + 2`、`for_test` 呼叫端同步調整 |
| n/a | `crates/jc-data/src/zh/lexicon.rs` | rewritten：`cmp_ids` 改用 `query.iter().enumerate().take(len)` |
| n/a | `crates/jc-core/src/decode/nbest.rs` | rewritten：`for at in 0..n` 改 enumerate、`map_or` 改 `is_none_or`、`ScoreContext` struct update |
| n/a | `crates/jc-core/src/decode/repeat.rs` | rewritten：`map_or(false, ..)` 改 `is_some_and`、`ScoreContext` struct update |
| n/a | `crates/jc-core/src/decode/untoned.rs` | rewritten：`map_or(false, ..)` 改 `is_some_and` |
| n/a | `crates/jc-core/src/decode/mod.rs` | rewritten：`map_or` 改 `is_none_or`、`for i in 0..n` 改 enumerate、`ScoreContext` struct update |
| n/a | `crates/jc-core/src/decode/across.rs` | rewritten：`ScoreContext` struct update |
| n/a | `crates/jc-core/src/decode/bias.rs` | rewritten：`ScoreContext` struct update |
| n/a | `crates/jc-core/src/decode/explain.rs` | rewritten：`ScoreContext` struct update |
| n/a | `crates/jc-core/src/ja/decode.rs` | rewritten：`JaBuilder` 改 `#[derive(Default)]`、巢狀 `if` 合併 |
| n/a | `crates/jc-core/src/lattice/build.rs` | rewritten：`sort_by` 改 `sort_by_key`、`ScoreContext` struct update |
| n/a | `crates/jc-core/src/lattice/viterbi.rs` | rewritten：`ScoreContext` struct update |
| n/a | `crates/jc-core/src/mixed.rs` | rewritten：移除多餘 `&` 與 `&mut`、`Router` 改 `#[derive(Default)]`、`!(x > 0.0)` 改 `x <= 0.0`、`matches!(c, 'a'..='z')` 改 `is_ascii_lowercase`、`map_or` 改 `is_none_or`、`ScoreContext` struct update |
| n/a | `crates/jc-core/src/score.rs` | rewritten：`ScoreContext` struct update |
| n/a | `crates/jc-core/src/segment.rs` | rewritten：`Splitter::default()` 改 `Splitter` |
| n/a | `crates/jc-core/src/session.rs` | rewritten：`map_or` 改 `is_none_or`、`len() >= 1` 改 `!is_empty()` |
| n/a | `crates/jc-core/src/zh/fuzzy.rs` | rewritten：`iter().any(|m| *m == X)` 改 `contains(&X)` |
| n/a | `crates/jc-proto/src/lib.rs` | rewritten：測試拿掉多餘 `format!` |
| n/a | `crates/jc-cli/src/converter.rs` | rewritten：`JaConverter` 直接持有 `JaDecoder`，移除 `Arc<JaDecoder>` |
| n/a | `crates/jc-cli/src/eval_cmd.rs` | rewritten：配合 `JaConverter::new` 移除 `Arc` |
| n/a | `crates/jc-engine/src/lib.rs` | rewritten：`SessionConfig` 改 struct update |

## 修掉的規則

- `clippy::chunks_exact_to_as_chunks`
- `clippy::while_let_on_iterator`
- `clippy::field_reassign_with_default`
- `clippy::identity_op`
- `clippy::unused_mut`
- `clippy::assertions_on_constants`
- `clippy::too_many_arguments`
- `clippy::needless_range_loop`
- `clippy::unnecessary_map_or`
- `clippy::derivable_impls`
- `clippy::collapsible_if`
- `clippy::unnecessary_sort_by`
- `clippy::needless_borrow`
- `clippy::neg_cmp_op_on_partial_ord`
- `clippy::manual_is_ascii_check`
- `clippy::unnecessary_mut_passed`
- `clippy::default_constructed_unit_structs`
- `clippy::len_zero`
- `clippy::manual_contains`
- `clippy::useless_format`
- `clippy::arc_with_non_send_sync`

## sha256 digest 驗證

`crates/jc-eval/src/sha256.rs` 改成 `as_chunks` 後，用 `jc_eval::sha256_hex`
讀 `jc-ime-data/packs/ja-lexicon-mozc-v2.pack`：

```
15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6
```

與 `sha256sum` 輸出一致。

## 驗證結果

```
cargo fmt --all -- --check                                  # exit 0
cargo clippy --workspace --all-targets -- -D warnings       # exit 0
cargo clippy --workspace --all-targets --all-features -- -D warnings  # exit 0
```

`jc-model` 的 `onnx` feature 已實際編譯，未下載或連結額外 ORT 執行檔。
