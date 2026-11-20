# segment-diff：兩種分詞方式的大規模差異盤點（2026-11-21）

## 1. 摘要

用固定種子產生 20000 筆混合輸入，同時跑 A（離線 DP，`mixed_latin::split`）與 B（正式路由，`Router::convert` 且 `mixed_latin_split: true`）。結果：

- 一致 1787 筆（8.94%），不一致 18213 筆（91.06%）。
- A 無法完整解析 15656 筆（78.28%）。這是 A 的輸入範圍比 B 窄造成的，並非 B 的轉換錯誤。
- A 有結果的 4344 筆中，一致 1787 筆（41.14%），不一致 2557 筆（58.86%）。
- 沒有任何 B 轉換失敗或 B 完全沒有片段的案例。

最重要的結論：

1. **不含空白的輸入，只要 A 能解析，兩邊只有在 run 開頭或尾端的標點上不一致。** 2420 筆無空白不一致中，A 有結果的只有 43 筆，全部是 `trailing-punct-stripped` 或 `leading-punct`。
2. **所有語言衝突（`ja-vs-zh`、`ja-vs-en`、`en-word-missing`、`en-word-extra`、`short-en`）都發生在含空白的輸入。** 原因是 A 對整串做全域 DP，B 先以空白切 run，再逐 run 呼叫同一個 DP。
3. A 把空白當一聲的規則，是 A 有結果時最大的不一致來源：`space-tone-vs-separator` 1678 筆，佔 A 有結果不一致的 65.6%。
4. 真正像 bug 的只剩兩類：A 把 run 邊界標點當成獨立注音音節（179 筆），以及 A/B 對同一個英文詞做出不同判斷（8 筆）。

## 2. 跑法與參數

```bash
cargo run -q -p jc-cli -- mixed-diff --seed 1 --count 20000 --top 5 \
  --cases /tmp/mixed-diff-cases.json
```

- seed：`1`
- count：`20000`，實際產生 20000 筆，20000 筆皆唯一。
- top：`5`
- 資料：`JC_TESTDATA` 未指定時用 `/mnt/work/jc/jc-ime-data/0c-testdata`；注音載入 `<資料根>/build-zh` 的 `lexicon.bin` 與 `lm.bin`，日文用預設 pack，英文用 `English::load_default()`（`<資料根>/english/words.bin`）。
- 執行時間：約 90 秒（debug build，含資料載入）。
- 相同參數重跑兩次，輸出逐位元組相同。

機器可讀輸出：

```bash
# 完整 JSON，含摘要、各桶統計、代表案例與所有不一致案例
cargo run -q -p jc-cli -- mixed-diff --seed 1 --count 20000 --json > /tmp/mixed-diff.json

# 只寫不一致案例，供之後當回歸 fixture
cargo run -q -p jc-cli -- mixed-diff --seed 1 --count 20000 --cases /tmp/mixed-diff-cases.json
```

## 3. 產生器與片段池

`mixed-diff` 的片段池分七類，每次組裝先均勻抽一類，再從該類抽一個片段，確保每類都會出現：

| 類別 | 內容 |
|---|---|
| 注音 | `zh::pinyin::inventory()` 的全部合法音節，轉成大千按鍵；每個音節有不帶調、二三四聲、輕聲，以及「音節＋空白」的一聲版本。自然涵蓋以 `.`（ㄡ）、`,`（ㄝ）、`;`（ㄤ）、`/`（ㄥ）、`-`（ㄦ）結尾的韻母。 |
| 羅馬字 | `default_romaji_table()` 的全部 ASCII 項目，外加促音（例如 `kka`、`ssa`）與 `n`、`nn`。 |
| 英文長詞 | `Dictionary` 中 3–12 個小寫字母的詞。 |
| 英文短詞 | 1–2 個字母的詞，並補上 `a`、`i`、`is`、`to` 等固定詞。 |
| 數字與大寫 | `5k42`、`Hello`、`ABC`、`API`、`3d`、`123`、`2026`、`v2`、`ＡＢＣ`。 |
| run 中間標點 | `hello.world`、`abc,def`、`sa;ku`、`ab/cd`、`ab-cd`、`foo:bar`、`hi!there`、`a?b`、`vul3.abc`、`5k42.g4`，另加全角 `hello，world`、`abc。def`。 |
| run 尾端標點 | `hello.`、`world,`、`abc;`、`foo:`、`ab/`、`cd-`、`hi!`、`no?`、`5k42.`、`vul3.`、`ej/;`、`g4n0,`。 |

