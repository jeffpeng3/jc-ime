# ja-script：片段假名變換與拆解

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/live/live.go` | `crates/jc-core/src/ja/live.rs` | rewritten：新增 `toggle_script`、`unconvert`、`lock_through`、`clause_spans` |
| `core/ja/live` 候選 | `crates/jc-core/src/ja/decode.rs` | rewritten：`JaCandidateSource` 增加 `Hiragana`／`Katakana`／`HalfwidthKana` |
| `core/session` 日文路徑 | `crates/jc-core/src/session/{mod,core,segments}.rs` | rewritten：片段候選、日文游標、F6–F10 |
| — | `crates/jc-core/src/key.rs` | n/a：Go 的按鍵模型沒有對應 F 鍵列舉；Rust 新增 `KeyCode::F(u8)` |
| — | 外殼鍵位映射 | 缺：TSF／macOS／Linux 尚未把實體鍵轉成 `KeyCode::F` |

## 鍵位

| 鍵 | 動作 | 設定 |
|---|---|---|
| `F6` | 游標所在片段 → 平假名（由讀音） | 固定 |
| `F7` | 平假名 ↔ 全角片假名；漢字先回平假名 | `SessionConfig::ja_toggle_key`，預設 `F(7)` |
| `F8` | 半角片假名 | 固定 |
| `F9` | 全角英數 | 固定 |
| `F10` | 半角英數 | 固定 |
| `Space` | 開游標所在片段候選；再按換下一項 | 固定 |
| `Enter` | 套用高亮候選到該片段，不送出整段 | 固定 |
| `←`／`→` | 移動日文組字游標 | 固定 |

## 候選順序

1. 轉換器候選（統計／神經的漢字）
2. 目前詞面
3. 原讀音平假名
4. 全角片假名
5. 半角片假名

`Session::open_ja_candidates` 會把目前詞面移到最前面再顯示。相同詞面只留第一筆；
讀音本身是平假名時不會重複。

## 拆解流程

1. 游標所在片段由 `Controller::clause_spans`＋`Session::active_segment` 決定。
2. `unconvert(span)` 把 `cut` 退到該片段起點，片段與其後已鎖定文節搬回 `tail`。
3. 左文脈仍是 `cut` 之前的鎖定詞面，不會被打亂。
4. 下一個かな或停頓觸發會從新的 `cut` 重新轉換；`generation` 讓舊的補正失效。

## 未接上的部分

- wrapper 顯示：`Snapshot` 已帶 `preedit`、`segments`、`cursor_rune`，但外殼尚未
  送出 F6–F10。
- Home／End 尚未移動日文組字游標。
- 數字鍵直接選日文片段候選尚未接。
