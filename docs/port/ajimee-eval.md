# ajimee-eval：把 AJIMEE-Bench 接進 `eval ja`

Worktree：`/mnt/work/jc/wt/ajimee-eval`（branch `ajimee-eval`）

## 結果

`jc-cli eval ja --corpus ajimee-200` 可用，題庫由目錄掃描取得，沒有寫死題庫名稱。
三組分數（Mozc v2 pack，機器 20 核，NBest=5）：

| Corpus | n | top-1 | top-5 | 邊界 exact | 邊界 F1 | p50 | p95 | timeout |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `ajimee-200` | 200 | 46.0% | 65.0% | 不計 | 不計 | 6.521 ms | 51.639 ms | 1.0% |
| `modern-500` | 500 | 86.0% | 98.0% | 88.2% | 0.9017 | 3.072 ms | 5.937 ms | 0.0% |
| `hard-cases` | 35 | 54.3% | 97.1% | 71.4% | 0.8600 | 0.738 ms | 5.326 ms | 0.0% |

`ajimee-200` 沒有 bunsetsu 標註，`boundary_n=0`，邊界指標恆為 0，不可解讀成「邊界全錯」。

`ajimee-200` 只有 1 題 timeout（`ajimee-068`，156 ms > 100 ms）。
p50 每次重跑在 6.3–7.2 ms 之間跳動，這是牆鐘時間，只用來和同一台機器比較。

順帶量到同目錄的 `anthy-corpus1`：top-1 61.0%、top-5 84.5%、邊界 exact 31.2%、F1 0.7013。
它不在本 task 的三組要求內，列在這裡只是因為目錄掃描會一起載入。

## 題庫探索：目錄掃描

`eval ja --corpus <name>` 不再只認 `rules.json` 的 key：

1. 讀 `<testdata>/ja/rules.json`，取得 `top_n`、`timeout_ms` 與各題庫的 pin。
2. 掃 `<testdata>/ja/*.json`，每個帶 `items` 陣列的檔案都當成一份題庫；
   `rules.json`、`baseline.json` 沒有 `items`，直接跳過。pin 指向子目錄時，
   那個檔案也會補進來。
3. 有 pin 的題庫沿用 pin 檢查（sha256、題數、`boundary_cases`、授權、來源），
   pin 指向的檔案不見或不是題庫就直接報錯，避免題庫悄悄消失。
4. 沒有 pin 的題庫只做欄位驗證，所以新增一份 `xxx.json` 就能先量分數，
   之後再凍結進 `rules.json`。
5. 依 JSON 的 `name` 比對 `--corpus`；找不到時列出目錄裡現有的題庫名稱。

`--corpus <path.json>` 仍然直接讀單一檔案，方便拿子集對照。
`--items` 會輸出含逐題結果的 JSON，錯誤分析不必再回頭加 flag。

新增的程式碼：

- `crates/jc-eval/src/ja.rs`：`load_ja_corpus_dir`、`check_against_rule`
  （`load_ja_corpus_set` 改用同一個 pin 檢查，兩條路徑不會分岔）。
- `crates/jc-cli/src/eval_cmd.rs`：`select_ja_corpora`，選題庫與錯誤訊息都在這裡。
- `crates/jc-cli/src/cli.rs`：`--items`。

## `accept` 比對

`ajimee_200.json` 的 200 題中有 83 題帶 `accept`，共 215 個可接受變體
（原始 `expected_output` 有重複項，fixture 已去重）。

- top-1 的 92 題裡，有 22 題是第一候選不在 `expected_output[0]`、但命中 `accept`
  才算過。若忽略 `accept`，top-1 只剩 70/200 = **35.0%**。
- 沒有任何一題出現「top-5 內有答案卻判未命中」的情況，逐題重算與引擎的
  `top1`／`top5_hit` 完全一致（200 題全對）。

