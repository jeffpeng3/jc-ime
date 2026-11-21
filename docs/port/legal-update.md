# legal-update：授權帳本同步

Worktree：`/mnt/work/jc/wt/legal-update`（branch `legal-update`）

這輪只改文件：Jinen 改列可散布、補 GPUI Kit／GPUI 條目、標點模型標成不再使用，
並把帳本與資料 manifest 的 `shipped` 清單逐項對照。沒有改程式。

> **現況（2026-11-21）**：`zh_punct` 已從 catalog 移除，不再是下載項。L-12 在
> `legal/README.md` 保留是正確的，由授權帳本繼續記載 Funmula 標點模型；本文的
> manifest 對照表是 2026-11-20 的歷史紀錄。
>
> **補記（2026-11-21，`data-pack-english-meta`）**：catalog 新增
> `english/manifest.json`、`english/NOTICE.txt` 兩筆，同屬 L-08、皆為
> `shipped: true`，是 `english/words.bin` 的驗證中繼檔；本文對照表保留舊值。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| — | — | n/a：純授權文件同步，沒有程式行為要移植 |

## 改了什麼

| 檔案 | 變更 |
|---|---|
| `legal/README.md` | Jinen small（L-21）改成隨附；新增 L-27 GPUI；修正 L-26 的 repo 歸屬與散布說明；reranker／日文標點／public-core pack 改標不散布；補「與資料 manifest 的散布對照」 |
| `legal/THIRD-PARTY-NOTICES.md` | 新增第 14 節 GPUI Kit／GPUI、第 15 節 Jinen；日文標點與 Funmula 標註不再使用；索引更新 |
| `legal/licenses/README.md` | `Apache-2.0.txt` 對應 L-07、L-26、L-27；`CC-BY-SA-4.0.txt` 對應 L-20～L-24 |
| `NOTICE` | 補 Jinen 與 GPUI Kit／GPUI 的署名與散布說明；標註注音標點不再載入 |
| `docs/models.md` | Jinen 標隨附；集合 manifest 與散布檢查更新；補 tokenizer 待補事項 |
| `docs/port/data-pack.md`、`docs/port/ja-models-fetch.md`、`docs/integration-log.md` | 只改掉舊的「Jinen 僅評估」敘述，避免與新帳本衝突 |

## manifest 與帳本對照

manifest 來源：[`docs/port/data-pack.md`](data-pack.md) 的實際產物，`schema`
`0c.data-pack/1`、`version` 2026-11-20、9 筆（隨附 5、下載 4）。

### 隨附（`shipped: true`）

| manifest 路徑 | role | 授權 | 帳本條目 | 帳本狀態 | 一致 |
|---|---|---|---|---|---|
| `packs/ja-lexicon-mozc-v2.pack` | `ja_lexicon` | L-01 | L-01 | 是 | 是 |
| `build-zh/lexicon.bin` | `zh_lexicon` | L-04、L-05、L-06、L-13～L-17 | L-04、L-05、L-06、L-13～L-17 | 是 | 是 |
| `build-zh/lm.bin` | `zh_lm` | L-04、L-05、L-06、L-13～L-17 | L-04、L-05、L-06、L-13～L-17 | 是 | 是 |
| `english/words.bin` | `english` | L-08 | L-08 | 是 | 是 |
| `english/manifest.json` | `english` | L-08 | L-08 | 是 | 是（2026-11-21 補） |
| `english/NOTICE.txt` | `english` | L-08 | L-08 | 是 | 是（2026-11-21 補） |
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | `ja_neural` | L-21 | L-21 | 是 | 是 |

### 下載（`shipped: false`）

| manifest 路徑 | role | 授權 | 帳本條目 | 帳本狀態 | 一致 |
|---|---|---|---|---|---|
| `models/zh-select/model.onnx` | `zh_select` | L-11 | L-11 | 否（下載） | 是 |
| `models/zh-select/vocab.json` | `zh_select` | L-11 | L-11 | 否（下載） | 是 |
| `models/zh-punct/model.onnx` | `zh_punct` | L-12 | L-12 | 否（下載） | 是 |
| `models/zh-punct/vocab.json` | `zh_punct` | L-12 | L-12 | 否（下載） | 是 |

### 帳本有、manifest 沒有的檔案

全部標「否」，不影響 `shipped` 一致性：

