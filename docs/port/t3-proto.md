# t3-proto：jc-proto 擴充 0c 需要的 op

基準：0c-ime fork main `8f443af`，T1a 回報 `/tmp/opencode/t1a-session-result.md`。
本輪只定義協定型別與訊框，不改引擎的組字行為，PROTOCOL_VERSION 維持 3。

## 逐檔對照

Go 檔案｜Rust 檔案｜狀態
---|---|---
`bridge/ipc/ipc.go`（Envelope 的 op/Reply 欄位）｜`crates/jc-proto/src/lib.rs`（Request／Response 新 variant、HealthInfo、VocabInfo）｜rewritten：改成 jc 的 internally tagged enum，保留 op 字串與 session/info/vocab 語意
`bridge/ipc/server.go`（health() 與 do() 的 op 分派）｜`crates/jc-proto/src/lib.rs`（ErrorCode::Unsupported）｜rewritten：T3 只定義「不支援」錯誤碼，實際分派留給 T4/T6
`core/proto/proto.go`（Request.Type 的 context/mode/caps_lock/output_simplified）｜`crates/jc-proto/src/lib.rs`（Request::Context／Mode／CapsLock／OutputSimplified）｜rewritten：改成獨立 op variant，欄位全部 serde default
`core/engine/vocab.go`（VocabRequest／VocabResponse）｜`crates/jc-proto/src/lib.rs`（Request::Vocab、Response::Vocab、VocabInfo）｜rewritten：T3 只固定 ok/changed/error，其餘用 raw JSON 攜帶，T6 再展開
`bridge/ipc/client.go`（ping/health/apply_settings/drain 的送出）｜T1a 的 `crates/jc-tsf-next/src/session.rs::to_jc_request()`｜n/a：T1a 已經保留原始 envelope，本輪不需要再改送出端

## 新增的 Request op

Ping、Health、ApplySettings{settings}、Drain、Vocab{directive,vocab}、ResetPersonal、
Close{session}、Context{on,context_id,context_enabled}、Mode{on}、CapsLock{on}、
OutputSimplified{on}。共 11 個（10 個任務書清單加 plan 要求的 ping）。

## 新增的 Response 型別

Acknowledged{session}、Health{info: HealthInfo}、Vocab{vocab: VocabInfo}，
以及 ErrorCode::Unsupported。HealthInfo 有 draining、connections、idle_seconds、
sessions、composing、pid、output_simplified、personal_reset、capabilities 九欄，
並提供 as_info() 轉成 0c 的 map[string]string 形狀。

## 向後相容

既有 Hello/Key/Toggle/Snapshot/Commit/Shutdown 的 JSON 逐字不變。新 op 都是獨立
variant，每個欄位都有 #[serde(default)] 與適當的 skip_serializing_if；T1a 只帶
on/context_id 的精簡 JSON 也能解。舊引擎遇到新 op 由 Request::Unknown 收下並回
UnknownOp。版本號維持 3。

## 驗證

cargo check -p jc-proto --all-targets --offline：通過。
cargo check --workspace --all-targets --offline：通過（含跨 crate 窮盡比對修補）。
cargo fmt --all -- --check：通過。沒有跑 test，沒有 commit。

詳細回報見 /tmp/opencode/t3-proto-result.md。
