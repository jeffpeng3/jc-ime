#!/usr/bin/env bash
# scripts/test-plan.sh 的案例測試：全部走 --files，不編譯、不執行 cargo。
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d /tmp/opencode/test-plan.XXXXXX)
trap 'rm -rf "$tmp"' EXIT
OUT=$tmp/out
fail=0

plan() {
  bash scripts/test-plan.sh "$@" >"$OUT"
}

expect() {
  local desc=$1 text=$2
  if grep -Fq -- "$text" "$OUT"; then
    printf 'ok   %s\n' "$desc"
  else
    printf 'FAIL %s（找不到：%s）\n' "$desc" "$text"
    sed 's/^/     /' "$OUT"
    fail=1
  fi
}

reject() {
  local desc=$1 text=$2
  if grep -Fq -- "$text" "$OUT"; then
    printf 'FAIL %s（不該出現：%s）\n' "$desc" "$text"
    sed 's/^/     /' "$OUT"
    fail=1
  else
    printf 'ok   %s\n' "$desc"
  fi
}

echo '== 只改 crates/jc-core/src/session/mixed.rs =='
plan --files crates/jc-core/src/session/mixed.rs
expect 'jc-core 帶模組過濾' "cargo nextest run -p jc-core -E 'test(/session::mixed/)'"
expect 'jc-core 補 doctest' 'cargo test -p jc-core --doc'
expect '理由寫出 crate 與模組' 'crates/jc-core/src/session/mixed.rs → jc-core，模組 session::mixed'
reject '不是 workspace' 'cargo nextest run --workspace'

echo '== 只改 crates/jc-tsf/src/service.rs =='
plan --files crates/jc-tsf/src/service.rs
expect 'jc-tsf 帶模組過濾' "cargo nextest run -p jc-tsf -E 'test(/service/)'"
expect '理由寫出 jc-tsf' '模組 service'
reject '沒有跑到 jc-core' '-p jc-core'

echo '== 只改 Cargo.toml =='
plan --files Cargo.toml
expect 'Cargo.toml 跑 workspace' 'cargo nextest run --workspace'
expect 'workspace 補 doctest' 'cargo test --workspace --doc'
expect '理由說明依賴設定' '整個 workspace（依賴或 workspace 設定）'
reject '沒有 -p 選擇' 'cargo nextest run -p'

echo '== 只改 Cargo.lock =='
plan --files Cargo.lock
expect 'Cargo.lock 跑 workspace' 'cargo nextest run --workspace'
expect 'Cargo.lock 補 doctest' 'cargo test --workspace --doc'

echo '== 只改 docs/ =='
plan --files docs/port/testing.md
expect 'docs 不跑 Rust 測試' '（無，這次改動不影響 Rust 測試）'
expect '理由標成不影響測試' 'docs/port/testing.md → 不影響 Rust 測試'
reject 'docs 不產生 nextest 指令' 'cargo nextest run'

echo '== 只改 scripts/ =='
plan --files scripts/test-plan.sh
expect 'scripts 不跑 Rust 測試' 'scripts/test-plan.sh → 不影響 Rust 測試'
reject 'scripts 不產生 nextest 指令' 'cargo nextest run'

echo '== 只改 installer/ =='
plan --files installer/validate-linux.sh
expect 'installer 不跑 Rust 測試' 'installer/validate-linux.sh → 不影響 Rust 測試'
expect 'installer 提醒 doctor' 'doctor'
expect 'installer 有額外提醒' 'installer 改動要另外驗 doctor'
reject 'installer 不產生 nextest 指令' 'cargo nextest run'

echo '== 只改 crates/jc-core/tests/foo.rs =='
plan --files crates/jc-core/tests/foo.rs
expect '整合測試用 --test' 'cargo nextest run -p jc-core --test foo'
expect '整合測試補 doctest' 'cargo test -p jc-core --doc'
expect '理由寫出 --test' '整合測試 foo（--test foo）'
reject '不是模組過濾' 'session::'

echo '== 只改 crates/jc-core/tests/foo/main.rs =='
plan --files crates/jc-core/tests/foo/main.rs
expect '目錄形式整合測試用 --test' 'cargo nextest run -p jc-core --test foo'

echo '== 測試共用目錄 tests/common/mod.rs =='
plan --files crates/jc-core/tests/common/mod.rs
expect '共用模組跑整包' 'cargo nextest run -p jc-core'
expect '理由標成整包' '測試共用目錄，整包'
reject '共用模組不加 --test common' '--test common'

