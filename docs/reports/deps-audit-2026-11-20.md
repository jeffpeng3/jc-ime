# deps-audit：依賴樹稽核

日期：2026-11-20　機器：GB10（aarch64，20 核）　toolchain：1.100.0
分支：`deps-audit`（base `main`）

## 直接答案

**直接宣告的依賴 101 條裡，真正沒用到的有 2 條，佔 2.0%。**

| 依賴 | 宣告位置 | 處置 |
|---|---|---|
| `log` | `crates/jc-settings-ui/Cargo.toml` | 已移除 |
| `sha2` | `crates/jc-englishpack/Cargo.toml` | 已移除 |

另外有 1 個沒用到的 feature 已移除：`jc-tsf-smoke` 的 `Win32_UI_Input_KeyboardAndMouse`。
這 3 項都不會讓建置圖少任何一個套件，因為 `log` 與 `sha2` 本來就被其他相依拉進來了。省下的是 manifest 正確性，編譯時間與二進位大小都是 0。

**建置圖層級「對特定產物不必要」的量比較大，但需要重構，不能直接刪：**

| 產物 | 目前套件數 | 只因為 `jc-core` 才被拉進來的套件 | 佔比 |
|---|---:|---:|---:|
| `jc-settings-ui` | 633 | 62（candle／tokenizers 堆疊） | 9.8% |
| `jc-proto`（連帶 `jc-windows`／`jc-linux`／`jc-mac`） | 165 | 147 | 89.1% |
| `jc-englishpack` | 165 | 135 | 81.8% |

原因：`jc-settings` 只用 `jc_core::{zh::fuzzy, inputstyle}`、`jc-proto` 只用 `jc_core::{Key, KeyCode, Mode, Snapshot, Candidate}`、`jc-englishpack` 只用 `jc_core::mixed` 的資料型別與 `sha256_hex`；但 `jc-core` 同時依賴 `jc-model`，所以 candle 全餐跟著進來。把這些型別抽到一個不依賴模型的 crate，就能一次解掉三處。

**重複版本 41 組、46 個多餘副本，全部來自第三方彼此卡版本，沒有我們能直接合併的。**

## 1. 總覽

| 指標 | 數值 | 指令 |
|---|---:|---|
| workspace 成員 | 18 | `Cargo.toml` |
| 直接依賴宣告（含 dev、target 專屬） | 101 → 99 | 解析各 `Cargo.toml` |
| host 目標唯一套件數 | 655（637 第三方） | `cargo metadata --filter-platform` |
| `cargo tree --workspace` 行數（Linux） | 1872 | `cargo tree --workspace \| wc -l` |
| `cargo tree --workspace` 行數（Windows） | 1458 | `cargo tree --workspace --target x86_64-pc-windows-msvc \| wc -l` |
| 重複名稱 | 41 | `cargo tree -d`（metadata 去重） |
| 多餘版本副本 | 46 | 同上 |
| 已移除的直接依賴 | 2 | 本報告 |
| 已移除的 feature | 1 | 本報告 |
| 需重構才能移除的套件（settings-ui） | 62 | 見第 7 節 |

各 crate 的相依子樹大小（含 dev-dependencies，host 目標）：

| crate | 套件數 | crate | 套件數 |
|---|---:|---|---:|
| `jc-settings-ui` | 633 | `jc-cli` | 202 |
| `jc-engine` | 178 | `jc-core` | 164 |
| `jc-windows`／`jc-linux`／`jc-mac` | 166 | `jc-proto` | 165 |
| `jc-settings` | 165 | `jc-englishpack` | 165 |
| `jc-model` | 153 | `jc-data` | 33 |
| `jc-jadic` | 29 | `jc-eval` | 19 |
| `jc-platform` | 18 | `jc-testdata` | 1 |
| `jc-tsf`／`jc-tsf-smoke`（Linux） | 1 | | |

## 2. 未使用依賴清單

工具：`cargo-machete 0.10.1`（`cargo install cargo-machete --locked`），再用 `use x`／`x::` 人工複核。

| crate | 依賴 | 證據 | 判定 |
|---|---|---|---|
| `jc-settings-ui` | `log` | machete 檢出；`grep -rn "log::\|use log"` 無結果 | 確定沒用，已移除 |
| `jc-englishpack` | `sha2` | machete 檢出；只用 `jc_core::mixed::sha256_hex`，沒有 `sha2::` 或 `Sha256` | 確定沒用，已移除 |
| `scripts/legacy/dump-inputs` | `serde` | machete 檢出 | 不在 workspace，也不在 owned paths，只記錄 |
| `jc-tsf` | `windows-core` | 原始碼沒有 `windows_core::` 路徑，但 `#[implement]` 展開後直接引用 `windows_core` | 不能移除，已加註解 |
| `jc-tsf-smoke` | `Win32_UI_Input_KeyboardAndMouse` | 全 crate 沒有 `Win32::UI::Input` 使用 | feature 沒用，已移除 |