組裝規則：抽 2–6 個片段，片段之間各有 50% 機率插入一個空白；長度不在 3–40 字元就重抽；重複輸入會被丟棄。隨機源是 SplitMix64，只依 seed 決定。

## 4. 比對與分類方法

### 4.1 不一致的判定

把兩邊都正規化成「語言＋輸入位元組範圍」的序列：

- A：每個 `Piece` 的 `lang` 與 `start..end`。注音片段結尾的一聲空白會落在範圍內。
- B：每個 `MixedSegment` 的 `mode`＋`script` 與 `input_start..input_end`。純空白片段忽略；非空白 `Script::Symbol` 記為 `sym`。

符合任一條就算不一致：

1. A 是 `None` 而 B 有非空白片段，或反過來。
2. 語言序列不同。
3. 任一段的位元組邊界不同。
4. 語言與邊界都相同，但字面不同。A 只產生讀音，B 會把日文與注音送進解碼器，因此第 4 條只對英文片段比較（A 的 `reading` 對 B 的 `surface`）。這次 20000 筆沒有命中第 4 條。

### 4.2 分類

一個案例可能同時命中多個訊號，報告採用第一個訊號當主分類。優先序：

`b-error` → `atom-uppercase-fullwidth` → `trailing-punct-stripped` → `leading-punct` → `space-tone-vs-separator` → `ja-vs-zh` → `en-word-missing` → `en-word-extra` → `short-en` → `ja-vs-en` → `mid-run-symbol` → `a-none` → `space-merged-segments` → `b-empty`

`--json` 的每個案例保留完整 `signals` 陣列，可還原多標籤情形。

## 5. 總覽統計

| 指標 | 數量 | 比例 |
|---|---:|---:|
| 產生輸入 | 20000 | 100% |
| 一致 | 1787 | 8.94% |
| 不一致 | 18213 | 91.06% |
| A 無法解析 | 15656 | 78.28% |
| A 有結果 | 4344 | 21.72% |
| A 有結果且一致 | 1787 | 41.14%（以 A 有結果為分母） |
| A 有結果且不一致 | 2557 | 58.86%（以 A 有結果為分母） |
| B 轉換失敗 | 0 | 0% |
| B 沒有非空白片段 | 0 | 0% |

各桶數量與佔不一致總數的比例：

| 分類 | 數量 | 比例 | A 有結果 |
|---|---:|---:|---:|
| `atom-uppercase-fullwidth` | 4145 | 22.76% | 0 |
| `trailing-punct-stripped` | 162 | 0.89% | 162 |
| `leading-punct` | 17 | 0.09% | 17 |
| `space-tone-vs-separator` | 1678 | 9.21% | 1678 |
| `en-word-missing` | 5 | 0.03% | 5 |
| `en-word-extra` | 3 | 0.02% | 3 |
| `short-en` | 152 | 0.83% | 152 |
| `ja-vs-zh` | 18 | 0.10% | 18 |
| `ja-vs-en` | 301 | 1.65% | 301 |
| `mid-run-symbol` | 1504 | 8.26% | 0 |
| `space-merged-segments` | 221 | 1.21% | 221 |
| `a-none` | 10007 | 54.94% | 0 |
| `b-empty` | 0 | 0% | 0 |
| `b-error` | 0 | 0% | 0 |
| `other` | 0 | 0% | 0 |

無空白子集：

| 指標 | 數量 |
|---|---:|
| 無空白不一致 | 2420 |
| 其中 A 無法解析 | 2377 |
| 其中 A 有結果 | 43 |
| A 有結果的無空白不一致：`trailing-punct-stripped` | 40 |
| A 有結果的無空白不一致：`leading-punct` | 3 |

所有 `ja-vs-zh`、`ja-vs-en`、`en-word-*`、`short-en`、`space-tone-vs-separator`、`space-merged-segments` 案例都含空白，沒有例外。

## 6. 各桶案例與原因

以下 A 的格式是「語言 位元組範圍 輸入片段 → 讀音」，B 是「模式/書寫系統 範圍 輸入片段 → 輸出字面」。

