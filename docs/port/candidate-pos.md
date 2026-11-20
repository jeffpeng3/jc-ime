# candidate-pos：候選帶詞性

把日文 pack 既有的 POS id 帶到候選與協定。`Candidate` 只保存 `Option<u32>`，
名稱由共用的 `PosTable` 查；注音詞典沒有詞性，維持 `None`。排序與成本計算
沒有改動。

## POS id 來源

- `JaLexicon::pos_name(id)` 直接查 pack 的 POS table，索引就是 pack entry 的
  `pos_id`。
- `JaDictionary` 建立 `詞性字串 -> id` 與 `PosTable` 各一份，詞弧建立時把
  entry 的 POS 字串換成 id，候選不必各自保存字串。
- 日文候選的 POS id 取第一個文節的第一個詞段（主要詞）。未知詞與沒有 POS
  表的自訂來源回傳 `None`。
- 注音候選一律 `None`；`CandidateDetail` 的 `pos` 因此省略。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| 無對應（Go 版候選沒有詞性） | `crates/jc-core/src/candidate.rs` | rewritten：新增 `PosTable`、`Candidate::pos`、`with_pos`、`clear_pos`、`pos_name`；注音候選預設 `None` |
| 無對應（Go 版沒有 pack POS 表查名） | `crates/jc-core/src/ja/lexicon.rs` | rewritten：`pos_name`、`pos_table` 直接映射 pack POS table |
| `core/ja/decode/decode.go`（`Clause`／`CandidateSegment`） | `crates/jc-core/src/ja/decode.rs` | rewritten：`JaClauseSegment` 與 `JaNode` 帶 POS id，`JaResult::pos_id()` 取主要詞；`JaWordSource` 新增 `pos_id`／`pos_name`／`pos_table`，預設 `None` |
| `core/ja/decode/decode.go`（`Decoder`） | `crates/jc-core/src/ja/decode.rs` | rewritten：`JaDecoder::pos_name`／`pos_table` 轉給詞條來源 |
| 無對應（Go 版 proto 沒有詞性） | `crates/jc-proto/src/lib.rs` | rewritten：`CandidateDetail` 新增選填 `pos`，映射時用 `Candidate::pos_name()` 查名 |
| 無對應 | `crates/jc-proto/tests/fixtures/response_state_v3_pos.json` | rewritten：手寫含 `pos` 的 v3 fixture |
| 無對應 | `crates/jc-proto/tests/fixtures.rs` | rewritten：含 `pos` 的往返測試 |
| 無對應 | `crates/jc-cli/src/cli.rs`、`crates/jc-cli/src/converter.rs` | rewritten：`convert --pos` 顯示詞性，不加時輸出不變 |
| 無對應 | `crates/jc-core/src/session/core.rs` | rewritten：日文一般與逐字変換建立候選時接上 POS；神經模型換掉字面時清掉詞性 |
| 無對應 | `docs/proto.md` | rewritten：`CandidateDetail.pos` 欄位、來源與注音不支援的說明 |

## 測試

- `candidate.rs`：注音候選預設沒有詞性；`with_pos` 只存 id，名稱由共用表查；
  id 越界回 `None`。
- `ja/lexicon.rs`：`pos_name` / `pos_table` 與 pack POS table 一致，越界回 `None`。
- `ja/decode.rs`：真實 pack 的候選有 POS id 且查得到名稱。
- `jc-proto`：含 `pos` 的 fixture 往返；沒有詞性的候選不輸出 `pos`。
- `jc-cli`：`--pos` 解析成開關，預設關閉。
