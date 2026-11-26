# language-branches：依語言／模式分岔的行為盤點

狀態：只調查，未改正式程式。基準 `main` `6d2e3a8`，worktree 分支 `lang-branches`。

範圍：`crates/jc-core/src/session/**`、`candidate*.rs`、`crates/jc-tsf/src/{service,session,keys}.rs`、
`crates/jc-engine/src/lib.rs`。`jc-core` 的 `Mode` 已經只剩 `Mixed`，所以真正的分岔軸是
**lane**（`zh_lane`／`in_ja_lane()`／mixed）、**TSF 外殼模式**（`english`／legacy `japanese`）、
以及 **候選模式**（`candidate_window`、`zh_window_last`、`menu`）。

## 先講結論

1. `Composition` 已經把「哪一段是使用者固定的」收成一份語言無關模型，但**按鍵、游標、候選套用**
   仍是三套平行狀態機。`docs/port/m1-mixed.md` 把 `key.go` 的 KeyPolicy 標成 n/a，這個缺口就是
   本輪看到的主要來源。
2. 最多餘的重複在 `finish_composition()` 與 `choose_candidate()`：同一個「Enter 先套候選」規則
   寫了 5 次，同一個「套用候選」動作寫了 6 條路徑。CHANGELOG 0.1.26／0.1.27 的 Enter、第一候選、
   前導變體問題都從這裡來。
3. TSF 仍把 `english` 與 `japanese` 當成輸入模式，core 已經把三者視為同一條混打管線。結果是
   F6–F10／Convert 到不了 core、日文文節樣式被剝掉、英文補全與注音 suggestion 共用同一個
   `english_pending` 旗標。
4. 真正必要的差異只有三件：注音的音節與聲調、日文的羅馬字與文節轉換、字面的 raw input。
   這三件可以用同一個 lane 介面表達，不需要在每個按鍵處理常式重新分派。
5. 最快的低風險清理是移除永遠成立的 `Mode::Mixed` 條件與死欄位 `candidate_kind`，但這不會修 bug；
   真正有價值的是下面「最值得先做的 3 件事」。

## 總表

必要性：`必要` 表示語言本身的需求；`部分` 表示概念必要但實作重複；`多餘` 表示可刪。
可否統一：`是` 表示可收成一份資料或一條路徑；`部分` 表示只收共用骨架；`否` 表示不建議現在動。

