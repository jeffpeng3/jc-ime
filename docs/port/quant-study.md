# quant-study：精度與效能取捨

本任務沒有移植 Go 程式，產物是量測腳本與報告。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `scripts/quant/dump-inputs/src/main.rs` | rewritten：用 jc-model／jc-core 的公開 API dump 固定輸入 |
| n/a | `scripts/quant/*.py` | rewritten：量化、ONNX／GGUF 量測與端到端比較 |

- 報告：`docs/reports/quant-study-2026-11-20.md`
- 量化檔：`/mnt/work/jc/jc-ime-data/models/quant/`
- 沒有修改 `jc-cli`、`jc-core`、`jc-model` 的行為；端到端用 `JC_MODELS`／`JC_JA_MODELS` 切換。
