# jc-ime 授權帳本

本目錄是 jc-ime 散布內容的授權帳本。每個實際散布的檔案都要對到下面其中一個
來源條目；不散布的第三方模型與內部評測題庫另列。授權全文與必要聲明放在
[`licenses/`](licenses/)，逐來源的完整聲明在
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md)。

- 本專案程式：AGPL-3.0-only，全文見根目錄 [`LICENSE`](../LICENSE)。
- 第三方資料、詞典、模型與題庫：依各自授權，不因本專案採用 AGPL-3.0-only 而改變。
- 本文件不是法律意見；散布前若新增或替換來源，必須先補帳本與 NOTICE。

## 散布邊界

| 類別 | 是否隨本專案散布 | 說明 |
|---|---|---|
| 程式、建置腳本、文件 | 是 | repo 內的原始碼與文件，AGPL-3.0-only |
| 日文詞典 pack（Mozc v2）、注音詞典／語言模型、英文詞表、OpenCC 字典 | 是 | 以資料包或安裝包散布；權重與二進位資料不進 git。public-core pack 只是備援，不在 manifest |
| 日文神經轉換模型（Jinen v2 small） | 是 | 隨安裝包散布，manifest 標 `shipped: true`；權重不進 git |
| 日文 reranker 與日文標點模型 | 否 | reranker 已棄用、標點模型自 2026-11-20 起不再載入；權重留在資料目錄供回溯，manifest 未列為隨附 |
| 注音選字／標點模型（Funmula） | **否** | 第三方模型，僅由使用者另外下載，本專案不轉散布 |
| GPUI Kit／GPUI（設定程式 UI） | 是 | Apache-2.0，連進 `jc-settings-ui` 執行檔；附授權全文與聲明 |
| `0c-testdata` 題庫 | 否 | 僅供內部評測，不隨產品散布 |

## 散布檔案 → 授權條目

資料根目錄在本機是 `/mnt/work/jc/jc-ime-data`；下表的相對路徑都相對於該目錄。
repo 內的檔案相對於本 repo 根目錄。同一個資料包目錄的 `manifest.json`、
`NOTICE.txt`、`report.txt` 與主檔視為同一散布單元，不另列。

### Repo 內容

| 散布檔案 | 授權條目 |
|---|---|
| `LICENSE` | L-00 |
| `crates/**`、`scripts/**`、`docs/**`、`Cargo.toml`、`Cargo.lock` | L-00 |
| `legal/**`、`NOTICE`、`README.md` | L-00 |

### 日文詞典

| 散布檔案 | 大小 | SHA-256 | 授權條目 | 散布 |
|---|---:|---|---|---|
| `packs/ja-lexicon-mozc-v2.pack` | 85,508,727 | `15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6` | L-01 | 是 |
| `packs/ja-lexicon-mozc-v2.manifest.json` | 4,040 | 見檔案內 `outputs.pack.sha256` | L-01 | 是（隨 pack） |
| `packs/ja-lexicon-public-core.pack` | 60,369,880 | `8e57a18deb64d0fdc8147b53b5e234ecda412fa49feb0e8096addc4349582840` | L-02、L-03、L-19 | 否（備援，未在 manifest） |
| `packs/ja-lexicon-public-core.manifest.json` | 17,946 | 見檔案內 `outputs.pack.sha256` | L-02、L-03、L-19 | 否（備援，未在 manifest） |

`ja-lexicon-mozc-v2.pack` 是目前產品使用的日文詞典，資料 manifest 標 `shipped: true`；
`ja-lexicon-public-core.pack` 只在 `resolve_ja_pack` 當備援候選，不在 manifest 的隨附清單。

### 注音詞典與語言模型

| 散布檔案 | 大小 | SHA-256 | 授權條目 |
|---|---:|---|---|
| `build-zh/lexicon.bin` | 8,791,416 | `46721ccb95e55c7fc36abd3a351f9191eef087e47bd30b580c5711c05dc30f57` | L-04、L-05、L-06、L-13～L-17 |
| `build-zh/lm.bin` | 4,766,114 | `353fd25aca9bf80cef0eb23fb6bbc551aa91abd46ee15f07b4d88f231bf858fb` | L-04、L-05、L-06、L-13～L-17 |
| `build-zh-bigram/lexicon.bin` | 8,791,416 | `46721ccb95e55c7fc36abd3a351f9191eef087e47bd30b580c5711c05dc30f57` | L-04、L-05、L-06、L-13～L-17 |
| `build-zh-bigram/lm.bin` | 32,879,418 | `10ffd7fccef6b66bfe1efed901117b40a3049d0274eeb24ad0c1385d3c168174` | L-04、L-05、L-06、L-13～L-17 |

