# Event flows：TSF、engine 與 core 的實際事件路徑

狀態：以 `main` `3ca8911` 的程式為準，分支 `event-flows` 只新增文件。這份文件不從舊設計稿推論，每個 sequence diagram 的函式名都能在 repo 找到；不確定的地方標 `（待確認）`。

## 先看結論

一次按鍵會經過三個決策層，責任邊界如下：

1. **TSF 決定「這一鍵要不要先攔」**。`OnTestKeyDown` 只做預測，`OnKeyDown` 才送 IPC。實際方法在 `crates/jc-tsf/src/service.rs` 的 `wants_key`、`english_key_policy`、`handle_key`。
2. **engine 決定「這個請求要落到哪個 session / 哪個 op」**。快捷鍵在 engine 先查 `settings_keymap`，一般按鍵才交給 `Session::press`。位置在 `crates/jc-engine/src/lib.rs` 的 `handle`。
3. **core 決定「按鍵語意、候選、游標、刪除、學習」**。入口是 `Session::press`，候選套用是 `CandidateAction`，顯示與提交共用 `display_text`／`commit_text`。

TSF 目前沒有把中英模式送給 engine。`english` 是外殼狀態，core 只看到按鍵與片段；`Request::Mode` 已從協定刪除，TSF 也不再送它。

## 1. 元件圖

```mermaid
flowchart LR
    subgraph HostProc["宿主行程 例如 notepad.exe"]
        HostThread["宿主 UI thread"]
        TSF["jc-tsf.dll<br/>ITfTextInputProcessor"]
        TSession["TSF Session<br/>IPC client 與 replay log"]
        EditSession["EditSession<br/>ITfEditSession"]
        LocalWindow["LocalWindow<br/>Direct2D 候選窗"]
    end

    subgraph EngineProc["jc-engine.exe 行程"]
        Supervisor["--supervise 監督者"]
        Accept["accept 迴圈<br/>建立 pipe instance"]
        Conn["jc-engine-conn<br/>每連線一個 worker thread"]
        SettingsWatch["jc-engine-settings<br/>每秒看 settings.json"]
    end

    subgraph SettingsProc["jc-settings-ui.exe 行程"]
        SettingsUI["GPUI 設定程式"]
    end

    Pipe[("named pipe<br/>jc-ime-SID")]
    SettingsFile["settings.json"]
    PersonalFile["personal 學習檔"]

    HostThread --> TSF
    TSF --> TSession
    TSF --> EditSession
    TSF --> LocalWindow
    TSession -->|"Request / Response<br/>4-byte LE + JSON"| Pipe
    Pipe --> Conn
    Supervisor -->|"spawn 子行程"| Accept
    Accept -->|"移交 pipe handle"| Conn
    SettingsUI -->|"寫入"| SettingsFile
    SettingsFile -->|"mtime 變更"| SettingsWatch
    SettingsWatch -->|"熱重載 keymap 與 SessionConfig"| Conn
    Conn -->|"flush"| PersonalFile
```

補充：

- `jc-tsf.dll` 載入宿主行程，`TextService` 是 COM 物件，掛在宿主收到按鍵的 UI thread。候選窗 `LocalWindow` 也在同一個行程自繪，沒有獨立候選窗程序或 pipe。
- `jc-engine.exe --supervise` 是常駐監督者，子行程才是 `--socket` 服務者。每條 pipe 連線一個 worker thread，session 屬於連線，不跨 thread 共用。
- `jc-settings` 是純資料 crate，設定程式與 engine 共用。`jc-settings-ui` 只寫 `settings.json`，engine 的 watcher 熱重載。
- `EditSession` 是 TSF 的 `ITfEditSession` 回呼，跑在宿主提供的 edit cookie 裡。`edit::apply` 用它更新組字區、提交文字與游標。

## 2. wire 訊息一覽

訊框格式：4-byte 小端長度 + JSON，上限 `jc_proto::frame::DEFAULT_MAX_FRAME`。`Request`／`Response` 的 serde tag 是 `op`，snake_case。以下以 `crates/jc-proto/src/lib.rs` 的 variant 為準。

### 2.1 `Request`