### 6.1 `atom-uppercase-fullwidth`（4145）

```
APIh
A：無法解析
B：en/latin 0..4 "APIh" → "APIh"

gＡＢＣ
A：無法解析
B：en/mixed 0..10 "gＡＢＣ" → "gＡＢＣ"

nABC
A：無法解析
B：en/latin 0..4 "nABC" → "nABC"

ＡＢＣd
A：無法解析
B：en/mixed 0..10 "ＡＢＣd" → "ＡＢＣd"
```

原因：A 的 `is_latin_split_input` 直接拒絕大寫與全角，B 的 atom 路徑把整段當英數照字面輸出。

### 6.2 `trailing-punct-stripped`（162）

```
ye,
A：ja 0..2 "ye" → "いぇ"、zh 2..3 "," → "ㄝ"
B：ja/latin 0..2 "ye" → "いぇ"、ja/symbol 2..3 "," → ","

yworld,
A：zh 0..1 "y" → "ㄗ"、en 1..6 "world" → "world"、zh 6..7 "," → "ㄝ"
B：zh/latin 0..1 "y" → "資"、en/latin 1..6 "world" → "world"、en/symbol 6..7 "," → ","

1i6hello.
A：zh 0..3 "1i6" → "ㄅㄛˊ"、en 3..8 "hello" → "hello"、zh 8..9 "." → "ㄡ"
B：zh/latin 0..3 "1i6" → "柏"、en/latin 3..8 "hello" → "hello"、en/symbol 8..9 "." → "."

chehello.
A：ja 0..3 "che" → "ちぇ"、en 3..8 "hello" → "hello"、zh 8..9 "." → "ㄡ"
B：ja/latin 0..3 "che" → "ちぇ"、en/latin 3..8 "hello" → "hello"、en/symbol 8..9 "." → "."
```

原因：A 把尾端 `,` 當 ㄝ、`.` 當 ㄡ，變成一個獨立注音音節；B 把它當標點。

注意：這 162 筆全部是「標點自己變成一個音節」的形式，不是「標點接在前一個注音音節尾巴」。後者已被 B 的 `split_latin_run` 與 `trailing_punctuation_is_zhuyin` 修掉，所以已知的 `5k42.` 類型不再出現在這桶。

### 6.3 `leading-punct`（17）

```
,3yj7
A：zh 0..5 ",3yj7" → "ㄝˇ ㄗㄨ˙"
B：en/symbol 0..1 "," → ","、en/latin 1..5 "3yj7" → "3yj7"

-3 njp4
A：zh 0..2 "-3" → "ㄦˇ"、zh 3..7 "njp4" → "ㄙㄨㄣˋ"
B：en/symbol 0..1 "-" → "-"、en/digit 1..2 "3" → "3"、en/symbol 2..3 " " → " "、zh/latin 3..7 "njp4" → "孫"

/vul3.ck
A：zh 0..8 "/vul3.ck" → "ㄥ ㄒㄧㄠˇ ㄡ ㄏㄜ"
B：zh/symbol 0..1 "/" → "/"、zh/latin 1..8 "vul3.ck" → "小歐喝"
```

原因：與 6.2 同一個根，A 允許 `. , ; / -` 在任何位置單獨成為韻母；B 把它們留在 run 邊界當符號。

### 6.4 `space-tone-vs-separator`（1678）

```
lo y
A：zh 0..4 "lo y" → "ㄠ ㄟ ㄗ"
B：en/latin 0..4 "lo y" → "lo y"

n eh
A：zh 0..2 "n " → "ㄙ"、ja 2..3 "e" → "え"、zh 3..4 "h" → "ㄘ"
B：en/latin 0..3 "n e" → "n e"、zh/latin 3..4 "h" → "差"

u pa
A：zh 0..2 "u " → "ㄧ"、ja 2..4 "pa" → "ぱ"
B：en/latin 0..4 "u pa" → "u pa"

"1ohk "（尾端一個空白）
A：zh 0..5 "1ohk " → "ㄅㄟ ㄘㄜ"
B：zh/latin 0..4 "1ohk" → "杯測"
```

