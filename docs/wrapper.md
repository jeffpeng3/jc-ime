# wrapper 作者規格（協定 v2）

這份文件給寫 Windows TSF、Fcitx5、macOS 外殼的人看。只描述 wrapper 與
`jc-engine` 之間的介面，不需要讀 Rust 原始碼。

契約本體是 [`docs/proto.md`](proto.md)。本文補的是「實作 wrapper 時實際上
會遇到的決策」：誰啟動引擎、按鍵什麼時候送、組字怎麼畫、錯誤怎麼收。

文中標 **未實作** 的能力，目前程式沒有，不能當成可用功能來設計。

## 0. 現況總表

| 能力 | 狀態 | 對應程式 |
|---|---|---|
| stdio 傳輸 | 已實作 | `crates/jc-engine/src/main.rs` |
| Unix domain socket | 已實作，限 unix | 同上 |
| Windows named pipe | **未實作**，非 unix 平台 `--socket` 直接回錯 | 同上 |
| 長度前綴＋JSON 訊框 | 已實作，上限 1 MiB | `crates/jc-proto/src/frame.rs` |
| 版本握手 | 已實作，精確比對 `2` | `crates/jc-engine/src/lib.rs` |
| 按鍵／快照／提交／關機 | 已實作 | `jc-proto` 的 `Request`／`Response` |
| IME 開關 | 引擎沒有這個概念，由 wrapper 自己管 | 無 |
| 設定頻道 | **未實作**，引擎只吃命令列參數 | `crates/jc-engine/src/main.rs` |
| 主動通知 | **未實作**，只有請求／回應 | `crates/jc-engine/src/lib.rs` |
| client crate | **未實作**，wrapper 自己實作 framing | 無 |
| 神經模型（注音選字） | 已接線，資料與模型存在時啟動即安裝 | `crates/jc-core/src/zh/select.rs` |
| 神經模型（日文重排） | 程式在，但引擎預設關閉，不會觸發 | `loader.rs` 的 `ja_rerank` |
| 學習資料持久化 | **未實作**，只存在引擎記憶體 | `jc-core::personal` |
| Windows TSF wrapper | **未實作** | 無 |

## 1. 生命週期

### 1.1 誰啟動引擎

wrapper 啟動引擎，引擎不自我常駐，也不是系統服務。引擎只有一個長期迴圈：
讀請求、回回應。

啟動指令：

```sh
jc-engine --data <資料根目錄> [--mode zh|ja|mixed|en] \
          [--socket <路徑>] [--max-frame <bytes>]
```

| 參數 | 說明 |
|---|---|
| `--data <dir>` | 資料根目錄；沒給時讀 `JC_DATA`，兩者都沒有就結束（exit 1） |
| `--mode` | 啟動時的輸入模式，預設 `zh`；**沒有執行期切換模式的請求** |
| `--socket <path>` | 改用 Unix domain socket；非 unix 平台回 `named pipe 尚未實作` |
| `--max-frame` | 單一訊框上限，預設 1048576 |
| `--allow-empty-data` | 不載入資料，只給協定測試用；**產品不可使用** |

資料根目錄至少要有：

```text
<root>/build-zh/lexicon.bin        必要，缺了直接啟動失敗
<root>/build-zh/lm.bin             選用
<root>/build-zh-bigram/lm.bin      選用，存在時優先當 bigram 來源
<root>/packs/ja-lexicon-mozc-v2.pack   日文用；缺了退回 ja-src/lexicon/words.tsv
<root>/models/                     選用；有就安裝注音選字模型與日文重排器
```

日文 pack 與 curated 的 `ja-src/lexicon/words.tsv` 都沒有時，引擎啟動失敗。

啟動會把注音詞典讀進記憶體、mmap 日文 pack，並在 `models/` 存在時載入
注音選字模型。repo 只有模型載入的間接數字，日文重排模型約 93–119 ms
（`docs/port/jinen-gating-v2.md`），沒有完整的引擎啟動量測。不要寫死
timeout；握手完成前把狀態顯示成「初始化中」。

`jc-engine --version` 會印 `jc-engine 0.1.0（協定 2）` 後結束。這段文字是給
人看的，不是穩定格式。wrapper 判斷版本一律以握手回應為準。

### 1.2 何時啟動

建議在輸入法第一次被啟用時啟動，不要等到第一個按鍵。第一個按鍵才啟動會
讓載入時間直接落在使用者的第一次輸入上。

同一使用者只開一個引擎。Unix socket 模式可以讓多個行程共用；stdio 模式
的引擎綁在單一 wrapper 行程上，wrapper 結束就結束。

### 1.3 閒置多久關閉

引擎**沒有**閒置計時器，連線開著就永遠等。要不要回收是 wrapper 的政策：

- 以「整個使用者工作階段」為單位計時，最後一次請求後 30 分鐘為合理起點。
- 只有當沒有任何 wrapper 連線、也沒有進行中的組字時才能送 `Shutdown`。
- 關閉後下一次啟用重新啟動並重新握手。

`Shutdown` 會讓整個引擎行程結束，不是只關一條連線。共用引擎時要在 broker
層做引用計數，不能讓某個應用程式關掉全體的引擎。

### 1.4 崩潰後怎麼重啟

引擎沒有 supervisor、沒有 health check、沒有自動重啟。wrapper 要自己做：

