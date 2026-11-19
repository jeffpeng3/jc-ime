# jc-model port 對照表

前身：`internal/neural`、`internal/neural/native`、`internal/spm`、
`platforms/neural`（Go，main `8f443af`）。

## 實作摘要

- `ort 2.0.0-rc.15`（`load-dynamic`、`api-31`）是 optional dependency，feature
  `onnx`，`default = []`。預設建置不拉入 `ort`，公開 API 也沒有 `ort` 型別。
- `ModelSet::load(dir)` 讀集合層 `manifest.json`（schema `jc.model-set/1`），
  逐角色回報 `Availability`。manifest、目錄、模型檔或 vocab 缺席都只是
  `Unavailable`，不是錯誤；只有 JSON／schema 損壞才回 `ModelError`。
- reranker 佈局照 `rerank_plan.go`：`<bos> [left 右對齊] <eos> candidate
  <eos> [pad]`，seq 124、candidate slot 62、單側 60 token。右文只留在介面，
  不進張量（前身部署沒有 lookahead）。
- 逾時用 `RunOptions::terminate` 加上看門狗執行緒，預設 100 ms；逾時回
  `None`／`ModelError`，呼叫端退回統計排序。
- `vocab.json` 是 `0c.sentencepiece.vocab/1`，只有 `id_to_piece`，**沒有
  piece log-probability**。因此 tokenizer 無法做 unigram Viterbi，改用最長
  前綴匹配＋byte fallback；這是與前身 `internal/spm` 最大的行為差異，已在
  `tokenizer.rs` 與此處標明。
- 標點模型可載入並取得 logits；`punctuate()` 仍是 identity stub。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/backend.go` | `crates/jc-model/src/rerank.rs`、`punctuation_model.rs` | rewritten：只保留單一模型的 session 與逾時契約，常駐管理屬下一個 task |
| `internal/neural/backend_test.go` | `crates/jc-model/src/rerank.rs` | rewritten：改成逾時與佈局參數的行為測試 |
| `internal/neural/bundle.go` | `crates/jc-model/src/manifest.rs` | rewritten：Rust 版只解析集合 manifest，不含 bundle 下載流程 |
| `internal/neural/bundle_manifest.go` | `crates/jc-model/src/manifest.rs` | rewritten：欄位縮到載入所需；簽章／大小核對留給安裝器 |
| `internal/neural/bundle_test.go` | `crates/jc-model/src/manifest.rs` | rewritten：manifest 解析與缺檔測試 |
| `internal/neural/ja_wiring.go` | — | n/a：`rerank.Verifier` adapter 屬 jc-core 接線，下一個 task |
| `internal/neural/ja_wiring_test.go` | — | n/a：同上，jc-model 不提供 decode 介面 |
| `internal/neural/manifest.go` | `crates/jc-model/src/manifest.rs` | rewritten：改用 `serde` 解析，雜湊驗證未移植 |
| `internal/neural/onnx.go` | — | n/a：protobuf 外框探測由 `ort` 載入失敗取代，沒有獨立完整性掃描 |
| `internal/neural/punct_manifest.go` | `crates/jc-model/src/manifest.rs`、`punctuation_model.rs` | rewritten：只留 tensor 契約與載入所需欄位 |
| `internal/neural/punct_manifest_test.go` | `crates/jc-model/src/manifest.rs` | rewritten：manifest 錯誤與缺檔案例 |
| `internal/neural/residency.go` | — | n/a：模型常駐、健康狀態與熱替換屬 jc-core 接線 |
| `internal/neural/residency_test.go` | — | n/a：同上 |
| `internal/neural/telemetry_cache.go` | — | n/a：常駐管理器的 telemetry，不在本 crate |
| `internal/neural/telemetry_cache_test.go` | — | n/a：同上 |
| `internal/neural/native/backend_ort.go` | `crates/jc-model/src/rerank.rs`、`punctuation_model.rs`、`runtime.rs` | rewritten：`ort` 2.x API、`load-dynamic` 與 Rust 所有權 |
| `internal/neural/native/backend_stub.go` | — | n/a：CGO build tag 由 Cargo feature `onnx` 取代 |
| `internal/neural/native/doc.go` | `crates/jc-model/src/lib.rs` | n/a：package 說明改寫進 crate 文件 |
| `internal/neural/native/input/input.go` | — | n/a：注音輸入前處理，尚無注音模型 |
| `internal/neural/native/input/input_test.go` | — | n/a：同上 |
| `internal/neural/native/input/neutral.go` | — | n/a：同上 |
| `internal/neural/native/input/neutral_test.go` | — | n/a：同上 |
| `internal/neural/native/manifest.go` | `crates/jc-model/src/manifest.rs` | rewritten：檔案路徑改由集合 manifest 決定 |
| `internal/neural/native/onnxio.go` | — | n/a：不預先讀 graph signature；`ort` commit 時直接回報錯誤 |
| `internal/neural/native/onnxio_test.go` | — | n/a：同上 |
| `internal/neural/native/parity_test.go` | — | n/a：parity harness 由 `bin/probe.rs` 的延遲／分數輸出取代 |
| `internal/neural/native/punct_ort.go` | `crates/jc-model/src/punctuation_model.rs` | rewritten：保留推論與視窗，`punctuate` 仍是 identity stub |
| `internal/neural/native/punct_parity_test.go` | — | n/a：整合階段另跑，不在本 task |
| `internal/neural/native/punct_plan.go` | `crates/jc-model/src/manifest.rs`、`punctuation_model.rs` | rewritten：只保留 tensor 契約與視窗常數 |
| `internal/neural/native/punct_plan_test.go` | `crates/jc-model/src/punctuation_model.rs` | rewritten：改成 stub 契約與視窗界線測試 |
| `internal/neural/native/rerank_ort.go` | `crates/jc-model/src/rerank.rs` | rewritten：批次包裝、逾時與 `Mutex<Session>` 依 Rust 重寫 |
| `internal/neural/native/rerank_plan.go` | `crates/jc-model/src/layout.rs` | ported：佈局、slot 與截斷行為一致；測試改寫成具體張量值 |
| `internal/neural/native/rerank_stub.go` | — | n/a：由 feature gate 與 `Availability` 取代 |
| `internal/neural/native/runtime.go` | `crates/jc-model/src/runtime.rs` | rewritten：`load-dynamic` 初始化與看門狗逾時 |
| `internal/neural/native/runtime_test.go` | `crates/jc-model/src/runtime.rs` | rewritten：只測平台 DLL 名稱 |
| `internal/neural/native/select_cases_test.go` | — | n/a：注音選字模型不存在，維持 `Unavailable` |
| `internal/neural/native/select_e2e_test.go` | — | n/a：同上 |
| `internal/neural/native/types.go` | `crates/jc-model/src/error.rs`、`manifest.rs` | rewritten：sentinel error 改成 `thiserror` 與 `Availability::reason` |
| `internal/spm/encode.go` | `crates/jc-model/src/tokenizer.rs` | rewritten：vocab 沒有 piece 分數，改最長前綴匹配＋byte fallback |
| `internal/spm/model.go` | `crates/jc-model/src/tokenizer.rs` | rewritten：讀 `0c.sentencepiece.vocab/1` JSON，不解析 SentencePiece protobuf |
| `internal/spm/spm_test.go` | `crates/jc-model/src/tokenizer.rs` | rewritten：正規化、最長匹配、byte fallback、真實詞表 roundtrip |
| `platforms/neural/**` | — | n/a：平台外殼與說明文件，不在 workspace 範圍 |
