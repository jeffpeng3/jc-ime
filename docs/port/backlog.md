# Port backlog

## 前導字元 u／b

微軟注音的反引號前導除了全形標點，還有兩個碼位查詢：

```
` u 4e00 空白   → 一（Unicode 碼位）
` b a440 空白   → 一（Big5 碼位）
```

- 來源：微軟注音輸入法的前導字元表。
- 目前狀態：**這一版不做**。`u`／`b` 列在
  `crates/jc-core/src/session/input_style.rs` 的 `PREFIX_RESERVED` 表，附
  `TODO(prefix-u)`／`TODO(prefix-b)`。實際行為和一般非表內鍵相同：反引號以
  字面送出，`u`／`b` 照常輸入，不會卡住。
- 鎖住目前行為的測試：
  - `prefix_u_is_reserved_and_falls_through_for_now`
  - `prefix_b_is_reserved_and_falls_through_for_now`
- 未來實作要動的地方：只有 `jc-core` 的前導解析
  （`input_style.rs` 的 `prefix_key` 與保留鍵表）。`proto` 與外殼不用動。
  實作後請刻意改寫上面兩個測試，不必當成回歸。
