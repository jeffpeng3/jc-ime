#!/usr/bin/env bash
# test-plan.sh：從 git diff 或明確檔案清單推算「內圈該跑哪些 crate 與模組」。
#
# 只印建議指令，--run 才執行。這不是 gate：checker／CI 一律跑完整 workspace。
set -euo pipefail

usage() {
  cat <<'EOF'
用法：scripts/test-plan.sh [--base <ref>] [--files <path>[,<path>...]]... [path ...] [--run]

從 git diff（預設 base main，取 merge-base）或明確檔案清單，推算內圈建議的
crate 與模組測試。預設只印指令，不執行。

  --base <ref>   比較基準，預設 main
  --files <path> 直接指定檔案，可重複或逗號分隔；指定後不讀 git
  <path>         也可以直接把檔案路徑放在位置參數
  --run          執行建議指令
  -h, --help     顯示這段說明

這不是 gate。gate（checker／CI）一律跑 `cargo test --workspace`。
EOF
}

base=main
do_run=0
explicit=0
files=()

while (($# > 0)); do
  case "$1" in
    --base)
      [[ $# -ge 2 ]] || { echo 'test-plan: --base 缺值' >&2; exit 2; }
      base=$2
      shift 2
      ;;
    --base=*)
      base=${1#*=}
      shift
      ;;
    --files)
      explicit=1
      shift
      while (($# > 0)) && [[ $1 != --* ]]; do
        IFS=',' read -r -a parts <<<"$1"
        files+=("${parts[@]}")
        shift
      done
      ;;
    --files=*)
      explicit=1
      IFS=',' read -r -a parts <<<"${1#*=}"
      files+=("${parts[@]}")
      shift
      ;;
    --run)
      do_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "test-plan: 未知參數 $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      explicit=1
      files+=("$1")
      shift
      ;;
  esac
done

root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo 'test-plan: 目前目錄不在 git repo 內' >&2
  exit 2
}
cd "$root"

if ((explicit)) && ((${#files[@]} == 0)); then
  echo 'test-plan: --files 沒有給路徑' >&2
  exit 2
fi

changed=()
if ((explicit)); then
  changed=("${files[@]}")
else
  merge_base=$(git merge-base "$base" HEAD) || {
    echo "test-plan: 找不到 $base 與 HEAD 的共同祖先" >&2
    exit 2
  }
  while IFS= read -r path; do
    [[ -n $path ]] && changed+=("$path")
  done < <(git diff --name-only "$merge_base")
  while IFS= read -r path; do
    [[ -n $path ]] && changed+=("$path")
  done < <(git ls-files --others --exclude-standard)
fi

normalize() {
  local path=$1
  path=${path//\\//}
  if [[ $path == "$root"/* ]]; then
    path=${path#"$root"/}
  fi
  while [[ $path == ./* ]]; do
    path=${path#./}
  done
  while [[ $path == */ && $path != / ]]; do
    path=${path%/}
  done
  printf '%s' "$path"
}

crate_package() {
  local dir=$1 manifest="crates/$1/Cargo.toml" name=
  if [[ -f $manifest ]]; then
    name=$(awk '
      /^\[package\]/ { in_pkg = 1; next }
      /^\[/ { in_pkg = 0 }
      in_pkg && /^name[[:space:]]*=/ {
        sub(/^name[[:space:]]*=[[:space:]]*"/, "")
        sub(/".*$/, "")
        print
        exit
      }
    ' "$manifest")
  fi
  printf '%s' "${name:-$dir}"
}

declare -A crate_whole=()
declare -A crate_modules=()
declare -A crate_tests=()
declare -A seen=()
workspace=0
installer_changed=0
reasons=()

for raw in "${changed[@]}"; do
  path=$(normalize "$raw")
  [[ -n $path ]] || continue
  [[ -z ${seen[$path]:-} ]] || continue
  seen[$path]=1

  case "$path" in
    docs|docs/*|scripts|scripts/*)
      reasons+=("$path → 不影響 Rust 測試")
      ;;
    installer|installer/*)
      installer_changed=1
      reasons+=("$path → 不影響 Rust 測試（installer 改動請跑 installer/validate-linux.sh 與 jc-tsf-smoke --json doctor）")
      ;;
    Cargo.toml|Cargo.lock|*/Cargo.toml|*/Cargo.lock)
      workspace=1
      reasons+=("$path → 整個 workspace（依賴或 workspace 設定）")
      ;;
    build.rs)
      workspace=1
      reasons+=("$path → 整個 workspace（build script）")
      ;;
    crates/*/*)
      rel=${path#crates/}
      crate_dir=${rel%%/*}
      rest=${rel#*/}
      crate=$(crate_package "$crate_dir")
      if [[ ! $crate =~ ^[A-Za-z0-9_.-]+$ ]]; then
        workspace=1
        reasons+=("$path → 整個 workspace（crate 名稱無法解析）")
        continue
      fi

      if [[ $rest == build.rs ]]; then
        workspace=1
        reasons+=("$path → 整個 workspace（build script）")
        continue
      fi

      case "$rest" in
        src/lib.rs|src/main.rs|src/mod.rs|src/bin/*)
          crate_whole[$crate]=1
          reasons+=("$path → $crate（整包）")
          ;;
        src/*.rs)
          module=${rest#src/}
          module=${module%.rs}
          if [[ $module == */mod ]]; then
            module=${module%/mod}
          fi
          module=${module//\//::}
          if [[ $module =~ ^[A-Za-z0-9_]+(::[A-Za-z0-9_]+)*$ ]]; then
            crate_modules[$crate]+="$module"$'\n'
            reasons+=("$path → $crate，模組 $module")
          else
            workspace=1
            reasons+=("$path → 整個 workspace（模組路徑無法解析）")
          fi
          ;;
        tests/*)
          test_rel=${rest#tests/}
          if [[ $test_rel == */* ]]; then
            test_name=${test_rel%%/*}
            if [[ $test_rel == "$test_name/main.rs" && $test_name =~ ^[A-Za-z0-9_.-]+$ ]]; then
              crate_tests[$crate]+="$test_name"$'\n'
              reasons+=("$path → $crate，整合測試 $test_name（--test $test_name）")
            else
              crate_whole[$crate]=1
              reasons+=("$path → $crate（測試共用目錄，整包）")
            fi
          elif [[ $test_rel == *.rs ]]; then
            test_name=${test_rel%.rs}
            if [[ $test_name =~ ^[A-Za-z0-9_.-]+$ ]]; then
              crate_tests[$crate]+="$test_name"$'\n'
              reasons+=("$path → $crate，整合測試 $test_name（--test $test_name）")
            else
              workspace=1
              reasons+=("$path → 整個 workspace（測試名稱無法解析）")
            fi
          else
            workspace=1
            reasons+=("$path → 整個 workspace（無法判斷的 tests 檔案）")
          fi
          ;;
        benches/*|examples/*)
          crate_whole[$crate]=1
          reasons+=("$path → $crate（整包，其他 Cargo target）")
          ;;
        *)
          workspace=1
          reasons+=("$path → 整個 workspace（無法判斷的 crate 內檔案）")
          ;;
      esac
      ;;
    crates/*)
      # crate 目錄本身或 crate 根目錄下的非原始碼檔案。
      workspace=1
      reasons+=("$path → 整個 workspace（無法判斷的 crate 檔案）")
      ;;
    *)
      workspace=1
      reasons+=("$path → 整個 workspace（無法判斷的路徑）")
      ;;
  esac
done

sorted_keys() {
  local -n map=$1
  local key
  for key in "${!map[@]}"; do
    printf '%s\n' "$key"
  done | sort -u
}

commands=()
if ((workspace)); then
  commands=("cargo test --workspace")
else
  while IFS= read -r crate; do
    [[ -n $crate ]] && commands+=("cargo test -p $crate")
  done < <(sorted_keys crate_whole)

  while IFS= read -r crate; do
    [[ -n $crate ]] || continue
    [[ -z ${crate_whole[$crate]:-} ]] || continue
    while IFS= read -r module; do
      [[ -n $module ]] || continue
      # cargo test 只收一個 TESTNAME；多個模組就一個模組一條指令，避免產生
      # `cargo test -p c a b` 這種會被 clap 拒絕的寫法。
      commands+=("cargo test -p $crate $module")
    done < <(printf '%s' "${crate_modules[$crate]}" | sort -u)
  done < <(sorted_keys crate_modules)

  while IFS= read -r crate; do
    [[ -n $crate ]] || continue
    [[ -z ${crate_whole[$crate]:-} ]] || continue
    while IFS= read -r test_name; do
      [[ -n $test_name ]] || continue
      commands+=("cargo test -p $crate --test $test_name")
    done < <(printf '%s' "${crate_tests[$crate]}" | sort -u)
  done < <(sorted_keys crate_tests)
fi

echo '內圈建議，不是 gate；gate（checker／CI）一律跑完整 workspace。'
echo
echo '建議指令：'
if ((${#commands[@]} == 0)); then
  if ((${#changed[@]} == 0)); then
    echo '  （無變更）'
  else
    echo '  （無，這次改動不影響 Rust 測試）'
  fi
else
  for command in "${commands[@]}"; do
    echo "  mise exec -- $command"
  done
fi
echo '理由：'
if ((${#reasons[@]} > 0)); then
  for reason in "${reasons[@]}"; do
    echo "  $reason"
  done
else
  echo '  （沒有變更的檔案）'
fi

if ((installer_changed)); then
  echo
  echo '提醒：installer 改動要另外驗 doctor／安裝流程，Rust 測試不會覆蓋。'
fi

if ((do_run)); then
  echo
  echo '--run：執行上方內圈指令。這不是 gate；gate（checker／CI）要跑完整 workspace。'
  if ((${#commands[@]} == 0)); then
    echo '沒有需要執行的 Rust 測試。'
    exit 0
  fi
  for command in "${commands[@]}"; do
    echo "+ mise exec -- $command"
    read -r -a argv <<<"mise exec -- $command"
    if "${argv[@]}"; then
      :
    else
      status=$?
      echo "test-plan: 指令失敗（exit $status）：mise exec -- $command" >&2
      exit "$status"
    fi
  done
fi
