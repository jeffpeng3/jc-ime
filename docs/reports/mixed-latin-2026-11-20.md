# mixed-latin：羅馬字與注音混在同一段拉丁字串的切分（研究原型）

日期：2026-11-20

## 結論

- `rishireej/ savul3` 實測輸出 **`りしれ供さ小`**。
- `rishireej/ sasmall` 實測輸出 **`りしれ供さsmall`**：`small` 由英文詞表判定
  為照字面的片段，沒有被拆成 `s`＋`mall` 或誤判成羅馬字。
- 開關 `mixed::Config::mixed_latin_split` 與 `SessionConfig::mixed_latin_split`
  預設都是 `false`；關閉時四組題庫完全不變（zh-golden 45/45、ja modern-500
  92.2%／top5 99.8%、fuzzy zh 44.5%、mixed 6/6）。
- 切分器本身每鍵成本在 GB10、release 下的實測：fixture p50 < 0.001 ms、
  p95 < 0.001 ms、最壞 0.002 ms；200 字元的病態輸入 p50 0.003 ms、
  p95 0.005 ms、最壞 0.029 ms。
- 注音轉換不是瓶頸：`ej/` ＋空白 → `供`（候選 供／宮／公／攻／共，第 1 名
  供）；`vul3` ＋空白 → `小`（第 1 名）。切分正確就有正確結果，不需要特殊
  詞條。
- 建議：可以開，但只建議開在 `Mode::Mixed`，且先修完「已知缺口」一節的
  逐鍵顯示跳動與空白語意統一；`Mode::Zh`／`Mode::Ja` 不要開。

## 演算法

### 狀態與轉移

動態規劃的狀態是 `(位置, 目前語言)`，位置以字元索引計算：

```
cells[0][Ja] = cells[0][Zh] = 0

for j = 1..=n:
    cells[j][Ja] = max over i, L:
        cells[i][L] + score_ja(syllable[i..j])
                     + switch_cost(L, Ja)      if L != Ja
    cells[j][Zh] = max over i, L:
        cells[i][L] + score_zh(syllable[i..j])
                     + switch_cost(L, Zh)      if L != Zh

    if input[j-1] is whitespace:
        # 停頓：空白不成片段，語言不變
        cells[j][L] = max(cells[j][L], cells[j-1][L])
        # 一聲：注音音節已完整時，空白屬於該音節
        if next char is not whitespace:
            for i where zh_syllable(i, j-1) is complete and untoned:
                cells[j][Zh] = max(..., cells[i][L] + score_zh(i, j-1, tone1))

answer = argmax over L of cells[n][L]
```

轉移一次吃一個完整音節。還原時從結尾格沿 `prev_pos`／`prev_lang` 走回 0，
再用同一個文法解析每一個片段取得讀音，因此不需要額外保存每個格子的字串。

### 分數

| 項目 | 分數 | 說明 |
|---|---|---|
| `piece_cost` | -2 | 每個音節的固定成本，抑制把一段拆太碎 |
| `switch_cost` | -3 | 切換語言的成本 |
| `zh_toned_bonus` | +2 | 有聲調鍵的注音音節 |
| `zh_untoned_cost` | -2 | 沒有聲調的注音音節 |
| `zh_word_bonus` | +3 | 有聲調的注音音節在詞典有完整詞條（有序） |
| `zh_prefix_bonus` | +1 | 有聲調的注音音節是詞條開頭（無序） |
| `ja_word_bonus` | +4 | かな在日文詞典有詞條（目前沒有接入） |
| `en_word_cost` | -6 | 英文詞照字面；只接受 3 個字元以上、全小寫的詞表單字 |
| `en_common_weight` | 100.0 | 英文詞權重達到這個值才算「常見英文」 |
| `en_common_bonus` | +4 | 常見英文的加分；讓 `code` 留在英文、`kanji` 走日文 |
| `prefer_ja` | 同分偏好日文 | 與現有路由的保守預設一致；語言順序是日文 → 注音 → 英文 |

注音詞典的鍵都帶聲調，`ZhLexiconAdapter` 對沒有聲調的音節依序試五個聲調，
所以 `ㄍㄨㄥ` 能命中 `供`、`公`。詞典加分只給**有聲調**的音節：沒有聲調的
讀音只是猜測，若也給加分，`small` 會被拆成三個「剛好有字」的無調音節。
分數相同時先比片段數，再依語言順序選路徑；結尾格同分時也依同一個順序。

