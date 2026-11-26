#!/usr/bin/env bash
# 內圈測試：nextest 跑單元＋整合，cargo test --doc 補 doctest。
#
# 這是內圈回饋迴路，不是 gate。checker 與 Linux CI 跑
# `cargo nextest run --workspace` 加 `cargo test --workspace --doc`，
# 那才是判準。
#
# 參數會原樣傳給 `cargo nextest run`，所以可以只跑單一 crate 或模組：
#   scripts/test.sh -p jc-core session::mixed
#
# nextest 不跑 doctest，最後一步補上。
set -euo pipefail

cargo nextest run "$@"
cargo test --workspace --doc