| variant | 誰送 | 誰收 | 什麼時候 |
|---|---|---|---|
| `Hello { protocol, client }` | TSF `Session::ensure_open`，其他 client | engine `handle` | 開 session、重連後第一次請求。版本不符會回 `VersionMismatch` 並關線 |
| `Key { key }` | TSF `handle_key`、`handle_chord`；英文 Enter 與 IMM32 word-end 也走這條 | engine `handle` | 一般按鍵、快捷鍵 chord、英文 Enter、IMM32 word-end |
| `Literal { text, session }` | TSF `handle_literal` | engine `handle` | Shift＋字面、外殼已決定不進注音／かな狀態機的文字 |
| `Snapshot` | TSF `to_jc_request` 的 `display`、重連對齊 | engine `handle` | 取得目前完整狀態，不改組字 |
| `Commit` | TSF `finish`、`complete_via_cuas` | engine `handle` | 明確送出目前組字。Enter 的一般路徑由 engine 直接呼叫 `Session::commit` |
| `Select { index }` | TSF 候選窗滑鼠點選 `select_in_edit` | engine `handle` | 直接套用第 `index` 個候選，和數字鍵共用 `choose_candidate` |
| `Idle { session }` | TSF 停頓計時器 `ja_live_idle` | engine `handle` | 日文逐字変換的停頓觸發；沒有日文組字時是 no-op |
| `Health` | TSF `refresh_bindings`、`learn_mode`、`simplified_hotkey` | engine `handle` | 查 bindings、能力旗標、簡體開關、慢請求診斷 |
| `Close { session }` | TSF `Session::reset`、`Drop` | engine `close_session` | 關閉這條連線的 session，會 flush 個人學習 |
| `OutputSimplified { on }` | TSF `set_output_simplified`、`output_simplified` | engine `handle` | 狀態列切換簡體輸出。只改提交，不改組字預覽 |
| `Ping` | 測試、smoke 或外部 client | engine `handle` | 存活檢查，回 `Acknowledged` |
| `ApplySettings { settings }` | 目前沒有 TSF 生產者；外部工具可送 | engine `handle` | 要求 engine 立刻從設定檔重讀，回 `Health` |
| `Drain` | 升級交接的外部 client | engine `handle` | 等組字結束、flush、斷線，讓外殼重連新版 |
| `Vocab { directive, vocab }` | 目前沒有 TSF 生產者；詞彙工具可送 | engine `handle` | 自訂詞彙指令。目前 engine 未實作，落到 `Unsupported` |
| `ResetPersonal` | 目前沒有 TSF 生產者；外部工具可送 | engine `handle` | 清空個人學習並存檔，回 `Acknowledged` |
| `Shutdown` | 測試、smoke 或外部 client | engine `handle` | 整個引擎行程結束，回目前 snapshot |
| `Unknown` | serde `other` | engine `handle` | 不認得的 op，回 `UnknownOp` |

### 2.2 `Response`

| variant | 誰送 | 誰收 | 什麼時候 |
|---|---|---|---|
| `Ready { protocol, engine, session, bindings }` | engine | TSF `ensure_open` | `Hello` 成功。`bindings` 是 action 到正規化 chord 的表 |
| `State { preedit, cursor, candidates, details, segments, highlighted, cursor_rune, suggestion, label, navigating, rows, columns, handled, menu }` | engine | TSF `parse` | 每一次按鍵、snapshot、idle、select 的完整狀態 |
| `Committed { text }` | engine | TSF `parse` | Enter 或 `Commit` 真的送出文字。`text` 已套用簡體與標點 |
| `Error { code, message }` | engine | TSF `parse`／`to_jc_request` 的錯誤路徑 | 版本不符、未知 op、frame 錯誤、內部錯誤 |
| `Acknowledged { session }` | engine | TSF、外部 client | `Ping`、`Close`、`ResetPersonal` 的成功回報 |
| `Health { info }` | engine | TSF `learn_*` | 能力、bindings、設定版次、慢請求、session 數 |
| `Vocab { vocab }` | 目前沒有生產者 | 詞彙工具 | variant 已定義，但 `Request::Vocab` 在 engine 仍落到 `Unsupported` |

`handled` 是 TSF 要不要吃鍵的權威值。`Some(false)` 用在候選 index 越界與混合英文沒有補全時的 Tab；引擎對組字中的 Backspace／Delete 即使把 preedit 刪空也會明確回 `Some(true)`。

`jc-proto::Response::State` 只有 `suggestion: Option<String>`，沒有補全來源。英文補全在 core 已進候選清單：混合模式打 `hellp` 按 Tab 會開候選窗 `[0] hellp`、`[1] hello`，由一般 `CandidateAction` 套用。英文模式（外殼 `english=true`）的文字輸入零 IPC：printable、Tab、方向鍵、Enter、Backspace 都交還宿主，也不產生組字、補全或候選窗；已註冊的全域 chord 是唯一例外，仍送一個 `Request::Key` 讓 engine 執行 keymap command。

## 3. 逐事件 sequence diagram

participant 對照：`Host` 宿主程式，`TSF` jc-tsf 的 `TextService`，`TSession` TSF 的 `Session`，`Pipe` named pipe，`Engine` jc-engine，`Core` jc-core 的 `Session`，`Edit` TSF EditSession，`UI` 候選窗。

### 3.1 啟動、hello、Ready、bindings 與重連

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Pipe
    participant Engine
    participant Core

    Host->>TSF: Activate
    TSF->>TSF: AdviseKeyEventSink 與 LangBarButton::create
    TSF->>TSession: Session::learn_mode false
    TSession->>Pipe: connect_existing 與 Health
    Pipe->>Engine: Request::Health
    Engine-->>TSession: Response::Health
    TSession-->>TSF: 起始模式與能力
    Note over TSF,TSession: Activate 不主動啟動引擎

    Host->>TSF: OnTestKeyDown
    TSF->>TSF: bound_chord
    TSF->>TSession: maybe_refresh_bindings
    TSession->>Pipe: Health 取得 bindings
    Pipe->>Engine: Request::Health
    Engine-->>TSession: bindings 與 settings_revision
    TSF->>TSF: wants_key
    TSF-->>Host: 要不要吃這一鍵

    Host->>TSF: OnKeyDown
    TSF->>TSession: key_with_flags
    TSession->>TSession: handle_retrying
    TSession->>Pipe: connect
    Pipe->>Engine: Request::Hello
    Engine->>Engine: open_session
    Engine-->>TSession: Response::Ready 與 session id
    TSession->>TSession: ensure_open 記住 id 與 generation
    TSession->>TSession: learn_bindings 與重置 replay 狀態

    TSession->>Pipe: Request::Key
    Pipe->>Engine: Request::Key
    Engine->>Core: Session::press
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSession-->>TSF: Snapshot
    TSF->>TSF: present

    alt 對端關線 引擎重啟
        TSession->>TSession: note_transport_failure 保留 replay log
        TSession->>Pipe: 重連與 Hello
        loop replay log
            TSession->>Engine: Request::Key
        end
    else 單次逾時 對端還活著
        TSession->>TSession: log.clear 與 align_pending
        Note over TSession: 這一次直接放行 下一鍵才重連並用 Snapshot 對齊
    end
