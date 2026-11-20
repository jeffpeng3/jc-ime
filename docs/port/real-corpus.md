# real-corpus：移植對照

這個 task 沒有移植 Go 程式；它用既有 Rust 子命令量測資料，新增一個純量測子命令。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（無 Go 對應） | `crates/jc-cli/src/corpus_cmd.rs` | added：真人語料收集與 A/B 比對 |
| n/a（無 Go 對應） | `crates/jc-cli/src/cli.rs` | modified：註冊 `corpus-diff` 參數與分派 |
| n/a（無 Go 對應） | `crates/jc-cli/src/lib.rs` | modified：註冊 `corpus_cmd` 模組 |
| `docs/reports/segment-diff-2026-11-21.md` 的分類程式 | `corpus_cmd.rs` 內的 `Case`／`Bucket`／`classify` | copied：不改 `diff_cmd.rs`，避免與 `segment-diff` 分支互踩 |
| n/a（無 Go 對應） | `docs/reports/real-corpus-2026-11-21.md` | added：量測報告 |

沒有搬行為測試：這個 task 只量測，不改 `mixed.rs`／`mixed_latin.rs`。`corpus_cmd.rs`
內的單元測試只覆蓋語料轉換（注音轉鍵、去聲調、かな反查、分類名稱），
最終由整合階段統一跑。