1. 從讀取端拿到 EOF、寫入得到 `EPIPE`，或子行程結束，都算引擎消失。
2. 立刻清掉畫面上的組字與候選，丟棄未送出的按鍵。
3. 用退避重啟：0.5 s、1 s、2 s、5 s、10 s，上限 30 s。
4. 一分鐘內連續失敗 5 次就停止重試，顯示「輸入法無法啟動」，等使用者
   手動排除。
5. 重啟後重新握手。舊的組字狀態無法還原。

反過來，wrapper 自己崩潰時：socket 模式的引擎會繼續活著，下一條連線會
拿到乾淨的組字狀態。stdio 模式的引擎會因為 stdin EOF 而結束。

### 1.5 多應用程式的現實

`serve_socket` 一次只服務一條連線：接受連線後一路處理到對方關閉，其他
連線排在 listen backlog。也就是說：

- 兩個應用程式同時各開一條連線，第二個會卡住，直到第一個關閉。
- 沒有多工、沒有 per-connection thread、沒有 multiplex。

**未實作**：並行連線。

因此共用引擎時，wrapper 端應該有一個 per-user broker，只維持一條連線，
把各應用程式的請求排隊送出。組字狀態是引擎的，不是每個應用程式一份；
焦點切換時一定要先收掉上一個應用程式的組字，見 §5.4。

Windows 目前沒有 named pipe，TSF wrapper 只能用 stdio，等於每個宿主行程
一個引擎、各自載入一次資料。這是現階段的架構限制，不是包裝問題。

### 1.6 責任分界

| 工作 | 負責 |
|---|---|
| 啟動、監控、重啟引擎 | wrapper |
| IME 開關、中英切換 | wrapper |
| OS 按鍵到 `Key` 的轉換 | wrapper |
| 組字區與候選窗的繪製 | wrapper |
| 把 `committed.text` 插入應用程式 | wrapper |
| 焦點遺失時收掉組字 | wrapper |
| 組字狀態、候選排序、學習 | 引擎 |

## 2. 傳輸

### 2.1 選項

| 平台 | 傳輸 | 參數 | 狀態 |
|---|---|---|---|
| 全部 | stdio | 無，預設 | 已實作 |
| Linux／macOS | Unix domain socket | `--socket <path>` | 已實作 |
| Windows | named pipe | 無 | **未實作** |

### 2.2 stdio

- 協定走 stdin／stdout，日誌走 stderr。stdout 只能有訊框，不能有其他輸出。
- Windows 上必須把 pipe 設成 binary mode，避免 `\n` 被轉成 `\r\n` 而破壞
  長度前綴與 JSON。
- wrapper 關閉 stdin 就等於要求引擎結束。
- 適合「一個宿主行程一個引擎」；不適合多應用共用。

### 2.3 Unix domain socket

- 路徑用 `--socket` 指定。建議放在 `$XDG_RUNTIME_DIR/jc-ime/engine.sock`
  或 `/run/user/<uid>/jc-ime/engine.sock`，目錄權限 `0700`。
- Linux 的 socket 路徑上限是 108 bytes，含結尾 NUL；太長會在 bind 失敗。
- 啟動時如果路徑已存在：能連上就拒絕啟動並回 `已經有引擎在 ... 監聽`；
  連不上就當殘留檔刪掉再 bind。
- 正常結束會刪掉 socket 檔；被 SIGKILL 時會留下殘留檔，下次啟動自動清。
- 沒有 chmod、沒有 peer credential 檢查。能連上 socket 的行程都能送
  `Shutdown`，所以目錄權限是唯一防線。
- 一次一條連線，序列處理。斷線後可重連；重連要重新握手。

### 2.4 訊框

```text
[ u32 little-endian length ][ UTF-8 JSON payload ]
```

| 規則 | 值 |
|---|---|
| 長度前綴 | 4 bytes，小端，只算 payload |
| 長度單位 | bytes，不是字元 |
| 上限 | 預設 1 MiB，可用 `--max-frame` 調整 |
| 0 長度 | 拒絕，回 `bad_frame` |
| 超長 | 拒絕，回 `frame_too_large`；引擎讀完前不會配置緩衝 |
| 寫出 | 每個回應立即 flush |

讀取端要「讀滿 4 bytes，再讀滿 length」；TCP 或 pipe 的資料可能分次到達，
不能假設一次 read 就是一個完整訊框。

`--max-frame` 同時套用在讀與寫。送出的訊框超過對端上限就會被拒絕。
沒有能力協商，也沒有在握手裡通告上限；保持預設值最安全。

### 2.5 逾時與斷線

引擎不設讀取逾時，連線可以永遠閒置。wrapper 要自己設：

| 動作 | 建議值 | 逾時處理 |
|---|---|---|
| connect | 1 s | 當引擎沒起來，進重啟流程 |
| 一般請求 | 500 ms | 視為引擎卡住，關線並重啟 |
| `Shutdown` | 2 s | 直接 kill 子行程 |

請求逾時後**不要重送同一個 `Key`**。回應可能已經產生但沒收到，重送會讓
組字多一個字。重連後用 `Snapshot` 對帳即可；新連線本來就是空組字。

## 3. 握手與版本

### 3.1 流程

```text
wrapper                                   engine
   │  {"op":"hello","protocol":2,          │
   │   "client":"jc-tsf/0.1.0"}            │
   │ ────────────────────────────────────▶ │
   │  {"op":"ready","protocol":2,          │
   │   "engine":"jc-engine 0.1.0"}         │
   │ ◀──────────────────────────────────── │
```

