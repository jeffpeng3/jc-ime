# fix-private-flag：把 private 旗標接到引擎的學習層

本任務把外殼既有的 `private` 旗標帶進 jc 的 `Key`，並補上內容側的學習守門。
Go 參考版本目前的 `core/session/session.go` 已有 `Request.Private` 與 `s.private`；
Rust 端依任務書的狀態生命週期實作，不整包照搬 Go 的「組字中 OR」規則。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/proto/proto.go`（`Request.Private`） | `crates/jc-core/src/key.rs` | rewritten：`Key.private` 預設 false，序列化時省略 |
| `platforms/windows/tsf/src/session.rs`（`private` 隨按鍵送出） | `crates/jc-tsf/src/session.rs` | rewritten：`to_jc_request` 把 `req.private` 轉進結構化 `Key` |
| `core/session/session.go`（`s.private` 學習閘門） | `crates/jc-core/src/session/mod.rs`、`session/core.rs`、`session/mixed.rs` | rewritten：`press` 逐鍵覆寫，`commit`／Esc 歸零；學習入口跳過寫入 |
| `core/session` 的內容守門（研究文件引用 `core/src/learn.rs`） | `crates/jc-core/src/learn_guard.rs` | rewritten：`is_sensitive` 擋數字、email、URL、卡號、身分證與長英數帳號 |

## 行為差異

- Go 在組字中把 private 做 OR，Rust 依任務書改成「每個按鍵明確覆寫」，避免
  切換欄位後殘留。
- Go 的 `is_sensitive` 不在目前參考 checkout 的路徑；Rust 版本依研究文件列出的
  規則重寫，並用 `looks_like_bopomofo_keys` 豁免注音按鍵序列。