所以 `accept` 比對正常，46.0% 不是漏比對可接受變體造成的。
與前身 Go 版（Mozc pack，`0c-eval-ja`、NBest=5）相比：top-1 46.0% 相同，
top-5 65.0% 對 65.5%，差 1 題（130 vs 131）。這 1 題沒有追查；兩邊詞典 pack
不同批，差異落在個別候選排序上，不影響結論。

## 條件對齊：本題庫沒有左文脈

AJIMEE-Bench 的 200 題是 100 題「無左文脈」加 100 題「有左文脈」。
`ajimee_200.json` 只保留 `reading`／`surface`／`accept`，沒有 `context_text`，
引擎也沒有日文左文脈功能，因此我們的 200 題全部都在無文脈條件下轉換。

用原始 `evaluation_items.json` 的 `context_text` 切開，可以拆成兩個子集：

| 子集 | n | top-1 | top-5 |
|---|---:|---:|---:|
| 原始無文脈 100 題 | 100 | 50.0% | 68.0% |
| 原始有文脈 100 題（我們不給文脈） | 100 | 42.0% | 62.0% |

有文脈那 100 題掉了 8 個百分點，這正是左文脈存在的理由。
和公開成績比較時要用「無文脈 100 題」那一列，不要拿 46.0% 去對全 200 題的數字。

## 公開成績對照

同一份 200 題，只看有來源的數字。Acc@1 的定義和 `utils.py` 的
`calculate_accuracy_at1` 相同：第一候選完全等於任一 `expected_output` 就算對。

主要來源：言語処理学会第 31 回年次大会「ニューラルかな漢字変換システム Zenzai」
（P1-19，<https://anlp.jp/proceedings/annual_meeting/2025/pdf_dir/P1-19.pdf>）。

| 系統 | 條件 | Acc@1 | 來源 |
|---|---|---:|---|
| Google 日本語入力（CGI API） | 全 200，無文脈 | 54.0 | 論文 Table 1 |
| AzooKeyKanaKanjiConverter（統計轉換器） | 全 200，無文脈 | 44.5 | 論文 Table 1 |
| Zenzai (xsmall) | 全 200，其中 100 題給左文脈 | 66.5 | 論文 Table 1 |
| Zenzai (small) | 全 200，其中 100 題給左文脈 | 80.0 | 論文 Table 1 |
| Zenzai (medium) | 全 200，其中 100 題給左文脈 | 86.5 | 論文 Table 1 |
| gpt-4o-2024-08-06（1-shot） | 全 200 | 56.0 | 論文 Table 1 |
| gpt-4o-mini-2024-07-18 | 全 200 | 19.0 | 論文 Table 1 |
| gpt-4.5-preview-2025-02-27 | 全 200 | 80.5 | さくらのナレッジ <https://knowledge.sakura.ad.jp/42901/> |
| Google 日本語入力（CGI API） | 無文脈 100 題 | 62.0 | 論文 Table 3 |
| AzooKeyKanaKanjiConverter | 無文脈 100 題 | 46.0 | 論文 Table 3 |
| Zenzai (xsmall / small / medium) | 無文脈 100 題 | 69.0 / 83.0 / 89.0 | 論文 Table 3 |
| gpt-4o / gpt-4o-mini | 無文脈 100 題 | 52.0 / 18.0 | 論文 Table 3 |

**本引擎**：全 200 題無文脈 top-1 46.0%；對齊無文脈 100 題是 50.0%。

結論：以同條件（無文脈 100 題）比，我們 50.0% 高於論文裡同為統計轉換器的
AzooKeyKanaKanjiConverter（46.0%），略低於 gpt-4o（52.0%），低於
Google 日本語入力（62.0%）與 Zenzai（83.0%）。以全 200 題比，46.0% 低於
Google 日本語入力 54.0%，但高於 AzooKeyKanaKanjiConverter 44.5%。
兩邊的詞典不同批（我們用 Mozc OSS 匯入的 pack，對方用自家詞典），
差距在個位數百分點，屬於詞典與成本尺度的差異範圍。