原因：A 把「沒有聲調的注音音節＋空白」解讀成一聲，空白屬於該片段，還會讓兩側注音片段在輸入上連續而合併。B 一律把空白當分隔，切完 run 才做 DP。這造成邊界不同，也常連帶讓語言判定不同。

### 6.5 `en-word-missing`（5）

```
luxe programming
A：en 0..4 "luxe" → "luxe"、en 5..16 "programming" → "programming"
B：ja/latin 0..4 "luxe" → "ぅぇ"、ja/symbol 4..5 " " → " "、en/latin 5..16 "programming" → "programming"

1l3ppwavelet caramp
A：zh 0..5 "1l3pp" → "ㄅㄠˇ ㄣ ㄣ"、en 5..12 "wavelet" → "wavelet"、en 13..19 "caramp" → "caramp"
B：zh/latin 0..5 "1l3pp" → "保嗯嗯"、en/latin 5..17 "wavelet cara" → "wavelet cara"、zh/latin 17..19 "mp" → "暈"
```

原因：A 用分數決定英文是否勝出；B 在 `latin_piece` 用 `is_english_word` 當硬性 gate。兩邊對同一個詞表得到不同結論。

### 6.6 `en-word-extra`（3）

```
hhe lick
A：ja 0..3 "hhe" → "っへ"、ja 4..6 "li" → "ぃ"、zh 6..8 "ck" → "ㄏㄜ"
B：ja/latin 0..3 "hhe" → "っへ"、ja/symbol 3..4 " " → " "、en/latin 4..8 "lick" → "lick"

ttya gasp
A：ja 0..4 "ttya" → "っちゃ"、ja 5..7 "ga" → "が"、zh 7..9 "sp" → "ㄋㄣ"
B：ja/latin 0..4 "ttya" → "っちゃ"、ja/symbol 4..5 " " → " "、en/latin 5..9 "gasp" → "gasp"

bbe cushion
A：ja 0..3 "bbe" → "っべ"、ja 4..11 "cushion" → "くしおん"
B：ja/latin 0..3 "bbe" → "っべ"、ja/symbol 3..4 " " → " "、en/latin 4..11 "cushion" → "cushion"
```

原因：與 6.5 同一個根，方向相反。

### 6.7 `short-en`（152）

```
ku u
A：ja 0..2 "ku" → "く"、ja 3..4 "u" → "う"
B：en/latin 0..4 "ku u" → "ku u"

a she
A：ja 0..1 "a" → "あ"、ja 2..5 "she" → "しぇ"
B：en/latin 0..5 "a she" → "a she"

bya i
A：ja 0..3 "bya" → "びゃ"、ja 4..5 "i" → "い"
B：ja/latin 0..3 "bya" → "びゃ"、en/latin 4..5 "i" → "i"
```

原因：B 的 `group_words` 對「以單一空白相連的多字片語」做整體判定，1 個假名單位的短字會被改成英文；A 沒有片語合併。

### 6.8 `ja-vs-zh`（18）

```
dk4 xi
A：zh 0..3 "dk4" → "ㄎㄜˋ"、zh 4..6 "xi" → "ㄌㄛ"
B：zh/latin 0..3 "dk4" → "克"、ja/latin 4..6 "xi" → "ぃ"

q/4 wu
A：zh 0..3 "q/4" → "ㄆㄥˋ"、zh 4..6 "wu" → "ㄊㄧ"
B：zh/latin 0..3 "q/4" → "碰"、ja/latin 4..6 "wu" → "う"

pa qup7
A：ja 0..2 "pa" → "ぱ"、ja 3..5 "qu" → "く"、zh 5..7 "p7" → "ㄣ˙"
B：ja/latin 0..2 "pa" → "ぱ"、zh/latin 3..7 "qup7" → "ㄆㄧㄣ˙"
```

原因：A 對整串做全域 DP，前後文會影響某一段的語言；B 先切 run，每個 run 只看自己。`xi`、`wu` 單獨看時兩邊可能都判日文，但在 A 的全域路徑裡被前一段的注音拉走。

### 6.9 `ja-vs-en`（301）