`build-zh` 與 `build-zh-bigram` 的 manifest 都列出 13 個來源；逐來源的版本、
署名與授權全文見 [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt)。

### 英文詞表

| 散布檔案 | 大小 | SHA-256 | 授權條目 |
|---|---:|---|---|
| `english/words.bin` | 1,522,028 | `aa03049852110645bce87b5a7bcb9a383a1b0748e59465827148adb534701782` | L-08 |
| `english/manifest.json`、`english/NOTICE.txt` | 1,376／5,360 | 見 `english/manifest.json` | L-08 |

### OpenCC 字典

| 散布檔案 | 大小 | SHA-256 | 授權條目 |
|---|---:|---|---|
| `opencc/CJK_Compatibility_Ideographs.txt` | 39,074 | `663cc10b7de7c7cee3ff9df1ee963977d6c26c00b23f8fe670fab481283ed4f9` | L-07 |
| `opencc/TSCharacters.txt` | 104,516 | `737c21c66f55a419dd6956cb3089476cdefc5a36877452631617696df1e5d925` | L-07 |
| `opencc/TSPhrases.txt` | 8,786 | `362fa1b9a7d6edd04b462a32e12c9fef3adae822ab1dee9c83561cc37c06cb1f` | L-07 |
| `opencc/TWVariants.txt` | 986 | `e187278e119c427ca561180ac5da5b20e9f8681190458f35c327ce499e95a6a5` | L-07 |
| `opencc/TWVariantsRevPhrases.txt` | 24,359 | `5ebfb4bdc938c2b14e01ace378988d5d3dc12462b3496ef1d424951ccd371256` | L-07 |

### 模型

`散布` 欄完全對齊資料根目錄 `manifest.json` 的 `shipped` 布林值。資料 manifest 只索引實際要
安裝的檔案，因此留在資料目錄、但不在 manifest 的權重一律標「否」。

| 散布檔案 | 大小 | SHA-256 | 授權條目 | 散布 |
|---|---:|---|---|---|
| `models/zh-select/model.onnx` | 130,723,137 | `4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261` | L-11 | 否（下載） |
| `models/zh-select/vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` | L-11 | 否（下載） |
| `models/zh-select/manifest.json`、`LICENSE`、`README.md`、`NOTICE` | 2,901／1,077／8,365／753 | 見 `models/zh-select/NOTICE` | L-11 | 否（下載） |
| `models/zh-punct/model.onnx` | 130,584,075 | `4db3ed40df777347d9c455da01285b01de192d4c210c5f7cd9f0646c727880d8` | L-12 | 否（下載） |
| `models/zh-punct/vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` | L-12 | 否（下載） |
| `models/zh-punct/manifest.json`、`LICENSE`、`README.md`、`NOTICE` | 2,270／1,077／5,596／745 | 見 `models/zh-punct/NOTICE` | L-12 | 否（下載） |
| `models/reranker-extra/model.int8.onnx` | 34,268,510 | `edf1afeacb89fa5da3d10462b5d9afd85392a91056634dd9ac60d41e37faeeb3` | L-09 | 否 |
| `models/reranker-extra/vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` | L-09 | 否 |
| `models/punctuation-public-core/model.int8.onnx` | 34,240,656 | `2860267deb3da1816b3f53decf786c5f5f478a1b9d327c8c80f2428c643f051b` | L-10 | 否（不再使用） |
| `models/punctuation-public-core/vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` | L-10 | 否（不再使用） |
| `models/ja-tokenizer/public_core_ja.model` | 614,451 | `b25260c9c3d90a754c719e62eea6b8125f08448d48dd4d1755c18edbd00e2078` | L-09、L-10 | 否 |
| `models/manifest.json` | 387 | — | L-00 | 否（安裝時重建） |

