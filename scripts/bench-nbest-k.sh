#!/usr/bin/env bash
# 量 nbest 候選上限（max_candidates）對單鍵延遲的影響。
#
# 用法：
#   scripts/bench-nbest-k.sh                 # 資料預設 /mnt/work/jc/jc-ime-data
#   JC_TESTDATA=/path/to/data scripts/bench-nbest-k.sh
#
# 腳本把 scripts/nbest_k_cap.rs 暫時放進 jc-engine 的 tests/，跑 release 量測，
# 結束後刪掉；repo 不會多出測試檔。輸出每列是 `RESULT <json>`，報告的表格
# 由這些數字整理而來。
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
test_file="$repo/crates/jc-engine/tests/nbest_k_cap.rs"

if [ -e "$test_file" ]; then
    echo "拒絕覆寫既有的 $test_file；請先確認是不是上一次中斷留下的。" >&2
    exit 1
fi

cleanup() {
    rm -f "$test_file"
}
trap cleanup EXIT

cp "$repo/scripts/nbest_k_cap.rs" "$test_file"

if command -v mise >/dev/null 2>&1; then
    cd "$repo"
    mise exec -- cargo test --release -p jc-engine --test nbest_k_cap -- --nocapture
else
    cd "$repo"
    cargo test --release -p jc-engine --test nbest_k_cap -- --nocapture
fi