```
te em
A：ja 0..2 "te" → "て"、ja 3..4 "e" → "え"、zh 4..5 "m" → "ㄩ"
B：en/latin 0..4 "te e" → "te e"、zh/latin 4..5 "m" → "淤"

va yi
A：ja 0..2 "va" → "ゔぁ"、ja 3..5 "yi" → "い"
B：ja/latin 0..2 "va" → "ゔぁ"、en/latin 3..5 "yi" → "yi"

an lyu
A：ja 0..2 "an" → "あん"、ja 3..6 "lyu" → "ゅ"
B：ja/latin 0..2 "an" → "あん"、en/latin 3..6 "lyu" → "lyu"
```

原因：全域 DP 與逐 run 切分的差異，加上 B 的片語合併會把短字改成英文。

### 6.10 `mid-run-symbol`（1504）

```
abc。defk
A：無法解析
B：en/latin 0..3 "abc" → "abc"、en/symbol 3..6 "。" → "。"、en/latin 6..9 "def" → "def"、zh/latin 9..10 "k" → "阿"

mabc。def
A：無法解析
B：en/latin 0..4 "mabc" → "mabc"、en/symbol 4..7 "。" → "。"、en/latin 7..10 "def" → "def"

abc。def p
A：無法解析
B：en/latin 0..3 "abc" → "abc"、en/symbol 3..6 "。" → "。"、en/latin 6..11 "def p" → "def p"
```

原因：任務要求的 ASCII 標點 `. , ; : / - ! ?` 全部都在 B 的 token run 字元集內，B 不會在 run 中間為它們斷開。這一桶全部來自額外加入的全角 `。` 與 `，`。若不加入全角標點，這桶是 0。

### 6.11 `space-merged-segments`（221）

```
ea kka
A：ja 0..2 "ea" → "えあ"、ja 3..6 "kka" → "っか"
B：ja/latin 0..6 "ea kka" → "えあっか"

ppe va
A：ja 0..3 "ppe" → "っぺ"、ja 4..6 "va" → "ゔぁ"
B：ja/latin 0..6 "ppe va" → "っぺゔぁ"

cche qa
A：ja 0..4 "cche" → "っちぇ"、ja 5..7 "qa" → "くぁ"
B：ja/latin 0..7 "cche qa" → "っちぇくぁ"
```

原因：B 會把以單一空白相連的同語言片段合併成一個輸出片段，A 的 `merge_pieces` 明確不合併被空白隔開的片段。語言序列相同，只是邊界不同。

### 6.12 `a-none`（10007）

```
042
A：無法解析
B：en/digit 0..3 "042" → "042"

dco
A：無法解析
B：en/latin 0..3 "dco" → "dco"

dri
A：無法解析
B：en/latin 0..3 "dri" → "dri"

h0r
A：無法解析
B：en/latin 0..3 "h0r" → "h0r"
```

原因：A 只接受全小寫、數字與 `,./;-` 組成的字串，而且整串必須能被注音、羅馬字或英文詞完整解析。`042`、`dco`、`dri`、`h0r` 都不符合，A 回 `None`；B 的 atom 路徑照字面輸出。

## 7. 哪些是 bug、哪些是設計差異

### 7.1 判定為 bug

| 分類 | 筆數 | 理由 |
|---|---:|---|
| `trailing-punct-stripped` | 162 | A 把 run 尾端標點變成獨立注音音節（`hello.` → `hello`＋ㄡ）。B 的規則是「只有前一個片段是注音、且標點接在該音節尾巴時才算韻母」。A 缺少這個限制。 |
| `leading-punct` | 17 | 同一個根，A 允許 `. , ; / -` 在 run 開頭單獨成音節。 |
| `en-word-missing` / `en-word-extra` | 8 | 兩邊用同一份英文詞表，卻對 `luxe`、`lick`、`gasp`、`cushion` 做出不同判斷。A 用分數競爭，B 用硬性 gate，這是規則不一致。 |

合計 187 筆，佔不一致 1.03%。

### 7.2 判定為設計差異