分類：

- 確定沒用：`jc-settings-ui/log`、`jc-englishpack/sha2`（已移除）。
- 只在特定平台用：`jc-engine` 的 `windows-sys`、`libc`；`jc-windows`、`jc-tsf`、`jc-tsf-smoke` 的全部依賴都掛在 `cfg(windows)`。這些不算沒用到。
- 只在測試用：`jc-testdata` 在 `jc-core`、`jc-data`、`jc-engine`、`jc-model`、`jc-eval`、`jc-jadic` 都是 dev-dependency，宣告正確。`jc-cli` 的 `jc-testdata` 是正常依賴，因為 `src/bin/ja-eval.rs` 執行期會解析題庫路徑，不能降級成 dev-dependency。

## 3. 重複版本清單

`cargo tree -d`：41 個名稱、46 個多餘副本。全部的反向依賴都是第三方 crate，我們的 manifest 沒有任何一條能直接合併。

| crate | 版本 | 來源 | 建議 |
|---|---|---|---|
| `base64` | 0.13.1／0.22.1／0.23.1 | `spm_precompiled`／`usvg`／`ureq`、`serde-saphyr` | 上游問題，不動 |
| `bit-set`、`bit-vec` | 0.8／0.10 | `fancy-regex 0.17`／`0.19`、`naga`、`wgpu` | 上游問題，不動 |
| `bitflags` | 1.3.2／2.13.2 | `fluent-uri`、`globwalk`、`png 0.17`／gpui、wgpu、zbus | 上游問題，不動 |
| `block-buffer`、`crypto-common`、`digest`、`cpufeatures` | 0.10／0.12 系列 | `sha2 0.10` 與 `sha2 0.11`（`rust-embed-utils`） | 我們全用 `sha2 0.10`，升 0.11 也去不掉 0.10（`oo7` 走 `digest 0.10`），不動 |
| `dirs`／`dirs-sys` | 5.0.1／6.0.0 | `zed-font-kit`／`shellexpand` | 上游問題，不動 |
| `fancy-regex` | 0.17.0／0.19.2 | `tokenizers`／`candle-transformers` | 上游問題，不動 |
| `font-types`、`read-fonts`、`skrifa` | 0.11／0.12、0.37／0.41、0.40／0.44 | `cosmic-text`／`swash`／`harfrust` | 上游問題，不動 |
| `getrandom` | 0.2／0.3／0.4 | `ring`、`ahash`、`ashpd`、`tempfile` | 上游問題，不動 |
| `hashbrown` | 0.14.5／0.16.1／0.17.1 | `zed-xim`、`accesskit`／`wgpu` | 上游問題，不動 |
| `itertools` | 0.11／0.13／0.14 | `rust-i18n-support`、gpui、`tokenizers` | 上游問題，不動 |
| `miniz_oxide` | 0.8.9／0.9.1 | `png`／`flate2` | 上游問題，不動 |
| `nom` | 7.1.3／8.0.0 | `spm_precompiled`／`av1-grain` | 上游問題，不動 |
| `pastey` | 0.1／0.2 | `av-scenechange`／`macro_rules_attribute` | 上游問題，不動 |
| `phf`、`phf_generator`、`phf_shared` | 0.11／0.13 | `markup5ever`、`accesskit` | 上游問題，不動 |
| `png` | 0.17.16／0.18.1 | `tiny-skia`／`image` | 上游問題，不動 |
| `rand`、`rand_chacha`、`rand_core` | 0.8／0.9 | candle 與 gpui 各用一半 | 上游問題，不動 |
| `roxmltree` | 0.20／0.21 | `fontconfig-parser`／`usvg` | 上游問題，不動 |
| `rustc-hash` | 1.1／2.1 | `naga`／`cosmic-text` | 上游問題，不動 |
| `serde_spanned`、`toml`、`toml_datetime`、`toml_edit`、`winnow` | 0.6／1.x、0.8／1.2 | `rust-i18n-support`、`proc-macro-crate`、`embed-resource` | 建置期相依，不動 |
| `sha2` | 0.10.9／0.11.1 | 我們的 crate／`rust-embed-utils` | 見上，不動 |
| `spin` | 0.9.9／0.10.1 | `flume`／`gpui-pre` | 上游問題，不動 |
| `syn` | 2.0.119／3.0.10 | 大量 proc-macro | 上游問題，不動 |
| `thiserror` | 1.0.69／2.0.21 | `filedescriptor`／其餘全部 | 上游問題，不動 |
| `zbus`、`zbus_macros` | 5.19／6.4 | `accesskit`、`ashpd`／`notify-rust` | 上游問題，不動 |

