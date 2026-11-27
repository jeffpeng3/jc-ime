# 參考文件索引

`docs/port/` 現在只放仍有效的設計與行為參考，不再保留逐 task 的 Go → Rust 對照
表。遷移過程與一次性調查留在 git 歷史，需要回溯時用 `git log -- <path>`。

## 核心行為

| 文件 | 內容 |
|---|---|
| [`language-engines.md`](language-engines.md) | 語言引擎介面、lane、前導 `u`／`b` 解析 |
| [`event-flows.md`](event-flows.md) | TSF／engine／core 的實際事件路徑與決策歸屬 |
| [`composition.md`](composition.md) | 組字模型、所有權、游標、片段編輯與 Mixed 規則 |
| [`candidate-unify.md`](candidate-unify.md) | 語言無關的候選清單、版面、分頁、上限與 Enter |
| [`keybinds.md`](keybinds.md) | 符號／表情快捷鍵的 chord 語法與熱重載 |
| [`personal.md`](personal.md) | 個人化學習的資料格式、規則與程式位置 |
| [`big5-table.md`](big5-table.md) | 前導 `b` 的 Big5 → Unicode 查表 |

## 語言與模型

| 文件 | 內容 |
|---|---|
| [`neural.md`](neural.md) | Jinen 門檻、驗證、逐字変換、執行緒與量測 |
| [`data.md`](data.md) | 資料根目錄、manifest、`data verify` 與下載 |
| [`crate-layout.md`](crate-layout.md) | crate 職責、相依方向與重組建議 |

## 設定與介面

| 文件 | 內容 |
|---|---|
| [`settings.md`](settings.md) | `settings.json`、引擎熱重載、設定程式與欄位 |
| [`tsf-ui.md`](tsf-ui.md) | TSF 候選窗、Direct2D 繪製與組字顯示 |
| [`tsf-smoke.md`](tsf-smoke.md) | 安裝診斷工具（doctor）的 13 項檢查與 JSON |

## 平台與安裝

| 文件 | 內容 |
|---|---|
| [`engine.md`](engine.md) | 引擎監督、按需啟動、pipe 與 argv |
| [`jc-platform.md`](jc-platform.md) | `InputHost`、`HostEvent`、`Clock`、`Scheduler` 契約 |
| [`x86-tip.md`](x86-tip.md) | 32／64 位元 TSF DLL 與 WOW64 註冊 |
| [`os-isolation.md`](os-isolation.md) | 平台 `cfg` 邊界與 Windows 目標型別檢查 |

## 工具與流程

| 文件 | 內容 |
|---|---|
| [`testing.md`](testing.md) | nextest、doctest、內圈 `test-plan.sh` 與 gate |
| [`mbx.md`](mbx.md) | clippy／check 的共用編譯快取 |
| [`toolchain-pin.md`](toolchain-pin.md) | Rust 工具鏈固定與升級流程 |

## 其他文件

使用者面向的安裝、模型、授權、發佈與協定在上一層：

- [`../install.md`](../install.md)、[`../models.md`](../models.md)、
  [`../release.md`](../release.md)
- [`../architecture.md`](../architecture.md)、[`../proto.md`](../proto.md)、
  [`../wrapper.md`](../wrapper.md)
- [`../status.md`](../status.md)、[`../CHANGELOG.md`](../CHANGELOG.md)
