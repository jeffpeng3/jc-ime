#!/usr/bin/env bash
# cache-prune.sh 的 fixture 測試：固定 PRUNE_NOW，用 CACHE_JSON 跑 dry-run 並檢查輸出。
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=$(mktemp)
SEC=$(mktemp)
trap 'rm -f "$OUT" "$SEC"' EXIT
fail=0

expect() {
  local desc=$1 text=$2 file=${3:-$OUT}
  if grep -Fq -- "$text" "$file"; then
    printf 'ok   %s\n' "$desc"
  else
    printf 'FAIL %s（找不到：%s）\n' "$desc" "$text"
    fail=1
  fi
}

reject() {
  local desc=$1 text=$2 file=${3:-$OUT}
  if grep -Fq -- "$text" "$file"; then
    printf 'FAIL %s（不該出現：%s）\n' "$desc" "$text"
    fail=1
  else
    printf 'ok   %s\n' "$desc"
  fi
}

delete_section() {
  awk '/^== 要刪除/{p=1;next} /^== 保留/{p=0} p' "$OUT" >"$SEC"
}

keep_section() {
  awk '/^== 保留/{p=1;next} /^== 總結/{p=0} p' "$OUT" >"$SEC"
}

echo '== 案例一：tag 快取全在 6 小時內，總量超標時刪掉最舊的四個 =='
PRUNE_NOW=2026-11-20T06:00:00Z \
  KEEP_PER_FAMILY=1 \
  CACHE_JSON=scripts/fixtures/cache-prune-total.json \
  bash scripts/cache-prune.sh --dry-run --max-total-gb 7 >"$OUT"
expect '待刪除 4 筆' '待刪除：4 筆'
expect '壓力清理追加 4 筆' '壓力清理：追加 4 筆'
expect '執行後預估 6.39 GB' '執行後預估：6.39 GB'
delete_section
expect '刪 rc.4 windows' 'v0-rust-release-windows-2a3b4c5d5e6f8093' "$SEC"
expect '刪 rc.4 linux' 'v0-rust-release-linux-2b3c4d5e6f708194' "$SEC"
expect '刪 rc.5 windows' 'v0-rust-release-windows-1a2b3c4d5e6f7082' "$SEC"
expect '刪 rc.5 linux' 'v0-rust-release-linux-1b2c3d4e5f607183' "$SEC"
keep_section
expect '留 rc.7 windows' 'v0-rust-release-windows-0a1b2c3d4e5f6071' "$SEC"
expect '留 rc.7 linux' 'v0-rust-release-linux-0b1c2d3e4f506172' "$SEC"

echo '== 案例二：預設 8 GB 時只刪到低於門檻就停 =='
PRUNE_NOW=2026-11-20T06:00:00Z \
  KEEP_PER_FAMILY=1 \
  CACHE_JSON=scripts/fixtures/cache-prune-total.json \
  bash scripts/cache-prune.sh --dry-run >"$OUT"
expect '待刪除 2 筆' '待刪除：2 筆'
expect '執行後預估 7.99 GB' '執行後預估：7.99 GB'
delete_section
expect '仍刪最舊的 rc.4 windows' 'v0-rust-release-windows-2a3b4c5d5e6f8093' "$SEC"
reject '較新的 rc.5 不留在本輪刪除' 'v0-rust-release-windows-1a2b3c4d5e6f7082' "$SEC"

echo '== 案例三：總量未超標，不額外刪 =='
PRUNE_NOW=2026-11-20T00:00:00Z \
  CACHE_JSON=scripts/fixtures/cache-prune.json \
  bash scripts/cache-prune.sh --dry-run >"$OUT"
expect '原規則仍刪 2 筆' '待刪除：2 筆'
expect '壓力清理顯示無' '壓力清理：無（'
expect '執行後預估 1.98 GB' '執行後預估：1.98 GB'

echo '== 案例四：全部落在保護窗，不刪且註明原因 =='
PRUNE_NOW=2026-11-20T06:00:00Z \
  CACHE_JSON=scripts/fixtures/cache-prune-protected.json \
  bash scripts/cache-prune.sh --dry-run >"$OUT"
expect '待刪除 0 筆' '待刪除：0 筆'
expect '全部受保護' '壓力清理：無可追加：全部受保護（建立未滿 1 小時）'
expect '執行後預估 9.00 GB' '執行後預估：9.00 GB'

echo '== 案例五：--max-total-gb 0 關閉總量規則 =='
PRUNE_NOW=2026-11-20T06:00:00Z \
  KEEP_PER_FAMILY=1 \
  CACHE_JSON=scripts/fixtures/cache-prune-total.json \
  bash scripts/cache-prune.sh --dry-run --max-total-gb 0 >"$OUT"
expect '待刪除 0 筆' '待刪除：0 筆'
expect '壓力清理已關閉' '壓力清理：已關閉（MAX_TOTAL_GB=0）'
expect '執行後預估 9.59 GB' '執行後預估：9.59 GB'

if ((fail)); then
  echo "失敗：$fail 項" >&2
  exit 1
fi
echo '全部通過'