| 組 | 位置 | 條件 | 各語言行為 | 必要性 | 可否統一 | 風險 |
|---|---|---|---|---|---|---|
| 分派 | `crates/jc-core/src/session/core.rs:279` `press_char` | `in_ja_lane()`、`zh_lane`、`is_han`、其餘 | 日文走 `RomajiInput`；注音走 `ZhInput`；漢字直接字面；其餘進 `AsciiInput` 後試日文推斷 | 必要 | 部分：用 `Lane` enum 委派，不在這裡 if/else | 高：所有輸入都經過 |
| 分派 | `crates/jc-core/src/session/core.rs:446,468` | `mixed_zh_lane_starts/continues` | 只有注音 lane 需要判斷「這一鍵還打得成注音嗎」；英文／日文靠 heuristic 退出 | 必要 | 部分：改成統一 planner 的判定結果，不再另寫一套 heuristic | 中高：混打切分邊界 |
| 分派 | `crates/jc-core/src/session/core.rs:502,549,606` | 離開注音 lane 的三個函式 | 都把音節還原成按鍵、清狀態、`ascii.replay`；只差在是否附加按鍵、是否凍結待定音節、是否回傳字面 span | 部分 | 是：抽一個 `reset_to_mixed(keys, literal_span)` | 中：漏清一個欄位就殘留 |
| 分派 | `crates/jc-core/src/session/mixed.rs:1108` | `try_japanese_run` | 2 個以上小寫、組不成注音、不是英文詞或前綴、不是長英文字面時，整段改走日文 | 必要 | 部分：併進 Router／planner 一次決定，不要每鍵事後改 lane | 中：英日邊界 |
| 按鍵 | `crates/jc-core/src/session/core.rs:824` `space` | 三個 lane | 注音：完成 pending 或字面空白，並設 `zh_space_closed`；日文：只插字面空白；mixed：`AsciiInput::space` 可能把空白當一聲 | 部分 | 部分：`Lane::space`，空白政策只留一份 | 中：`ji3 hello`、`5pm `、`rishireej/ ` |
| 按鍵 | `crates/jc-core/src/session/core.rs:1208` `backspace` | `literal_tail`、日文、注音、mixed、一般字面 | 字面尾巴刪一個 rune；日文退羅馬字單元；注音退暫存或整顆音節；mixed 退 rune 或片段 | 必要 | 部分：`Lane::backspace` 回傳是否消耗 | 高：游標與重建順序 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1250` `escape` | 字面尾巴、候選窗、注音 pending、其他 | 字面尾巴直接清整段；候選窗先關；注音只清未完成音節；其他清整段 | 部分 | 是：固定優先序 `候選窗 → lane.pending → 整段` | 中：太早清掉使用者文字 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1292` `delete_forward` | mixed、注音、其他 | mixed 刪片段或按鍵流區間；注音刪後一個音節；日文沒有刪除動作，只關候選窗 | 日文那段是漏洞，非必要 | 是：`Lane::delete_forward` | 中：新增日文刪除行為 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1341` `down` | 日文、mixed、注音 pending、注音音節 | 日文開整段或文節候選且游標不動；mixed 在游標右側 rune 開候選；注音在 pending 時完全不動；注音音節開那一格 | 必要：候選範圍不同 | 部分：`Lane::candidates_at_cursor()` 回傳範圍與目標 | 中高：日文 `↓` 語意 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1391,1406` `up`／`page_step` | 無語言分支 | 三個 lane 共用同一份候選走訪 | 不適用 | 已統一 | 低 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1492` `candidate_edge` | 日文、mixed、注音 | 日文移 byte 游標；mixed 移 rune；注音移音節格 | 必要：游標單位不同 | 是：統一的 `move_cursor()` 經 lane 換算 | 中：Home／End 位置 |
| 按鍵 | `crates/jc-core/src/session/core.rs:1910` `navigate` | 候選介面、日文、mixed、注音 pending、注音音節、其他 | 日文移 `ja_cursor`；mixed 移 rune；注音 pending 先凍結成字面再交 mixed；注音移音節；其他在候選間移動 | 必要：單位不同 | 是：同上，注音 pending 由 lane 決定 | 高：CHANGELOG 0.1.25「注音未打完移動游標變成日文」 |
| 按鍵 | `crates/jc-core/src/session/mod.rs:807` `Tab` | 字面尾巴、`navigating`、mixed 英文補全、其他 | 英文 raw input 沒有補全時 no-op；選取模式換下一項；mixed 先試英文補全；否則注音補全或下一項 | 部分 | 是：`Lane::tab()` 回傳「接受／下一項／no-op」 | 中：Tab 被吃掉或補錯字 |
| 按鍵 | `crates/jc-core/src/session/mod.rs:720,874` 字面尾巴 | `literal_tail` 非空 | Backspace 只刪一個 rune；Esc 清整段；Enter 一起送出；Tab 沒有 suggestion 時 no-op | 必要：raw input | 部分：變成 `LiteralLane`，與其他 lane 同介面 | 中：前面中文被重讀 |
| 按鍵 | `crates/jc-core/src/session/segments.rs:320` `segments_press` | 在 lane 分派前執行，只檢查 `ascii.is_empty()` | 聲調鍵、toneless 斷音、孤立字面空白都在這裡先處理；日文／英文 lane 也會經過 | 注音必要，其他 lane 多餘 | 部分：先判斷 `lane == Zh`，其他直接跳過 | 中：日文／英文的數字被當聲調 |
| 按鍵 | `crates/jc-core/src/session/input_style.rs:737` 全形空白 | `in_zh_lane()` | 只有注音 lane 的空白會轉全形；mixed 與日文走一般空白 | 語言無關設定被 lane 限制 | 是：用 `composing()` 或 lane 能力判斷 | 低中：全形空白在英日失效 |
| 候選 | `crates/jc-core/src/session/core.rs:686` `choose_candidate` | mixed choice、日文、注音 edit、`zh_window_last`、mixed English、menu／一般 | 六條套用路徑：重播按鍵流、換文節、鎖定音節、只換最後一格、英文補完、插入字面 | 套用機制必要，路徑重複 | 是：資料化成 `CandidateAction`，lane 只提供 apply hook | 高：鎖定、重播、學習順序 |
| 候選 | `crates/jc-core/src/session/core.rs:1590` `finish_composition` | mixed choice、`zh_window_last`、日文、注音 edit、任何候選窗 | 五段「Enter 先套高亮候選，關窗，組字留著」的實作；選單除外 | 兩段式必要，五份實作多餘 | 是：合併成一個 `if actionable_candidate` 再走 lane finalize | 高：雙重送出、前導變體例外 |
| 候選 | `crates/jc-core/src/session/core.rs:655` `select_by_digit` | 注音 pending、mixed navigating、menu | 注音 pending 時數字是聲調，不能選字；mixed 只有 `navigating` 才用數字；選單例外 | 部分：聲調是真的需求 | 部分：統一 `CandidateMode` 規則 | 中：聲調鍵被候選吃掉 |
| 候選 | `crates/jc-core/src/session/core.rs:2474` `rebuild` | 日文、注音、mixed | 三後端各自重建；日文保留候選；注音重設 `literal_offset` | 必要：後端不同 | 部分：`Lane::rebuild()` 加 `candidates_still_valid()` | 中：候選閃爍或殘留 |
| 候選 | `crates/jc-core/src/session/core.rs:2566,2958`、`session/mixed.rs:2095` | 三個 rebuild | 三套 rank、literal 候選、模糊音／音近、去重、截斷、顯示字排第一 | 轉換必要，組裝重複 | 是：共用 `CandidateSet` builder | 中：排序、名額、成本 |
| 候選 | `crates/jc-core/src/session/mixed.rs:2149` | mixed 候選 | 英文字面、全形變體、注音讀音、英文拼字建議；`long_literal` 與注音前綴各有特例 | 部分 | 部分：候選來源資料化 | 中：英文建議與數字鍵 |
| 顯示 | `crates/jc-core/src/session/core.rs:2392` `displayed_base` | 注音 lane、mixed、其他候選窗 | 注音／mixed 顯示 `base`；日文與一般候選窗把高亮候選放進組字區 | 顯示預覽不一致 | 是：候選只預覽在 target，不再取代整段 | 低中：日文預覽 |
| 顯示 | `crates/jc-core/src/session/core.rs:2346` `current_text` | menu、mixed、completion、edit、注音、候選 | 多種文字來源；送出與畫面可能各算一次 | 部分 | 是：`display_text()` 與 `commit_text()` 單一來源 | 中：送出字與畫面不一致 |
| 顯示 | `crates/jc-core/src/session/core.rs:1521` `snapshot_suggestion` | 字面尾巴、mixed 英文、注音 | 字面尾巴一律沒有 suggestion；mixed 英文用 `AsciiInput`；其餘用注音補全 | 必要：來源不同 | 部分：`Lane::suggestion()` | 低中 |
| 顯示 | `crates/jc-core/src/session/core.rs:1548` `highlighted_label` | 只注音 lane、非 menu | 只有注音候選有 personal／related／common 標籤 | 目前只實作注音，非語言本質 | 是：標籤來源資料化，任何 lane 可用 | 低 |
| 選單 | `crates/jc-core/src/session/core.rs:752,1256,1750,1830` | `menu.is_some()` | 選單候選一次送出；Esc／Backspace 只關選單；不學習；不顯示 label | 選單是 UI 模式，立即插入是政策 | 是：`CandidateAction { commit_on_apply: true }`，刪掉散落判斷 | 中：Enter 重複插入 |
| 學習 | `crates/jc-core/src/session/core.rs:2021`、`session/mod.rs:968` | 注音、日文、mixed | 三套學習鍵；`commit` 只在 mixed 非注音／日文時 flush ascii | 必要：資料鍵不同 | 部分：`Lane::learn()`、`Lane::flush_pending()` | 中：重複學習或漏學 |
| 顯示 | `crates/jc-core/src/session/input_style.rs:360` `composing` | 只算注音 lane 的 pending／syllables | 風格鍵、全形、IMM32 word-end 用這個值判斷「是否在組字」 | 部分：其他 lane 也需要同一個問題的答案 | 部分：`composing()` 改問 lane | 中：全形標點與 word-end |
| TSF | `crates/jc-tsf/src/service.rs:1734` `wants_key` | English、Japanese、composing、Chinese idle | 英文直接 false；日文 idle 只收 printable；composing 用 `composing_wants`，英文的 `Down` 例外；中文 idle 只收 `idle_key` | 部分 | 部分：改成 capability 或「先送 core，依 handled 決定」 | 高：吃鍵或重複輸入 |
| TSF | `crates/jc-tsf/src/service.rs:1500` `english_key_policy` | 外殼 `english` | 英文模式自己處理 literal、Enter／Backspace／Esc／Tab、補全、IMM32 word-end | 與 core 的 mixed English 重複 | 部分：printable 一律交 core `literal`／`key`，只留 IMM32 word-end | 高：宿主相容與 Caps Lock |
| TSF | `crates/jc-tsf/src/service.rs:1599` | `english_pending` 且字母 | `EnglishKey::Eat(true)` 直接吃鍵，沒有送引擎 | 疑似 bug | 是：先送 core，再依 `handled` 決定是否吃 | 中：英文補全無法續打 |
| TSF | `crates/jc-tsf/src/service.rs:354,375,381,389` | `session.english()`／`japanese()` | `vocab_keys`、`japanese_key`、Ctrl 標點、簡體熱鍵都依語言互斥 | 部分：Ctrl 標點必要，簡體與 vocab 與語言無關 | 部分：只保留真正語言相關的 gate | 低中 |
| TSF | `crates/jc-tsf/src/service.rs:757,879` | `present_in_edit` | `show` 只看 composing＋navigating；`candidate_kind` 直接傳空字串 | 已語言無關；`kind` 是死欄位 | 是：刪 `kind` | 低 |
| TSF | `crates/jc-tsf/src/session.rs:446,487` | `state_to_snapshot_value` | 任何日文片段就 `japanese`；全部英文才 `english`；任何 suggestion 都變成 `english_pending` | 把片段語言與外殼模式混在一起 | 部分：拆成 `mode`（外殼）與 `languages`（片段） | 中高：`su`＋數字、英文補全 |
| TSF | `crates/jc-tsf/src/session.rs:1832` | `adopt_engine_mode` | key／literal／idle 一律用外殼 `english`；`japanese` 用 legacy capability 覆寫成 false | 與 core 已無日文模式矛盾 | 是：英文用外殼，日文用片段與 capability | 中：F6–F10、文節樣式 |
| TSF | `crates/jc-tsf/src/session.rs:1924` | 英文組字中的 `Up/Down/PageUp/PageDown` | 外殼直接吞掉，不送引擎 | 與 core 候選政策重複 | 部分：交給 core 回 `handled=false` | 低 |
| TSF | `crates/jc-tsf/src/session.rs:1967` `toggle` | 外殼英文切換 | 外殼自己切，core 的 `toggle_english()` 只回快照 | 目前架構必要 | 是：刪掉 core／engine 的 toggle 死路徑 | 低 |
| TSF | `crates/jc-tsf/src/keys.rs:297` `composing_wants` | English 的 `Down` | 只有英文的 `Down` 不給 core，其他具名鍵都給 | 可疑特例 | 是：英文 `Down` 也交 core，core 回 handled | 低 |
| TSF | `crates/jc-tsf/src/session.rs:1609` | `start_mode` | `start_mode` 只有 `english`／`zhuyin`；`japanese` 分支還需要 `japanese_cap`，jc-engine 從不廣告 | 日文開機分支永遠不成立 | 是：改用 capability 或 core 的 mixed 設定 | 低中 |
| 死碼 | `crates/jc-core/src/lang.rs`、`session/mod.rs:816,823`、`core.rs` 16 處 | `config.mode == Mode::Mixed` | 永遠成立 | 多餘 | 是：刪條件，保留 Mixed | 低 |
| 死碼 | `crates/jc-core/src/composition.rs:42` | `Lang::from_mode` | 永遠回 `None` | 多餘 | 是：刪 | 低 |
| 死碼 | `crates/jc-tsf/src/session.rs:1040` | `candidate_kind` | jc 沒有生產者，UI 也沒有消費者 | 多餘 | 是：刪欄位或補 producer | 低 |
| 死碼 | `crates/jc-tsf/src/service.rs:375` `japanese_key` | `session.japanese()` | jc-engine 不廣告 `japanese`，F6–F10／Convert 永遠到不了 core | 死分支，但造成功能不可達 | 是：改成 mixed 能力，不再用語言模式 gate | 中 |

## 必要與多餘

### 真正必要的差異

| 差異 | 為什麼必要 | 現在的位置 |
|---|---|---|
| 注音音節與聲調 | 一個音節是多個按鍵，游標與刪除以音節為單位；數字是聲調鍵 | `zh::input`、`syllables`、`cursor`、`segments_press` |
| 日文羅馬字與文節 | 羅馬字有未完成前綴，轉換以文節為單位，F6–F10 與 Convert 有語意 | `ja::input`、`live`、`rebuild_ja` |
| 字面 raw input | 英文與 Shift 字面不應被前面的注音重新規劃 | `literal`、`literal_tail`、`AsciiInput` |
| 候選套用機制 | 注音要鎖定音節、日文要換文節、mixed 要重播按鍵流 | `choose_candidate` 的分支 |

### 多餘的重複

| 重複 | 位置 | 收斂方式 |
|---|---|---|
| Enter 先套候選的 5 段實作 | `finish_composition` | 一個 `CandidateAction` 判斷 |
| 候選套用的 6 條路徑 | `choose_candidate` | 同上，lane 只提供 apply hook |
| 三份 rebuild 的候選組裝 | `rebuild_zh`、`rebuild_ja`、`rebuild_mixed*` | 共用 `CandidateSet` builder |
| 三份 lane 離開／重置 | `leave_zh_lane_for_mixed`、`freeze_pending_zhuyin_for_mixed`、`flush_zh_lane_for_mixed` | 一個 `reset_to_mixed` |
| 三種游標單位換算 | `cursor_byte`、`navigate`、`candidate_edge` | 一個 canonical rune 位移加 lane 映射 |
| TSF 英文按鍵政策與 core 英文 lane | `english_key_policy`、`wants_key`、`Session::key` | printable 統一送 core，只留 IMM32 特例 |
| `suggestion` 與 `english_pending` | `state_to_snapshot_value` | 分成 `suggestion_kind`／`pending_language` |
| 語言 gate 與 capability gate | `vocab_keys`、`simplified_hotkey`、`japanese_key` | 只用 capability，不用語言模式 |

## 分組建議

### G1：候選套用收成 `CandidateAction`

合併成什麼：每個候選帶一個資料化動作，`choose_candidate` 只負責找 action 與套用。

```text
CandidateAction {
  typed_span,          // 要取代的原始按鍵範圍，沒有時為 0..0
  rune_span,           // 要取代的顯示範圍
  insert, leftover,    // 插入字面與尾巴
  cursor,              // Start / AfterFirst / AfterInsert / TokenEnd
  force_literal,
  syllable_edit,       // 注音片段編輯，沒有時 None
  learn,               // 這次要不要學習
  commit_on_apply,     // 選單為 true，一般候選為 false
}
```

`choose_candidate` 只做三件事：找 action、呼叫 lane 的 `apply_action`、收尾。
`finish_composition` 只做一件事：候選窗有 action 就套用並保留組字，否則 lane finalize。

要改的檔：`session/core.rs` 的 `choose_candidate`／`finish_composition`、
`session/mixed.rs` 的 `MixedChoice`／`apply_mixed_choice`、`session/mod.rs` 的 `ChoiceCursor`。

可能壞什麼：注音鎖定與 `Composition` 的 User 標記、日文文節 ownership、字面重播的 span、
選單的一次送出。建議先把「套用」與「學習」分開，避免一次改太多。

### G2：候選清單共用 `CandidateSet` builder

合併成什麼：lane 只提供原始 hypotheses 與 lane 專屬 extras，下面五步只寫一次：

1. 排序：nbest、個人學習、rerank。
2. 插入 lane 專屬候選：注音模糊音／音近、日文 POS、英文拼字建議。
3. 去重與截斷。
4. 把顯示字排到第一。
5. 產生 `CandidateAction` 與 `Candidate` 的平行清單。

要改的檔：`session/core.rs` 的 `rebuild_zh`、`rebuild_ja`、`session/mixed.rs` 的
`rebuild_mixed_candidates`／`rebuild_mixed_zh_candidates`／`fill_zh_candidates`。

可能壞什麼：候選成本、名額保留、英文建議數量、模糊音插入位置。這些都有測試，
建議先加一個「三個 lane 的共同契約」測試再動。

### G3：按鍵政策收成一份 `KeyPolicy`

合併成什麼：`press()` 只做一次決策，lane 不再各自攔鍵。

```text
priorities:
  1. 候選介面：Esc / Enter / Tab / 方向鍵 / 數字鍵
  2. lane 專屬：注音聲調、日文 F6–F10、字面 raw input
  3. 全域：清組字、提交、忽略
