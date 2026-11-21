# 模型來源與雜湊

本表列出模型來源、固定 revision、授權、實際檔案雜湊與散布方式。2026-11-20
起標點模型已從程式移除，`zh-punct` 與 `ja-punct` 只保留歷史紀錄，模型檔仍在
資料目錄，不再被載入，也不隨安裝包出貨。2026-11-21 起兩者也不再列於集合
manifest，解析器明確過濾舊資料目錄的殘留鍵。資料來源：

- 注音：`jc-ime-data/models/zh-select/manifest.json`；`zh-punct/manifest.json` 是
  歷史來源，2026-11-21 從 catalog 移除，仍保留供回溯（下載時追加的 `download` 區塊）。
- 日文 reranker：`jc-ime-data/models/manifest.json` 的角色對應，以及前身固定的
  `ref/0c-ime/models/pairing.lock.json`。
- 日文神經轉換：`jc-ime-data/models/external/jinen-v2-small/manifest.json`。
- 集合 manifest：`jc-ime-data/models/manifest.json`（schema `jc.model-set/1`）。

授權帳本與完整聲明見 [`legal/README.md`](../legal/README.md) 與
[`legal/THIRD-PARTY-NOTICES.md`](../legal/THIRD-PARTY-NOTICES.md)。

## 角色與來源

| 角色 | 目錄 | repo | revision | 授權 | 隨安裝包出貨 | 狀態 |
|---|---|---|---|---|---|---|
| `zh-select` | `models/zh-select` | `Funmula/0c-sequence-d512` | `3533e34ef25896afeacc928d136ba7f35783bba7`（`3533e34e`） | MIT | 是，Q8_0 GGUF 與 vocab.json | 使用中，candle Q8_0 |
| `zh-punct` | `models/zh-punct` | `Funmula/0c-punct-d512` | `105c2770e0b7cf4ce44a792b3b186000dd1727d9`（`105c2770`） | MIT | 否 | 已移除（2026-11-20） |
| `ja-rerank` | `models/reranker-extra` | `jeffpeng3/0c-ja-reranker` | `7450e10fe8b673560044c5db08ad3006a06d7a6e`（`7450e10f`） | CC-BY-SA-4.0 | 否 | 棄用，僅保留模型與訓練碼 |
| `ja-punct` | `models/punctuation-public-core` | `jeffpeng3/0c-ja-punct` | `ff9166abc620b6716ac7de73cfa67335e6b985d2`（`ff9166ab`） | MIT | 否 | 已移除（2026-11-20） |
| `ja-neural` | `models/external/jinen-v2-small` | `togatogah/jinen-v2-small.gguf`（訓練：`jeffpeng3/0c-training`） | `3461d0573ab447985badde3174165b967d06076c` | CC-BY-SA-4.0 | 是，隨附；資料 manifest 標 `shipped: true` | 使用中，預設 |

`Funmula` 的 `zh-select` 是第三方模型，MIT 全文允許散布，安裝包帶本地從 ONNX 轉出的
`model.q8_0.gguf`（35,515,136 bytes）、`vocab.json` 與兩份 manifest，並保留
`legal/licenses/MIT-Funmula.txt` 的 copyright 與授權全文。`model.onnx` 只是轉檔來源，
不進 MSI。`zh-punct` 已移除，不散布。日文 reranker 不再是預設路徑：`SessionConfig::ja_rerank`
預設 `false`，由 Jinen small 的門檻混合取代。`ja-neural` 沒有 `Role` 列舉，
`jc-engine` 直接依 `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf`
載入；tokenizer 沿用 `models/quant/jinen-v2-xsmall/tokenizer.json`。資料根目錄
`manifest.json` 以 `ja_neural` 收錄 Jinen GGUF、以 `ja_tokenizer` 收錄 tokenizer，
兩筆都標 `shipped: true`、授權 `L-21`；引擎優先用 manifest 解析這兩條路徑，
manifest 沒有對應項目時才退回寫死的候選路徑。

標點模型的資料檔案保留在原目錄，方便回溯；程式與集合 manifest 都不再引用它們。
舊資料目錄的 `models/manifest.json` 若仍列著 `zh-punct`、`ja-punct`，可用
`scripts/clean-model-set-manifest.sh` 清除；解析器會靜默忽略這兩個鍵。

## 推論 runtime

2026-11-20 起只有 candle 一個推論後端，權重是 safetensors（fp32）或 GGUF
（量化）：

```text
jc-core / jc-cli / jc-engine
        │
        └── jc-model ── candle-core（CPU，Rayon）── safetensors／GGUF
```