- 版本必須完全相同。`PROTOCOL_VERSION` 目前是 `2`。
- 不同版本回 `{"op":"error","code":"version_mismatch",...}` 然後關線。
- 沒握手就送 `Key`／`Snapshot`／`Commit` 會回 `internal`，訊息是
  `尚未完成握手`。`Shutdown` 不檢查握手，見 §8.4。
- `client` 是自由字串，建議 `<產品>/<版本>`，方便看日誌。
- `Ready.engine` 只供顯示與診斷，不要解析它來判斷能力。

### 3.2 版本不符怎麼辦

不要重試、不要降級、不要跳過握手。版本不符代表兩邊安裝不一致，重試一百
次也不會成功。

處理順序：

1. 顯示「輸入法元件版本不符」，附上兩邊版本。
2. 停止送按鍵，維持 pass-through，讓使用者至少能打字。
3. 重新啟動引擎一次，確認不是短暫的安裝替換。
4. 仍不符就等使用者更新，不要自動下載。

### 3.3 重試策略

| 情況 | 重試 |
|---|---|
| 版本不符 | 不重試 |
| connect 失敗 | 退避重試，見 §1.4 |
| 請求逾時／EOF | 重連並重新握手，不重送按鍵 |
| `unknown_op` | 不重試，這是 wrapper 的 bug |
| `bad_frame` | 關線重連；同一個請求不要再送 |

## 4. 按鍵對應

### 4.1 Key 的形狀

```json
{"op":"key","key":{"code":{"kind":"char","value":"ㄇ"},"text":"ㄇ","shift":false}}
{"op":"key","key":{"code":{"kind":"backspace"},"text":null,"shift":false}}
```

| 欄位 | 型別 | 說明 |
|---|---|---|
| `code` | tagged enum | `{"kind": ...}`；`char` 多一個 `value` |
| `text` | char 或 null | 這個鍵實際產生的字元；具名鍵是 `null` |
| `shift` | bool | Shift 是否按著 |
| `ctrl` | bool，可省略 | `false` 時不寫進 JSON |

**`code.value` 才是引擎真正讀的字元**。`Session::press` 對 `Char(ch)` 直接
用 `ch`，不看 `text`。`text` 請填同一個字元，避免未來語意改變時出錯。

### 4.2 KeyCode 清單

| `kind` | 意義 | 備註 |
|---|---|---|
| `char` | 可列印字元 | 值放 `value`，例如 `{"kind":"char","value":"ㄅ"}` |
| `backspace` | 退格 | |
| `enter` | 確定並提交 | 不會產生換行 |
| `escape` | 取消 | 依序關候選窗、清暫存注音、清整段組字 |
| `space` | 空白 | 注音轉換／開候選窗／選字 |
| `arrow_left` | 左 | 候選導航或移動注音游標 |
| `arrow_right` | 右 | 同上 |
| `up` | 上 | 候選導航 |
| `down` | 下 | 開候選窗並進入選取模式 |
| `home` | 移到最前 | 選取模式中跳第一項 |
| `end` | 移到最後 | 選取模式中跳最後一項 |
| `page_up` | 上一頁 | |
| `page_down` | 下一頁 | |
| `tab` | 補全／下一個候選 | |
| `shift_tab` | 往回換段／往回選 | 要另外把 `shift` 設 `true` |
| `delete` | 刪除 | 有候選窗時只關窗 |

沒有 `A` 到 `Z`、數字、標點的專屬鍵碼。這些一律是 `char`。

### 4.3 修飾鍵

協定只認得 `shift` 與 `ctrl`，沒有 Alt、Meta、AltGr、Caps Lock。

| 情況 | wrapper 怎麼做 |
|---|---|
| Shift | 把實際產生的字元放進 `code.value`，並設 `shift: true` |
| Caps Lock | 直接送大寫字元，`shift` 設 `false` |
| AltGr | 先用鍵盤配置解出字元，當普通 `char` 送，不要設 `ctrl` |
| Ctrl／Alt／Win 組合 | 不送引擎，直接給應用程式 |
| 死鍵 | 先組合成最終字元再送 |

Shift+Tab 有兩種表示：`{"kind":"shift_tab"}` 配 `shift:true`，或
`{"kind":"tab"}` 配 `shift:true`。兩者在不同路徑都會用到，wrapper 一律送
前者並把 `shift` 設 `true`。

`ctrl` 目前只有一個用途：`Ctrl+,` 輸出全形逗號，而且要先開啟
`SessionConfig::ctrl_punct`。`jc-engine` 沒有對應的命令列參數，所以**未實作**
從引擎開啟。其他 Ctrl 組合引擎直接忽略。

### 4.4 IME 開關

引擎沒有開關請求，也沒有「目前是否啟用」的狀態。開關完全由 wrapper 管：

- 關閉時：一個按鍵都不送引擎，全部 pass-through；同時要把引擎裡殘留的
  組字收掉。
- 開啟時：依 §4.5 的規則送。
- 中英切換：引擎沒有切換請求。要嘛關掉 IME 讓英文直通，要嘛用
  `--mode mixed`；`mixed` 的混合候選尚未完成，不能當穩定方案。
- 全形／半形、標點樣式、Shift+Space：都是 wrapper 的 UI 決策。引擎沒有
  這些請求，Shift+Space 也不要送給引擎。

### 4.5 pass-through 決策

引擎收到不認得的具名鍵會**默默吃掉**，回應仍是 `State`，不會告訴 wrapper
「這個鍵我沒處理」。所以 wrapper 必須自己決定哪些鍵不要送。

以最近一次 `State` 判斷：