| 分類 | 筆數 | 理由 |
|---|---:|---|
| `atom-uppercase-fullwidth` | 4145 | A 刻意拒收大寫與全角，避免把 `ABC`、`ＡＢＣ` 誤判成注音；B 有專門的 atom 分支。這是 A 的覆蓋範圍較窄。 |
| `a-none` | 10007 | 同上，A 只處理「全小寫＋數字＋`,./;-`」且整串可完整解析的輸入。 |
| `space-tone-vs-separator` | 1678 | 一邊把空白當一聲，一邊把空白當分隔。兩邊各自自洽，但沒有共同規格。 |
| `ja-vs-zh` | 18 | A 全域 DP、B 逐 run DP。同一段在不同前後文下得到不同語言，屬於演算法範圍差異。 |
| `ja-vs-en` | 301 | 同上，再加上 B 的片語合併。 |
| `short-en` | 152 | B 的 `group_words` 是 A 沒有的功能。 |
| `space-merged-segments` | 221 | B 合併空白兩側的同語言片段，A 不合併。語言序列一致，差在邊界。 |
| `mid-run-symbol` | 1504 | A 沒有符號片段的概念；B 只在 token run 以外的字元（這裡是全角標點）斷開。任務指定的 ASCII 標點不會觸發。 |

### 7.3 一句話回答「哪些情況兩種分詞會不一樣」

1. 輸入含大寫、全角或不屬於 A 可解析範圍的字元：A 直接 `None`，B 產片段。這是最大宗。
2. 輸入含空白：A 可能把空白當一聲並合併兩側注音，B 一律當分隔；B 還會做片語合併。所有語言衝突都從這裡來。
3. run 開頭或尾端是 `. , ; / -`：A 當韻母，B 當符號。這是 A 的規則缺陷。
4. run 中間是全角標點：B 斷開，A 失敗。任務指定的 ASCII 標點不會造成這個差異。
5. 英文詞：A 用分數決定是否留在英文，B 用硬性詞表 gate，少數詞會得到相反結果。

## 8. 建議修法與優先序

只寫建議，這次沒有改動 `mixed.rs` 或 `mixed_latin.rs`。

| 優先 | 建議 | 影響 |
|---|---|---|
| P0 | 先寫下唯一的分詞規格：空白屬於一聲還是分隔、符號是否成為片段、`mixed_latin_split` 的 DP 範圍是整串還是逐 run。目前 A 與 B 各自都合理，但沒有共同定義，任何比較都會得到大量「差異」。 | 決定後續所有修正方向 |
| P1 | A 的標點規則收緊成 B 的版本：`. , ; / -` 只有在緊接在合法注音音節尾巴時才當韻母；run 開頭或獨立出現時不要產生注音片段。 | 直接消掉 179 筆 bug |
| P1 | 統一空白規則。若 B 是正式路徑，讓 A 也先以空白切 run，再逐 run 做 DP；或反過來讓 B 用整串 DP。 | 可消除大部分 `space-tone-vs-separator`、`ja-vs-zh`、`ja-vs-en` 與 `space-merged-segments` |
| P2 | 把英文詞判斷抽成共用函式，A 與 B 都只呼叫它。若要保留 A 的分數競爭，至少讓「詞表命中」成為必要條件。 | 消掉 8 筆英文判斷相反 |
| P2 | 把 B 的 `group_words` 片語規則搬進 A，或明確標示這是 B 專屬行為、A 不支援。 | 消掉 `short-en` 與部分 `ja-vs-en` |
| P3 | 擴充 A 的輸入範圍，或把 `mixed-split` 的 A 行標成「僅支援小寫英數與 `,./;-`」，避免使用者把 `None` 當成轉換失敗。 | 降低 `atom-uppercase-fullwidth` 與 `a-none` 的誤解 |
| P3 | 決定符號片段是否要進 A。若 A 要取代 B，必須能表示標點與全角字元。 | 影響 `mid-run-symbol` 與所有標點邊界 |

## 9. 重現與限制

- 重現指令與參數見第 2 節。相同 seed 與 count 會產生完全相同的 20000 筆輸入與統計。
- 這份量測只比較「分詞結果」，不評估候選字好壞，也不比較注音或日文解碼後的選字品質。
- 統計比例以本次片段池的分布為準。片段池刻意加重了大寫、全角、短英文與全角標點，因此 `atom-uppercase-fullwidth`、`short-en`、`mid-run-symbol` 的比例會高於一般使用者輸入。
- A 的 `None` 有 15656 筆，這不代表 B 出錯；只代表 A 的接受範圍較窄。
- 分類採單一主分類，重疊情形保留在 JSON 的 `signals` 欄位。若要重新分桶，不需要重跑量測，直接讀 `--cases` 的輸出即可。