- `models/<角色>/model.safetensors` 走 fp32。
- `models/<角色>/model.gguf` 與 `models/zh-select/model.q8_0.gguf` 走量化；副檔名自動分派。
- `JC_RERANK_WEIGHTS`、`JC_ZH_SELECT_WEIGHTS` 可覆寫權重路徑。
- ONNX Runtime 與 `--backend ort` 已移除；資料目錄的 `*.onnx` 保留，只當
  `candle-zh-check --convert` 的轉檔來源。

## 檔案雜湊

以下雜湊是實際檔案紀錄。標點的檔案仍留在資料目錄，但程式不再使用。

### 注音模型

| 角色 | 檔案 | 大小 | SHA-256 |
|---|---|---:|---|
| `zh-select` | `model.q8_0.gguf`（隨 MSI 散布） | 35,515,136 | `fbbafa210ad640ae4cba79b21ce7a28ca993d3f2f0528385e3be3d4ea5bbaed4` |
| `zh-select` | `model.safetensors` | 130,559,104 | `098e8d47b9e658dd6859e3668783942abafebdf97de2f53592f42dc0daba85d4` |
| `zh-select` | `model.onnx` | 130,723,137 | `4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261` |
| `zh-select` | `vocab.json`（隨 MSI 散布） | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` |
| `zh-select` | `manifest.json`（下載時追加 `download` 區塊） | 2,901 | `8a7ee717f17c8449666f4c128c96a64a2b2e595f314f303f59822b0e51a67847` |
| `zh-select` | `LICENSE` | 1,077 | `ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f` |
| `zh-select` | `README.md` | 8,365 | `c1cda3b9ab51321317bfafc12a634b404ca393611cdbc97ac0c98b229648a027` |
| `zh-punct` | `model.onnx` | 130,584,075 | `4db3ed40df777347d9c455da01285b01de192d4c210c5f7cd9f0646c727880d8` |
| `zh-punct` | `vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` |
| `zh-punct` | `manifest.json`（下載時追加 `download` 區塊） | 2,270 | `e82313a3ff80656295fff06575219ed2432d6a043260f382adc7848e95ddc0c7` |
| `zh-punct` | `LICENSE` | 1,077 | `ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f` |
| `zh-punct` | `README.md` | 5,596 | `0402fdc8c81618f6f9d51284ec0b0bef1a5444ff8f74e25280e2436c73a12f77` |

上游 `manifest.json` 的原始雜湊：`zh-select` 為 1,165 bytes／
`6545fed2d7092aec70f929c2fce8360e1456778e99f6eeadc9d8299e5ad91763`；
`zh-punct` 為 556 bytes／`70909d272d81fe8556efb5a0e2301c872f3c6fbdb9932a6651fedd214e39527c`。
本機檔案是下載腳本追加 `download` 區塊後的版本，因此雜湊不同。

Q8_0 是量測與建議使用的注音選字權重：33.9 MiB、推論 p50 9.36 ms、逐對排序
一致 99.51%（5439/5466）、golden 45/45。fp32 safetensors 與 ORT 的
`model.onnx` 保留作對照。

### 日文模型

| 角色 | 檔案 | 大小 | SHA-256 |
|---|---|---:|---|
| `ja-rerank` | `model.int8.onnx` | 34,268,510 | `edf1afeacb89fa5da3d10462b5d9afd85392a91056634dd9ac60d41e37faeeb3` |
| `ja-rerank` | `vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` |
| `ja-punct` | `model.int8.onnx` | 34,240,656 | `2860267deb3da1816b3f53decf786c5f5f478a1b9d327c8c80f2428c643f051b` |
| `ja-punct` | `vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` |
| 共用 | `ja-tokenizer/public_core_ja.model` | 614,451 | `b25260c9c3d90a754c719e62eea6b8125f08448d48dd4d1755c18edbd00e2078` |

`ja-rerank` 已棄用，仍保留權重、manifest 與訓練碼供回溯；`SessionConfig::ja_rerank`
預設 `false`，`jc-engine` 只在明確開啟時使用。`ja-punct` 已不再載入。

### 日文神經轉換（Jinen）

| 檔案 | 大小 | SHA-256 |
|---|---:|---|
| `jinen-v2-small-Q5_K_M.gguf` | 81,117,824 | `80482707513d6b67dafc31774371cf95d765542abf8d74eebf5f32f92d788bd3` |
| `tokenizer.json`（`models/quant/jinen-v2-xsmall/`，manifest 角色 `ja_tokenizer`） | 2,047,498 | `477c91a788bf97e7ed993a4441cfb2c4d60033e505ade17d6c367bc09ed6e6ba` |

來源 `togatogah/jinen-v2-small.gguf`，revision
`3461d0573ab447985badde3174165b967d06076c`，授權 CC-BY-SA-4.0。Qwen3
109.5M 級，Q5_K_M 量化，檔案 77.4 MiB（約 78 MB）。用途是日文轉換的
第二階段：統計結果永遠先算好，`top2 - top1 < 460` 時才送 Jinen，通過五條
驗證才替換，180 ms 逾時回退統計結果。`jc-engine` 預設載入，`JC_JINEN=off`
關閉。詳細門檻與量測見 [`docs/port/jinen-default.md`](port/jinen-default.md)。

## 集合 manifest

`models/manifest.json` 只做角色到目錄的對應，不含權重：

```json
{
  "schema": "jc.model-set/1",
  "models": {
    "zh-select": { "dir": "zh-select", "model": "model.q8_0.gguf", "vocab": "vocab.json" },
    "ja-rerank": { "dir": "reranker-extra" }
  }
}
```

`model` 欄位只決定模型集合的可用性判定；實際載入的權重由 candle 的 `weights_path`
找 `model.safetensors`、`model.gguf` 或 `model.q8_0.gguf`。未填 `model` 時預設
`model.safetensors`。引擎在集合 manifest 仍指向 ONNX 或 manifest 缺席時退回
`models/zh-select`，因此舊資料根目錄也能載入隨 MSI 安裝的 GGUF。

`zh-punct` 與 `ja-punct` 已於 2026-11-20 從程式移除，2026-11-21 也從集合
manifest 移除。`crates/jc-model/src/role.rs` 把兩者列為已知舊角色，
`ModelSet::load` 遇到時直接跳過且不印警告；其他不認得角色仍照舊警告。
舊資料目錄可用 `scripts/clean-model-set-manifest.sh` 清除兩鍵，
`scripts/prepare-zh-select.sh` 重新產生集合 manifest 時也會一併移除。
Jinen 不在這個集合 manifest 內，路徑由 `jc-engine` 的 `resolve_ja_neural` 解析，
可用 `JC_JINEN_GGUF` 覆寫；資料根目錄的 `manifest.json` 另有 `ja_neural` 一筆，
`shipped` 為 `true`。

清理前資料目錄的 SHA-256：`c069d44ff0aaa68fcec47f8030fe5075468dba0ef0c63ebc27a02a7acce0f56c`
（387 bytes，仍含兩個標點鍵）。移除兩鍵後大小與雜湊都會變，orchestrator 重新
打包時要更新此值。

Jinen 的 GGUF 與 tokenizer 另由資料根目錄的 `manifest.json` 記錄：
`ja_neural` 與 `ja_tokenizer` 各一筆、`shipped: true`、授權 `L-21`。
`jc-engine` 先讀 manifest 解析路徑，缺 `ja_tokenizer` 只記一次警告並維持統計
轉換，不擋啟動；`data verify` 仍把缺席的隨附檔算成錯誤，出貨前會擋下。
`models/ja-tokenizer/public_core_ja.model` 是 reranker 與標點時代的共用分詞器，
引擎啟動已不再探測它；檔案只供 `jc-cli tokenize ja` 與回溯使用，因此不需要
進 catalog 的隨附清單。

## 首次啟動

GGUF 隨 MSI 安裝，引擎第一次啟動就直接由 `install_select_model()` 載入，沒有背景下載、
沒有轉檔、也不會重啟子行程。`vocab.json` 與兩份 manifest 是 `SelectModel::load` 的必要檔，
一起隨 MSI 安裝。清除 `models/zh-select` 後會退回統計選字，需重新安裝或手動補回檔案。

## 散布檢查

1. `models/zh-select/model.q8_0.gguf`、`vocab.json` 與兩份 manifest 隨 MSI 散布，
   catalog 全標 `shipped: true`、授權 `L-11`，並附 `MIT-Funmula.txt` 全文。
   `model.onnx` 與 `models/zh-punct/` 不得出現在 release、安裝包或 git
   （`zh-punct` 已於 2026-11-21 從 catalog 移除）。
2. Jinen v2 small 隨安裝包出貨，資料 manifest 標 `shipped: true`。散布時附
   `legal/licenses/CC-BY-SA-4.0.txt` 與 `legal/THIRD-PARTY-NOTICES.md` 第 15 節，
   保留作者 Hitoshi Togasaki 與國立國會圖書館的署名；衍生權重同樣採 CC-BY-SA-4.0。
3. 日文 reranker 與日文標點模型不隨安裝包出貨，權重留在資料目錄供回溯；若重新
   納入散布，附 `legal/THIRD-PARTY-NOTICES.md` 第 9、10 節的署名與授權。
4. Jinen 的 tokenizer 在 `models/quant/jinen-v2-xsmall/`，以 `ja_tokenizer` 角色列進
   資料 manifest，`shipped: true`；安裝器依 manifest 複製，缺少時 `data verify`
   會擋下。
5. 換 revision 時同步更新本表、下載腳本與帳本；不得只改 manifest 而留下舊雜湊。
