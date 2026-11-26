# 第三方授權與聲明

本檔逐來源列出 jc-ime 使用的第三方資料、模型與題庫。散布時保留本檔與
[`licenses/`](licenses/) 的完整條款。來源方未背書本專案，也未參與本專案的模型訓練或輸出。

Rust crate 的授權另外列在 [`RUST-THIRD-PARTY-NOTICES.md`](RUST-THIRD-PARTY-NOTICES.md)，
由 `scripts/gen-rust-notices.py` 從 `Cargo.lock` 產生：DLL 與執行檔靜態連結了 600 多個
crate，那些不是資料，所以不混進這份由人工維護的清單。

## 1. 本專案

- 授權：AGPL-3.0-only。
- 著作權：Funmula Co., Limited 與 0C contributors。
- 全文：[`../LICENSE`](../LICENSE)。

## 2. Mozc OSS `dictionary_oss`（L-01）

- 來源：<https://github.com/google/mozc>，tag `3.35.6354`，commit `0136c35782536cb43d710575c311cd4185982738`。
- 使用：`ja-lexicon-mozc-v2.pack`。
- 授權：BSD-3-Clause AND NAIST-IPAdic AND ICOT-Free-Software；沖繩辭書 Public Domain。
- 著作權與聲明：
  - Copyright 2010-2018, Google Inc. All rights reserved.
  - Copyright 2000, 2001, 2002, 2003 Nara Institute of Science and Technology. All Rights Reserved.
  - ICOT Free Software 的 `NO WARRANTY` 與散布條件。
  - 沖繩辭書為 Public Domain。
- 全文與上游說明：[`licenses/Mozc-LICENSE.txt`](licenses/Mozc-LICENSE.txt)、
  [`licenses/Mozc-README.txt`](licenses/Mozc-README.txt)。

Mozc 資料不可整體改標成單一 BSD-3-Clause；上述每一段聲明都要保留。

## 3. 現代書き言葉UniDic 2025.12（L-02）

- 來源：`unidic-cwj-202512`，<https://clrd.ninjal.ac.jp/unidic/>。
- 使用：`ja-lexicon-public-core.pack`。
- 授權：GPL-2.0-only OR LGPL-2.1-only OR BSD-3-Clause；本專案選 BSD-3-Clause。
- 著作權：Copyright (c) 2023 National Institute for Japanese Language and Linguistics；
  網站版另記 `Copyright (c) 2011-2021, The UniDic Consortium`。
- 著作者表示：The UniDic Consortium／National Institute for Japanese Language and
  Linguistics；現代書き言葉UniDic 2025.12；實作：小木曽智信。
- 全文與著作者：
  [`licenses/UNIDIC-BSD-2025.12-artifact.txt`](licenses/UNIDIC-BSD-2025.12-artifact.txt)、
  [`licenses/UNIDIC-BSD-consortium.txt`](licenses/UNIDIC-BSD-consortium.txt)、
  [`licenses/UNIDIC-AUTHORS.md`](licenses/UNIDIC-AUTHORS.md)、
  [`licenses/UNIDIC-COPYING.txt`](licenses/UNIDIC-COPYING.txt)。

兩份 BSD 的著作權行不同，散布時兩份都附，不合併成單一著作權人。

## 4. MeCab IPADic 2.7.0（L-03）

- 來源：`mecab-ipadic-2.7.0-20070801`。
- 使用：`ja-lexicon-public-core.pack` 的 fallback 詞條。
- 授權：NAIST-IPAdic ＋ ICOT Free Software。
- 著作權：Copyright 2000, 2001, 2002, 2003 Nara Institute of Science and Technology.
- 全文：[`licenses/IPADIC-COPYING.txt`](licenses/IPADIC-COPYING.txt)。NAIST 聲明與 ICOT
  `NO WARRANTY` 不得刪除。

## 5. 0C 自有詞條與詞頻（L-04、L-05、L-19）

- 來源：`curated`、`0c_collect`。
- 使用：`ja-lexicon-public-core.pack` 的 curated 詞條、注音詞典與語言模型。
- 授權：
  - 產品注音詞典與語言模型的 `curated`、`0c_collect` 系列：CC-BY-SA-4.0。
  - public-core 詞典的 `lex-curated-0c`：CC-BY-4.0（manifest 標為 project-owned,
    outbound CC-BY-4.0）。
