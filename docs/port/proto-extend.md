# proto-extend：補齊 session 需要的協定欄位

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。
Base：`main`（`1225b73`）。
這輪只加型別、欄位與 hook，不改 `Session` 的解碼與候選行為。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/displayed.go` | 無 | n/a：這輪只補 `KeyCode` 的 `Up`／`Down`／`Home`／`End`／`PageUp`／`PageDown`。逐音節 `displayed` 保底表要改 `rebuild_zh`，留給分段呈現任務。 |
| `core/session/candidate_mode.go` | 無 | n/a：協定缺口已補（方向鍵、`Snapshot::navigating`、`cursor_rune`），但「候選顯示中」與「明確選取」的狀態機要改 `Session`，這輪不動。 |
| `core/session/candidate_labels.go` | 無 | n/a：`Snapshot::label` 已補；`common`／`related`／`personal` 的判定需要詞頻與個人證據介面，留給候選標示任務。 |
| `core/session/phonetic_hint.go` | 無 | n/a：`Snapshot::suggestion` 已補；Tab 接受後寫入鎖定的流程要改 `Session`，這輪不動。 |
| `core/session/tone_check.go` | `crates/jc-core/src/zh/input.rs` | rewritten：新增 `cursor()`／`cursor_move()`／`tone_blocked()`／`set_tone_gate()`。上游 `refuseTone` 的「讀音有沒有字」判定需要詞典，留在 session 端；前端只提供擋聲調與游標 hook。 |

## KeyCode

| 新變體 | JSON `kind` | 上游常數 |
|---|---|---|
| `Up` | `up` | `KeyUp` |
| `Down` | `down` | `KeyDown` |
| `Home` | `home` | `KeyHome` |
| `End` | `end` | `KeyEnd` |
| `PageUp` | `page_up` | `KeyPageUp` |
| `PageDown` | `page_down` | `KeyPageDown` |
| `ShiftTab` | `shift_tab` | `KeyBackTab` |

`KeyCode::Tab` 原本就有。`Session::press` 對新鍵加 `_ => {}`，行為與原本相同（忽略）；
CLI 的 golden 具名鍵 `Down`／`Up` 仍對應 `ArrowRight`／`ArrowLeft`，既有題庫不受影響。

## Snapshot

| 欄位 | 型別 | 說明 |
|---|---|---|
| `cursor_rune` | `usize` | UTF-8 位元組位移換算後的 rune 位置；`Snapshot::from_preedit` 與 `Session::snapshot` 都填結尾。 |
| `suggestion` | `Option<String>` | Tab 會接受的補全尾綴；這輪固定 `None`。 |
| `label` | `Option<String>` | 高亮候選的顯示原因；這輪固定 `None`。 |
| `navigating` | `bool` | 是否已進入候選選取模式；這輪固定 `false`。 |

換算工具在 `jc_core::output`：`rune_len()`、`rune_offset()`（超出長度或落在字元中間時取前一個合法邊界）。

## 協定版本

`PROTOCOL_VERSION` 由 1 升到 2。`Response::State` 新增四個欄位，全部有
`#[serde(default)]` 與 `skip_serializing_if`：

| 欄位 | 省略條件 |
|---|---|
| `cursor_rune` | 值為 0 |
| `suggestion` | `None` |
| `label` | `None` |
| `navigating` | `false` |

舊欄位 `preedit`／`cursor`／`candidates` 的名稱、順序與單位不變；`cursor` 仍是
組字字串結尾的字元數，空欄位不寫進 JSON。`Response::state()` 集中做
`jc_core::Snapshot` → `Response::State` 的轉換。

握手照舊比對 `PROTOCOL_VERSION`，版本 1 的客戶端會收到
`ErrorCode::VersionMismatch` 並被關線，不會靜默接受。

## ZhInput hook

| API | 說明 |
|---|---|
| `cells()` | 組字格數：每個完成音節一格，未完成暫存注音算最後一格。 |
| `cursor()` | 目前游標格，夾在 `0..=cells()`。 |
| `cursor_move(delta)` | 移動游標並回傳是否真的移動；任何非 `Ignored` 的按鍵會把游標放回結尾。 |
| `tone_blocked()` | 最近一次按鍵是否擋下聲調；下一個按鍵清掉旗標。 |
| `set_tone_gate(gate)` | 安裝 `ToneGate`；預設 `allow_all_tones`，行為與原本相同。 |

擋下聲調時 `press` 回 `Consumed`，暫存注音完全不動，不會產生新事件變體。

## 測試

| Go 測試 | Rust 測試 |
|---|---|
| `TestSnapshotCursorRune`（新增） | `cursor_rune_converts_byte_offsets_on_char_boundary` |
| `TestSnapshotOptionalFieldsDefault`（新增） | `suggestion_label_and_navigating_default_to_unset` |
| `TestProtocolStateKeepsOldShape`（新增） | `state_omits_empty_optional_fields`、`state_accepts_version_one_shape` |
| `TestProtocolVersion`（新增） | `protocol_version_is_two` |
| `TestKeyCodeNavigationNames`（新增） | `navigation_keys_use_snake_case_names`、`shift_tab_is_distinct_from_tab` |
| `TestZhInputCursor`（新增） | `cursor_move_starts_at_end_and_clamps`、`cursor_counts_completed_syllables_and_pending` |
| `TestZhInputToneForceBlocked`（上游 `tone_check_test.go`） | `tone_gate_can_block_a_tone`、`tone_gate_lets_other_tones_through`、`default_tone_gate_allows_everything` |

## 驗證

- `cargo check --workspace --all-targets`：通過（只剩環境既有的 `-Ctarget-feature` 警告）。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過。
- `cargo test --workspace --no-fail-fast`：全部通過，`jc-core` 496 passed、`jc-proto` 97 passed、`jc-engine` 4 個整合測試 passed。
- `jc-cli eval zh-golden`：45/45。

## 超出 owned paths 的必要修改

只做編譯與欄位傳遞所需的最小修改，沒有改行為：

| 檔案 | 修改 |
|---|---|
| `crates/jc-core/src/session/mod.rs` | `press` 加 `_ => {}`；`snapshot` 填 `cursor_rune` 並用 `..Snapshot::default()` 補三個新欄位。 |
| `crates/jc-engine/src/lib.rs` | `state()` 改呼叫 `Response::state()`；測試改用 `state_of()`。 |
| `crates/jc-engine/tests/stdio.rs`、`tests/socket.rs` | 預期 State 改用 `Response::state()`。 |
| `crates/jc-proto/tests/framing.rs` | State 字面補新欄位。 |