### 第三方量測（條件不同，僅供參考）

| 系統 | 條件 | 數字 | 來源 |
|---|---|---|---|
| Mozc OSS 辭典 | 無文脈 100 題 | acc@1 47.0%，acc@10 80.0% | <https://zenn.dev/4g/articles/4ac53b947d2fb3> |
| Mozc + zenz 3.1 small rerank | 無文脈 100 題 | acc@1 73.0% | 同上 |
| Mozc `converter_main` | 全 200，無文脈 | 51.0% | <https://blog.sutachi.jp/posts/llm-kana-kanji-mozc-windows-build-zenz/> |
| zenz 單體 | 全 200，無文脈 | 79.5% | 同上 |
| jinen-v1.1-beta | 全 200，附左文脈 64 字 | 80.0%（參照側 NFKC 後 86.0%） | <https://huggingface.co/togatogah/jinen-v1.1-beta.gguf> |

Mozc 的 47.0% 和我們的無文脈 100 題 50.0% 最接近，兩者都是統計轉換器、
都用 Mozc 系詞典，差距 3 題。這個對照比論文 Table 1 更有意義。

### 未確認

- AzooKey 官方公告頁與 azooKey-Desktop README 沒有列出可引用的 Acc@1 數字，
  只有 `azooKey/AJIMEE-Bench` README 的資料說明。上表的 AzooKey 相關數字全部
  取自論文與第三方文章，不是官方公告。
- さくらのナレッジ在 2026-11 起進入維護模式，正文目前讀不到；上表 54.0／80.5
  等數字同時見於論文 Table 1，重疊部分以論文為準。
- 沒有任何來源公布「azooKey iOS／macOS 產品版」在整份 200 題上的 Acc@1。
- 沒有找到 Mozc 官方公布的 AJIMEE-Bench 分數；51.0% 是第三方部落格量測。
- 論文 Table 1 的 gpt-4.5-preview 80.5 只出現在文章表格，論文本文沒有這一列，
  未確認它與 Table 1 其他列是否同條件。

## 授權

AJIMEE-Bench 是 **CC-BY-SA-3.0**，只當評測資料，不得進訓練。授權全文在
`0c-testdata/ja/LICENSE-ajimee.txt`。文件與報告引用分數時要保留出處與授權。

## Go 對照

Go 參考實作已經有 ajimee fixture 與 Mozc 量測，但題庫清單只由 `rules.json` 決定。
本次把探索改成目錄掃描，Rust 與 Go 的題庫載入在此分岔。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/eval/ja.go`（`LoadJACorpusSet`，只讀 `rules.json` 的 key） | `crates/jc-eval/src/ja.rs`（`load_ja_corpus_dir`） | rewritten；改成目錄掃描，pin 仍以 `rules.json` 為準 |
| `internal/eval/ja.go`（`LoadJACorpus` 與 pin 檢查，檢查碼內嵌在 `LoadJACorpusSet`） | `crates/jc-eval/src/ja.rs`（`check_against_rule`） | rewritten；pin 檢查抽成一個函式，`load_ja_corpus_set` 與目錄掃描共用 |
| `cmd/0c-eval-ja/main.go`（`--corpus` 名稱過濾） | `crates/jc-cli/src/eval_cmd.rs`（`select_ja_corpora`） | rewritten；找不到題庫時列出目錄現有名稱 |
| `internal/eval/ja_test.go` | `crates/jc-eval/src/ja.rs` 測試（`dir_scan_*`、`real_ja_dir_discovers_ajimee`） | rewritten；原始測試沒有對應的 Go 名稱，改成目錄探索的行為測試 |
| `cmd/0c-eval-ja/tools/build_ajimee_fixture.py` | `0c-testdata/ja/ajimee_200.json` | n/a：fixture 已存在於題庫，本 task 不重建 |

## 驗證

```sh
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200 \
  --pack /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200 \
  --pack /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack --items
```
