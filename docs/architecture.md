# 核心與 wrapper 的邊界

## 形狀

```text
wrapper（各平台獨立）            jc-engine（核心，跨平台）
  TSF DLL ─┐                      ┌─ jc-core
  候選窗   ├── IPC（長度前綴 JSON）──┤  jc-lang-*
  設定程式 ┘                      ├─ jc-data
                                  ├─ jc-model
                                  └─ jc-eval（僅開發）
```

## 責任

| 元件 | 負責 | 不負責 |
|---|---|---|
| `jc-engine` | 組字狀態、候選、轉換、學習、模型推論 | 任何視窗、任何 OS API |
| TSF wrapper | 按鍵轉發、組字顯示、候選窗、送出結果 | 轉換邏輯、詞典 |
| 設定程式 | 偏好設定、模型狀態顯示 | 執行期轉換 |
| 更新器 | 版本下載、安裝、排程 | 輸入邏輯 |

## 為什麼分離

1. 核心可在 DGX 上以 Linux 開發與評測，不必每次回到 Windows。
2. TSF DLL 只做轉發，崩潰不會把核心一起帶走，也不會讓宿主程式（explorer）承擔核心風險。
3. 之後的 Fcitx5／macOS 只需要新的 wrapper。

## 協定

- 傳輸：Windows named pipe、Unix domain socket，開發期可用 stdio。
- 編碼：4-byte 小端長度前綴 + JSON（serde）。
- 版本：`jc_proto::PROTOCOL_VERSION`，握手不一致即拒絕。
- 語意：請求／回應皆為冪等可重放的快照式設計；取消由 wrapper 端以「丟棄過期回應」實現。

## 平台抽象

`jc-platform::InputHost` 由 wrapper 實作，核心只呼叫：

```rust
fn update_preedit(&mut self, text: &str, cursor_utf16: usize);
fn show_candidates(&mut self, items: &[String], highlight: usize);
fn commit(&mut self, text: &str);
```

核心不持有 OS handle，也不做 UTF-16 以外的假設。
