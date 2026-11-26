#!/usr/bin/env bash
# clippy 的統一入口：有 mbx 就用 mbx（共用編譯快取），沒有就退回 cargo。
# test 走 scripts/test.sh／test-plan.sh（nextest）；CI 有 CI 的做法。
#
# 參數原樣傳給 clippy，例如：
#   scripts/check.sh --workspace --all-targets -- -D warnings
#
# 環境變數：
#   JC_NO_MBX=1  強制走真正的 cargo（CI、除錯、懷疑快取污染時用）。cargo 本身
#                是 mbx shim 時 MBX_DISABLE 無效，所以直接呼叫 rustup 的
#                cargo 絕對路徑（$CARGO 若已設定則優先）。
set -euo pipefail

if [[ ${JC_NO_MBX:-0} == 1 ]]; then
  real=${CARGO:-$HOME/.cargo/bin/cargo}
  if [[ -x $real ]]; then
    echo "使用 cargo（JC_NO_MBX=1，走真正的 cargo：${real}）"
    exec "$real" clippy "$@"
  fi
  fallback=$(command -v cargo || true)
  echo "使用 cargo（JC_NO_MBX=1，${real} 不可用；退回 ${fallback:-cargo}，可能仍是 mbx shim）" >&2
  exec "${fallback:-cargo}" clippy "$@"
fi

if ! command -v mbx >/dev/null 2>&1; then
  echo '使用 cargo（找不到 mbx）'
  exec cargo clippy "$@"
fi

echo '使用 mbx（共用快取）'
exec mbx clippy "$@"