```

重點：

- `ensure_open` 只在 `id == 0` 或 generation 改變時重開。重開失敗會把 `id` 歸零，下一鍵再試。
- `Ready.bindings` 與 `Health.bindings` 都會進 `learn_bindings`。`bound_chord` 每 5 秒最多刷新一次，避免每個按鍵都問 engine。
- 重播只重送組字中的請求。TSF 在收到 commit、cancel 或組字結束時清掉 log。

### 3.2 一般可列印鍵

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Pipe
    participant Engine
    participant Core
    participant Edit

    Host->>TSF: OnTestKeyDown
    TSF->>TSF: allowed 與 observe_shift
    TSF->>TSF: english_key_policy
    Note over TSF: None 表示走一般路徑
    TSF->>TSF: wants_key
    TSF->>TSession: available 與 idle_key
    TSF-->>Host: 吃掉這一鍵

    Host->>TSF: OnKeyDown
    TSF->>TSF: allowed 與 commit_before_host_key
    TSF->>TSession: key_with_flags
    TSession->>Pipe: Request::Key
    Pipe->>Engine: Request::Key
    Engine->>Engine: keymap 未命中
    Engine->>Core: Session::press
    Core->>Core: key_preprocess 與 key_policy
    Core->>Core: lane_key Char 與 press_char
    Core->>Core: rebuild
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSession-->>TSF: Snapshot
    TSF->>Edit: edit::apply preedit 與 cursor
    Edit-->>TSF: Applied
    TSF->>TSF: present_in_edit 更新 composing
    TSF-->>Host: snapshot.handled
```

`OnTestKeyDown` 的答案會被 `key_tested` 記住，讓緊接的 `OnKeyDown` 不再問一次；這個紀錄只給同一個 vk 用一次。若 `OnKeyDown` 最後得到 `handled=false`，TSF 回 `false`，宿主收到原始按鍵。

中文 idle 時不是所有 printable 都會進來。`wants_key` 只在 `idle_key` 認定這一鍵是注音起始、聲調鍵或全形字元時回 `true`；其他可列印字元直接放行給宿主。英文模式一律放行，不經引擎。

### 3.3 注音輸入

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant Engine
    participant Core
    participant UI

    Host->>TSF: OnKeyDown j
    TSF->>TSF: wants_key true
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_preprocess 呼叫 segments_press
    Core->>Core: j 不是聲調 回 false
    Core->>Core: key_policy 與 lane_key Char
    Core->>Core: press_char 與 mixed_zh_lane_starts
    Core->>Core: press_zh_char 與 ZhInput::press
    Core->>Core: rebuild 與 rebuild_zh
    Core-->>Engine: Snapshot 含 pending 或 syllables
    Engine-->>TSF: Response::State
    TSF->>UI: present 只更新組字區

    Host->>TSF: OnKeyDown 3
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: segments_press 判定聲調
    Core->>Core: ZhInput::press 完成音節
    Core->>Core: rebuild 與 rebuild_zh
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State

    Host->>TSF: OnKeyDown Down
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_policy 與 candidate_key Down
    Core->>Core: down 與 open_zh_cursor_candidates
    Core->>Core: navigating true 與 candidate_window true
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State
    TSF->>UI: wants_candidate_window true
    UI-->>Host: 顯示候選窗

    Host->>TSF: OnKeyDown 1
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: press_char 與 select_by_digit
    Core->>Core: choose_candidate 與 candidate_action
    Core->>Core: apply_zh_syllable_action 或 apply_insert_action
    Core->>Core: finish_choice
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State
    TSF->>UI: 更新或收起候選窗

    Host->>TSF: OnKeyDown Enter 第一次
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press 與 finish_composition
    Core->>Core: enter_action 與 apply_candidate_action
    Core-->>Engine: Snapshot 仍在組字
    Engine-->>TSF: Response::State
    TSF->>UI: 關候選窗 保留組字

    Host->>TSF: OnKeyDown Enter 第二次
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press 與 finish_composition
    Core->>Core: finalize_lane 與 commit_text 與 pending_commit
    Core-->>Engine: Snapshot 沒有組字
    Engine->>Core: Session::commit
    Core-->>Engine: 提交文字
    Engine-->>TSF: Response::Committed
    TSF->>TSF: present 與 edit::apply 送出