| 狀態 | 送引擎 | 傳給應用程式 |
|---|---|---|
| `preedit` 空、`candidates` 空 | 可列印字元，含 `` ` `` 與 `=` | Enter、Tab、Esc、Backspace、Delete、Space、方向鍵、Page、Home、End、F1–F24 |
| `preedit` 非空 | 可列印字元、Enter、Esc、Backspace、Delete、Space、Tab、Shift+Tab、方向鍵、Page、Home、End | F1–F24、Ctrl／Alt／Win 組合 |
| `candidates` 非空 | 同 `preedit` 非空 | 同上 |

規則的理由：

- `preedit` 空時送 Enter，引擎會回一個空 `State`，應用程式收不到換行。
- `preedit` 空時送 Space，引擎什麼都不做，應用程式收不到空白。
- `preedit` 非空時一定要吃掉 Enter，否則提交與換行會同時發生。
- 組字中送給應用程式的方向鍵會讓游標跑掉，與候選導航衝突。

`Esc` 不保證一次清空：候選窗開著時只關窗，暫存注音還在時只清暫存。焦點
遺失請連續送 `Esc` 直到 `preedit` 變空，最多 3 次。

### 4.6 模式差異

| 模式 | 行為 | wrapper 注意 |
|---|---|---|
| `zh` | 注音鍵序列轉中文 | 一般情況 |
| `ja` | 羅馬字轉かな再轉漢字 | 需要日文 pack，缺了引擎起不來 |
| `en` | 直接累積英數到組字區 | Enter 會提交但不產生換行，見 §7.3 |
| `mixed` | 自動判斷英數與注音／日文 | 候選清單尚未完成，視為實驗性 |

模式只能在啟動時指定。要換模式就是重啟引擎，或關掉 IME 直通。

## 5. 組字顯示

### 5.1 State 欄位

```json
{"op":"state","preedit":"我","cursor":1,"candidates":["我","窩"],"cursor_rune":1}
```

`cursor_rune` 為 0、`suggestion`／`label` 為 `null`、`navigating` 為
`false` 時，這些欄位不會出現在 JSON 裡。上例是完整形態；只有
`preedit`、`cursor`、`candidates` 一定存在。

| 欄位 | 說明 |
|---|---|
| `preedit` | 目前要顯示的組字字串，已經是目前高亮候選的結果 |
| `cursor` | 固定是 `preedit` 的 Unicode scalar 數，也就是結尾 |
| `cursor_rune` | 核心游標的 rune 位置；值為 0 時省略 |
| `candidates` | 候選字面，順序就是顯示順序 |
| `suggestion` | Tab 會接受的補全尾綴；沒有時省略 |
| `label` | 目前高亮候選的顯示原因；沒有時省略 |
| `navigating` | 是否已進入明確的候選選取模式 |

`preedit` 可能包含注音、かな、英數、標點與 emoji。空字串代表沒有組字。

### 5.2 UTF-16 轉換

`cursor` 與 `cursor_rune` 的單位是 Unicode scalar，不是 UTF-16 code unit。
BMP 的漢字、注音、仮名兩者相同；emoji 這類增補平面字元在 UTF-16 佔 2 個
unit，直接拿 `cursor` 當 UTF-16 位移會偏掉。

Windows 的游標 API 要 UTF-16 位移，請自己換算：

```csharp
// 回傳前 runeIndex 個 Unicode scalar 佔幾個 UTF-16 code unit
static int Utf16Offset(string text, int runeIndex) {
    int units = 0, runes = 0;
    foreach (var r in text.EnumerateRunes()) {
        if (runes == runeIndex) break;
        units += r.Utf16SequenceLength;
        runes++;
    }
    return units;
}
```

```cpp
// w 已經由 UTF-8 轉成 UTF-16；回傳前 runeIndex 個 scalar 的 code unit 數
int Utf16OffsetForRunes(const std::wstring& w, int rune_index) {
    int units = 0;
    for (size_t i = 0; i < w.size() && rune_index > 0;) {
        bool pair = i + 1 < w.size() && w[i] >= 0xD800 && w[i] <= 0xDBFF;
        i += pair ? 2 : 1;
        units += pair ? 2 : 1;
        --rune_index;
    }
    return units;
}
```

一般情況 `cursor_rune == cursor`，而且都在結尾，直接送整串的 UTF-16 長度
就正確。首選用 `cursor_rune`，它才是核心真正的位置。

`cursor` 與 `cursor_rune` 可能不同：候選窗開著但還沒進入選取模式時，
Home／End 會移動注音的游標格，此時 `cursor` 仍是結尾，`cursor_rune` 才是
真實位置。

### 5.3 什麼時候清空組字區

| 事件 | 動作 |
|---|---|
| 收到 `preedit: ""` | 清空組字區，隱藏候選窗 |
| `Esc` 後 `preedit` 變空 | 清空 |
| `Committed` | 插入文字，清空組字區 |
| 連線中斷或引擎重啟 | 立即清空，丟棄未提交內容 |
| 焦點遺失 | 送 `Esc` 直到清空，再清畫面 |
| IME 關閉 | 同焦點遺失 |

不要自己保留上一份 `preedit` 再拼接。每次 `State` 都是完整快照。

### 5.4 焦點與組字

引擎不知道焦點在哪個應用程式，組字狀態是全引擎共用的。切換焦點時沒有
`FocusLost` 事件可送，`HostEvent` 目前只存在於 `jc-platform`，**尚未接進
引擎**。

wrapper 必須在焦點離開時主動收尾，否則使用者在另一個應用程式打字會接續
上一個應用程式的組字。建議送 `Esc` 取消；如果產品政策是「切換焦點自動
確定」，就在焦點還有效時送 `Commit` 並插入文字。

### 5.5 suggestion 與 label

- `suggestion` 是 Tab 會接受的尾綴，適合用淡色接在 `preedit` 後面顯示。
  它只在候選窗關閉、且沒有已接受的補全時出現。
- `label` 只描述「引擎目前高亮的那一項」，但 `State` **沒有** highlight
  索引。字面唯一時可以用 `preedit` 比對出是哪一項；字面重複或無法判斷時
  建議不要顯示 label。這是 **未實作** 的欄位缺口，見 §11。

## 6. 候選窗

### 6.1 顯示順序與數量

- `candidates[0]` 是引擎的第一順位，依序往後畫，不要重排、不要過濾。
- 一般轉換最多 5 項，由 `SessionConfig::max_candidates` 決定。
- 符號選單與表情／顏文字選單可能超過一頁，每頁 10 項。
- 沒有候選時 `candidates` 是空陣列，關掉候選窗。
- 協定沒有「候選窗是否開啟」的旗標。實務上就用 `candidates` 非空當作
  顯示條件。
- 在 `preedit` 空的時候按 `` ` `` 或 `=` 會開符號／表情選單，這時
  `preedit` 仍是空字串但 `candidates` 有內容。這是唯一會出現「組字空、
  候選不空」的常見情況。

