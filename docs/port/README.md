# Port 對照表

每個移植 task 在 `docs/port/<task>.md` 留下一份對照表，說明 Go 參考實作
的每個檔案去了哪裡。

## 格式

表格固定三欄：

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/segment/segment.go` | `crates/jc-core/src/segment.rs` | ported |
| `core/lattice/candidate.go` | `crates/jc-core/src/candidate.rs` | rewritten |
| `cmd/0c-ibus/engine.go` | — | n/a：屬於平台外殼，不在本 workspace 範圍 |

## 狀態值

- `ported`：行為與 Go 版本一致，測試也一起搬過來（`TestFoo_Bar` →
  `fn test_foo_bar()`）。
- `rewritten`：行為等價，但因為 Rust 的型別或所有權模型而重寫結構；
  必須在同一個 task 的說明裡寫出動機。
- `n/a`：不移植。**一定要寫原因**，不能只寫 `n/a`。常見原因是平台相關、
  已被其他 crate 取代，或 Go 版本只是內部實作細節。

## 規則

- 每個 Go 檔案都要有一列，包含測試檔；純內部結構的測試改寫成行為測試，
  狀態標 `rewritten` 並註明。
- 需要外部資料的測試用 `jc_testdata` 的 `require_data!()`，缺資料時跳過。
- 測試要斷言具體值，不要只斷言 `is_ok()`。

## 索引

### 協定、平台與傳輸

| 文件 | 一句話 |
|---|---|
| [`c0-contracts.md`](c0-contracts.md) | 第一批共用契約、目錄骨架與錯誤型別 |
| [`w1-proto-engine.md`](w1-proto-engine.md) | 訊框、請求／回應與引擎請求迴圈的 Go 對照 |
| [`proto-extend.md`](proto-extend.md) | 補上 `cursor_rune`、`suggestion`、`label`、`navigating` |
| [`proto-candidates.md`](proto-candidates.md) | v3 的 `details`、`segments`、`highlighted` |
| [`proto-compat.md`](proto-compat.md) | v1／v2 形狀仍可解析與 fixture 版本守門 |
| [`jc-platform.md`](jc-platform.md) | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 契約 |
| [`client-crates.md`](client-crates.md) | `jc-windows` client 與 `jc-linux`／`jc-mac` 佔位 |
| [`named-pipe.md`](named-pipe.md) | Windows named pipe 傳輸、名稱規則與錯誤處理 |
| [`onnx-default.md`](onnx-default.md) | 移除 `onnx` feature，改成執行期偵測動態庫 |

### 注音

| 文件 | 一句話 |
|---|---|
| [`z1-zh-front.md`](z1-zh-front.md) | 注音音節、鍵盤與前端輸入 |
| [`z2-zh-mid.md`](z2-zh-mid.md) | lattice、Viterbi 與候選建立 |
| [`z3-zh-back.md`](z3-zh-back.md) | 注音解碼與輸出組裝 |
| [`zh-layout-daqian-only.md`](zh-layout-daqian-only.md) | 只保留大千鍵盤排列 |
| [`zh-data.md`](zh-data.md) | `lexicon.bin`／`lm.bin` 的零拷貝讀取 |
| [`bigram-data.md`](bigram-data.md) | 新酷音 bigram 下載與詞典重建 |
| [`zh-bigram.md`](zh-bigram.md) | bigram 接進注音解碼與 N-best |
| [`zh-golden.md`](zh-golden.md) | golden 45 例的第一段 |
| [`zh-golden-rest.md`](zh-golden-rest.md) | zhconv 簡繁與 mixed ASCII 推斷，補到 45/45 |
| [`zh-symbols.md`](zh-symbols.md) | 符號、emoji、全形數字與 Ctrl 標點 |
| [`zh-models.md`](zh-models.md) | 注音模型下載、manifest 與雜湊 |
| [`zh-select-calibrate.md`](zh-select-calibrate.md) | 注音選字模型的門檻與校準 |
| [`eval-fuzzy.md`](eval-fuzzy.md) | 模糊音題庫與 919 題評測 |
| [`personal-zh.md`](personal-zh.md) | 注音個人化學習、自訂詞與 tombstone |

### 日文

| 文件 | 一句話 |
|---|---|
| [`j1-ja-input.md`](j1-ja-input.md) | 羅馬字輸入、かな改寫與半截讀音 |
| [`j2-ja-lexicon.md`](j2-ja-lexicon.md) | 日文詞典、連接矩陣與 POS 成本 |
| [`j3-ja-decode.md`](j3-ja-decode.md) | 日文 Viterbi 解碼與詞邊界 |
| [`ja-pack-v2.md`](ja-pack-v2.md) | `0CJADIC` pack v2 的讀取端 |
| [`ja-import.md`](ja-import.md) | Mozc `dictionary_oss` 匯入與 pack v2 產出 |
| [`ja-context.md`](ja-context.md) | context ID、左文脈與評測數字 |
| [`eval-ja-ambiguous.md`](eval-ja-ambiguous.md) | 日文歧義題庫與 public-core 對照 |
| [`ja-particle-rewrite.md`](ja-particle-rewrite.md) | 助詞讀音交替與かな鏈 |
| [`gating-lowcall.md`](gating-lowcall.md) | 低呼叫率門檻與 `auto`／`quality`／`off` |
| [`ja-live-neural.md`](ja-live-neural.md) | 逐字変換接 Jinen 與五條驗證 |
| [`ja-cost-diag.md`](ja-cost-diag.md) | 拆解錯誤轉換的成本來源 |
| [`ja-live-rewrite.md`](ja-live-rewrite.md) | 逐字変換與即時改寫 |
| [`ja-personal.md`](ja-personal.md) | 日文個人學習的 Go 對照 |
| [`ja-personal-wire.md`](ja-personal-wire.md) | 把日文學習接進 `Session` |
| [`ajimee-eval.md`](ajimee-eval.md) | AJIMEE-Bench 接線、`accept` 比對與目錄掃描 |
| [`fix-cli-ja.md`](fix-cli-ja.md) | `jc-cli` 與 `ja-eval` 的分數對齊 |
| [`m2-cli-session.md`](m2-cli-session.md) | CLI、session 與日文資料載入 |
| [`mixed-e2e.md`](mixed-e2e.md) | Mixed 端到端轉換與片語合併 |
| [`ja-size.md`](ja-size.md) | 日文 payload 的量測與縮減方案 |

### 模型與神經轉換

| 文件 | 一句話 |
|---|---|
| [`jc-model.md`](jc-model.md) | 模型 manifest、tokenizer 與 reranker 接線 |
| [`model-wire-zh.md`](model-wire-zh.md) | 注音選字模型接進候選融合 |
| [`model-wire-ja.md`](model-wire-ja.md) | 日文 reranker 接進轉換 |
| [`rerank-fix.md`](rerank-fix.md) | reranker 的位置對齊、左文與融合權重 |
| [`rerank-tokenizer.md`](rerank-tokenizer.md) | 用 SentencePiece unigram 取代最長前綴 |
| [`rerank-wordlevel.md`](rerank-wordlevel.md) | 把 reranker 用在詞層級與詞群門檻 |
| [`rerank-gating.md`](rerank-gating.md) | 只重排統計不確定的詞群 |
| [`candle-spike-ja.md`](candle-spike-ja.md) | 日文 reranker 的 candle 實作 |
| [`candle-quant.md`](candle-quant.md) | candle 量化權重與精度門檻 |
| [`candle-zh.md`](candle-zh.md) | 注音選字搬 to candle，Q8_0 出貨權重 |
| [`quant-study.md`](quant-study.md) | 量化等級的精度與效能取捨 |
| [`jinen-integrate.md`](jinen-integrate.md) | Jinen v2 離線轉換後端（candle） |
| [`jinen-gating.md`](jinen-gating.md) | 輸入側信心門檻組合統計與 Jinen |
| [`jinen-gating-v2.md`](jinen-gating-v2.md) | 多目標門檻與 Jinen small 重測 |
| [`jinen-default.md`](jinen-default.md) | Jinen small 接進 `Session` 並設為預設 |
| [`manifest-tokenizer.md`](manifest-tokenizer.md) | Jinen tokenizer 納入資料 manifest |
| [`ja-models-fetch.md`](ja-models-fetch.md) | 外部日文模型抓取、授權與 smoke test |
| [`zenz-eval.md`](zenz-eval.md) | Zenz v2.5 medium 的可行性、量化與 top-1／延遲對照 |
| [`punct-wire.md`](punct-wire.md) | 標點模型接進輸出（2026-11-20 已移除） |
| [`candle-punct.md`](candle-punct.md) | 日文標點模型的 candle 實作（已移除） |
| [`drop-punct.md`](drop-punct.md) | 移除標點模型推論路徑 |

### Session 與其他收尾

| 文件 | 一句話 |
|---|---|
| [`session-core-split.md`](session-core-split.md) | session 重構 Stage 0：拆檔與狀態邊界 |
| [`session-g1-input.md`](session-g1-input.md) | 輸入風格與鍵位群組 |
| [`session-g2-segments.md`](session-g2-segments.md) | 分段、候選呈現與片段欄位 |
| [`session-g3-symbols.md`](session-g3-symbols.md) | 符號與選單群組 |
| [`session-g4-mixed.md`](session-g4-mixed.md) | 混合與語言群組 |
| [`session-g5-misc.md`](session-g5-misc.md) | 待定字面、組字加詞與本地證據 |
| [`session-g6-core.md`](session-g6-core.md) | 主狀態機整合與候選選取模式 |
| [`session-backfill.md`](session-backfill.md) | 補回 G2 的五個 n/a 與協定欄位 |
| [`core-misc.md`](core-misc.md) | vocab、suggest、inputstyle 與 lm 收齊 |
| [`personal-improve.md`](personal-improve.md) | rollback、弱負向訊號與最短觸發長度 |
| [`defaults-policy.md`](defaults-policy.md) | 預設啟用學習、模糊音與注音選字模型 |
| [`english-pack.md`](english-pack.md) | 英文詞表建置與接線 |
| [`english-suggest.md`](english-suggest.md) | 英文拼字建議與前綴補完 |
| [`m1-mixed.md`](m1-mixed.md) | 混合路由與英文直通 |
| [`settings-core.md`](settings-core.md) | `jc-settings` 的設定與模型包資料模型 |
| [`engine-data.md`](engine-data.md) | 引擎的資料載入與模型安裝 |
| [`w1-data-ja.md`](w1-data-ja.md) | jadic pack 的 Go 讀取對照 |
| [`w1-eval.md`](w1-eval.md) | 評測 harness、指標與 gate |
| [`fix-tests.md`](fix-tests.md) | 全量測試修到綠 |
| [`fix-windows-cfg.md`](fix-windows-cfg.md) | Windows 編譯的 `cfg` 邊界 |
| [`ci-clippy.md`](ci-clippy.md) | 清掉 workspace 的 clippy 與 fmt 問題 |
| [`toolchain-pin.md`](toolchain-pin.md) | 固定 Rust 工具鏈版本與升級流程 |
| [`ci-lint.md`](ci-lint.md) | 修掉 `zh-symbols` 之後的 clippy 錯誤 |
| [`candidate-paging.md`](candidate-paging.md) | 候選分頁、數字選字與 `CANDIDATE_PAGE_SIZE` |
| [`candidate-unify.md`](candidate-unify.md) | 語言無關的候選資料契約、十列／四欄版面與 Enter 語意 |
| [`candidate-cap.md`](candidate-cap.md) | 候選上限 200 的單鍵延遲、JSON 大小與記憶體量測 |
| [`composition-cursor.md`](composition-cursor.md) | 組字游標、任意位置編輯與片段換字 |
| [`language-branches.md`](language-branches.md) | 依語言／模式分岔的行為盤點、統一建議與前三件事 |

### TSF、設定與安裝

| 文件 | 一句話 |
|---|---|
| [`tsf-skeleton.md`](tsf-skeleton.md) | TSF 文字服務骨架、COM 註冊、按鍵與非同步 IPC |
| [`tsf-candidates.md`](tsf-candidates.md) | `ITfCandidateListUIElement` 候選窗與分頁 |
| [`tsf-polish.md`](tsf-polish.md) | 組字虛線底線與候選窗滑鼠點選 |
| [`tsf-smoke.md`](tsf-smoke.md) | 免管理員的 TSF 註冊與載入冒煙測試 |
| [`activate-diagnose.md`](activate-diagnose.md) | `Activate` 回 `0x80070057` 的逐步日誌與 `HINSTANCE` 修正 |
| [`tsf-installer.md`](tsf-installer.md) | per-machine MSI、TIP 註冊與反安裝 |
| [`installer-payload.md`](installer-payload.md) | 授權文件與 Jinen tokenizer 進 MSI payload |
| [`candidate-click.md`](candidate-click.md) | 滑鼠點選改成只送一個數字鍵 |
| [`d2d-candidates.md`](d2d-candidates.md) | TSF DLL 內自繪候選窗：Direct2D＋DirectComposition，不新增程序 |
| [`pipe-name.md`](pipe-name.md) | pipe 名稱加入 SID、`JC_IME_PIPE` 與機碼覆寫 |
| [`os-isolation.md`](os-isolation.md) | 平台 cfg 邊界與 Windows 目標型別檢查 |
| [`settings-ui.md`](settings-ui.md) | GPUI Kit 設定程式與 `settings.json` |
| [`settings-wire.md`](settings-wire.md) | 引擎啟動讀取設定、CLI 參數優先、設定熱重載 |
| [`keybinds.md`](keybinds.md) | 符號／表情快捷鍵的 chord 語法、引擎解析與 TSF 轉發 |
| [`engine-supervisor.md`](engine-supervisor.md) | `--supervise` 單一實例、重啟與環狀 log |
| [`data-pack.md`](data-pack.md) | 資料 manifest、驗證與安裝包 payload |
| [`data-download.md`](data-download.md) | 首次啟動下載、`.part` 與 SHA-256 |
| [`legal-update.md`](legal-update.md) | 授權帳本與第三方聲明同步 |

### 工具與流程

| 文件 | 一句話 |
|---|---|
| [`testing.md`](testing.md) | 內圈與 gate 都用 nextest，doctest 另外跑，排程比對 cargo test |
| [`mbx.md`](mbx.md) | clippy／check 走 `scripts/check.sh`（mbx 共用快取），test 與 CI 不走 mbx |

### 任務書索引對照

這一輪任務書列的 `x86-tip`、`release-flow`、`settings-neural`、`live-default`
沒有獨立對照檔：

- `release-flow` 的流程與檢查清單在 [`docs/release.md`](../release.md)。
- `settings-neural` 併在 [`settings-wire.md`](settings-wire.md) 與
  [`settings-core.md`](settings-core.md)。
- `live-default` 併在 [`ja-live-neural.md`](ja-live-neural.md)。
- `x86-tip` 不在這個 worktree；目前只建 x64 TSF，等合併後再補對照表。
