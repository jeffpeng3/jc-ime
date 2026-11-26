#!/usr/bin/env bash
# clippy 的統一入口：有 mbx 就用 mbx（共用編譯快取），沒有就退回 cargo。
# 只影響 check/clippy；test 走 scripts/test-plan.sh（nextest），CI 有 CI 的做法。
#
# 參數原樣傳給 clippy，例如：
#   scripts/check.sh --workspace --all-targets -- -D warnings
#
# 環境變數：
#   JC_NO_MBX=1  強制走 cargo（CI、除錯、懷疑快取污染時用）
set -euo pipefail

if [[ ${JC_NO_MBX:-0} == 1 ]]; then
  echo '使用 cargo（JC_NO_MBX=1）'
  exec cargo clippy "$@"
fi

if ! command -v mbx >/dev/null 2>&1; then
  echo '使用 cargo（找不到 mbx）'
  exec cargo clippy "$@"
fi

echo '使用 mbx（共用快取）'
exec mbx clippy "$@"