### 6.2 navigating 的意義

`navigating` 代表使用者已經進入「明確選字」的模式，不是「候選窗有開」。

| 動作 | navigating |
|---|---|
| 打字產生候選 | `false` |
| 空白開候選窗 | `false` |
| 第一次按上／下 | `true`，高亮不動 |
| PageUp／PageDown | `true` |
| 候選窗開著、尚未選取時的左右鍵 | 移動注音游標，`navigating` 仍 `false` |
| 其他情況的左右鍵 | 開窗並進選取模式 |
| 打字、選字、關窗、送出 | 回到 `false` |

wrapper 可以用它決定候選窗的視覺強調，例如 `true` 時才畫選取框。

### 6.3 翻頁與選擇怎麼送回

全部都是送 `Key`，沒有專用的候選請求。

| 使用者動作 | 送出的 Key | 引擎行為 |
|---|---|---|
| 下一項 | `down` | 第一次開窗並進選取模式，之後往下換一項 |
| 上一項 | `up` | 第一次開窗並進選取模式，之後往上換一項 |
| 下一頁 | `page_down` | 換頁，頭尾相接 |
| 上一頁 | `page_up` | 換頁，頭尾相接 |
| 第一項 | `home` | 選取模式中跳到第一項 |
| 最後一項 | `end` | 選取模式中跳到最後一項 |
| 確定高亮項 | `space` | 取高亮項，清組字 |
| 確定（Enter） | `enter` | 取高亮項並提交 |
| 補全 | `tab` | 接受 suggestion，否則換下一項 |

**選字不會直接回 `Committed`**。Space 或數字選字後，引擎只回一個空
`preedit` 的 `State`，選到的文字放在引擎內部的待提交區。wrapper 必須在
組字與候選都清空之後補送一次 `Commit`，拿回 `Committed { text }`。
細節與完整流程見 §7.2。

### 6.4 highlight 與滑鼠點選

**未實作**：`State` 沒有 `highlighted` 索引，也沒有滑鼠事件。點選候選只能
由 wrapper 轉成按鍵序列。

若要支援滑鼠，wrapper 必須自己鏡射引擎的高亮狀態。引擎規則：

| 事件 | highlight 變化 |
|---|---|
| 候選清單出現 | 0 |
| `down`，尚未開窗 | 不變，開窗並進選取模式 |
| `down`，已開窗但未選取 | 不變，進選取模式 |
| `down`，選取中 | `(h + 1) mod n` |
| `up`，選取中 | `(h - 1) mod n` |
| `page_down`／`page_up` | 跳到該頁第一項，頁碼頭尾相接 |
| `home`／`end`，選取中 | 0／`n - 1` |
| 打字、送出、取消 | 回到 0 |

有了自己的 `h`，點第 `k` 項時：先送 `down` 直到 `navigating` 為 `true`，
第一次只開窗不移動；再送 `(k - h + n) mod n` 次 `down`，最後送 `space`，
再依 §7.2 補 `Commit`。這條路徑容易因狀態不同步而出錯，v2 建議先不支援
滑鼠，等 §11 的欄位補上再做。

### 6.5 數字鍵與其他選字鍵

數字鍵只有在引擎內部的候選窗已開時才會選字。候選窗沒開時，數字會變成
字面輸入，直接進組字區。

wrapper 看不到「候選窗已開」這個狀態，所以：

- 不要自行把 1–9 當成選字鍵。
- 若產品一定要數字選字，先送 `down` 開窗，再送數字。這件事同樣依賴
  highlight 狀態，見 §6.4。
- `Shift+Delete`／`Shift+Backspace` 在候選窗開著時是「忘記這個候選」的
  學習操作，不要對應到應用程式的刪除。

## 7. 提交

### 7.1 什麼時候會收到 Committed

只有兩條路徑：

1. 送 `Key` 且 `code.kind` 是 `enter`，引擎提交後回
   `{"op":"committed","text":"..."}`。
2. 送 `{"op":"commit"}`，引擎一定回 `Committed`，文字可能是空字串。

`Committed.text` 是完整的一次提交結果，可能為空字串。收到後插入應用
程式，並清空組字區。不要再送 `clear_preedit` 給引擎，引擎已經清了。