### 逐鍵增量

`cells[i]` 只依賴 `i` 以前的字元，往前追加字元不會改變任何既有格。因此
`LatinSplitter::set_text` 在「新內容是舊內容的前綴延伸」時只把新增字元餵進
尾端，每次按鍵只計算新的一格；只有內容被改寫（退格、換字）才整串重建。
`test_incremental_matches_full` 驗證增量結果等於整串重算。

### 空白：一聲、停頓與候選觸發

空白同時有三種角色，原型照下列順序處理：

1. **注音音節的一聲**：注音音節已完整（有韻母或可獨立）且後面是空白時，
   空白是那個音節的一聲，屬於該音節，不是分隔符。`ej/ ` 是一個片段
   `ㄍㄨㄥ`（一聲），不是「`ej/` 然後分段」。連續空白只讓最後一個當一聲。
2. **日文段尾的停頓**：日文音節不吃空白；空白只當停頓，不產生片段。
3. **候選觸發**：空白原本在注音模式是「開候選／觸發轉換」。原型先把它當
   一聲／音節終止，再讓外殼觸發轉換；不把空白當段分隔。

`ej/ `、`vul3`、`ej/ sa` 三個案例都在 `test_space_is_first_tone_for_zhuyin`、
`test_space_after_kana_is_pause`、`test_space_tone_then_japanese` 裡。

## 文法覆蓋範圍

### 羅馬字段

- 來源是 `ja::input::default_romaji_table()` 的全部對照項：`a`、`ka`、
  `shi`、`kya`、`-` 等，輸出かな。
- `n`、`nn` → `ん`。
- 促音：相同子音連打時，推導「促音＋かな」的完整音節（`kka` → `っか`、
  `kitte` → `きって`），避免 `k` 留在中間無法解釋。
- 不使用促音以外的特殊寫法；`tcha` 這類非對照表寫法不支援。

### 注音段

- 使用既有大千鍵盤 `zh::keyboard::Keyboard::standard()`。
- 按鍵必須照聲母、介音、韻母、聲調的順序，每一類最多一個；音節本體必須
  通過 `zh::pinyin::is_legal_base`，例如 `ㄍㄨㄥ`、`ㄒㄧㄠ`。
- 聲調鍵 `6`／`3`／`4`／`7` 固定音節；空白是一聲（見上一節）。
- 音節讀音不帶調時保留原字串，轉換前用 `toned_syllables` 依詞典補聲調。

### 英文字段

- 只接受 3 個字元以上、全小寫、英文詞表查得到的單字；整串照字面。
- 單字母與未知字串不當英文，避免 `s`、`a` 這種單字母把正常的羅馬字切壞。
- `small` 這類「開頭像某個音節」的詞由詞表判定，不會在 `sma` 的位置被切成
  半個詞；`rishireej/ sasmall` 的 `sa` 仍走羅馬字，`small` 走英文。
- 完整羅馬字與常見英文的取捨用英文詞表的權重決定：
  - `code`（權重 288）是常見英文，加分後留在 `code`。
  - `kanji`（權重 40）是低頻借詞，完整かな解析（かんじ）分數較高，走日文。
  - `hello`（297）留在英文；`small`（65）雖然權重不高，但 `sm` 不是合法
    羅馬字，英文仍然勝出。
- 英文只在羅馬字與注音都無法完整解析，或英文詞權重達到常見門檻時勝出。

## 實測

### 目標字串

`./target/release/jc-cli mixed-split --keys "rishireej/ savul3"`：

```
輸出："りしれ供さ小"
切分（score -11）：
  ja   0..7   "rishire" → "りしれ"
  zh   7..11  "ej/ "    → "ㄍㄨㄥ"     # 空白是一聲
  ja  11..13  "sa"      → "さ"
  zh  13..17  "vul3"    → "ㄒㄧㄠˇ"
```

`./target/release/jc-cli mixed-split --keys "rishireej/ sasmall"`：

```
輸出："りしれ供さsmall"
切分（score -20）：
  ja   0..7   "rishire" → "りしれ"
  zh   7..11  "ej/ "    → "ㄍㄨㄥ"     # 空白是一聲
  ja  11..13  "sa"      → "さ"
  en  13..18  "small"   → "small"      # 英文詞表判定為照字面
```

`small` 沒有被拆成 `s`＋`mall`，也沒有被當成羅馬字：`sm` 不是合法羅馬字，
詞表命中 `small` 後整串照字面。這組同時驗證注音一聲、羅馬字與英文三種片段。

