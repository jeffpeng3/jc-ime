# zh-models port 對照表

下載注音選字與注音標點 ONNX 模型。前身沒有對應的 Rust 實作；這裡只處理
資料取得、雜湊固定與 manifest，沒有改動 Rust 程式碼。

> **現況（2026-11-21）**：`zh_punct` 已從 catalog 移除，不再是下載項。本文件的
> 模型來源與雜湊表保留供回溯，檔案仍在資料目錄。

## 來源

| 角色 | Hugging Face repo | revision | 授權 |
|---|---|---|---|
| `zh-select` | `Funmula/0c-sequence-d512` | `3533e34ef25896afeacc928d136ba7f35783bba7`（`3533e34e`） | MIT |
| `zh-punct` | `Funmula/0c-punct-d512` | `105c2770e0b7cf4ce44a792b3b186000dd1727d9`（`105c2770`） | MIT |

兩個 repo 都提供 `model.onnx`（fp32）與 `vocab.json`，沒有 safetensors。
`vocab.json` 的 schema 是 `zuin-phonetic-v1`（27,857 個字、416 個注音 base），
不是 `jc-model` tokenizer 目前支援的 `0c.sentencepiece.vocab/1`；模型可用性
探測只檢查檔案存在，實際注音推論接線是後續 task。

## 檔案

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `zh-select/model.onnx` | 130,723,137 | `4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261` |
| `zh-select/vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` |
| `zh-select/manifest.json` | 上游 1,165 | 上游 `6545fed2d7092aec70f929c2fce8360e1456778e99f6eeadc9d8299e5ad91763` |
| `zh-select/LICENSE` | 1,077 | `ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f` |
| `zh-select/README.md` | 8,365 | `c1cda3b9ab51321317bfafc12a634b404ca393611cdbc97ac0c98b229648a027` |
| `zh-punct/model.onnx` | 130,584,075 | `4db3ed40df777347d9c455da01285b01de192d4c210c5f7cd9f0646c727880d8` |
| `zh-punct/vocab.json` | 312,708 | `556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3` |
| `zh-punct/manifest.json` | 上游 556 | 上游 `70909d272d81fe8556efb5a0e2301c872f3c6fbdb9932a6651fedd214e39527c` |
| `zh-punct/LICENSE` | 1,077 | `ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f` |
| `zh-punct/README.md` | 5,596 | `0402fdc8c81618f6f9d51284ec0b0bef1a5444ff8f74e25280e2436c73a12f77` |

角色目錄的 `manifest.json` 保留上游 runtime 欄位（`zuin-sequence-runtime-v2`、
`zuin-punct-runtime-v1`），再追加 `download` 區塊，記錄 repo、完整 revision、
授權、上游 manifest 雜湊，以及每個檔案的大小、SHA-256 與下載 URL。`NOTICE`
記來源與 revision。

`/mnt/work/jc/jc-ime-data/models/manifest.json` 是 `jc-model` 讀的集合
manifest（schema `jc.model-set/1`），列出既有 `ja-rerank`、`ja-punct` 與新的
`zh-select`、`zh-punct`。注音模型的檔名是 `model.onnx`，因此明確覆寫
`model` 欄位。

## 與前身 lock 的差異

`ref/0c-ime/models/models.lock.json` 記錄的 select `manifest.json` 是
1,232 bytes、SHA-256 `1d0d989d…`，但 HF 在 `3533e34e` 實際提供的是
1,165 bytes、`6545fed2…`；前身 lock 的內容多出 `placeholder` 等欄位，且
`dictionary_sha256` 是舊值 `4e4c4cdf…`。本次以下載當下的 HF 內容為準，
固定 `6545fed2…`，並保留上游的 `dictionary_sha256` `517e6c8b…`。其餘
model／vocab／LICENSE／README 雜湊與前身 lock 一致。

## 驗證

```sh
sh scripts/fetch_zh_models.sh
ls -la /mnt/work/jc/jc-ime-data/models/zh-select /mnt/work/jc/jc-ime-data/models/zh-punct
```

腳本對既有檔案重新檢查大小與 SHA-256，不符直接失敗，不覆寫；缺少的檔案才
下載到 `.part`，驗證後改名。上游 `manifest.json` 快取在
`/tmp/opencode/zh-models/<role>/`，每次重跑都重新產生角色目錄的 manifest。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `ref/0c-ime/models/models.lock.json` | `scripts/fetch_zh_models.sh` | rewritten：下載鎖改成腳本內嵌表格，來源與雜湊寫進角色 `manifest.json` |
| `ref/0c-ime/models/upstream/select/manifest.json` | `jc-ime-data/models/zh-select/manifest.json` | rewritten：保留上游 runtime 欄位，追加 `download` 區塊 |
| `ref/0c-ime/models/upstream/punct/manifest.json` | `jc-ime-data/models/zh-punct/manifest.json` | rewritten：同上 |
| `ref/0c-ime/internal/neural/native/input/input.go` | — | n/a：本 task 只下載模型，注音輸入前處理未移植 |
| `ref/0c-ime/internal/neural/native/select_e2e_test.go` | — | n/a：Rust 尚無注音模型載入與推論路徑 |