- 署名：Funmula Co., Limited；<https://0c.biggo.com>。
- 全文與逐來源聲明：[`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt)、
  [`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt)、
  [`licenses/CC-BY-4.0.txt`](licenses/CC-BY-4.0.txt)。

## 6. libchewing-data `chewing_v4`（L-06）

- 來源：<https://codeberg.org/chewing/libchewing-data>，commit `e5d726d9c09afd562309b05d9adad29940de5ebb`。
- 使用：`build-zh-bigram/lm.bin` 的注音 bigram。
- 授權：CC-BY-4.0。
- 署名：libchewing-data（新酷音詞庫）`dict/chewing_v4`，libchewing Core Team。
- 全文：[`licenses/CC-BY-4.0.txt`](licenses/CC-BY-4.0.txt)；逐來源聲明見
  [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt)。

## 7. OpenCC（L-07）

- 來源：<https://github.com/BYVoid/OpenCC>，ver.1.4.2，commit `025f371dc76b598d77384fbdab90c937471844d8`。
- 使用：`CJK_Compatibility_Ideographs.txt`、`TSCharacters.txt`、`TSPhrases.txt`、
  `TWVariants.txt`、`TWVariantsRevPhrases.txt`。
- 授權：Apache-2.0。
- 署名：Copyright 2010-2026 Carbo Kuo and contributors。
- 全文：[`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt)。

## 8. English Speller Database／SCOWL（L-08）

- 來源：<https://github.com/en-wl/wordlist/tree/7e99edab8e32f9f9ea2b15f249ca8d4d67237410>，
  2026.02.25；ESDB 匯出 SHA-256 `770dd3e42d20b47bb2dd87d150db4ee5472841a7f062876592f67840f5a0b902`。
- 使用：`english/words.bin`。
- 授權：SCOWLv2 permission（MIT-like）。
- 著作權：Copyright 2000-2026 by Kevin Atkinson；特別感謝 Alan Beale（12dicts／ENABLE2K）。
- 全文：[`licenses/ESDB-SCOWL-LICENSE.txt`](licenses/ESDB-SCOWL-LICENSE.txt)；建置與
  選用規則見 [`licenses/ENGLISH-DATA-NOTICE.txt`](licenses/ENGLISH-DATA-NOTICE.txt)。

SCOWL 部分資料來自 COCA 的 3-gram；該使用受採購時的 NDA 約束。散布時保留上游
LICENSE 全文，不移除 COCA 段落。

## 9. 日文 reranker（L-09）

- 來源：`jeffpeng3/0c-ja-reranker`，rev `7450e10fe8b673560044c5db08ad3006a06d7a6e`；
  <https://huggingface.co/jeffpeng3/0c-ja-reranker/tree/7450e10fe8b673560044c5db08ad3006a06d7a6e>。
- 使用：`models/reranker-extra/model.int8.onnx`、`vocab.json`，以及共用的
  `models/ja-tokenizer/public_core_ja.model`。
- 授權：CC-BY-SA-4.0。
- 訓練資料：CC BY-SA 日文 experimental 語料（Wikipedia 與 Stack Exchange）。
  修改：正規化、去重、抽樣、斷詞、tokenization，模型由 0C 從零訓練。
- 署名：Wikipedia contributors；Stack Exchange Network 與各投稿作者、原問題 URL。
  來源方未背書本專案。
- 全文：[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt)。

模型權重不進 git；2026-11-20 起 reranker 不再預設使用，也不在資料 manifest 的隨附清單。
若未來重新納入散布，必須附本節與 CC BY-SA 4.0 全文。

## 10. 日文標點（L-10）

- 來源：`jeffpeng3/0c-ja-punct`，rev `ff9166abc620b6716ac7de73cfa67335e6b985d2`；
  <https://huggingface.co/jeffpeng3/0c-ja-punct/tree/ff9166abc620b6716ac7de73cfa67335e6b985d2>。
- 使用：`models/punctuation-public-core/model.int8.onnx`、`vocab.json`，以及共用的
  `models/ja-tokenizer/public_core_ja.model`。
- 授權：MIT。
- 附帶聲明：模型卡含 UniDic 與 IPADic 的來源聲明；相關全文見第 3、4 節。
- 狀態：2026-11-20 起不再使用，程式不載入，權重留在資料目錄供回溯；不隨安裝包散布。
- 全文：[`licenses/MIT.txt`](licenses/MIT.txt)。

## 11. 注音模型：第三方 Funmula（L-11、L-12）

這兩個模型是第三方（Funmula）提供的 MIT 模型。`MIT-Funmula.txt` 明確允許
distribute，條件是保留 copyright 與授權全文；本專案已遵守，因此 `zh-select`
**隨 MSI 散布本地轉出的 Q8_0 GGUF**（L-11）。`zh-punct` 已移除，仍不散布（L-12）。

| 角色 | repo | rev | 來源 |
|---|---|---|---|
| `zh-select` | `Funmula/0c-sequence-d512` | `3533e34ef25896afeacc928d136ba7f35783bba7`（`3533e34e`） | <https://huggingface.co/Funmula/0c-sequence-d512/tree/3533e34ef25896afeacc928d136ba7f35783bba7> |
| `zh-punct` | `Funmula/0c-punct-d512` | `105c2770e0b7cf4ce44a792b3b186000dd1727d9`（`105c2770`） | <https://huggingface.co/Funmula/0c-punct-d512/tree/105c2770e0b7cf4ce44a792b3b186000dd1727d9> |

`zh-punct` 自 2026-11-20 起不再被程式載入，2026-11-21 從 catalog 移除；檔案仍標「不轉散布」。

- 授權：MIT；Copyright (c) 2026 Funmula Corp Limited。
- 全文：[`licenses/MIT-Funmula.txt`](licenses/MIT-Funmula.txt)，已隨 MSI 安裝到 `legal\licenses\`。
- 散布檢查（L-11）：`model.q8_0.gguf`、`vocab.json`、`models/manifest.json` 與
  `models/zh-select/manifest.json` 隨 MSI 散布，四筆都是 catalog 的 `shipped: true`；
  `model.onnx` 只是本機轉檔來源，不進 MSI、Release 或 git。
- 散布檢查（L-12）：`models/zh-punct/` 全部檔案不進 MSI、Release 或 git。

## 12. 注音詞典其他來源（L-13～L-17）

`build-zh` 的 13 個來源與完整聲明在
[`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt)。摘要：