注音轉換實測：`ej/` ＋空白 → 供（第 1 候選，候選為 供／宮／公／攻／共）；
`vul3` ＋空白 → 小（第 1 候選）。所以問題只在切分，不在注音解碼。

### 合成測試（19 組）

| 輸入 | 期望 | 實測 |
|---|---|---|
| `rishireej/ savul3` | りしれ＋ㄍㄨㄥ＋さ＋ㄒㄧㄠˇ | 一致 |
| `rishireej/ sasmall` | りしれ＋ㄍㄨㄥ＋さ＋`small` | 一致 |
| `ej/ ` | ㄍㄨㄥ（一聲） | 一致 |
| `ej/` | ㄍㄨㄥ（無調） | 一致，分數低於一聲 |
| `vul3` | ㄒㄧㄠˇ | 一致 |
| `vul3 ` | ㄒㄧㄠˇ，空白是停頓 | 一致 |
| `ej/ sa` | ㄍㄨㄥ＋さ | 一致（供さ） |
| `sakura` | さくら | 一致 |
| `kitte` | きって | 一致 |
| `shinjuku` | しんじゅく | 一致 |
| `kanji` | かんじ（低頻借詞走日文） | 一致 |
| `code` | code（常見英文留在英文） | 一致 |
| `ru/3` | ㄐㄧㄥˇ | 一致 |
| `2j/4` | ㄉㄨㄥˋ | 一致 |
| `y3` | ㄗˇ | 一致 |
| `ji` | じ（同分偏好日文） | 一致 |
| `ji3` | ㄨㄛˇ（j＝ㄨ、i＝ㄛ） | 一致 |
| `ru3` | ㄐㄧˇ（r＝ㄐ、u＝ㄧ） | 一致 |
| `qqq`／`123`／`3d` | 無法完整解析，回 `None` | 一致 |

### 每鍵延遲

環境：GB10、`--release`、`mixed-split --incremental --repeat`，只量切分器
（增量 DP，不含注音與日文解碼）。

| 輸入 | 長度 | p50 | p95 | 最壞 |
|---|---|---|---|---|
| `rishireej/ savul3` | 17 字元 | < 0.001 ms | < 0.001 ms | 0.004 ms |
| 同片段重複 4 次 | 68 字元 | 0.001 ms | 0.001 ms | 0.006 ms |
| `jiji…` 重複 50 次 | 200 字元 | 0.003 ms | 0.005 ms | 0.021 ms |

每鍵成本與前面已解析的長度無關，只跟新尾端附近的音節長度
（`MAX_SYLLABLE_CHARS = 8`）與英文詞長度上限（`MAX_ENGLISH_CHARS = 48`）有關。

### 對現有題庫的影響

開關預設關閉，以下四組與基準相同（release 實測）：

| 題庫 | 結果 |
|---|---|
| `eval zh-golden` | 45/45（100.0%） |
| `eval ja --corpus modern-500` | 92.2%（top5 99.8%） |
| `eval fuzzy` | zh 44.5%（44.5%／44.5%） |
| `eval mixed` | 6/6 |

## 已知缺口

1. **逐鍵顯示會跳動**：切分器只能看到目前的字串。打到 `rishireej` 時
   `ej` 已是合法注音音節，畫面會先顯示 `りしれㄍㄨ`，補上 `/` 才變
   `りしれ供`。送出結果正確，顯示穩定性要另外處理（例如尾端音節延後固定）。
2. **1–2 個字母的英文**：英文詞只接受 3 個字元以上；`hi`、`go` 在開關
   開啟時會被當成かな。完整英文單字（`hello`、`small`）沒有這個問題。
3. **低頻英文詞的取捨**：英文詞用權重 100 當「常見」門檻，這是用
   82k 詞表的分布取的近似值。`code`（288）、`hello`（297）留在英文；
   `kanji`（40）、`sake`（65）走完整羅馬字。若某個低頻英文詞剛好能完整
   排成かな，會被當成日文；需要逐詞調整時再改門檻或加白名單。
4. **注音亂序快打**：DP 只接受聲母→介音→韻母→聲調的順序；`2/u` 這類
   快打由既有 `AsciiInput` 處理，不在切分器裡。
5. **日文詞典加分未接入**：`JaDecoder` 沒有公開詞條查詢入口，`ja_word_bonus`
   目前永遠是 0。接上之後可以改善純羅馬字的歧義。
