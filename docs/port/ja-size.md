# ja-size：日文 payload 量測與縮減方案

Worktree：`/mnt/work/jc/wt/ja-size`（分支 `ja-size`，從 `main` `4493772` 開）
這份只量測與提案，沒有改任何產品行為、版本號或 `docs/CHANGELOG.md`。

## 結論

| 方案 | 安裝後省 | setup.exe 省 | 實測品質 | 工作量 | 建議 |
|---|---:|---:|---|---|---|
| Jinen Q5_K_M → Q4_K_M | 9.0 MB | 9.8 MB | 模型單獨 -0.5 個百分點；引擎端到端反而 +14 題（ajimee）/ +1 題（modern） | 0.5 人日 | **做**。品質在誤差內，速度更快、逾時更少 |
| 詞條只留成本最低 30 萬筆 | 55.1 MB | 10.5 MB | ajimee top-1 46.0% → 35.5%（-10.5）；anthy 60.5% → 57.0% | 0.5–1 人日 | **不建議**。長句與專有名詞明顯變差 |
| 詞條只留 50 萬筆 | 44.0 MB | 8.2 MB | ajimee 46.0% → 39.0%（-7.0）；anthy 60.5% → 59.5% | 0.5–1 人日 | **不建議**。省得比 30 萬少，品質仍明顯掉 |
| 連接矩陣 i16 → 8-bit（全域位移） | 7.14 MB | 0.51 MB | 935 題零退步、1 題進步 | 2–3 人日 | **可做，但只為安裝後大小**。setup.exe 幾乎沒差 |
| 連接矩陣 i16 → 8-bit（逐列 min/max） | 7.12 MB | 0.51 MB | 935 題零退步、2 題進步 | 2–3 人日 | 同全域版，誤差更小、row table 多 16 KB |

只挑一個做：先換 Jinen Q4_K_M，9.8 MB 的成本最低、風險最小。詞條瘦身不建議。
矩陣量化省的是安裝後空間，對下載沒有實質幫助。

2026-11-26 更新：已採用 Q4_K_M，隨附檔改為 `jinen-v2-small-Q4_K_M.gguf`；
下面的量測數字保留當時的 Q5_K_M 對照，沒有重跑。

## 1. payload 實際組成

資料來自 `jc-ime-data/manifest.json` 的 `shipped: true` 檔案，加上本機用
`cargo xwin` 交叉編譯的 Windows MSVC release 產物。大小單位是 10^6 bytes。

### 資料 219.6 MB

| 檔案 | 大小 MB | xz -9e MB |
|---|---:|---:|
| `packs/ja-lexicon-mozc-v2.pack` | 85.51 | 15.43 |
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | 81.12 | 80.38 |
| `models/zh-select/model.q8_0.gguf` | 35.52 | 29.68 |
| `build-zh/lexicon.bin` | 8.79 | 2.10 |
| `build-zh/lm.bin` | 4.77 | 1.23 |
| `models/quant/jinen-v2-xsmall/tokenizer.json` | 2.05 | 0.28 |
| `english/words.bin` | 1.52 | 0.26 |
| `models/zh-select/vocab.json` | 0.31 | 0.01 |
| manifest／NOTICE | 0.02 | 0.01 |
| **合計** | **219.6** | **129.4** |

### 程式 59.9 MB

MSVC x64 release，含靜態 CRT：

| 檔案 | 大小 MB | xz -9e MB |
|---|---:|---:|
| `jc-settings-ui.exe` | 26.86 | 6.28 |
| `jc-cli.exe` | 15.52 | 3.91 |
| `jc-engine.exe` | 13.34 | 3.13 |
| `jc-tsf.dll`（x64） | 1.43 | 0.46 |
| `jc-tsf-x86.dll` | 1.37（以 GNU i686／x64 比例推估） | 0.44 |
| `jc-tsf-smoke.exe` | 1.02 | 0.35 |
| `jc-tip-register.exe` | 0.40 | 0.16 |
| **合計** | **59.9** | **14.7** |

授權文件 `legal/` 與 `LICENSE`、`NOTICE` 另約 1.3 MB（xz 約 0.3 MB）。

### 對 setup.exe 的貢獻

- 安裝後總量約 **280.8 MB**：資料 78%、程式 21%、授權 0.5%。
- 以逐檔 `xz -9e` 推估，`setup.exe` 內約 **129 MB 資料 + 15 MB 程式 + 0.3 MB 授權**；
  實際 `setup.exe` 是 159.5 MB。差額 15 MB 來自 Inno 標頭、圖示、安裝器本體、
  未逐檔計入的小檔，以及 LZMA2 與 xz 的壓縮差。資料仍佔 setup.exe 約 **81%**。
- 結論：要讓下載變小，只能動資料。程式再怎麼縮，上限就是 15 MB 左右。