```

注音決策順序：

1. `segments_press` 先處理聲調、聲調選填、孤立字面空白。回傳 `true` 代表這一鍵已被吃掉。
2. `key_policy` 依 `candidate_key`、`lane_key`、`global_key` 的固定順序決策。
3. 數字鍵只有 `candidate_window` 開著才走選字；`select_by_digit` 先找高亮所在頁與欄，再換算絕對 index。
4. Enter 第一次只套用高亮候選，第二次才 `finalize_lane`。選單是例外，套用即送出。

注音補全仍會出現在 `suggestion`；英文補全則同時進了候選清單。TSF 不再區分 suggestion 來源，也不再有 `english_pending`。

### 3.4 英文模式

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF

    Note over TSF: english 是外殼狀態，文字鍵不送 engine；chord 例外
    Host->>TSF: OnTestKeyDown a
    TSF->>TSF: english_key_policy / wants_key
    Note over TSF: 英文模式 printable、Tab、方向鍵、Backspace 一律不處理
    TSF-->>Host: 放行這一鍵

    Host->>TSF: OnKeyDown a
    TSF-->>Host: 放行這一鍵

    alt IMM32 沒有插入點
        TSF->>TSF: english_key_policy 回 Literal 或放行
    end
```

英文模式文字輸入是純 raw：外殼 `english=true` 時 printable、Tab、方向鍵、
Enter、Backspace 都不產生 `Request`，也不建組字、不顯示候選窗；Shift＋空白
同樣直接交給應用程式，不由外殼插入全形空白。`english_key_policy` 只保留
IMM32 raw passthrough；Caps Lock 與其他按鍵交給系統與應用程式。已註冊的
全域 chord（例如 Ctrl+Alt+,）例外，仍會送一個 `Request::Key`，由 engine 的
keymap 執行 command。混合模式的英文補全改走 Tab 開候選窗，不屬於英文模式。

### 3.5 Caps Lock

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Engine
    participant Core

    Host->>TSF: OnKeyDown A 且 Caps Lock 開
    TSF->>TSF: keys::printable 依鍵盤狀態轉出 A
    TSF->>TSF: handle_key caps true
    TSF->>TSession: key_with_flags key a caps true
    TSession->>Engine: Request::Key key a text A caps true
    Engine->>Core: Session::press
    Core->>Core: key.caps 時把字母改回基礎小寫
    Core->>Core: key_policy 與 lane_key 不變
    Core->>Core: press_char 與 press_caps 只改字面大小寫
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSession-->>TSF: Snapshot
```

Caps Lock 不改變按鍵路由，也不切換中英模式。注音模式仍把字母當注音鍵，`Key.caps` 只影響最後的字面大小寫；英文與混打的 Latin 字面由 `press_caps` 決定輸出。Caps Lock 狀態本身不會為了同步模式而送出 health 或任何 `Request`。

### 3.6 日文

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Engine
    participant Core
    participant UI

    Host->>TSF: OnKeyDown r o m a
    TSF->>TSF: wants_key
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: press_char 與 RomajiInput::press
    Core->>Core: rebuild 與 rebuild_ja_live
    Core->>Core: Controller::input
    Core->>Core: apply_live_plan
    Core-->>Engine: Snapshot 含かな 或轉換結果
    Engine-->>TSession: Response::State
    TSF->>TSF: schedule_ja_live_idle
    TSF->>UI: present 更新組字與文節

    TSF->>TSF: ja_live_idle 停頓到期
    TSF->>TSession: Session::idle
    TSession->>Engine: Request::Idle
    Engine->>Core: Session::ja_live_idle
    Core->>Core: Controller::idle
    Core->>Core: apply_live_plan
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSF->>UI: present

    Host->>TSF: OnKeyDown F6 到 F10
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_policy 與 JapaneseTransform 或 ToggleJapaneseScript
    Core->>Core: ja_transform 或 ja_toggle_script
    Core->>Core: Controller::transform_clause 或 toggle_script
    Core->>Core: capture_ja_users 與 finish_ja_script_edit
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State

    Host->>TSF: OnKeyDown Convert
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_policy 與 Down
    Core->>Core: down 與 open_ja_cursor_candidates
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSF->>UI: 顯示整段或文節候選

    Host->>TSF: 候選選字
    TSF->>Engine: Request::Key 或 Select
    Engine->>Core: choose_candidate 或 finish_composition
    Core->>Core: candidate_action 的 JaClause 或 JaWhole
    Core->>Core: apply_ja_action 與 learn_ja_choice
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
```

日文的 ownership 在 core 的 `Composition`：

- `rebuild_ja_live` 先用 `common_prefix_units` 保留共同讀音，再 `retain_units_through` 決定哪些 User 片段不能重轉。
- `Controller::input` 只回傳 `LivePlan`，真正換字在 `apply_live_plan`。
- 刪除後的畫面用 `mark_user_units` 固定成 User 片段，避免 `わし` 被重新轉成 `はし`。
- 文節切分、鎖定與 `clause_spans` 都由 `Controller` 提供；TSF 只收到 `segments[].mode == japanese` 與候選字串，不自己判斷文節。

