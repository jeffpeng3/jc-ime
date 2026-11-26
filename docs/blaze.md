# blaze

這個 workspace 的日常檢查用 [blaze](https://github.com/…/blaze)，交件前的正式驗證用
`cargo`。兩者讀同一份 `Cargo.toml`，不需要額外設定。

## 指令

| 時機 | 指令 |
|---|---|
| 開發中反覆檢查 | `blaze check -p <crate> --all-targets` |
| 交件前必跑 | `cargo check -p <crate> --all-targets` |
| 格式化 | `cargo fmt -p <crate>` |
| 測試 | 內圈用 `scripts/test-plan.sh --run`（nextest＋doctest），交件 gate 用 `cargo nextest run --workspace` 加 `cargo test --workspace --doc` |

`blaze check` 可以一直跑，很快；但它只是加速器，不能取代交件前的
`cargo check`。兩者結果不一致時以 `cargo check` 為準。

## 禁止事項

- 不要 `blaze enable`：這個 workspace 不需要、也不應該把 blaze 設成
  cargo 的 wrapper，否則其他工具與 CI 會拿到非預期的行為。
- 不要用 `blaze test` 驗證個別 task；內圈用 `scripts/test-plan.sh --run`，
  最終 gate 由 checker 跑 nextest＋doctest。

## 效能参考

- 冷啟一個 workspace 約 5 秒。
- 熱檢查約 0.02 到 2 秒（視 crate 與是否改動上游而定）。

## 環境

這個環境的 blaze 已可正常建立 sandbox，**不需要** `BLAZE_SANDBOX=off`。
若遇到 sandbox 建立失敗，先回報，不要自行關掉 sandbox 繞過。