## 2. ja pack 內部組成

`packs/ja-lexicon-mozc-v2.pack`：85,508,727 bytes、1,220,427 詞條、983,412 個唯一詞面、
2,672 個 context。

| Section | 大小 MB | 佔比 |
|---|---:|---:|
| 詞面表 | 13.43 | 15.7% |
| 讀音表 | 20.64 | 24.1% |
| 詞條固定欄位（24 B × 1,220,427） | 29.29 | 34.3% |
| 詞面索引（8 B × 983,412） | 7.87 | 9.2% |
| POS＋活用表 | 0.003 | 0.0% |
| 連接成本矩陣（2,672² × i16） | 14.28 | 16.7% |
| SHA-256 trailer | 0.00003 | 0.0% |

詞條固定欄位佔最多，因為每筆都存詞面 offset／長度、讀音 offset／長度、POS、
活用型、活用形、成本、flags、left/right context id。詞面平均 13.7 bytes、
讀音平均 16.9 bytes；237,015 筆與前一筆共用同一個詞面。

### 成本分布

Mozc 原始成本，mean 7,284、p50 7,336、p90 9,096、max 18,318。

| 成本上限 | 詞條數 | 佔全部 | 這批詞條的 pack 大小（含矩陣） | 非矩陣部分 |
|---:|---:|---:|---:|---:|
| ≤ 1,000 | 2,813 | 0.23% | 14.41 MB | 0.13 MB |
| ≤ 3,000 | 10,055 | 0.82% | 14.77 MB | 0.49 MB |
| ≤ 5,000 | 94,057 | 7.7% | 19.27 MB | 5.00 MB |
| ≤ 10,000 | 1,158,967 | 95.0% | 82.02 MB | 67.74 MB |
| > 10,000 | 61,460 | 5.0% | 3.49 MB | 3.49 MB |

成本 5,000 到 10,000 這段佔了 62.7 MB，是體積的主體。

### 只留成本最低 N 筆

| 保留 | 成本上限 | pack 大小 | 安裝後省 | xz -9e | setup 省 | 詞面數 |
|---:|---:|---:|---:|---:|---:|---:|
| 100,000 | 5,067 | 19.59 MB | 65.9 MB | 未量 | 未量 | 84,082 |
| 300,000 | 6,362 | 30.39 MB | 55.1 MB | 4.98 MB | 10.5 MB | 239,812 |
| 500,000 | 7,024 | 41.54 MB | 44.0 MB | 7.22 MB | 8.2 MB | 392,992 |
| 800,000 | 7,828 | 59.02 MB | 26.5 MB | 未量 | 未量 | 633,921 |

`xz -9e` 欄是整個 pack 的實測壓縮大小；原始 pack 是 15.43 MB。setup 省是
15.43 減該列的大小。未量的兩列只拿來對照安裝後大小。

### 連接矩陣

- 目前每格 **2 bytes**（i16，row-major），7,139,584 格，全部非負。
- 值域 0–15,281，10,685 個不同值，0 佔 5.3%，最常見值 4,104（1.2%）。
- 全域 8-bit 位移量化：`q = round((v - 0) / 59.93)`，存 1 byte；
  平均誤差 14.6、最大 30、p99 30。矩陣從 14.28 MB 降到 7.14 MB。
- 逐列 8-bit：每列存 i16 最小值＋f32 scale（16 KB）；平均誤差 1.4、最大 30、
  p99 21。矩陣 14.28 → 7.16 MB。
- 直接 clamp 成 i8 不可行：95% 的格值大於 127，平均誤差 7,775。
- 壓縮後差距變小：原始矩陣 xz 1.62 MB，8-bit 版 1.11 MB，所以 setup.exe 只省
  約 **0.51 MB**。

## 3. 品質量測

方法：用本 worktree 建出的 Linux release `jc-cli`，
`--neural off` 隔離詞典與矩陣的影響，題庫固定，逐題輸出 `--items` 比對。

### 詞條瘦身

| 題庫 | 基準 top-1 | 30 萬 | 50 萬 | 退步／進步（30 萬） |
|---|---:|---:|---:|---:|
| modern-500 | 86.0% | 86.0% | 86.0% | 0 / 0 |
| hard-cases（35 題） | 54.3% | 54.3% | 54.3% | 0 / 0 |
| anthy-corpus1 | 60.5% | 57.0% | 59.5% | 7 / 0 |
| ajimee-200 | 46.0% | 35.5% | 39.0% | 21 / 0 |

掉最多的是 ajimee-200（Wikipedia 長句、專有名詞）：30 萬筆掉 10.5 個百分點，
50 萬筆掉 7.0 個百分點。退步全部來自基準答對、瘦身後答錯，沒有任何一題變好。

實際會壞的例子（括號是該詞在原始 pack 的成本）：