`zh-select` 與 `zh-punct` 是 Funmula 的第三方模型：只列出下載來源與雜湊，**不隨本專案散布**。
`reranker-extra` 與 `punctuation-public-core` 已不在資料 manifest 的隨附清單，權重留在資料目錄
供回溯；`ja-tokenizer` 是兩者共用的分詞器，若重新散布，兩份 notice 都要附。`zh-punct` 與
`punctuation-public-core` 自 2026-11-20 起不再被程式載入，檔案保留。模型清單與逐檔雜湊另見
[`docs/models.md`](../docs/models.md)。

### 日文神經轉換（Jinen）

| 散布檔案 | 大小 | SHA-256 | 授權條目 | 散布 |
|---|---|---:|---|---|
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | 81,117,824 | `80482707513d6b67dafc31774371cf95d765542abf8d74eebf5f32f92d788bd3` | L-21 | 是 |
| `models/quant/jinen-v2-xsmall/tokenizer.json` | 2,047,498 | `477c91a788bf97e7ed993a4441cfb2c4d60033e505ade17d6c367bc09ed6e6ba` | L-21 | 是（隨安裝包散布） |
| `models/external/jinen-v2-small/manifest.json`、`README.md` | 824／2,427 | 見目錄內檔案 | L-21 | 否 |

Jinen small 是日文轉換的預設神經後端，GGUF 與 tokenizer 兩筆在資料 manifest 都標 `shipped: true`。
GGUF 同目錄沒有 `tokenizer.json`，`jc-engine` 會退到 `models/quant/jinen-v2-xsmall/tokenizer.json`；
該路徑已列入 catalog 的 `JaTokenizer`（L-21），並隨資料包與安裝包散布。
訓練與署名見 [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) 第 15 節。

### 其他外部日文模型（僅本機留存，不散布）

`models/external/` 其餘目錄是 2026-11-20 日文調查留下來的 spike 模型，只在本機留存，
**不隨產品散布，不進 git、release 或安裝包**。每個目錄的 `manifest.json` 有 repo、revision、
逐檔大小與 SHA-256；量測結果與 prompt 格式見
[`docs/port/ja-models-fetch.md`](../docs/port/ja-models-fetch.md)。Jinen small 已改列隨附，
見上一節。

| 目錄 | 主檔 | 大小 | SHA-256 | 授權條目 | 散布 |
|---|---|---:|---|---|---|
| `jinen-v2-xsmall/` | `jinen-v2-xsmall-Q5_K_M.gguf` | 28,261,056 | `24ff3af5db712fbbb4aa9254ee28ec4d731207134471ab68b06c1828726284c2` | L-20 | 否 |
| `zenz-v3.2-small/` | `ggml-model-Q5_K_M.gguf` | 73,871,936 | `29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673` | L-22 | 否 |
| `zenz-v3.2-xsmall/` | `ggml-model-Q5_K_M.gguf` | 20,970,304 | `00c64b3d318045a708d0cad5434faccab10f5481a49e6362864551fd0995fa58` | L-23 | 否 |
| `duet-ja-kana-kanji-9m/` | `app/JapaneseConverter.pte`＋embedding／vocab／merges | 12,281,664（整組 14,769,047） | `554135eba0f61466ede7c2c1c0934c88294ab4c5bdfa92761b55aba7aaed102d` | L-24 | 否 |
| `shizuku-judge/` | `model_q8_0.gguf`＋`tokenizer.json` | 41,405,248（整組 44,633,723） | `86727ed56aa0b669552b4c24cdf25c2f98300ed019938b75a6ec986eb4fc0856` | L-25 | 否 |

L-20～L-24 都是 CC-BY-SA 4.0：**量化、轉檔、微調會產生 Adapted Material，衍生或散布需同授權**，
也不能改標成 AGPL-3.0-only。L-25 的權重來源是 MIT，產生程式是 GPL-3.0-or-later，這次只取產物。


### 內部評測題庫（不散布）

`0c-testdata/golden`、`eval`、`fuzzy`、`ja` 只用於評測，不隨產品散布。來源與授權見
L-18；`ja/modern_500.json`、`ja/hard_cases.json` 是 CC0-1.0，`ja/anthy_corpus1.json` 是
Public Domain，`ja/ajimee_200.json` 是 CC-BY-SA-3.0。若要隨產品散布題庫，必須先把它們
移到散布清單並附上對應全文。

## 來源總表