6. **かな只輸出讀音**：切分產生的日文片段照かな輸出，不送かな漢字変換。
   自動変換會把非詞彙的 `りしれ` 誤轉成 `里`；原型先保留かな，変換留給後續
   候選。
7. **空白語意待統一**：注音模式的空白是「一聲＋開候選」，日文與 Mixed 模式
   目前忽略空白。切分器已照注音語意把空白當一聲／音節終止，Mixed 模式的
   外殼還沒有統一的「先一聲、再觸發轉換」流程。待修項目；不要把空白改成
   段分隔。統一時建議照這個順序：先當一聲／音節終止，再觸發轉換。

## 能不能開、建議用在哪個模式

可以開，但只建議開在 `Mode::Mixed`。理由：

- 切分器需要同時有羅馬字與注音按鍵才有意義；`Mode::Zh` 與 `Mode::Ja` 的
  既有行為更單純，開啟只會增加誤判。
- Mixed 模式本來就會遇到「一段拉丁字到底算哪種語言」的問題，這個原型把
  判斷從「整段二選一」換成「音節層級的最佳路徑」，`rishireej/ savul3` 與
  `rishireej/ sasmall` 就是現有路由做不到的例子。
- 開啟前建議先處理逐鍵顯示跳動與空白語意；這兩項不影響送出正確性，但會
  影響使用體驗。

## 檔案

- `crates/jc-core/src/mixed_latin.rs`（新）
- `crates/jc-core/src/mixed.rs`：`Config::mixed_latin_split`、切分器接入、
  `visible_plans`
- `crates/jc-core/src/session/mod.rs`、`crates/jc-core/src/session/mixed.rs`：
  `SessionConfig::mixed_latin_split`、增量同步與 `Composition` 片段
- `crates/jc-core/src/ja/input.rs`：`Table::entries`、`can_sokuon` 可見性
- `crates/jc-cli/src/split_cmd.rs`（新）、`cli.rs`、`lib.rs`：`mixed-split`
- `docs/port/mixed-latin.md`

## 附錄：`watashi` 被當成注音與「卡住」案例（2026-11-21）

### 靜態重現

`JC_TESTDATA=/mnt/work/jc/jc-ime-data/0c-testdata ./target/debug/jc-cli mixed-split --keys watashi --steps`：

```
輸出："私"
DP（1 個 run）：
  run "watashi"
    ja   0..7   "watashi" → "わたし"
後處理 3/3 片段合併：
  ja latin   0..7   "watashi"
片段：
  ja latin "watashi" → "私"
```

DP（A）與路由（B）都能把 `watashi` 解成 `わたし`，有日文解碼器時輸出 `私`。
路由沒有 bug。

逐鍵模擬 `Session`（同一份注音詞典與資料）顯示：

| 模式 | 逐鍵結果 | 空白與後續 |
|---|---|---|
| `Mode::Zh` | `w a t a s h i` 各鍵覆蓋同一個注音槽，依次顯示 `ㄊ ㄇ ㄔ ㄇ ㄋ ㄘ ㄘㄛ` | 空白只把 `ㄘㄛ` 放進候選；外觀像卡在注音 |
| `Mode::Mixed` | `w → w → わ → わ吃 → わた → watas → watash → わたし` | 空白後接 `desu` 得 `わたしです`，Enter 送出 `わたしです` |

`Mode::Zh` 把拉丁鍵當注音鍵是設計；模式中每一鍵仍會改組字區，核心沒有
狀態死結。`Mode::Mixed` 下所有按鍵都能推進，也沒有卡住。

### 歸因與處置

`crates/jc-engine/src/config.rs` 的 `mode_from_settings` 順序是「混打 → 日文 →
英文 → 注音」。原本 `mixed-input = off` 讓引擎進 `Mode::Zh`，才是使用者看到
注音與無法繼續打字的根本原因。

依使用者指示，`mixed-input` 開關直接移除，引擎對中文使用者永遠進
`Mode::Mixed`。本 task 不改開關、不改預設值、不改路由邏輯，只補回歸測試：
`watashi` 進 mixed 題庫、路由器語言判定與表面輸出、`Mode::Mixed` session 的
連續輸入與送出。

### 驗證

- `cargo test -p jc-core -p jc-settings`：890 passed。
- `cargo test -p jc-core watashi`：3 passed。
- `mixed-split --keys watashi --steps`：輸出 `私`，片段 `ja latin`。