| 題目 | 基準 | 30 萬筆後 | 被砍的詞 |
|---|---|---|---|
| ajimee-010 | 変身ベルト | 返信ベルト | `変身ベルト` 7,101 |
| ajimee-017 | 相撲部屋 | 住もう部屋 | `相撲部屋` 7,033 |
| ajimee-009 | 月遅れ | つき遅れ | `月遅れ` 7,214 |
| anthy-135 | 鶴亀算 | 鶴亀残 | `鶴亀算` 7,962 |
| anthy-057 | 一身上 | 一新上 | `一身上` 6,637（50 萬筆會留） |
| anthy-092 | 情報処理振興事業協会 | 情報処理進行事業協会 | 該詞 6,851（50 萬筆會留） |

未知詞路徑的預設成本是每個かな 8,000（`UNKNOWN_WORD_COST`），而未知詞只會產生
かな，不會產生漢字。因此被砍掉的漢字詞沒有「排名後退」這條路，就是完全組不出來。
30 萬筆的切點 6,362 低於 8,000，模型仍然會用かな把讀音吃完，結果就是表中那種
「讀音對了、漢字錯了」。保留未知詞路徑是必要的，但它救不回被砍的詞。

### 矩陣量化

| 題庫 | 基準 top-1 | 全域 8-bit | 逐列 8-bit | 退步／進步 |
|---|---:|---:|---:|---:|
| modern-500 | 86.0% | 86.0% | 86.0% | 0 / 0 |
| hard-cases | 54.3% | 54.3% | 54.3% | 0 / 0 |
| anthy-corpus1 | 60.5% | 61.0% | 61.5% | 0 / 1、0 / 2 |
| ajimee-200 | 46.0% | 46.0% | 46.0% | 0 / 0 |

935 題合計：全域版 0 題退步、1 題進步；逐列版 0 題退步、2 題進步。量化誤差
最大 30 個成本單位，相對詞成本總和（數千到上萬）很小，實測看不到品質代價。
逐列版誤差更小，但要多存 16 KB 的 row table，兩者對 setup.exe 的影響相同。

## 4. Jinen 量化

### 現成檔案

HF repo `togatogah/jinen-v2-small.gguf`（rev `3461d057`）只有四個檔：
f16 220 MB、Q8_0 117 MB、Q5_K_M 81 MB、Q4_K_M 72 MB。
**沒有 Q4_0、IQ3**。引擎的 candle 路徑（`crates/jc-model/src/candle/quant.rs`）
只支援 `Q8_0`、`Q5K`、`Q4K`；要用 Q4_0 或 IQ3 得先換 runtime 或補 candle 的
量化型別，不在這次範圍。

Q4_K_M 實測 72,123,008 bytes（72.1 MB），比 Q5_K_M 少 8,994,816 bytes；
xz -9e 從 80.38 MB 降到 70.59 MB。

### 品質

模型單獨跑 AJIMEE-200（`jinen-eval`，同一 tokenizer、greedy、無左文脈）：

| 模型 | literal exact | NFKC exact | accept | accept+NFKC | p50 |
|---|---:|---:|---:|---:|---:|
| Q5_K_M | 57.0% | 64.0% | 77.5% | 82.5% | 63.7 ms |
| Q4_K_M | 56.5% | 63.5% | 77.0% | 82.0% | 52.0 ms |

200 題有 8 題輸出不同，淨結果是少 1 題 accept。差異雙向，例如 Q4 把
「浄土真宗事典」改對，也把「夜駆け」變成「夜がけ」。p50 的差距在負載
（loadavg 17–19）下只能看方向，不能當精確延遲。

引擎端到端（`jc-cli eval ja --neural quality`，同一批題目與參數）：

| 題庫 | Q5_K_M | Q4_K_M | 差異 |
|---|---:|---:|---:|
| ajimee-200 | 107/200（53.5%） | 135/200（67.5%） | +28 題 |
| modern-500 | 468/500（93.6%） | 469/500（93.8%） | +1 題 |

Q4 在 ajimee 大幅領先，原因是推論更快、逾時更少：Q5 的 neural 統計是
`timeout 232`、`inferences 110`，Q4 是 `timeout 45`、`inferences 284`。
這是「同樣 100 ms 預算下能補正更多句」，不是模型本身比較準。以本機負載如實
記錄，換到更快的機器兩者都會更接近。

modern-500 的五個 tag（每個 100 題）沒有任何一類退步，只有
`person-object-verb` 多對 1 題。

### 換模型要改的地方

1. `crates/jc-engine/src/loader.rs` 的 `JA_NEURAL_GGUF` 常數與兩個測試字串。
2. `crates/jc-data/src/manifest.rs` 的 `JA_NEURAL` 候選路徑。
3. `crates/jc-settings-ui/src/data.rs` 的模型候選與測試。
4. `scripts/release-notes.sh`、`legal/README.md`（大小、SHA-256、L-21 條目）。
5. `jc-ime-data` 換檔後重跑 `jc-cli data manifest`，更新 `manifest.json`。