| 來源 | 授權 | 署名 |
|---|---|---|
| 小麥注音（McBopomofo） | MIT | Copyright (c) 2011-2026 Mengjuei Hsieh et al.；BPMFMappings 最初簡化自 libtabe `tsi.src`（BSD） |
| 全字庫 CNS11643 | 政府資料開放授權條款第 1 版 | 全字庫（CNS11643 中文標準交換碼全字庫） |
| CC-CEDICT | CC-BY-SA-4.0 | CC-CEDICT，MDBG 與 CC-CEDICT 編者 |
| 中文維基百科 n-gram | CC-BY-SA-4.0 | Wikipedia contributors |
| 內政部、教育部統計處、健保署、檔案管理局 | 政府資料開放授權條款第 1 版 | 各資料集提供機關 |

政府資料開放授權條款第 1 版允許重製、改作、散布與商業利用，要求署名；條款全文與
各資料集出處見 `ZH-DATA-NOTICE.txt`。

## 13. 評測題庫（L-18，不散布）

`0c-testdata` 只用於內部評測，不隨產品散布。若未來要散布，必須逐檔補上：

- `ja/modern_500.json`、`ja/hard_cases.json`：CC0-1.0。
- `ja/anthy_corpus1.json`：Public Domain（Anthy `calctrans/corpus.1.txt`）。
- `ja/ajimee_200.json`：CC-BY-SA-3.0；程式 CC0-1.0。全文見
  [`licenses/CC-BY-SA-3.0.txt`](licenses/CC-BY-SA-3.0.txt) 與
  `0c-testdata/ja/LICENSE-ajimee.txt`。
- `golden`、`eval`、`fuzzy`：0c 自製或由上述題庫衍生；來源與授權見各目錄 README。

## 14. GPUI Kit 與 GPUI（L-26、L-27）

- 來源：
  - GPUI Kit：<https://github.com/longbridge/gpui-kit>，crates.io `gpui-kit` 0.10.0、
    `gpui-base` 0.10.0、`gpui-component` 0.10.0、`gpui-kit-assets` 0.10.0、
    `gpui-component-macros` 0.10.0。
  - GPUI：<https://github.com/zed-industries/zed>，crates.io `gpui-pre` 0.3.12；GPUI Kit
    以 `gpui` 名稱引入，另含 `gpui-pre-*` 系列。
- 使用：`crates/jc-settings-ui/` 的視窗、繪圖與 UI 元件，連進設定程式執行檔散布。
- 授權：Apache-2.0。
- 著作權：
  - Copyright 2024 - 2026 Longbridge <https://longbridge.com>（GPUI Kit 各 crate 的
    `LICENSE-APACHE`）。
  - Copyright 2022 - 2025 Zed Industries, Inc.（`gpui-pre` 的 `LICENSE-APACHE`）。