### 7.2 選字後的補提交

Space 或數字選字時，引擎把結果放進待提交區並清空組字，但回應是 `State`，
不是 `Committed`。如果 wrapper 就這樣等下一個按鍵，會有兩個問題：

- 選到的字沒有插入應用程式。
- 下一個 `Enter` 會用新的組字覆蓋待提交區，先前的選字消失。

必要的 wrapper 流程：

```text
送出 Key
  │
  ├─ 收到 Committed ──▶ 插入文字，結束
  │
  └─ 收到 State
        │
        ├─ preedit 非空 ──▶ 更新畫面，結束
        │
        ├─ preedit 空、candidates 非空 ──▶ 更新候選窗，結束
        │
        └─ 兩者都空，而上一個 State 不是這個狀態
              │
              └─▶ 送 {"op":"commit"}
                    ├─ Committed.text 非空 ──▶ 插入文字
                    └─ Committed.text 空   ──▶ 忽略，這是 Esc 或一般清空
```

條件要看 `preedit` 與 `candidates` 兩個欄位。符號或表情選單可以在
`preedit` 空的時候選字，只看 `preedit` 會漏掉這一種。

補送的 `Commit` 要在處理下一個使用者按鍵之前完成。wrapper 本來就該序列化
請求，這一點自然成立。

`Esc` 也會讓 `preedit` 變空，所以補 `Commit` 可能拿到空字串；也可能拿到
先前 Space 選取、尚未取走的文字。空字串直接忽略。

這是目前協定的缺口，不是 wrapper 的錯。§11 有建議的修正。

### 7.3 Enter 與換行

- `preedit` 非空時，Enter 被引擎吃掉並回 `Committed`，應用程式不會收到
  換行。這是注音與日文的預期行為。
- `preedit` 空時，wrapper 必須把 Enter 傳給應用程式，不要送引擎。
- `--mode en` 時，Enter 會提交整段英數但**不產生換行**。如果產品要保留
  換行，wrapper 在收到 `Committed` 後要自己補一個 Enter 給應用程式。這是
  目前的行為，不是理想設計，已列入 §11。

### 7.4 Undo 的責任

協定沒有 undo、沒有 `forget`、沒有「取消上一次提交」。`Session` 內有
`undo_learning` 與 `forget_word`，但沒有對應的請求，**未實作**。

- 使用者按 Ctrl+Z 時，wrapper 直接把鍵送給應用程式，讓應用程式自己的
  undo 處理。
- wrapper 不要嘗試「再送一次 Backspace」來模擬 undo，提交後的文字已經
  不屬於引擎。
- 學習資料的撤銷同上，目前做不到。

### 7.5 空提交

`{"op":"commit"}` 在沒有組字時回 `{"op":"committed","text":""}`。這是合法
回應，不要當錯誤，也不要插入空字串以外的東西。

## 8. 錯誤處理

### 8.1 ErrorCode

| code | 意義 | wrapper 處置 |
|---|---|---|
| `version_mismatch` | 握手版本不同 | 不重試；顯示版本不符；引擎已關線 |
| `bad_frame` | JSON 解不開、欄位錯、長度 0、資料截斷 | 關線重連；檢查 wrapper 的序列化 |
| `frame_too_large` | 超過 `--max-frame` | 關線重連；確認兩邊上限一致 |
| `unknown_op` | 不認得的 `op` | 不關線；修正 wrapper，不要重試 |
| `internal` | 尚未握手，或引擎內部狀態不允許 | 先確認握手；仍發生就當 bug 記錄 |

`bad_frame` 與 `frame_too_large` 發生後，引擎回完 `Error` 就關線，因為資料
流已經不可信。`unknown_op` 不會關線，可以繼續用同一條連線。

常見的 `bad_frame` 來源：

- KeyCode 拼錯，例如 `pageup` 而不是 `page_up`。
- 把 `text` 寫成字串而不是單一字元，或漏了 `shift`。
- 長度前綴用了大端，或算了字元數而不是 bytes。
- Windows 上 pipe 被文字模式轉換，JSON 裡多了 `\r`。

### 8.2 連線中斷

EOF、`ECONNRESET`、`EPIPE` 都當引擎消失：

1. 清組字與候選。
2. 丟棄手上的按鍵與未完成請求。
3. 依 §1.4 重啟或重連。
4. 重連後重新握手，用 `Snapshot` 取得空白狀態。

不要假設斷線前的 `State` 還有效。重連是新 session，組字一定從空白開始。
已提交的左文與學習資料留在引擎記憶體裡，但那不影響畫面。

### 8.3 逾時

引擎不會回「忙線中」。請求沒有回應就是卡住。關線、重啟，不要在同一條
連線上排隊等第二個請求。

神經模型有內部 100 ms 逾時，會自動回退統計結果，所以正常情況不會無限等。
wrapper 的 500 ms 是保險，不是常態。

### 8.4 Shutdown 與安全

- `Shutdown` 會設全行程結束旗標，回完最後一個回應就結束，退出碼 0。
- `docs/proto.md` 寫「無回應」，目前實作會在關線前回一個 `State`。
  以契約為準時 wrapper 不該依賴它；送完 `Shutdown` 就直接關連線並等子
  行程結束。這項差異列在 §11。
- `Shutdown` 不檢查握手。任何能連上 socket 的行程都能關掉引擎。
- socket 沒有認證，也沒有 ACL。請把 socket 放在只有本人能進入的目錄。

### 8.5 日誌