| ID | 來源 | 版本／commit | 授權 | 用途 | 是否散布 | 檔案位置 |
|---|---|---|---|---|---|---|
| L-00 | 本專案（Funmula Co., Limited／0C contributors） | 本 repo HEAD | AGPL-3.0-only | 程式、建置腳本、文件、repo 設定 | 是 | [`LICENSE`](../LICENSE) |
| L-01 | Mozc OSS `dictionary_oss` | tag 3.35.6354，commit `0136c35782536cb43d710575c311cd4185982738` | BSD-3-Clause AND NAIST-IPAdic AND ICOT-Free-Software；沖繩辭書 Public Domain | 日文詞典 pack v2 | 是 | `packs/ja-lexicon-mozc-v2.*`；[`licenses/Mozc-LICENSE.txt`](licenses/Mozc-LICENSE.txt)、[`licenses/Mozc-README.txt`](licenses/Mozc-README.txt) |
| L-02 | 現代書き言葉UniDic（`unidic-cwj-202512`） | 2025.12 | BSD-3-Clause（本專案選用）＋ NINJAL AUTHORS | `ja-lexicon-public-core.pack` | 否（備援 pack，未在 manifest） | [`licenses/UNIDIC-BSD-2025.12-artifact.txt`](licenses/UNIDIC-BSD-2025.12-artifact.txt)、[`licenses/UNIDIC-BSD-consortium.txt`](licenses/UNIDIC-BSD-consortium.txt)、[`licenses/UNIDIC-AUTHORS.md`](licenses/UNIDIC-AUTHORS.md)、[`licenses/UNIDIC-COPYING.txt`](licenses/UNIDIC-COPYING.txt) |
| L-03 | MeCab IPADic | 2.7.0-20070801 | NAIST-IPAdic ＋ ICOT Free Software | public-core pack 的 fallback | 否（備援 pack，未在 manifest） | [`licenses/IPADIC-COPYING.txt`](licenses/IPADIC-COPYING.txt) |
| L-04 | 0C curated 詞條（產品注音） | 2026-10-19；rows SHA-256 `90cc8608a20d7784607d1dad41cff486bafb405be38b8ef7cf34234c7bc628c1` | CC-BY-SA-4.0 | 注音詞典與語言模型 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-05 | 0c_collect（unigram 詞頻） | 2026-10-17；以 `5a057dd` 詞庫斷詞 | CC-BY-SA-4.0 | 注音詞典與語言模型 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-06 | libchewing-data `chewing_v4` | commit `e5d726d9c09afd562309b05d9adad29940de5ebb` | CC-BY-4.0 | 注音 bigram | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt)、[`licenses/CC-BY-4.0.txt`](licenses/CC-BY-4.0.txt) |
| L-07 | OpenCC `tw2s` 字典 | ver.1.4.2，commit `025f371dc76b598d77384fbdab90c937471844d8` | Apache-2.0 | 台灣正體轉簡體 | 是 | [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| L-08 | English Speller Database（ESDB／SCOWL） | 2026.02.25，commit `7e99edab8e32f9f9ea2b15f249ca8d4d67237410` | SCOWL／ESDB permission（MIT-like） | 英文詞表 `words.bin` | 是 | [`licenses/ESDB-SCOWL-LICENSE.txt`](licenses/ESDB-SCOWL-LICENSE.txt)、[`licenses/ENGLISH-DATA-NOTICE.txt`](licenses/ENGLISH-DATA-NOTICE.txt) |
| L-09 | `jeffpeng3/0c-ja-reranker` | rev `7450e10fe8b673560044c5db08ad3006a06d7a6e` | CC-BY-SA-4.0 | 日文 reranker 與共用分詞器 | 否 | [`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt)、`models/reranker-extra/` |
| L-10 | `jeffpeng3/0c-ja-punct` | rev `ff9166abc620b6716ac7de73cfa67335e6b985d2` | MIT | 日文標點（2026-11-20 起不再使用；共用分詞器仍由 ja-rerank 使用） | 否（不再使用） | [`licenses/MIT.txt`](licenses/MIT.txt)、`models/punctuation-public-core/` |
| L-11 | `Funmula/0c-sequence-d512`（第三方） | rev `3533e34ef25896afeacc928d136ba7f35783bba7` | MIT | 注音選字 | **否，僅下載** | `models/zh-select/`；[`licenses/MIT-Funmula.txt`](licenses/MIT-Funmula.txt) |
| L-12 | `Funmula/0c-punct-d512`（第三方） | rev `105c2770e0b7cf4ce44a792b3b186000dd1727d9` | MIT | 注音標點（2026-11-20 起不再使用） | **否，僅下載** | `models/zh-punct/`；[`licenses/MIT-Funmula.txt`](licenses/MIT-Funmula.txt) |
| L-13 | 小麥注音（McBopomofo）詞庫 | commit `a5ad2e94a173d6721b9d52e60f4d0266251a76ba` | MIT | 注音詞條與破音字優先表 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-14 | 全字庫 CNS11643 | `Properties.zip` SHA-256 `3d56ef14cc809989…`；`MapingTables.zip` SHA-256 `4502fcf7b433d679…` | 政府資料開放授權條款第 1 版 | 注音字表與 CNS 對照 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-15 | CC-CEDICT | 2026-09-19；SHA-256 `38e1697e3925a2c7…` | CC-BY-SA-4.0 | 注音詞條 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-16 | 中文維基百科 n-gram（0c-corpus） | 2026-09-01；SHA-256 `c95f29db704eb6a5…` | CC-BY-SA-4.0 | 注音語言模型 fallback | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-17 | 政府資料開放資料集（內政部、教育部、健保署、檔案管理局） | 115 年版／2026-10-17 下載 | 政府資料開放授權條款第 1 版 | 地名、學校、醫院、機關名稱 | 是 | [`licenses/ZH-DATA-NOTICE.txt`](licenses/ZH-DATA-NOTICE.txt) |
| L-18 | `0c-testdata` 題庫 | `modern_500`／`hard_cases`／`anthy_corpus1`／`ajimee_200`／golden／eval／fuzzy | CC0-1.0、Public Domain、CC-BY-SA-3.0；0c 自製題庫見各 README | 評測 | 否（僅內部） | `/mnt/work/jc/jc-ime-data/0c-testdata/`；[`licenses/CC-BY-SA-3.0.txt`](licenses/CC-BY-SA-3.0.txt) |
| L-19 | 0C curated 詞條（public-core） | public-core manifest 的 `lex-curated-0c`；SHA-256 `a2a940641b0cd514f4aee87396c8736ef6488c9b25bf2ff93cadb8461a79de3e` | CC-BY-4.0 | `ja-lexicon-public-core.pack` 的 6 筆詞條 | 否（備援 pack，未在 manifest） | [`licenses/CC-BY-4.0.txt`](licenses/CC-BY-4.0.txt) |
| L-20 | `togatogah/jinen-v2-xsmall.gguf` | rev `3910fd01bf4ba86eca89617f18db9b0c1c5b2283` | CC-BY-SA-4.0 | 日文かな漢字変換 spike | **否，僅本機留存** | `models/external/jinen-v2-xsmall/`；[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) |
| L-21 | Jinen v2 small：訓練在 `jeffpeng3/0c-training`，權重發佈於 `togatogah/jinen-v2-small.gguf` | rev `3461d0573ab447985badde3174165b967d06076c` | CC-BY-SA-4.0 | 日文かな漢字変換（預設） | 是（隨附） | `models/external/jinen-v2-small/`；[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) |
| L-22 | `Miwa-Keita/zenz-v3.2-small-gguf` | rev `ef3e66e3665fe6a19598e6578e9b88a994b647fd` | CC-BY-SA-4.0 | 日文かな漢字変換 spike | **否，僅本機留存** | `models/external/zenz-v3.2-small/`；[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) |
| L-23 | `Miwa-Keita/zenz-v3.2-xsmall-gguf` | rev `42cab8392a3727d1cf80932cf2275b7d523fbb9f` | CC-BY-SA-4.0 | 日文かな漢字変換 spike | **否，僅本機留存** | `models/external/zenz-v3.2-xsmall/`；[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) |
| L-24 | `jcbutter/duet-ja-kana-kanji-9m` | rev `18585dad1e2fc6cdc28e590b013a3e016df05b95` | CC-BY-SA-4.0 | edge かな漢字変換 spike（ExecuTorch） | **否，僅本機留存** | `models/external/duet-ja-kana-kanji-9m/`；[`licenses/CC-BY-SA-4.0.txt`](licenses/CC-BY-SA-4.0.txt) |
| L-25 | `rinna/japanese-gpt2-xsmall`（Shizuku judge 權重） | rev `8e91527b3276e0565154935e84a08bf0137ed99f`；本地 q8_0 轉檔 | MIT；產生程式 `n-seiji/shizuku-ime` 為 GPL-3.0-or-later | reranker fusion spike | **否，僅本機留存** | `models/external/shizuku-judge/`；[`licenses/MIT.txt`](licenses/MIT.txt) |
| L-26 | GPUI Kit（`longbridge/gpui-kit`：`gpui-kit`、`gpui-base`、`gpui-component`、`gpui-kit-assets`、`gpui-component-macros`） | `gpui-kit`／`gpui-base`／`gpui-component`／`gpui-kit-assets`／`gpui-component-macros` 皆 0.10.0 | Apache-2.0 | 設定程式的視窗與 UI 元件 | 是（連進 `jc-settings-ui` 執行檔） | `crates/jc-settings-ui/Cargo.toml`；[`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| L-27 | GPUI（Zed 的 `gpui-pre`，crate 名 `gpui`，含 `gpui-pre-*` 系列） | `gpui-pre` 0.3.12 | Apache-2.0 | GPUI Kit 的繪圖與視窗底層 | 是（連進 `jc-settings-ui` 執行檔） | `crates/jc-settings-ui/Cargo.toml`；[`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |


## 與資料 manifest 的散布對照

來源是 [`docs/port/data-pack.md`](../docs/port/data-pack.md) 記錄的實際產物：`schema`
`0c.data-pack/1`、`version` 2026-11-20，共 9 筆，隨附 5 筆、下載 4 筆。下表逐項對回本帳本。

| manifest 路徑 | role | manifest `license` | 帳本條目 | manifest `shipped` | 帳本「散布」 |
|---|---|---|---|---|---|
| `packs/ja-lexicon-mozc-v2.pack` | `ja_lexicon` | L-01 | L-01 | `true` | 是 |
| `build-zh/lexicon.bin` | `zh_lexicon` | L-04、L-05、L-06、L-13～L-17 | L-04、L-05、L-06、L-13～L-17 | `true` | 是 |
| `build-zh/lm.bin` | `zh_lm` | L-04、L-05、L-06、L-13～L-17 | L-04、L-05、L-06、L-13～L-17 | `true` | 是 |
| `english/words.bin` | `english` | L-08 | L-08 | `true` | 是 |
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | `ja_neural` | L-21 | L-21 | `true` | 是 |
| `models/zh-select/model.onnx` | `zh_select` | L-11 | L-11 | `false` | 否（下載） |
| `models/zh-select/vocab.json` | `zh_select` | L-11 | L-11 | `false` | 否（下載） |
| `models/zh-punct/model.onnx` | `zh_punct` | L-12 | L-12 | `false` | 否（下載） |
| `models/zh-punct/vocab.json` | `zh_punct` | L-12 | L-12 | `false` | 否（下載） |

對照結果：manifest 的 5 筆 `shipped: true` 與帳本「是」的資料檔完全一致；4 筆下載項與帳本
「否（下載）」一致。帳本另有幾筆資料目錄檔案（public-core pack、reranker、日文標點、共用
分詞器、`models/manifest.json`）標「否」，不進 manifest 的隨附清單；Jinen tokenizer 已列入
`JaTokenizer` 並標「是」，與 catalog 一致。

### 已補

- `models/quant/jinen-v2-xsmall/tokenizer.json` 是 Jinen 執行時必要檔，現已列入 catalog 的
  `JaTokenizer`、`shipped: true`，並隨資料包與安裝包散布。

## 維護規則

1. 新增、替換或升級來源時，先更新本表與 [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md)，
   再更新對應資料包的 manifest／NOTICE。
2. 每個散布檔案都必須能在「散布檔案 → 授權條目」找到條目；找不到就不可散布。
3. 不可把 Mozc 整體改標成單一 BSD-3-Clause，也不可把 UniDic 的 BSD 分支改寫成其他授權。
4. 第三方模型若要改為隨產品散布，必須先取得對應授權與完整 notice；Funmula 兩個模型目前
   只允許下載，不允許放進本專案的 release、安裝包或 git。
5. 資料根目錄 `manifest.json` 的 `shipped` 是本帳本「是否散布」的唯一依據。改 manifest 時
   同步改本表；兩邊不一致就不可出貨。
