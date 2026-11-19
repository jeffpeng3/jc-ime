# zh-golden：注音 golden 45 例

現況：`jc-cli eval zh-golden` 通過 **35/45**。18 個失敗已逐一分類，沒有 A（詞典
缺詞）、C（分段錯）類；B 類 1 個、D 類 4 個，其餘是驅動或子系統沒接上。
D 類沒有硬修，也沒有改期望值。

執行方式（這個 branch 的 `jc-cli` 還有 `ja-eval`，要指定 bin）：

```sh
cargo run -p jc-cli --release --bin jc-cli -- eval zh-golden
```

## 分類表

| # | 案例 | 類別 | 原因 | 狀態 |
|---:|---|---|---|---|
| 1 | `toneless-word` | E | 聲調選填沒接：session 直接拿沒有聲調的字串查詞典，查不到就原樣輸出注音。 | 已修：`SessionConfig::toneless` 展開五個聲調，挑成本最低的組合。 |
| 2 | `candidates-paging` | E | `page` 請求被當成 no-op，且候選只留 5 個；期望第二頁第 2 項（畿）選不到。 | 已修：`Session::page` 加 `PAGE_SIZE = 10`；只有翻頁案例把候選上限提高到 13。排序本身正確（機…畿 與期望逐項相同）。 |
| 3 | `tab-completion` | E | Tab 被當成「移到下一個候選」，沒有補全建議；另外 `Decoder` 的補全權重對毫成本取 `exp` 會下溢成 0，永遠找不到建議。 | 已修：`complete` 權重除以 1000 換回 nats，session 在 Tab 接受補全。 |
| 4 | `learning-select-preference` | D | 期望第二次打同讀音時手選的「井」排第一，依賴被動學習／個人詞庫（M4）。 | 不修。 |
| 5 | `learning-forget-request` | D | `forget` 請求要撤銷學習證據；個人化未實作。 | 不修。 |
| 6 | `undo-learned-word` | D | 候選窗 `Shift+Delete` 要忘記學到的詞並留 tombstone；依賴學習資料。 | 不修。 |
| 7 | `undo-custom-word` | D | seed 的自訂詞「世傑」與 `Shift+Backspace` 刪詞，屬個人詞庫。 | 不修。 |
| 8 | `simplified-commit` | F | `zhconv` 尚未移植，`output` 只有 identity 轉換器（`docs/port/z3-zh-back.md` 已註明）。 | 未修：需要 OpenCC tw2s 字典與 `zhconv` 移植。 |
| 9 | `simplified-sentence` | F | 同上。 | 未修。 |
| 10 | `mixed-chinese-english-chinese` | F | Mixed 模式把 ASCII 直接丟給 `mixed::Router`，Router 只認注音符號／かな；`ji3`、`hello`、`t` 都沒進注音鍵盤，結果整串照字面。 | 未修：`core/session/mixed.go`（1748 行）的按鍵推斷未移植。 |
| 11 | `mixed-english-prefix` | F | 同上。 | 未修。 |
| 12 | `english-completion` | F | 英文補完依賴 Mixed 前綴與 `English` 學習層；session 沒有接。 | 未修。 |
| 13 | `english-mode` | E | `toggle` 請求不支援，session 模式固定。 | 已修：`Session::toggle_english`。 |
| 14 | `cjk-latin-space` | F | `CjkLatinSpace` 已在 `output` 實作，但 commit 路徑沒接；且本例依賴 Mixed 先把 `ji3hellot ` 切成中英段。 | 未修：等 Mixed 接上。 |
| 15 | `ctrl-punct` | E | `Ctrl+,` 不是合法具名鍵，`Key` 也沒有 Ctrl 欄位。 | 已修：`Key::ctrl` 加 `ctrl_punct` 選項，`Ctrl+,` 輸出「，」。 |
| 16 | `punctuator-commit` | E | 補標點替身沒接，commit 直接送出「今天」。 | 已修：`punct_mark` 在最後一個漢字前插入標點。 |
| 17 | `rerank-on` | B | 詞都在，成本讓「程式」勝出；zh 路徑沒有重排掛鉤（Go 用固定替身評分器）。 | 已修：`RerankOptions` 固定替身，偏好命中扣 `weight × prefer_score × 1000` 毫成本。 |
| 18 | `fuzzy-select` | F | 模糊音規則完全沒移植，`ㄗㄨㄥ` 不會展開 `ㄓㄨㄥ`。 | 已補最小實作：四組配對的 `select` 模式，近音候選插在原讀音之前。`auto` 模式仍未實作。 |

類別統計：A 0、B 1、C 0、D 4、E 5、F 8。

## D 類清單（不修，等控制中樞決定）

- `learning-select-preference`
- `learning-forget-request`
- `undo-learned-word`
- `undo-custom-word`

這四例的期望值都假設被動學習、`forget`、tombstone 或自訂詞庫；Rust 的
`decode` 只留回傳 0 的個人化掛鉤（`decode/bias.rs`），沒有 M4 的證據層。

## 剩餘失敗

| 類別 | 案例 | 阻塞原因 |
|---|---|---|
| D | 4 例 | M4 個人化 |
| F | `simplified-*` | `zhconv`／OpenCC tw2s 未移植 |
| F | `mixed-*`、`english-completion`、`cjk-latin-space` | `core/session/mixed.go` 的 ASCII 按鍵推斷未移植 |

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/golden/golden.go` | `crates/jc-cli/src/eval_cmd.rs` | rewritten：只比對 commit 串；案例選項映射到 `SessionConfig` |
| `internal/golden/mixed.go` | `crates/jc-cli/src/eval_cmd.rs` | rewritten：seed 與混合案例的資料準備未搬，D 類不跑 |
| `internal/golden/golden_test.go` | `crates/jc-cli/src/eval_cmd.rs` 的 `mod tests` | rewritten：只驗證 45 例都跑到 |
| `core/engine/engine.go`（選項） | `crates/jc-core/src/session.rs` 的 `SessionConfig` | 部分：toneless、ctrl_punct、rerank、punct、fuzzy 已接；simplified、mixed、english autocomplete 未接 |
| `core/engine/engine.go`（toggle） | `Session::toggle_english` | rewritten |
| `core/session/candidates.go`（翻頁） | `Session::page`、`PAGE_SIZE` | rewritten：保留全部候選，翻頁只移動高亮 |
| `core/session/ctrl_punct.go` | `Key::ctrl`、`Session::press_ctrl` | rewritten：只支援 `Ctrl+,` |
| `core/punct`（固定替身） | `SessionConfig::punct_mark`、`Session::puncted` | rewritten：在最後一個漢字前插入 |
| `core/rerank`（固定替身） | `RerankOptions`、`Session::apply_rerank` | rewritten：成本差值重排 |
| `core/fuzzy/fuzzy.go` | `Session::fuzzy_candidates` | 部分：四組配對、`select` 模式 |
| `core/suggest` | `Decoder::complete`、`Session::accept_suggestion` | rewritten：尾端完整詞補全 |
| `core/session/mixed.go` | `crates/jc-core/src/mixed.rs` | 未接：session 沒有 ASCII 注音按鍵推斷 |
| `core/zhconv`、`internal/simplified` | — | n/a：尚未移植，屬後續 task |
| `core/personal`、`core/vocab` | — | n/a：M4 |
| `core/output/space.go` | `crates/jc-core/src/output/mod.rs` | ported：`CjkLatinSpacer` 已有，session commit 未接 |