| 檔案 | 帳本條目 | 原因 |
|---|---|---|
| `packs/ja-lexicon-public-core.pack`、`.manifest.json` | L-02、L-03、L-19 | 只在 `resolve_ja_pack` 當備援，不在 manifest |
| `models/reranker-extra/model.int8.onnx`、`vocab.json` | L-09 | reranker 已棄用，不在 manifest |
| `models/punctuation-public-core/model.int8.onnx`、`vocab.json` | L-10 | 2026-11-20 起不再使用 |
| `models/ja-tokenizer/public_core_ja.model` | L-09、L-10 | 僅 reranker／標點使用，未隨附 |
| `models/manifest.json` | L-00 | 角色索引，安裝時重建 |
| `models/quant/jinen-v2-xsmall/tokenizer.json` | L-21 | Jinen 執行時需要，manifest 待補 |

## Jinen 的授權與散布

- 訓練：`jeffpeng3/0c-training`。權重發佈：`togatogah/jinen-v2-small.gguf`，rev
  `3461d0573ab447985badde3174165b967d06076c`。
- 授權：CC-BY-SA-4.0，上游模型卡與 `jc-ime-data` 的 manifest 一致。任務書允許「授權與
  本專案相同或標明」；公開資料只支持 CC-BY-SA-4.0，因此照實標，不改成 AGPL-3.0-only。
  散布需附署名與同授權。
- 署名：模型由 Hitoshi Togasaki 製作；訓練語料部分使用國立國會圖書館「全国書誌データ」
  （2026-07-09 取得）與國立國會圖書館サーチ書誌資料，國立國會圖書館未背書。
- 檔案：`jinen-v2-small-Q5_K_M.gguf`，81,117,824 bytes，
  `80482707513d6b67dafc31774371cf95d765542abf8d74eebf5f32f92d788bd3`。

## GPUI Kit／GPUI

| crate | 版本 | repo | 授權 | 著作權 |
|---|---|---|---|---|
| `gpui-kit` | 0.10.0 | `longbridge/gpui-kit` | Apache-2.0 | Copyright 2024 - 2026 Longbridge |
| `gpui-base` | 0.10.0 | `longbridge/gpui-kit` | Apache-2.0 | Copyright 2024 - 2026 Longbridge |
| `gpui-component` | 0.10.0 | `longbridge/gpui-kit` | Apache-2.0 | Copyright 2024 - 2026 Longbridge |
| `gpui-kit-assets` | 0.10.0 | `longbridge/gpui-kit` | Apache-2.0 | Copyright 2024 - 2026 Longbridge |
| `gpui-component-macros` | 0.10.0 | `longbridge/gpui-kit` | Apache-2.0 | Copyright 2024 - 2026 Longbridge |
| `gpui-pre`（crate 名 `gpui`） | 0.3.12 | `zed-industries/zed` | Apache-2.0 | Copyright 2022 - 2025 Zed Industries, Inc. |

依賴宣告只有 `crates/jc-settings-ui/Cargo.toml` 的 `gpui-kit = "0.10"`，其餘由 Cargo
解析。上游 crate 沒有額外 `NOTICE` 檔，保留各 `LICENSE-APACHE` 的著作權行與 Apache-2.0
全文即可。

## 驗證

- `legal/`、`docs/`、`NOTICE` 已無 Jinen 的舊評估字樣（`grep` 0 筆）。
- 帳本的每一筆 `shipped: true` 都對到 manifest 的 5 筆隨附檔；4 筆下載項也逐筆對到
  `shipped: false`。
- 沒有改程式；`cargo check` 不適用。

實際變更檔案：

| 檔案 | 狀態 |
|---|---|
| `legal/README.md` | 修改 |
| `legal/THIRD-PARTY-NOTICES.md` | 修改 |
| `legal/licenses/README.md` | 修改 |
| `NOTICE` | 修改 |
| `docs/models.md` | 修改 |
| `docs/port/data-pack.md` | 修改 |
| `docs/port/ja-models-fetch.md` | 修改 |
| `docs/integration-log.md` | 修改 |
| `docs/port/legal-update.md` | 新增 |

## 邊界說明

owned paths 是 `legal/**`、`NOTICE`、`docs/models.md`、`docs/port/legal-update.md`。另外改了
`docs/port/data-pack.md`、`docs/port/ja-models-fetch.md`、`docs/integration-log.md` 各 1～2 行，
只移除「Jinen 僅評估」的舊敘述，用來滿足舊評估字樣在 `legal/`、`docs/` 無殘留的完成條件。
沒有動程式。

## 待補

1. `models/quant/jinen-v2-xsmall/tokenizer.json` 不在資料 manifest。Jinen 是預設路徑，
   出貨前要補這筆 `shipped: true`。
2. 如果之後恢復日文 reranker 或標點模型，先改 manifest 與帳本，再出貨。