```

每個 lane 只需要回答：`press_char`、`space`、`backspace`、`delete_forward`、`tab`、
`move_cursor`、`candidates_at_cursor`、`suggestion`。

要改的檔：`session/mod.rs` 的 `press`、`session/core.rs` 的 `space`／`backspace`／`escape`／
`delete_forward`／`down`／`navigate`／`finish_composition`、`session/segments.rs` 的
`segments_press`。

可能壞什麼：按鍵被吃掉、Esc 層級、數字鍵是聲調還是選字、Tab 是補全還是換候選。
這是最大的一步，建議先做 G1，讓「套用」穩定後再搬按鍵。

### G4：游標 canonical 化成 rune 位移

合併成什麼：內部只存一個 `cursor_rune`，lane 提供 `to_lane_cursor`／`from_lane_cursor`。
注音的音節格、日文的 byte 位移、mixed 的 rune 位移都變成推導值。

要改的檔：`session/segments.rs` 的 `cursor`／`cursor_byte`、`session/core.rs` 的
`navigate`／`candidate_edge`／`delete_forward`／`backspace`、`session/mixed.rs` 的
`mixed_target_at_cursor`。

可能壞什麼：片段邊界、`active_segment`、音節鎖定、候選 target。這一組風險最高，
先加不變量測試再動。

### G5：TSF 只保存「外殼模式」，語言交給 core

合併成什麼：

- 外殼只保留 `english_on: bool` 與 `caps_english: bool`。
- `english`／`japanese` 不再從 `segments[].mode` 推導成輸入模式。
- 日文能力只看 capability，不再看 `initial_mode == "japanese"`。
- `suggestion` 增加來源欄位，`english_pending` 只由英文來源設定。
- printable 一律送 core；外殼只保留 IMM32 word-end、Caps Lock 與無法送 core 的鍵。

要改的檔：`jc-tsf/src/service.rs` 的 `wants_key`／`english_key_policy`／`japanese_key`、
`jc-tsf/src/session.rs` 的 `state_to_snapshot_value`／`handle_retrying`／`ensure_open`／`toggle`、
`jc-tsf/src/keys.rs` 的 `composing_wants`。

可能壞什麼：IMM32 程式的順序、Caps Lock 大小寫、WordEnd、重連後模式還原。
這一組要保留現有 host test，先關掉 `english_key_policy` 的 `Eat(true)` 路徑再逐項搬。

### G6：顯示與提交單一來源

合併成什麼：`display_text()` 與 `commit_text()` 都從同一份 `CandidateAction` 與 lane 狀態算，
不再各自處理 menu、completion、edit、candidate。

要改的檔：`session/core.rs` 的 `current_text`／`displayed_base`／`base_with_highlight`／
`insert_literal`／`compose_literal`。

可能壞什麼：重複提交、預覽消失、前導符號插入位置。

### G7：死分支清理

合併成什麼：刪掉 `Mode::Mixed` 的永遠成立條件、`composition::Lang::from_mode`、
`candidate_kind`、core 的 `toggle_english` 死路徑、legacy `japanese` 模式。

要改的檔：`lang.rs`、`session/mod.rs`、`session/core.rs`、`composition.rs`、
`jc-tsf/src/session.rs`、`jc-tsf/src/service.rs`。

可能壞什麼：幾乎沒有行為改變，但要先確認沒有舊 wrapper 依賴 wire 欄位。

## 最值得先做的 3 件事

### 1. 把 Enter 與候選套用收成 `CandidateAction`（G1）

範圍：`finish_composition`、`choose_candidate`、`MixedChoice`。

為什麼先做：同一個規則寫了 5 次，任何一次漏改就是使用者看得到的錯。這一步不需要改 lane 的
轉換演算法，只需要把「套用什麼」變成資料。

會一起消失的 bug 類型：

- 候選窗開著時 Enter 直接送出整段，或第一次 Enter 送出、第二次才套用。
- 選單 Enter 重複插入，前導變體窗套用兩次。
- 高亮候選與組字區顯示不一致（CHANGELOG 0.1.26／0.1.27 的「顯示我、第一項是捰」）。
- 片段選字換錯段，或換字後游標跳到整段結尾。
- 日文文節與注音音節的 Enter 語意分叉。

預估：中等。`jc-core` 兩個檔案，主要風險在鎖定與學習順序。

### 2. TSF 只保存外殼模式，日文改用 capability（G5）

範圍：`english_key_policy`、`wants_key`、`state_to_snapshot_value`、`handle_retrying`。

為什麼先做：現在 core 已經只有 Mixed，TSF 仍用 `japanese` 這個 legacy 模式 gate。jc-engine
從不廣告 `japanese`，所以 F6–F10／Convert 根本到不了 core，日文文節樣式也會被剝掉。

會一起消失的 bug 類型：

- 日文 F6／F7／F8／F9／F10 沒有反應（目前被 `japanese_key` 擋住）。
- 自動推斷的日文沒有文節樣式，`clause_display` 永遠 false。
- 英文補全時字母被 `Eat(true)` 吃掉，無法續打。
- 注音 suggestion 被當成 `english_pending`，`su` 後面按數字被英文補全搶走。
- 重連後引擎模式與外殼模式互相覆蓋（`mode_memory=shared` 的已知限制）。

預估：中等偏高。要保留 IMM32 host test，先處理 `english_pending` 與 printable 路徑。

### 3. 把 space／backspace／delete／escape 收成 `KeyPolicy`（G3）

範圍：`press`、`space`、`backspace`、`escape`、`delete_forward`、`segments_press`。

為什麼先做：這幾個鍵是使用者每天按的，現在每個 lane 各寫一次，且日文 `Delete` 根本沒有動作。
先把優先序固定，再逐 lane 補行為，比繼續在每個 handler 加 if 安全。

會一起消失的 bug 類型：

- 日文組字按 Delete 沒反應，只關候選窗。
- 注音、日文、英文的 Backspace 刪除單位不一致，或把已送出的字多刪一個。
- Esc 在字面尾巴、候選窗、注音 pending 三種狀態下優先序不同，偶爾整段消失。
- `ji3 hello` 這類「注音後空白再打英文」的邊界錯亂。
- Tab 在英文模式被吃掉，或在注音模式補錯字。
- 日文／英文 lane 的數字被 `segments_press` 當成注音聲調。

預估：大。建議排在 G1 之後，先讓候選套用穩定，再搬按鍵優先序。

## 已經統一、不要重做的部分

| 項目 | 現況 | 文件 |
|---|---|---|
| 候選窗版面 | 10 列、4 欄、直排編號、數字鍵只在目前欄 | `docs/port/candidate-unify.md` |
| 候選編號 | 核心 `selection_key`，英文候選也有編號 | 同上 |
| Enter 兩段式 | 一般候選先套用、選單一次送出、前導變體例外 | 同上 |
| 片段所有權 | `Composition` 的 `Auto`／`User`，注音與日文共用 | `docs/port/composition-model.md` |
| 中英模式 | 外殼保存，引擎 `toggle` 只回快照 | `docs/port/mode-unification.md` |
| 日文逐字／延遲整句 | `live` 控制器與 `apply_live_plan` | `docs/port/ja-live-rewrite.md` |

## 待實測或待確認

1. `Request::Mode` 在 `jc-engine` 落到 `Unsupported`（`lib.rs:802`），但 TSF 的
   `restore_requests` 仍會送 `mode`（`session.rs:893`）。重連時英文模式與 start_mode 不同時，
   `ensure_open` 是否會因此失敗，需要實機或 mock 驗證。
2. `english_key_policy` 的 `Eat(true)`（`service.rs:1599`）在 suggestion 存在時是否真的吃掉字母。
   程式路徑看起來會，但現有測試只驗 `english_wants_key` 的預測，沒有驗實際送鍵。
3. `segments_press` 在日文 lane 收到數字時的行為。注音 gate 先跑，可能讓數字停在字面，
   沒有結束日文推斷、交回混打。
4. `state_to_snapshot_value` 把任何 `suggestion` 都映射成 `english_pending`。注音補全與英文
   補全目前共用同一個旗標，這是 `su` 問題的來源。
5. `japanese` capability 目前沒有任何生產者。若確認短期內不接 legacy 引擎，應直接刪除
   TSF 的日文模式分支，不要留死碼。

## 建議的落地順序

```text
G1 CandidateAction
  ↓
G3 KeyPolicy（space/backspace/delete/escape）
  ↓
G5 TSF 模式與 capability
  ↓
G2 CandidateSet builder
  ↓
G4 游標 canonical 化
  ↓
G6 顯示與提交單一來源
  ↓
G7 死分支清理
```

G1 與 G3 會碰到同一批測試，建議同一個 worktree 分兩個 commit；G5 單獨一個 worktree，
避免 Windows host test 與 core 測試互相干擾。