重新量化不是必要：HF 已有官方 Q4_K_M。若日後要自己產生，用 llama.cpp
`llama-quantize <f16.gguf> <out.gguf> Q4_K_M`（模型卡用 b10200），產生後跑
`jinen-eval --corpus ajimee-200 --gguf <out>` 對照，再跑一次
`jc-cli eval ja --neural quality`。

## 5. 方案細節

### 方案 A：Jinen 換 Q4_K_M

- 省：安裝後 9.0 MB、setup.exe 9.8 MB。
- 品質：模型單獨 -0.5 個百分點；引擎端到端不變或更好。沒有任何 tag 退步。
- 工作量：0.5 人日，改 5 個檔案加 manifest 與授權表。
- 風險：低。少數句子輸出改變（8/200），其中 3 題是全形括號變半形；
  若在意，可在後處理把括號正規化，與量化無關。
- 建議：**做**。

### 方案 B：詞條瘦身

- 省：30 萬筆安裝後 55.1 MB、setup.exe 10.5 MB；50 萬筆 44.0 MB／8.2 MB。
- 品質：ajimee -10.5 / -7.0 個百分點，anthy -3.5 / -1.0；modern 與 hard 不變。
  35 題的 hard-cases 太小，看不出長尾損傷。
- 工作量：0.5–1 人日（`jc-jadic` 匯入時加成本上限，重產 pack，重跑 eval）。
- 風險：高。被砍的詞無法用未知詞路徑救回，使用者會遇到「讀音對、漢字錯」；
  且 setup 只省 10 MB，與品質損失不成比例。
- 建議：**不建議**。若一定要做，先只考慮 800,000 筆並補跑 eval，不要直接砍到 30 萬。

### 方案 C：矩陣 8-bit 量化

- 省：安裝後 7.14 MB、setup.exe 0.51 MB。
- 品質：935 題零退步，1–2 題進步；全域版平均誤差 14.6、最大 30，
  逐列版平均 1.4。兩者都遠小於詞成本量級。
- 工作量：2–3 人日。要動 pack 格式版本（v3 或 v2 的 capability flag）、
  `jc-data` 讀取、`jc-jadic` 寫入、驗證與測試；舊版引擎讀新 pack 要能明確報錯，
  或同時保留 v2 寫入。
- 風險：中。格式升級的相容性比壓縮本身麻煩；setup.exe 只省 0.5 MB，
  若目標是下載大小，這個方案不值得。目標若是安裝後硬碟佔用才做。
  逐列版多 16 KB row table，換來 10 倍小的誤差，若要做就選逐列版。
- 建議：可做，優先度低於方案 A。

## 6. 已排除：zstd 存檔＋執行時解壓

使用者已明確排除。實測 pack 以 zstd -19 是 19.8 MB（比現有 LZMA 15.4 MB 大），
解壓後要常駐約 85 MB RSS，與「縮小系統佔用」的目標相反。不再評估。

## 7. 重跑方式

```sh
# 1) pack 組成、成本分布、變體 pack
uv run --with zstandard scripts/ja-size/analyze_pack.py \
    /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack \
    --json /tmp/opencode/ja-pack-analysis.json

# 2) 建 Linux jc-cli
cargo build --release -p jc-cli

# 3) 重跑全部評測（會產生 trim 與矩陣變體到 /tmp/opencode/ja-size-variants）
scripts/ja-size/run-evals.sh \
    target/release/jc-cli /mnt/work/jc/jc-ime-data /tmp/opencode/ja-size-evals
```

`run-evals.sh` 預設跑 modern-500、anthy-corpus1、ajimee-200、hard-cases，
輸出逐題 JSON 與退步／進步統計。Jinen 的 Q4 評測需要先下載
`jinen-v2-small-Q4_K_M.gguf`（腳本註解有指令）。

## 限制

- Windows 程式大小是本機 `cargo xwin` 交叉編譯的 MSVC release，與 CI 產物可能有
  數 MB 差異；`jc-tsf-x86.dll` 因 i686 MSVC 連結缺少 SDK lib，以 GNU 版本比例推估。
- `jc-settings-ui.exe` 的 DirectX shader 在 Linux 上用空 stub 取代，對大小影響在
  KB 等級。
- 評測期間機器負載高（loadavg 16–19），延遲數字只看方向；正確率不受影響。
- 詞條瘦身只量成本排序，沒有量「保留高成本常用詞」的加權策略。
- 矩陣量化以 dequantize 回 i16 餵給現有引擎模擬，尚未實作真正的 8-bit 讀取路徑。
