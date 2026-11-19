# z3-zh-back：解碼與輸出

注音後段移植：N-best 解碼、無聲調處理、疊字還原、跨段補全、輸出組裝。
已合併 `main` 的 `z1-zh-front` 與 `z2-zh-mid`，直接使用它們的型別與 API。

## 與 z2／z1 的介面

- 詞格、邊、詞典、成本：`crate::lattice::{Lattice, Edge, LexEntry, Lexicon,
  InMemoryLexicon, Viterbi}`、`crate::score::{ScoreContext, Scorer}`。
  成本是 `i32`，越小越優先；Go 的機率與 bigram 由詞性連接成本取代。
- 音節假設格（模糊音、無聲調）：`crate::zh::input::{Lattice, Hyp}`，
  在 decode 內以 `SyllableLattice` 轉出。
- 本 module 不重複定義詞格或評分型別；只有 `Seg`、`Lock`、`Conversion`、
  `Completion`、`Hypothesis` 這些 Go 契約的對外型別。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/decode/decode.go` | `crates/jc-core/src/decode/mod.rs` | rewritten：吃 `z2` 的邊圖與 `z2::score` 的 `i32` 成本；前向 DP 自己實作以支援鎖定與尾端前綴，tie-break（成本、段數、穩定鍵）與 `Viterbi` 對齊 |
| `core/decode/decode_test.go` | `crates/jc-core/src/decode/mod.rs` 的 `mod tests` | rewritten：改用 `InMemoryLexicon` 與 `ScoreContext`；`TestLatticeWithAlternatives` 等依賴機率/成本假設的測試改成成本版 |
| `core/decode/limits_test.go` | `crates/jc-core/src/decode/mod.rs` 的 `test_limits_control_word_length` | rewritten：詞長上限改由 `ScoreContext::max_word_units` 與 `Lexicon::longest_word_units` 決定 |
| `core/decode/context_test.go` | — | n/a：Go 測試的 `contextScore` 在 Rust 由 `score::Scorer`（連接成本）取代，沒有對應正式程式碼 |
| `core/decode/weak_prefix_test.go` | `crates/jc-core/src/decode/nbest.rs` 的 `weak_prefix` | rewritten：尾端弱前綴改用詞格最後位置的邊判斷 |
| `core/decode/nbest.go` | `crates/jc-core/src/decode/nbest.rs` | rewritten：k-best DP 走 `Lattice::edges_from`，支援鎖定與前綴；`AuditTarget` 保留可達性分層 |
| `core/decode/nbest_test.go` | `crates/jc-core/src/decode/nbest.rs` 的 `mod tests` | rewritten：成本版 N-best、鎖定、前綴與 `AuditTarget` 行為測試 |
| `core/decode/repeat.go` | `crates/jc-core/src/decode/repeat.rs` | rewritten：機率比較改成固定成本懲罰 `REPEAT_COST`；只在同讀音有詞庫詞、而路徑走非詞疊字時加成本 |
| `core/decode/repeat_test.go` | `crates/jc-core/src/decode/repeat.rs` 的 `mod tests` | rewritten：`repeat_kind`、`may_cap` 與非詞疊字懲罰的行為測試 |
| `core/decode/untoned.go` | `crates/jc-core/src/decode/untoned.rs` | rewritten：音節假設格改成 `SyllableLattice`；候選裁切依成本排序，讀音還原走 `Lexicon::lookup`／`prefix` |
| `core/decode/untoned_test.go` | `crates/jc-core/src/decode/untoned.rs` 的 `mod tests` | rewritten：裁切與讀音還原的具體數值測試 |
| `core/decode/across.go` | `crates/jc-core/src/decode/across.rs` | rewritten：詞群權重由 `exp(-成本)` 換算，比例定義為詞群 ÷（詞群＋目前路徑） |
| `core/decode/across_test.go` | `crates/jc-core/src/decode/across.rs` 的 `mod tests` | rewritten：合併詞與比例範圍 |
| `core/decode/bias.go` | `crates/jc-core/src/decode/bias.rs` | rewritten：`arcLog`／`evidenceMass`／`correctionFactor` 改成成本組合；個人化掛鉤回傳 0 |
| `core/decode/explain.go` | `crates/jc-core/src/decode/explain.rs` | rewritten：貢獻改成詞條成本與連接成本兩項；`DocumentContext` 沒有個人化來源 |
| `core/decode/explain_test.go` | `crates/jc-core/src/decode/explain.rs` 的 `mod tests` | rewritten：逐片段成本 |
| `core/decode/personal_mass.go` | — | n/a：個人化，M4 |
| `core/decode/personal_mass_test.go` | — | n/a：個人化，M4 |
| `core/output/output.go` | `crates/jc-core/src/output/mod.rs` | rewritten：語言標籤用 c0 的 `Mode`；`OutputVariant` 改成 `Variant` 列舉 |
| `core/output/assemble.go` | `crates/jc-core/src/output/mod.rs` | ported：逐片段轉換、標點、間距與片段索引錯誤 |
| `core/output/render.go` | `crates/jc-core/src/output/mod.rs` | rewritten：`Simplifier` 改成 `Converter::to_traditional`；這一輪只有 identity，簡體輸出回 `MissingConverter` |
| `core/output/punctuate.go` | `crates/jc-core/src/output/mod.rs` | ported：`LanguagePunctuator` 與 `is_cjk_word` |
| `core/output/space.go` | `crates/jc-core/src/output/mod.rs` | ported：`CjkLatinSpacer` |
| `core/output/output_test.go` | `crates/jc-core/src/output/mod.rs` 的 `mod tests` | rewritten：測試替身以 trait 物件注入 |
| `core/output/output_impl_test.go` | `crates/jc-core/src/output/mod.rs` 的 `mod tests` | rewritten：renderer、標點、間距、組裝器的行為測試；`zhconv` 案例改驗證 `MissingConverter` |

## 刻意不搬的部分

- 個人化：`Evidence`、`Explicit`、`Layered`、`CorrectionPenalty`、`massScale`
  （M4）。`bias` 與 `explain` 留下回傳 0 的掛鉤。
- Go 的機率／bigram／`TopM`／`Beam`／`MaxCost`：`z2` 的成本模型不需要；
  `SearchStats::pruned_contexts` 固定 0。
- `CandidatesInContextWindows` 的多視窗與 `innerScores`（#172）：成本模型的
  候選只有邊，沒有帶成本假設的視窗；需要時由前端產生不同詞格再查候選。
- `repeat` 的 `wins`／`common`／`lexical` 機率比較：成本版只判斷「同讀音是否
  有詞庫詞」，避免把非詞疊字壓到詞庫詞之後。