引擎用 `env_logger`，預設層級 `warn`，全部寫 stderr。wrapper 應該把
stderr 接到自己的日誌檔，並在回報問題時附上。不要讓 stderr 混進協定的
stdout。

## 9. 效能期望

### 9.1 預算

| 項目 | 目標 | 來源 |
|---|---|---|
| 一般按鍵往返，不含模型 | < 5 ms | `docs/proto.md` §6 |
| 注音選字模型觸發 | p50 < 60 ms、硬上限 180 ms | 同上 |
| 模型內部逾時 | 100 ms，逾時回退統計結果 | `zh/select.rs` 的 `DEFAULT_TIMEOUT_MS` |

180 ms 是預算，不是程式裡的常數。程式真正保證的是 100 ms 的模型逾時；
超過就放棄模型分數，用統計排序回應。

### 9.2 實測數字

| 情境 | 數字 | 來源 |
|---|---|---|
| 日文解碼 p50 | 3.1 ms（`modern-500`） | `docs/status.md` |
| 注音選字模型，candle Q8_0 | 推論 p50 9.36 ms、p95 15.0–17.5 ms | `docs/port/candle-zh.md` |
| 注音選字模型，ORT fp32 | 推論 p50 11.8–12.6 ms | 同上 |
| 第一次推論 | p50 48.45 ms、p95 69.67 ms | `docs/port/defaults-policy.md` |
| 暖機後 | p50 12.12 ms、p95 19.76 ms | 同上 |

這些是引擎內部的推論與解碼數字，不是端到端 IPC。repo 沒有「行程＋socket
＋JSON」的往返量測。5 ms 的預算要用在沒有模型的一般按鍵上；注音每次轉換
若命中模型，多 9–12 ms 是正常的。

### 9.3 wrapper 的實作建議

- 按鍵處理保持同步：送 `Key`、等 `State`、一次更新畫面。不要先樂觀更新
  `preedit` 再對帳，會閃爍。
- 不要在 wrapper 端 debounce。引擎沒有取消機制，送出去的按鍵一定會被
  處理。
- 第一個按鍵會碰上模型的第一次推論，實測首次 p50 約 48 ms。這段時間畫面
  可以先顯示原始按鍵，或顯示「轉換中」，但不要丟鍵。
- 請求逾時設 500 ms；超過就當引擎卡住。使用者的體感上限比 180 ms 更嚴格，
  所以真的卡住時要盡快重啟，不要等。
- 候選窗更新頻率就是按鍵頻率，不需要額外節流。

## 10. 已知限制

以下都是目前 v2 的真實限制，不是設定問題。

1. **候選只有字串**。沒有讀音、詞性、來源、是否為學習詞。要顯示這些資訊
   需要 v3 的結構化候選。
2. **沒有 highlight 索引**。`label` 有值，但 wrapper 不知道它對應哪一項；
   滑鼠點選只能靠自行鏡射狀態。
3. **沒有滑鼠事件**。點選、滾輪都要 wrapper 轉成 `Key` 序列。
4. **沒有候選窗開關旗標**。只能用 `candidates` 是否為空推測。
5. **沒有選字即時提交**。Space／數字選字後要補一次 `Commit`，見 §7.2。
6. **沒有主動通知**。學習完成、模型狀態、候選變更都不會推送。
7. **一次一條連線**。多應用程式共用會排隊，沒有多工。
8. **Windows named pipe 未實作**。Windows 目前只有 stdio，等於每個宿主
   行程一個引擎。
9. **沒有 config 頻道**。簡繁轉換、Ctrl 標點、英文自動補完、輸入風格、
   模糊音，都無法從 wrapper 開啟。引擎只吃命令列。
10. **模式不能切換**。`--mode` 只影響啟動。
11. **學習資料不持久化**。引擎重啟就消失。
12. **日文重排預設關閉**。`jc-engine` 會安裝 reranker，但 `ja_rerank`
    預設 `false`，所以不會觸發。
13. **組字不能任意位置編輯**。候選窗開著但未選取時，Home／End 可以移動
    注音游標格，但插入與刪除都在尾端。
14. **`--mode en` 的 Enter 不產生換行**，見 §7.3。
15. **沒有 client crate**。framing、重連、UTF-16 轉換都要 wrapper 自己寫。

## 11. 待補

以下是寫 wrapper 時發現的協定缺口。這輪只寫文件，沒有改協定。