- 上游 crate 沒有額外的 `NOTICE` 檔；散布時保留上述著作權行與 Apache-2.0 全文，
  不得移除或改寫。
- 全文：[`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt)。

## 15. 日文神經轉換 Jinen v2 small（L-21）

- 來源：訓練在 `jeffpeng3/0c-training`；權重發佈於
  <https://huggingface.co/togatogah/jinen-v2-small.gguf>，rev
  `3461d0573ab447985badde3174165b967d06076c`。
- 使用：`models/external/jinen-v2-small/jinen-v2-small-Q4_K_M.gguf`，日文轉換的預設神經
  後端；tokenizer 沿用 `models/quant/jinen-v2-xsmall/tokenizer.json`。
- 量化：Q4_K_M 由上游在同一個 HF revision 發佈（`jinen-v2-small-Q4_K_M.gguf`，
  72,123,008 bytes，SHA-256
  `26a6a71020a8d12908615d6ed2541ad6d6d9b73fc179d8d5472eb1c709bb1300`），
  本專案沒有自行重新量化。
- 授權：CC-BY-SA-4.0（上游模型卡與 `manifest.json` 都標這個授權）。
- 訓練資料與署名：
  - 訓練語料部分使用國立國會圖書館「全国書誌データ」（2026-07-09 取得）與國立國會
    圖書館サーチ的書誌資料，經加工後使用。
  - 加工與模型製作：Hitoshi Togasaki。國立國會圖書館未製作、提供或保證本模型。
  - 其餘訓練語料組成上游未完整公開，模型卡只列上述書誌資料。
- 散布條件：附本節與 CC-BY-SA-4.0 全文；標明模型名稱、作者與來源 revision。量化、
  轉檔或微調會產生 Adapted Material，散布時必須同採 CC-BY-SA-4.0，不能改標成
  AGPL-3.0-only。模型權重與本專案程式分開授權。
- 全文：[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt)。

## 16. 完整條款索引

| 檔案 | 用途 |
|---|---|
| [`licenses/Mozc-LICENSE.txt`](licenses/Mozc-LICENSE.txt) | Mozc BSD-3-Clause、NAIST／IPAdic、ICOT、沖繩辭書 |
| [`licenses/Mozc-README.txt`](licenses/Mozc-README.txt) | Mozc 上游來源與 IPAdic 說明 |
| [`licenses/UNIDIC-BSD-2025.12-artifact.txt`](licenses/UNIDIC-BSD-2025.12-artifact.txt) | UniDic artifact 版 BSD |
| [`licenses/UNIDIC-BSD-consortium.txt`](licenses/UNIDIC-BSD-consortium.txt) | UniDic 網站版 BSD |
| [`licenses/UNIDIC-AUTHORS.md`](licenses/UNIDIC-AUTHORS.md) | UniDic 著作者表示 |
| [`licenses/UNIDIC-COPYING.txt`](licenses/UNIDIC-COPYING.txt) | UniDic 三重授權聲明 |
| [`licenses/IPADIC-COPYING.txt`](licenses/IPADIC-COPYING.txt) | IPADic NAIST／ICOT |
| [`licenses/CC-BY-4.0.txt`](licenses/CC-BY-4.0.txt) | CC-BY-4.0 全文 |
| [`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) | CC-BY-SA-4.0 全文 |
| [`licenses/CC-BY-SA-3.0.txt`](licenses/CC-BY-SA-3.0.txt) | CC-BY-SA-3.0 全文（題庫用） |
| [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) | OpenCC、GPUI Kit、GPUI 的 Apache-2.0 全文 |
| [`licenses/MIT.txt`](licenses/MIT.txt) | 日文標點 MIT 全文 |
| [`licenses/MIT-Funmula.txt`](licenses/MIT-Funmula.txt) | Funmula 注音模型 MIT 全文 |
| [`licenses/ESDB-SCOWL-LICENSE.txt`](licenses/ESDB-SCOWL-LICENSE.txt) | ESDB／SCOWL 全文 |
| [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) | 注音詞典 13 個來源的版本、署名與授權 |
| [`licenses/ENGLISH-DATA-NOTICE.txt`](licenses/ENGLISH-DATA-NOTICE.txt) | 英文詞表的建置與來源聲明 |
