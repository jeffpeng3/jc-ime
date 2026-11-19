# jc-ime

Windows 優先的輸入法，核心全部以 Rust 實作，平台整合由獨立 wrapper 完成。

- 目標：注音（台灣）、日文（羅馬字→かな→漢字）、英文，同一個 Mixed 模式直接輸入。
- 核心：純 Rust、不依賴作業系統 API，可在 Linux（DGX）開發與評測。
- 整合：Windows 先做（TSF），其他平台之後以同一個核心接上。
- 資料：詞典與模型不進 git，由腳本下載並以雜湊固定。
- 腳本：允許使用 Python。

## 目錄

| 路徑 | 內容 |
|---|---|
| `crates/jc-core` | 鍵序列、片段、lattice、路徑評分、候選 |
| `crates/jc-data` | 詞典／連接矩陣／語言模型的二進位格式 |
| `crates/jc-model` | ONNX Runtime 接線（`ort`） |
| `crates/jc-proto` | 引擎與 wrapper 的協定 |
| `crates/jc-platform` | 平台抽象 trait |
| `crates/jc-eval` | 凍結題庫評測 |
| `crates/jc-engine` | 引擎行程（IPC 伺服端） |
| `crates/jc-cli` | 開發工具：匯入、評測、bench |

## 必讀

- [`docs/history.md`](docs/history.md)：為什麼重寫、從 0c-ime 學到什麼、可沿用的資產與品質基準。
- [`docs/architecture.md`](docs/architecture.md)：核心與 wrapper 的邊界。

## 建置與測試

```sh
cargo test --workspace
cargo run -p jc-engine
```

Windows 產物一律以 MSVC 建置；Linux 只用來開發核心與跑評測。

## 致謝

本專案的輸入法設計、注音與日文題庫、詞典資料與模型，來自前身專案 [0c-ime](https://github.com/0c-biggo/0c-ime)。`jc-ime` 是它的核心以 Rust 重寫的版本，功能範圍與品質基準都由它啟發；沒有 0c-ime 累積的題庫與資料，這個專案無法在同樣時間內達到可驗證的品質。

0c-ime 同為 AGPL-3.0 授權。

## 授權

AGPL-3.0-only，見 `LICENSE`。