用 `cargo tree -d -i sha2@0.11.1`、`-i thiserror@1.0.69` 反查，這些版本只被單一上游 crate 使用。合併需要等上游改版，現在動我們自己的版本只會增加衝突。

## 4. Feature 清單

| 位置 | 現況 | 可關？ | 影響與風險 |
|---|---|---|---|
| `candle-core`／`candle-nn`／`candle-transformers` | `default-features = false` | 已最小 | 沒有 `cuda`／`metal`／`mkl`／`accelerate`，維持現狀 |
| `tokenizers` | `default-features = false` | 已最小 | 沒有 `onig`、`progressbar`、`esaxx_fast`；BPE 仍可用 |
| `gpui-kit` | 預設（`component`、`assets`） | 沒有肥肉 | `tree-sitter`、`inspector`、`webview`、`speech`、`syntect` 都沒開；`assets` 可關但 `gpui-component` 自己就依賴 `gpui-kit-assets`，關了省 0 個套件 |
| `ureq` | `default-features = false` + `rustls`、`gzip` | 已最小 | 沒有原生 TLS |
| `windows-sys`（`jc-engine`） | 10 個 feature | 1 個已驗證不可關 | `Win32_Graphics_Gdi` 看似沒用，但測試的 `WNDCLASSW` 掛在它底下，關掉 Windows check 失敗 |
| `windows`（`jc-tsf`） | 12 個 feature | 0 個可關 | `Win32_System_Ole` 是 `VARIANT` 的 `cfg(all(Com, Ole))` 條件；`Win32_Security` 擋著 `RegCreateKeyExW` 與 `VARIANT`，兩者關掉都會編不過 |
| `windows`（`jc-tsf-smoke`） | 6 → 5 個 feature | 1 個可關 | `Win32_UI_Input_KeyboardAndMouse` 完全沒用到，已移除 |
| `windows-sys`（`jc-windows`） | 7 個 feature | 0 個可關 | 全部有實際 API 使用 |

Windows feature 的驗證方式：`cargo check -p jc-tsf -p jc-tsf-smoke -p jc-engine --target x86_64-pc-windows-msvc`。逐一刪除再編譯，失敗的就還原並在 manifest 註明原因。

## 5. 前 10 大依賴

以 host 目標的子樹套件數排序：

| 排名 | 依賴 | 子樹套件數 | 為什麼需要 | 更輕的替代 | 移除風險 |
|---:|---|---:|---|---|---|
| 1 | `gpui-kit` 0.10 | 559 | `jc-settings-ui` 的唯一 UI 框架，原始碼直接用 `gpui_kit::application`、`open_window` | 換 framework 等於重寫整個設定程式 | 高，維持 |
| 2 | `candle-transformers` 0.11 | 145 | Jinen v2（Qwen3 架構 GGUF）forward | 自己寫 forward，工作量大 | 高 |
| 3 | `candle-nn` 0.11 | 138 | LayerNorm、Linear、RotaryEmbedding 等層 | 無 | 高 |
| 4 | `candle-core` 0.11 | 137 | Tensor 與 CPU backend | 換 tract／ort，會回到已移除的 ORT 路線 | 高 |
| 5 | `tokenizers` 0.23 | 83 | 讀 Jinen 的 `tokenizer.json`（BPE＋NFKC＋byte_fallback） | 自寫 BPE 可省 83 個套件，但要完全對齊 tokenizer 輸出 | 中，見第 7 節 |
| 6 | `ureq` 3.5 | 38 | `jc-cli` 下載模型與資料 | `reqwest` 更重；`curl` 要外部程序 | 低，維持 |
| 7 | `env_logger` 0.11 | 18 | `jc-engine` 與 `jc-settings-ui` 的 `RUST_LOG` 初始化 | 無標準替代 | 低 |
| 8 | `sha2` 0.10 | 11 | 資料包與 manifest 的 SHA-256 校驗 | 規格就是 SHA-256，不能換 | 低 |
| 9 | `prost` 0.14 | 10 | 解析 SentencePiece `.model` 的 protobuf | 手寫 protobuf 解析可省約 10 個套件 | 低到中 |
| 10 | `serde_json` 1.0 | 9 | 協定、manifest、設定檔 | 無 | 低 |

`jc-settings-ui` 只能靠 GPUI：整支程式建立在 `gpui_kit` 的 API 上，沒有抽象層。它的定位是「必要但可選」的獨立 binary，不應放進引擎或 wrapper 的建置路徑。

## 6. 二進位大小

`cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui`：609 個 crate，71 秒（冷啟動，含先前 check 的共用 artifact）。
`jc-tsf-smoke` 在 Linux 只是印訊息後結束的 stub，不具代表性。

