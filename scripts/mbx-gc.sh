#!/usr/bin/env bash
# 回收 mbx 的磁碟空間。預設只跑 dry-run，加 --yes 才真的清。
#
# 注意：cargo clean 不會釋放 mbx 的空間。mbx 的 clippy 產物在
# target/check/<triple>/debug（host 是 target/check/debug；cargo 是
# target/<triple>/debug），實際資料在 mbx 的共用 store 裡；要回收請用這支
# 腳本或直接跑 mbx gc。
#
# 用法：
#   scripts/mbx-gc.sh                 # 只印 dry-run 摘要
#   scripts/mbx-gc.sh --yes           # 真的清除
#   scripts/mbx-gc.sh --yes --max-size 20GiB   # 其他參數原樣傳給 mbx gc
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/{/^set -euo/d;p}' "$0"
}

confirm=0
args=()

while (($# > 0)); do
  case "$1" in
    --yes|-y)
      confirm=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      args+=("$@")
      break
      ;;
    *)
      args+=("$1")
      shift
      ;;
  esac
done

if ! command -v mbx >/dev/null 2>&1; then
  echo 'mbx-gc: 找不到 mbx；cargo 的 target 請用 cargo clean。' >&2
  exit 1
fi

echo '注意：cargo clean 不會釋放 mbx 的空間。'
echo 'mbx 的 clippy 產物在 target/check/<triple>/debug（host 是 target/check/debug）。'
echo

if ((confirm)); then
  mbx gc ${args[@]+"${args[@]}"}
  echo
  echo 'mbx gc 完成。'
else
  mbx gc --dry-run ${args[@]+"${args[@]}"}
  echo
  echo '以上是 dry-run，沒有刪除任何東西；確定要清就加 --yes。'
fi
