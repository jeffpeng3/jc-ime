# Port backlog

## 前導字元 u／b

微軟注音的反引號前導除了全形標點，還有兩個碼位查詢：

```
` u 4e00 空白   → 一（Unicode 碼位）
` b a440 空白   → 一（Big5 碼位）
```

- 來源：微軟注音輸入法的前導字元表。
- 目前狀態：**已完成（P1.5）**。`engine/prefix.rs` 的 `PrefixEngine` 是控制
  引擎：反引號或前導序列進行中才 `Claim`，`u`／`b` 收集碼位，空白解析後插入
  一個字；錯誤與取消保留 raw 或整段取消，Big5 走 `jc-core` 既有的
  `big5.bin` 跨平台查表。設計與狀態機見
  [`language-engines.md`](language-engines.md) §7。
- 覆蓋測試：
  - `test_prefix_u_inserts_unicode_scalar`
  - `test_prefix_b_inserts_big5_char`
  - `test_prefix_invalid_code_keeps_raw_literal`
  - `test_prefix_code_escape_cancels`
  - `test_prefix_code_backspace_edits_digits`