| # | 缺口 | 影響 | 建議 |
|---|---|---|---|
| 1 | `State` 沒有 `highlighted` | 候選框無法正確高亮；滑鼠點選不可靠 | 加 `highlighted: usize`，預設 0 |
| 2 | 選字後回 `State` 而不是 `Committed` | 必須補送 `Commit`，否則掉字 | Space／數字選字直接回 `Committed`，或加 `pending: true` |
| 3 | 沒有候選窗開關旗標 | 數字鍵與 Space 的語意無法預測 | 加 `candidate_window: bool` |
| 4 | 沒有「這個鍵被吃掉」的回應 | pass-through 只能靠 `preedit` 猜 | 加 `handled: bool` 或 `consumed` |
| 5 | `Shutdown` 實際會回 `State`，`docs/proto.md` 寫無回應 | 文件與實作不一致 | 更新 `proto.md`，或改成真的不回 |
| 6 | `Shutdown` 不檢查握手 | 任何本機行程都能關引擎 | 要求先 `Hello`，或加權限檢查 |
| 7 | `docs/proto.md` 說 Esc 後 `preedit` 為空 | 候選窗開著時 Esc 只關窗 | 文件補上 Esc 的三段優先序 |
| 8 | `docs/proto.md` 說每個 Key 都回 `State` | Enter 回 `Committed`，錯誤回 `Error` | 文件補上回應矩陣 |
| 9 | `docs/proto.md` 說組字游標固定在前端且不支援中途插入 | Home／End 可移動注音游標格；`cursor` 固定結尾，`cursor_rune` 才是真實位置 | 文件改用 `cursor_rune` 描述真實游標 |
| 10 | `Ready` 沒有通告 `max_frame` 與能力 | wrapper 只能猜 | 加 `max_frame`，之後再加 `features` |
| 11 | 沒有切換模式、IME 開關的請求 | 每個模式要一個引擎 | 加 `set_mode`，或明確寫成 wrapper 責任 |
| 12 | 沒有設定頻道 | 簡繁、標點、風格都打不開 | 加 `configure` 請求，或在啟動時吃設定檔 |
| 13 | 沒有 undo／forget | 無法撤銷學習或提交 | 加 `undo`／`forget` |
| 14 | 沒有焦點事件 | 共用引擎時組字會跨應用程式殘留 | 加 `focus_lost`，或由 wrapper 保證送 Esc |
| 15 | 多連線序列處理 | 第二個應用程式會卡住 | per-connection 執行緒或 broker 契約 |
| 16 | 沒有 named pipe | Windows 無法共用引擎 | 見 `named-pipe` 任務 |
| 17 | 沒有結構化候選 | 無法顯示讀音與來源 | v3 |
| 18 | 學習資料不落盤 | 重啟就忘 | 加儲存路徑與 flush 時機 |
| 19 | `docs/proto.md` 尚未 commit 進 `main` | 本分支的相對連結會斷；合約以該檔為準 | 由協定任務把它入版控，再合併 |

第 1、2、3 項對 wrapper 的影響最大。在這三項補上之前，候選窗只能做
「顯示字串、上下鍵選、Space 確定、再補 Commit」這一條最保守的路徑。

## 附錄 A：最小往返

以下流程和 `crates/jc-engine/tests/stdio.rs` 的整合測試一致，用
`--allow-empty-data` 就能重現。

```text
→ [len]"{"op":"hello","protocol":2,"client":"jc-tsf/0.1.0"}"
← [len]"{"op":"ready","protocol":2,"engine":"jc-engine 0.1.0"}"

→ [len]"{"op":"key","key":{"code":{"kind":"char","value":"ㄇ"},"text":"ㄇ","shift":false}}"
← [len]"{"op":"state","preedit":"ㄇ","cursor":1,"candidates":[],"cursor_rune":1}"

→ [len]"{"op":"key","key":{"code":{"kind":"char","value":"ㄚ"},"text":"ㄚ","shift":false}}"
← [len]"{"op":"state","preedit":"ㄇㄚ","cursor":2,"candidates":[],"cursor_rune":2}"

→ [len]"{"op":"snapshot"}"
← [len]"{"op":"state","preedit":"ㄇㄚ","cursor":2,"candidates":[],"cursor_rune":2}"

→ [len]"{"op":"key","key":{"code":{"kind":"backspace"},"text":null,"shift":false}}"
← [len]"{"op":"state","preedit":"ㄇ","cursor":1,"candidates":[],"cursor_rune":1}"

→ [len]"{"op":"commit"}"
← [len]"{"op":"committed","text":"ㄇ"}"

→ [len]"{"op":"snapshot"}"
← [len]"{"op":"state","preedit":"","cursor":0,"candidates":[]}"

→ [len]"{"op":"shutdown"}"
← [len]"{"op":"state","preedit":"","cursor":0,"candidates":[]}"
   （引擎隨後結束；契約文件寫無回應，見 §11）
```

有載入注音資料時，依序送 `j`、`i`、`3`，`candidates[0]` 會是 `我`；
再送 `Commit` 會拿到 `{"op":"committed","text":"我"}`。這是
`crates/jc-engine/tests/engine_data.rs` 驗證過的行為。要拿到兩個以上候選
與 Space 選字，需要實際詞典資料，流程見 §6.3 與 §7.2。

`[len]` 是 4 bytes 小端，值是後面 JSON 的 byte 數。上面的 JSON 為了可讀
才換行，實際傳送是一整段。

## 附錄 B：程式對照

| 主題 | 檔案 |
|---|---|
| 請求／回應型別、版本常數 | `crates/jc-proto/src/lib.rs` |
| 訊框編解碼、上限 | `crates/jc-proto/src/frame.rs` |
| 讀寫完整訊息 | `crates/jc-proto/src/message.rs` |
| 引擎命令列、傳輸 | `crates/jc-engine/src/main.rs` |
| 握手、請求分派、連線迴圈 | `crates/jc-engine/src/lib.rs` |
| 資料載入與模型安裝 | `crates/jc-engine/src/loader.rs` |
| 按鍵型別 | `crates/jc-core/src/key.rs` |
| 按鍵行為、候選、提交 | `crates/jc-core/src/session/core.rs`、`session/mod.rs` |
| 快照欄位 | `crates/jc-core/src/output/mod.rs` |
| 注音選字模型與逾時 | `crates/jc-core/src/zh/select.rs` |
| stdio 整合測試 | `crates/jc-engine/tests/stdio.rs` |
| socket 整合測試 | `crates/jc-engine/tests/socket.rs` |
| 未來的行程內契約 | `crates/jc-platform/src/host.rs`，尚未接進引擎 |
