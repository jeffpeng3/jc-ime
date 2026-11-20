# 模型來源與雜湊

本表列出 `jc-model` 的角色來源、固定 revision、授權與實際檔案雜湊。2026-11-20
起標點模型已從程式移除，`zh-punct` 與 `ja-punct` 只保留歷史紀錄，模型檔仍在
資料目錄，不再被載入。資料來源：

- 注音：`jc-ime-data/models/zh-select/manifest.json`、`zh-punct/manifest.json`（下載時
  追加的 `download` 區塊）。
- 日文：`jc-ime-data/models/manifest.json` 的角色對應，以及前身固定的
  `ref/0c-ime/models/pairing.lock.json`（HF 套件沒有在資料根目錄留下 runtime manifest）。
- 集合 manifest：`jc-ime-data/models/manifest.json`（schema `jc.model-set/1`）。

授權帳本與完整聲明見 [`legal/README.md`](../legal/README.md) 與
[`legal/THIRD-PARTY-NOTICES.md`](../legal/THIRD-PARTY-NOTICES.md)。

## 角色與來源

| 角色 | 目錄 | repo | revision | 授權 | 是否散布 | 狀態 |
|---|---|---|---|---|---|---|
| `zh-select` | `models/zh-select` | `Funmula/0c-sequence-d512` | `3533e34ef25896afeacc928d136ba7f35783bba7`（`3533e34e`） | MIT | **否，第三方僅下載** | 使用中 |
| `zh-punct` | `models/zh-punct` | `Funmula/0c-punct-d512` | `105c2770e0b7cf4ce44a792b3b186000dd1727d9`（`105c2770`） | MIT | **否，第三方僅下載** | 已移除（2026-11-20） |
| `ja-rerank` | `models/reranker-extra` | `jeffpeng3/0c-ja-reranker` | `7450e10fe8b673560044c5db08ad3006a06d7a6e`（`7450e10f`） | CC-BY-SA-4.0 | 是（日文模型包） | 使用中 |
| `ja-punct` | `models/punctuation-public-core` | `jeffpeng3/0c-ja-punct` | `ff9166abc620b6716ac7de73cfa67335e6b985d2`（`ff9166ab`） | MIT | 是（日文模型包） | 已移除（2026-11-20） |

`Funmula` 兩個模型是第三方模型，只隨下載腳本取得，不進 git，也不得放進本專案的
release 或安裝包。日文兩個角色以同一個模型包散布，權重同樣不進 git。

標點模型的資料檔案保留在原目錄，方便回溯；程式與 manifest 不再引用它們。

## 檔案雜湊

以下雜湊是歷史紀錄。標點的檔案仍留在資料目錄，但程式不再使用。

### 注音模型

| 角色 | 檔案 | 大小 | SHA-256 |
|---|---|---:|---|
| `zh-select` | `model.onnx` | 130,723,137 | `4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261` |
| `zh-select` | `vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` |
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

### 日文模型

| 角色 | 檔案 | 大小 | SHA-256 |
|---|---|---:|---|
| `ja-rerank` | `model.int8.onnx` | 34,268,510 | `edf1afeacb89fa5da3d10462b5d9afd85392a91056634dd9ac60d41e37faeeb3` |
| `ja-rerank` | `vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` |
| `ja-punct` | `model.int8.onnx` | 34,240,656 | `2860267deb3da1816b3f53decf786c5f5f478a1b9d327c8c80f2428c643f051b` |
| `ja-punct` | `vocab.json` | 515,605 | `4528ac15d2c17da1246d85dd6bac9fb09ec987f699c38933ae8b7d38b905e18a` |
| 共用 | `ja-tokenizer/public_core_ja.model` | 614,451 | `b25260c9c3d90a754c719e62eea6b8125f08448d48dd4d1755c18edbd00e2078` |

上游另提供 fp32 權重，但本專案出貨與本機資料使用 int8：

| 角色 | fp32 檔案 | 大小 | SHA-256 |
|---|---|---:|---|
| `ja-rerank` | `model.onnx` | 135,613,420 | `4b74ae019b15443f2a30936e9cf53959e60716a30548f17bb1f0d8569fa978e9` |
| `ja-punct` | `model.onnx` | 135,599,640 | `998b3df5af330a7f2106d6ecd4185a15760bb4ea942eb3ebf9f24b9ca7151a06` |

`ja-tokenizer/public_core_ja.model` 是 `ja-rerank` 與 `ja-punct` 共用的 SentencePiece
模型；兩個模型包各自的 notice 都要附。`ja-rerank` 的權重依 CC-BY-SA-4.0 散布，
`ja-punct` 依 MIT 散布。`ja-punct` 已不再載入，分詞器仍由 `ja-rerank` 使用。

## 集合 manifest

`models/manifest.json` 只做角色到目錄的對應，不含權重：

```json
{
  "schema": "jc.model-set/1",
  "models": {
    "zh-select": { "dir": "zh-select", "model": "model.onnx", "vocab": "vocab.json" },
    "ja-rerank": { "dir": "reranker-extra" }
  }
}
```

`zh-punct` 與 `ja-punct` 已於 2026-11-20 從程式移除；本專案的 manifest 範本
不再列出。資料目錄的既有 `manifest.json` 仍可能保留這兩個鍵，載入時會忽略
不支援的角色。

資料目錄現況的 SHA-256：`c069d44ff0aaa68fcec47f8030fe5075468dba0ef0c63ebc27a02a7acce0f56c`
（387 bytes，仍含兩個標點鍵）。

## 散布檢查

1. `models/zh-select`、`models/zh-punct` 不得出現在 release、安裝包或 git。
2. 散布日文模型包時，附上 `legal/licenses/CC-BY-SA-4.0.txt`、`legal/licenses/MIT.txt`
   與 `legal/THIRD-PARTY-NOTICES.md` 第 9、10 節。
3. 換 revision 時同步更新本表、下載腳本與帳本；不得只改 manifest 而留下舊雜湊。
