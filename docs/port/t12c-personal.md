# T12c：個人化全域套用 Port 對照

本任務把 T4 的 per-session 個人資料改成 engine 單一共用 store，沒有對應的
Go 新檔案；下表列出行為來源與 Rust 落點。

Go 檔案｜Rust 檔案｜狀態
---|---|---
`core/personal/personal.go`（Store／Load／Save）| `crates/jc-core/src/personal/mod.rs` | n/a：Store 的 load/save 已存在，本任務直接呼叫
`bridge/ipc/server.go`（personal_reset 能力與 reset_personal）| `crates/jc-engine/src/lib.rs` | rewritten：能力旗標仍為 true，reset 改成清空共用 store 並寫回前綴檔
`cmd/0c-update/personal.go`（清除後立即寫回 user.json）| `crates/jc-engine/src/lib.rs` | rewritten：ResetPersonal 立刻寫回空白 personal.json
n/a（T4 的 per-session 存檔取捨）| `crates/jc-engine/src/personal_merge.rs` | ported：新增增量合併，取代各自存檔
n/a（T9 前綴參數）| `crates/jc-engine/src/main.rs` | rewritten：啟動讀回前綴檔，不再寫 per-session 檔

Rust 側新增／修改：

- `crates/jc-core/src/session/mod.rs`：新增 `Session::set_personal_store`。
- `crates/jc-engine/src/personal_merge.rs`：新增共用 store 的增量合併。
- `crates/jc-engine/src/lib.rs`：共用 store、讀回、Close／Drain／Shutdown／週期寫回、Health 能力旗標、測試。
- `crates/jc-engine/src/main.rs`：啟動讀回。

下一次 rc 移除舊版後，引擎統一用 `personal` 前綴（`personal.json`）。個人化
**不搬移**：rc.15 留下的 `personal-next.json` 留在目錄裡，不讀不寫；全新安裝
從空白開始。舊的 per-session 檔（`personal-<id>.json`）也不處理。
