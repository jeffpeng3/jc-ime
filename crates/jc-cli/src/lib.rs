//! 開發工具：資料載入、題庫評測、單句轉換與 bench。
//!
//! 這個 crate 只是 `jc-core` 與 `jc-eval` 的膠水：不實作轉換演算法，也不
//! 讀寫個人化資料。所有子命令都走同一組資料載入與轉換器，讓評測與互動
//! 模式看到完全一樣的排序。

pub mod bench;
pub mod cli;
pub mod converter;
pub mod data;
pub mod eval_cmd;
pub mod tokenize;

pub use cli::{run, CliError, Command};