### 3.7 刪除

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant Engine
    participant Core
    participant UI

    Host->>TSF: OnKeyDown Backspace 或 Delete
    TSF->>TSF: wants_key 回 true
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    alt literal_tail 非空
        Core->>Core: delete_literal_tail_rune
        Core->>Core: 關候選窗 清 suggestion
        Core-->>Engine: Snapshot
    else 一般路徑
        Core->>Core: key_policy 與 DeleteForward 或 Backspace
        Core->>Core: delete_visible
        Core->>Core: 關候選窗 與清 edit 與 highlighted
        Core->>Core: delete_zh_visible 或 delete_ja_visible 或 delete_mixed_visible
        Core->>Core: rebuild
        Core-->>Engine: Snapshot
    end
    Engine-->>TSF: Response::State
    TSF->>UI: 更新候選窗 可能收起
    TSF->>TSF: present 更新組字
```

刪除單位是一個可見 rune，不是一個按鍵或一個音節。若一個可見字元底下涵蓋多個讀音單位，`zh_units_at_rune` 或 `ja_units_at_rune` 會回傳整個範圍，`remove_syllable_range` 或 `delete_units` 一次移除該字元涵蓋的單位；正常一對一時就是刪一個字。

刪除後剩下的字面會留在原本的 lane：混打用 `AsciiInput::replay` 重建剩下的按鍵，日文用 `ja_delete_surface` 與 `mark_user_units` 凍結刪除後的畫面，注音用剩下的按鍵重播未完成音節。這樣後面的字元不會被重新判成注音或かな。

`Shift+Delete`／`Shift+Backspace` 在候選窗開著時先走 `CandidateAction::ForgetCandidate`，不會直接刪字。

### 3.8 候選操作

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant Engine
    participant Core
    participant UI

    Host->>TSF: OnKeyDown Down 或 Up
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_policy 與 candidate_key Down 或 Up
    Core->>Core: down 或 up 與 move_row
    Core-->>Engine: Snapshot navigating true
    Engine-->>TSF: Response::State

    Host->>TSF: OnKeyDown Left 或 Right
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: candidate_key MoveCursor
    Core->>Core: navigate 與 move_column
    Note over Core: 第一次左右鍵先展開四欄 第二次才換欄
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State

    Host->>TSF: OnKeyDown PageUp 或 PageDown
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: key_policy 與 Page
    Core->>Core: page_step
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State

    Host->>TSF: OnKeyDown 數字鍵
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: press_char 與 select_by_digit
    Core->>Core: choose_candidate 與 apply_candidate_action
    Core-->>Engine: Snapshot
    Engine-->>TSF: Response::State

    Host->>TSF: 候選窗滑鼠點選
    TSF->>TSF: select_candidate 與 candidate::request
    TSF->>TSF: RequestEditSession 與 select_in_edit
    TSF->>Engine: Request::Select index
    Engine->>Core: Session::choose_candidate
    Core-->>Engine: Snapshot 或 handled=false
    Engine-->>TSF: Response::State
    TSF->>UI: 更新或收起
```

游標單位：

- `move_row` 用 `CandidateLayout::up`／`down`，上下鍵是線性移動，跨欄、跨頁自然接續。
- `move_column` 用 `CandidateLayout::left`／`right`，左右鍵跳一整欄，同一個列號；單欄時不移動。
- `page_step` 用 `page_size` 換頁，頁碼頭尾相接。
- 注音的左右鍵在候選窗開著但尚未 `navigating` 時是移動組字游標；進入選取後才換候選。

### 3.9 選單

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Engine
    participant Core
    participant UI

    Host->>TSF: OnKeyDown Ctrl Alt 逗號
    TSF->>TSF: bound_chord
    TSF->>TSession: maybe_refresh_bindings
    TSession-->>TSF: symbol_menu
    TSF->>TSession: key_chord
    TSession->>Engine: Request::Key ctrl alt 逗號
    Engine->>Engine: chord_of_key 與 settings_keymap
    Engine->>Core: Session::command symbol_menu
    Core->>Core: open_symbol_menu 與 open_menu
    Core->>Core: rebuild_menu
    Core-->>Engine: Snapshot menu 有值
    Engine-->>TSession: Response::State
    TSession-->>TSF: Snapshot
    TSF->>UI: wants_candidate_window 因 menu 為 true
    UI-->>Host: 顯示分頁列與候選

    Host->>TSF: OnKeyDown Tab
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: menu_key 與 tab 切換
    Core->>Core: rebuild_menu
    Core-->>Engine: Snapshot menu.tab 改變
    Engine-->>TSession: Response::State
    TSF->>UI: 更新分頁

    Host->>TSF: OnKeyDown Enter
    TSF->>Engine: Request::Key
    Engine->>Core: Session::press 與 finish_composition
    Core->>Core: enter_action 與 candidate_action
    Core->>Core: apply_menu_action 設定 pending_commit 與 clear_composition
    Core-->>Engine: Snapshot 沒有組字
    Engine->>Core: Session::commit
    Core-->>Engine: 提交文字
    Engine-->>TSession: Response::Committed
    TSF->>UI: 收起選單
    TSF->>Host: 插入符號或表情