echo '== 未追蹤／未知路徑 =='
plan --files some/unknown/file.bin
expect '未知路徑跑 workspace' 'cargo nextest run --workspace'
expect '未知路徑補 doctest' 'cargo test --workspace --doc'
expect '理由標成無法判斷' 'some/unknown/file.bin → 整個 workspace（無法判斷的路徑）'

echo '== 多個模組各自成一個指令 =='
plan --files crates/jc-core/src/session/mixed.rs crates/jc-core/src/session/input_style.rs
expect 'mixed 模組指令' "cargo nextest run -p jc-core -E 'test(/session::mixed/)'"
expect 'input_style 模組指令' "cargo nextest run -p jc-core -E 'test(/session::input_style/)'"
expect '多個模組只補一次 doctest' 'cargo test -p jc-core --doc'
reject '不產生合併的舊寫法' 'session::input_style session::mixed'

echo '== src/lib.rs 跑整包 =='
plan --files crates/jc-core/src/lib.rs
expect 'lib.rs 整包' 'crates/jc-core/src/lib.rs → jc-core（整包）'
expect 'lib.rs 產生 crate 指令' 'cargo nextest run -p jc-core'
reject 'lib.rs 不當成模組' 'jc-core lib'

echo '== crate 根目錄 build.rs 跑 workspace =='
plan --files crates/jc-core/build.rs
expect 'crate build.rs 跑 workspace' '整個 workspace（build script）'

echo '== src 內的 build.rs 視為模組 =='
plan --files crates/jc-core/src/lattice/build.rs
expect 'src build.rs 當模組' 'crates/jc-core/src/lattice/build.rs → jc-core，模組 lattice::build'
expect 'src build.rs 用 nextest 過濾' "cargo nextest run -p jc-core -E 'test(/lattice::build/)'"
reject 'src build.rs 不跑 workspace' 'cargo nextest run --workspace'

echo '== docs 與程式碼混合 =='
plan --files docs/port/foo.md crates/jc-engine/src/lib.rs
expect '程式碼照跑' 'cargo nextest run -p jc-engine'
expect 'docs 仍標不影響' 'docs/port/foo.md → 不影響 Rust 測試'

echo '== --run 明講不是 gate，且 docs 不執行任何指令 =='
plan --files docs/port/testing.md --run
expect 'run 模式提醒不是 gate' '這不是 gate；gate（checker／CI）要跑完整 workspace'
expect 'run 模式沒有要跑的測試' '沒有需要執行的 Rust 測試'
reject 'run 模式不執行 cargo' '+ cargo'

echo '== 位置參數也能指定檔案 =='
plan crates/jc-tsf/src/service.rs
expect '位置參數走檔案清單' "cargo nextest run -p jc-tsf -E 'test(/service/)'"

echo '== --help =='
plan --help
expect 'help 有用法' '用法：scripts/test-plan.sh'
expect 'help 說明不是 gate' '這不是 gate'

echo '== git 模式：分支上的變更與未追蹤檔案 =='
repo=$tmp/repo
mkdir -p "$repo/scripts" "$repo/crates/jc-core/src/session"
cp scripts/test-plan.sh "$repo/scripts/test-plan.sh"
cat >"$repo/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/jc-core"]
TOML
cat >"$repo/crates/jc-core/Cargo.toml" <<'TOML'
[package]
name = "jc-core"
version = "0.0.0"
edition = "2021"
TOML
printf 'pub fn before() {}\n' >"$repo/crates/jc-core/src/session/mixed.rs"
git -C "$repo" init -q -b main
git -C "$repo" add -A
git -C "$repo" -c user.name=test -c user.email=test@example.com commit -qm base

printf '\n// changed\n' >>"$repo/crates/jc-core/src/session/mixed.rs"
(cd "$repo" && bash scripts/test-plan.sh) >"$OUT"
expect 'git diff 抓到已追蹤變更' "cargo nextest run -p jc-core -E 'test(/session::mixed/)'"
reject 'git 模式不是 workspace' 'cargo nextest run --workspace'

printf 'x\n' >"$repo/untracked.bin"
(cd "$repo" && bash scripts/test-plan.sh) >"$OUT"
expect '未追蹤檔案跑 workspace' 'cargo nextest run --workspace'
expect '未追蹤檔案補 doctest' 'cargo test --workspace --doc'
expect '未追蹤檔案列在理由' 'untracked.bin → 整個 workspace（無法判斷的路徑）'

echo
if ((fail)); then
  echo 'test-plan-test: FAIL'
  exit 1
fi
echo 'test-plan-test: PASS'