| binary | 預設 release | `strip` 後 | 變化 | `opt-level = "s"` | 變化 |
|---|---:|---:|---:|---:|---:|
| `jc-engine` | 13,898,768（13.3 MiB） | 10,033,624（9.6 MiB） | −27.8% | 14,331,712（13.7 MiB） | +3.1% |
| `jc-cli` | 17,050,648（16.3 MiB） | 12,393,024（11.8 MiB） | −27.3% | 17,541,032（16.7 MiB） | +2.9% |
| `jc-settings-ui` | 58,056,472（55.4 MiB） | 39,269,544（37.5 MiB） | −32.4% | 56,095,272（53.5 MiB） | −3.4% |
| `jc-tsf-smoke`（Linux stub） | 466,744 | 未量 |  | 未量 |  |

`opt-level = "s"` 用 `CARGO_PROFILE_RELEASE_OPT_LEVEL=s` 搭配獨立 target dir 量測，沒有改專案設定。結果對引擎與 CLI 反而變大，只有 settings-ui 小 2.0 MB，不值得全域套用。

`jc-settings-ui` 的 55.4 MiB 幾乎都來自 GPUI／wgpu。`nm -C target/release/jc-settings-ui | grep -c candle` 為 0，candle 只有編譯成本，沒有進到這支 binary。

編譯成本量測：只依賴 candle 三件套＋`tokenizers`＋`prost`＋`rayon` 的空殼 crate，冷啟動 release 編譯 142 個套件要 27.7 秒（GB10，`-C target-feature=+fp16`）。這是把 candle 從不需要它的產物移掉時，上限可以期待的節省。

## 7. 建議行動

### 低風險，可直接做（已完成）

1. 移除 `jc-settings-ui` 的 `log`。
2. 移除 `jc-englishpack` 的 `sha2`。
3. 移除 `jc-tsf-smoke` 的 `Win32_UI_Input_KeyboardAndMouse`。
4. 補註解說明 `jc-engine` 的 `Win32_Graphics_Gdi`、`jc-tsf` 的 `Win32_System_Ole`／`Win32_Security`／`windows-core` 為什麼看起來沒用卻不能刪。

驗證結果：

- `cargo check --workspace --all-targets`：通過。
- `cargo check -p jc-tsf -p jc-tsf-smoke -p jc-engine --target x86_64-pc-windows-msvc`：通過。
- `cargo machete`：workspace 內只剩 `scripts/legacy/dump-inputs` 的 `serde`（不在 workspace）。

### 需要驗證（只提建議）

1. **抽出不依賴模型的共用型別 crate**（`jc-types`）。
   把 `Key`、`KeyCode`、`Mode`、`Snapshot`、`Candidate`、`zh::fuzzy`、`inputstyle::Rules`、`mixed` 的資料型別與 `sha256_hex` 從 `jc-core` 移到新 crate。
   效果：`jc-settings-ui` −62 個套件（−9.8%）、`jc-proto` 與三個 wrapper −147 個套件（−89%）、`jc-englishpack` −135 個套件（−82%）。
   風險：動到 `jc-core` 的公開 API 與 `pub use` 路徑，要同步 `jc-proto`、`jc-settings`、`jc-englishpack`、`jc-cli` 的 import。
   預估節省：settings-ui 冷編譯約 15 到 25 秒；wrapper 與 englishpack 的 CI 檢查約 10 到 20 秒。二進位大小不變，因為 linker 本來就會丟掉沒用到的 candle 符號。

2. **評估 `tokenizers` 是否值得自寫 BPE**。
   可省 83 個套件（`jc-model` 從 153 降到約 70），但要做 NFKC、Digits、byte_fallback 的逐位元對齊測試，風險比省下的成本高。建議先不做。

3. **`prost` 換手寫 protobuf**。
   只省約 10 個套件，SentencePiece 的欄位很少。適合當低優先度的清理。

### 不建議動

- 合併重複版本：41 組全部卡在第三方，強行升級只會增加衝突。
- `opt-level = "s"`：引擎與 CLI 變大。
- 關閉 `gpui-kit` 的 `assets`：`gpui-component` 自己就依賴 `gpui-kit-assets`，省不到套件。
- 移除 `jc-settings-ui` 的 GPUI：等於重寫整個 UI，且這支 binary 本來就不在建置熱路徑上。

## 附錄：重現指令

```sh
cargo machete
cargo tree -d
cargo tree --workspace | wc -l
cargo tree --workspace --target x86_64-pc-windows-msvc | wc -l
cargo metadata --format-version 1 --filter-platform "$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui
strip target/release/jc-engine   # 量測用副本，不改原檔
CARGO_TARGET_DIR=/tmp/opencode/target-s CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
  cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui
```