```

選單的 chord 由 engine 的 `settings_keymap` 決定，TSF 只用 `bindings` 決定要不要攔這一鍵。`settings_keymap` 由 `config_watch::reload` 從 `settings.json` 重建，`settings_revision` 改變時 TSF 下一次 `bound_chord` 會刷新本地表。

### 3.10 提交

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Engine
    participant Core
    participant Edit

    alt Enter 注音片段或日文文節
        Host->>TSF: OnKeyDown Enter
        TSF->>Engine: Request::Key Enter
        Engine->>Core: Session::press
        Core->>Core: finish_composition 與 enter_action
        Core->>Core: 套用候選但保留組字
        Core-->>Engine: Snapshot 仍在組字
        Engine-->>TSession: Response::State
        TSF->>Edit: edit::apply 更新預覽
        Note over TSF,Engine: 第二次 Enter 才 finalize_lane 與 commit
    else Enter 直接插入候選或選單
        Host->>TSF: OnKeyDown Enter
        TSF->>Engine: Request::Key Enter
        Engine->>Core: Session::press
        Core->>Core: apply_insert_action 或 apply_menu_action
        Core->>Core: 設定 pending_commit 與 clear_composition
        Core-->>Engine: Snapshot 沒有組字
        Engine->>Core: Session::commit
        Core->>Core: output_text 簡體轉換
        Core-->>Engine: 文字
        Engine-->>TSession: Response::Committed
        TSF->>Edit: edit::apply commit 與結束組字
    else 明確 Commit
        TSF->>TSF: finish commit
        TSF->>TSession: handle commit
        TSession->>Engine: Request::Commit
        Engine->>Core: Session::commit
        Core-->>Engine: 文字
        Engine-->>TSession: Response::Committed
        TSF->>Edit: edit::apply commit
    else IMM32 程式
        TSF->>TSF: complete_via_cuas
        TSF->>TSession: handle commit
        TSession->>Engine: Request::Commit
        Engine-->>TSession: Response::Committed
        TSF->>TSF: present 把 commit 放進組字區
        TSF->>TSF: Cuas::complete
    end
```

`preedit` 與 `commit` 的差異：

- `Snapshot.preedit` 是 `composed_text`：畫面文字，包含尚未完成的注音、日文かな與字面尾巴。
- `commit_text` 與 `display_text` 是同一份來源。`display_text` 先看待提交的 `pending_commit`，沒有才回 `composed_text`。
- `Session::commit` 才套用 `output_text` 的簡體轉換；`finalize` 的標點與中英空格在 Enter 的 `finalize_lane` 先做一次。組字中的預覽保持繁體。
- `edit::apply` 的 `RequestEditSession` 先要求同步 `TF_ES_SYNC`。同步被拒時，若有提交文字會改排非同步 `TF_ES_ASYNCDONTCARE`；已寫入的提交不會重跑，避免重複插入。

### 3.11 學習

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant Engine
    participant Core
    participant Personal

    Host->>TSF: Enter 或候選選字
    TSF->>Engine: Request::Key 或 Select
    Engine->>Core: Session::press 或 choose_candidate
    Core->>Core: 套用 candidate action
    alt private
        Core-->>Engine: 跳過學習 只回 Snapshot
    else menu 開著
        Core-->>Engine: 跳過學習 選單只插入
    else learning 關閉
        Core-->>Engine: 跳過學習
    else 一般路徑
        Core->>Core: learn_composition 或 learn_candidate
        Core->>Core: note_manual 或 note_passive 或 note_passive_evidence
        Core->>Personal: 寫入 personal store
        Core-->>Engine: Snapshot
    end
    Engine->>Personal: 定期 flush 或 Close 時 flush
```

實際條件：

- `private` 為真時 `learn_composition` 與 `learn_candidate` 都直接返回。TSF `handle_retrying` 會把 `private` 放進每個請求。
- `menu.is_some()` 時不學習。選單候選由 `apply_menu_action` 插入，`CandidateAction.learn` 預設 `false`。
- `config.learning == false` 時不學習。注音短讀音還有 `min_trigger_len` 的 passive 證據路徑。
- 手選與被動接受的證據量不同：`candidate_window` 開著時 `manual` 為真，走 `note_manual`；否則被動走 `note_passive` 或 `note_passive_evidence`。
- 下一次按鍵若是 Backspace、Esc 或 Ctrl+Z，`resolve_pending_learning` 會在 rollback 窗口內補償上一次學習。

重播沒有獨立的「不要學習」旗標。`Engine::replay_into` 直接對新 session 重跑 log 裡的 `Request::Key`、`Request::Literal`、`Request::Commit` 等；會不會寫入取決於 log 內容。TSF 的 log 會在 commit、cancel 或組字結束時清空，一般不會把已提交的 Enter 留在重播裡；若 log 真的包含會套用候選的 Enter，重播仍會再走一次學習。（待確認：是否要在 replay 路徑明確關閉學習。）

### 3.12 模式切換

```mermaid
sequenceDiagram
    autonumber
    participant Host
    participant TSF
    participant TSession
    participant Engine
    participant Core

    alt 單按 Shift
        Host->>TSF: OnKeyUp Shift
        TSF->>TSF: shift_released
        TSF->>TSession: Session::toggle
        TSession->>TSession: english 反轉
        TSession-->>TSF: 本機 Snapshot
        TSF->>TSF: sync_langbar
    else 狀態列按鈕
        Host->>TSF: LangBarButton 點擊
        TSF->>TSF: toggle_mode
        TSF->>TSession: Session::toggle
        TSession-->>TSF: 本機 Snapshot
        TSF->>TSF: sync_langbar
    else Ctrl Space
        Host->>TSF: 鍵盤開關 compartment 改變
        TSF->>TSF: keyboard_openclose_changed
        TSF->>TSF: toggle_mode
        TSF->>TSession: Session::toggle
    end

    Note over TSF,Core: 中英模式只存在 TSF 外殼
    TSF->>TSF: 下一鍵帶 english 狀態
    TSF->>TSession: key_with_flags
    TSession->>Engine: Request::Key
    Engine->>Core: Session::press
    Core->>Core: 依實際字元選 lane
    Core-->>Engine: Snapshot
    Engine-->>TSession: Response::State
    TSession->>TSession: english 由外殼補回
```

- `Session::toggle` 不送 IPC，也不提交組字。英文待選字在切回中文時丟掉，中文組字保留。
- `Request::Mode` 已從協定與 engine 刪除；TSF 不送。Caps Lock 只影響字面大小寫，不切換中英模式，也不送模式請求。
- 日文不是外殼模式。core 依片段推斷日文，F6–F10／Convert 只看 capability，不看 `english`。

## 4. 決策歸屬表

| decision | TSF | engine | core | 目前程式位置 |
|---|---|---|---|---|
| 這一鍵要不要吃 | `OnTestKeyDown` 先問 `wants_key`，`key_tested` 只給緊接的 `OnKeyDown` 用 | `handled` 由 core 快照決定，候選越界明確回 `false` | `press` 的 `KeyOutcome` 與 lane 狀態 | `service.rs:1731` `wants_key`、`service.rs:2291` `OnTestKeyDown`、`service.rs:2433` `OnKeyDown`、`engine/lib.rs:604` `handle`、`session/mod.rs:801` `press` |
| 送什麼 op | `to_jc_request` 把 0c 形狀轉成 `Request` | `handle` 分派到 `Key`、`Literal`、`Commit`、`Select`、`Idle` 等 | 不決定 op | `session.rs:646` `to_jc_request`、`engine/lib.rs:604` `handle` |
| 候選範圍 | 只轉發 Down／Up／Page／左右鍵，不決定範圍 | 不決定範圍 | `down` 依 lane 開整段、片段或最後一格；`zh_window_last`、`edit`、`mixed_target` 決定範圍 | `core.rs:1490` `down`、`core.rs:1086` `open_ja_cursor_candidates`、`mixed.rs:2023` `open_zh_cursor_candidates` |
| 游標單位 | `present_in_edit` 只用 `cursor_utf16` 定位 | 不決定單位 | 注音是音節格，日文是 byte 再換算 rune，混打是 rune；權威值 `cursor_rune` | `core.rs:2055` `navigate`、`core.rs:1591` `move_cursor_to`、`core.rs:1656` `candidate_edge` |
| 字面大小寫 | `keys::printable` 依鍵盤狀態轉字元；`handle_key` 帶 `caps` 與 `shift` | 不決定大小寫 | `press` 把 Caps Lock 字母改回基礎小寫，路由不變；`press_caps` 只改 Latin 字面 | `service.rs:1163` `handle_key`、`session/mod.rs:801` `press`、`core.rs:344` `press_char` |
| 候選排序 | 不改順序 | 不改順序 | lane 的 nbest、個人學習、rerank 先排；`CandidateSet` 負責插 extras、去重、截斷、顯示字排第一 | `session/candidate_set.rs:96` `from_hypotheses`、`mixed.rs:1821` `zh_choice_candidates`、`core.rs:3043` `rebuild_ja` |
| 補全來源 | 不再推導來源；`parse` 只保存 `suggestion` 字串 | 只帶 `suggestion` 字串，沒有來源欄位 | `snapshot_suggestion` 決定 suggestion 內容；混合英文補全同時進候選清單 | `session.rs` `state_to_snapshot_value`／`parse`、`core.rs:1681` `snapshot_suggestion`、`session/key_policy.rs` `tab` |
| Enter 語意 | 只送 `Key` 或 `Commit`，不自行判斷 | `Key Enter` 先 `press`；還在組字回 `State`，否則 `commit` 回 `Committed` | `finish_composition` 先 `enter_action`；注音片段與日文文節保留組字，選單與直接插入候選清掉組字 | `engine/lib.rs:655` `enters` 分支、`core.rs:1750` `finish_composition`、`core.rs:902` `apply_menu_action`、`core.rs:1005` `apply_insert_action` |
| 模式 | `english` 只在外殼；`toggle` 不送 IPC。Caps Lock 只改字面大小寫，不切換模式 | 沒有 toggle 或 mode op | 只依字元與片段選 lane，沒有中英模式旗標 | `session.rs` `toggle` |
| 刪除單位 | 只送 Backspace／Delete | 只回 `handled` | `delete_visible` 一次刪一個可見 rune；字元涵蓋多個讀音單位時整個範圍一起刪 | `key_policy.rs:279` `delete_visible`、`key_policy.rs:331` `delete_zh_visible`、`key_policy.rs:427` `delete_ja_visible` |
| 候選窗何時顯示 | `wants_candidate_window` 與 `show` 條件 | 只帶 `navigating`、`menu`、`candidates` | `down`／`up` 設定 `navigating`；`open_menu` 設定 `menu` | `session.rs:423` `wants_candidate_window`、`service.rs:752` `show`、`core.rs:1891` `open_menu` |
| 候選編號 | 不產生 | `Response::state` 把 `selection_key` 放進 `CandidateDetail.key` | `selection_key` 只回傳頁內位置 `1` 到 `0` | `jc-proto/lib.rs:589`、`jc-core/src/candidate.rs:133` `selection_key` |
| 學習 | 只帶 `private` | `replay_into` 重跑 log 時暫停學習，不寫 personal、不改統計 | `learn_candidate`／`learn_ja_choice` 檢查 `private`、`learning`、`menu`、`suppress_learn` | `core.rs` `learn_candidate`、`core.rs` `learn_composition`、`engine/lib.rs` `replay_into` |
| 選單分頁 | 只轉發 Tab／Enter／方向鍵 | 查 `settings_keymap` 執行 `symbol_menu`／`emoji_palette` | `menu_key` 切分頁，`apply_menu_action` 插入 | `core.rs:1971` `menu_key`、`core.rs:902` `apply_menu_action`、`engine/config_watch.rs:116` |

## 5. 常見誤解

### 5.1 Caps Lock 會不會改變按鍵路徑

你可能以為：Caps Lock 開著時，字母會走英文或大字模式，注音鍵會失效。

實際是：`Session::press` 在 `key.caps` 為真時只把 ASCII 字母改回基礎小寫，`key_policy` 仍照原本 lane 走。大小寫在最後的字面輸出才套用，注音字母仍然是注音鍵。`crates/jc-core/src/session/mod.rs:803` 到 `813`。

### 5.2 刪除單位

你可能以為：Backspace 刪一個注音按鍵，或刪一個音節。

實際是：`delete_visible` 先關候選窗，再用 `cursor_runes` 找游標左邊或右邊的一個可見 rune。若這個字元底下涵蓋多個隱藏讀音單位，`zh_units_at_rune`／`ja_units_at_rune` 會回傳整個範圍，一次刪掉。`crates/jc-core/src/session/key_policy.rs:279`。

### 5.3 候選 `[1]` 原始輸入是誰加的

這裡有兩個不同的東西：

- 候選窗上的編號 `1`、`2`、`0` 是 `jc_proto::Response::state` 呼叫 `selection_key` 放進 `CandidateDetail.key`，TSF 只負責畫。位置 `crates/jc-proto/src/lib.rs:589`。
- 候選清單第 2 項的原始輸入是 core 加的。`zh_choice_candidates` 先放目前值，再放 `raw`，所以索引 `1` 是原始按鍵；選單與 TSF 都不會插入這一項。位置 `crates/jc-core/src/session/mixed.rs:1821`。

### 5.4 Enter 兩段式與選單一次送出

你可能以為：所有 Enter 都是先套候選、再送出。

實際是：

- 注音片段與日文文節候選開著時，第一次 Enter 的 `finish_composition` 只套用高亮候選並保留組字，engine 回 `State`；第二次 Enter 才 `finalize_lane`，engine 回 `Committed`。
- 選單候選的 `CandidateAction.commit_on_apply` 為真，`apply_menu_action` 直接設 `pending_commit` 並 `clear_composition`；同一次 Enter 的 engine 就會走到 `Session::commit`，只插入一次。
- `ActionLane::Insert` 的 `apply_insert_action` 也是一次送出。其他 lane 的套用動作若保留組字，就還是兩段式；engine 只依 `snapshot.composing()` 決定回 `State` 或 `Committed`。

位置 `crates/jc-core/src/session/core.rs:1750`、`crates/jc-core/src/session/core.rs:902`、`crates/jc-core/src/session/core.rs:1005`、`crates/jc-engine/src/lib.rs:655`。

### 5.5 日文 live 與文節 ownership

你可能以為：TSF 選了候選就直接改字，core 下次重建會重猜。

實際是：日文的文節切分、鎖定與 User 片段都在 core 的 `Composition` 與 `ja::live::Controller`。`rebuild_ja_live` 先保留共同讀音與 User 片段，再讓 `Controller::input` 回 `LivePlan`；`apply_live_plan` 才換畫面。TSF 只收到片段與候選，不持有文節所有權。位置 `crates/jc-core/src/session/core.rs:3198`、`crates/jc-core/src/ja/live.rs:778`。

### 5.6 候選窗何時顯示

你可能以為：只要 engine 有候選或 `navigating` 為真就會顯示。

實際是：`Snapshot::wants_candidate_window` 是 `menu.is_some() || composing() && navigating`。`present_in_edit` 的 `show` 還要至少一個內容來源：候選、suggestion、reading hint 或 notice。所以：

- 一般打字只更新組字區，不彈候選窗。
- 按 ↓／↑ 後 `navigating` 為真才顯示。
- 選單開著時 `menu` 讓它一律顯示，不受 `navigating` 影響。

位置 `crates/jc-tsf/src/session.rs:423`、`crates/jc-tsf/src/service.rs:752`。

## 6. 待確認

1. 英文模式已無組字，舊 `key_with_flags` 的 `Up`／`Down`／`PageUp`／`PageDown` 本機快照特例已刪除；候選窗只存在於混合模式。
