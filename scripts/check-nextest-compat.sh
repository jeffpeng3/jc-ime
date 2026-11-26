#!/usr/bin/env bash
# 每週相容性守門：同一份 workspace 上，nextest 與傳統 cargo test 的統計
# 必須一致。CI 用 schedule／workflow_dispatch 跑；平常 gate 只跑 nextest。
#
# nextest 不跑 doctest，這裡的 cargo test 統計會排除 Doc-tests，另外由 gate
# 的 `cargo test --workspace --doc` 負責。
set -euo pipefail

tmp=$(mktemp -d "${TMPDIR:-/tmp}/jc-nextest-compat.XXXXXX")
trap 'rm -rf "${tmp}"' EXIT

set +e
cargo nextest run --workspace --no-fail-fast 2>&1 | tee "${tmp}/nextest.log"
nextest_status=${PIPESTATUS[0]}
cargo test --workspace --no-fail-fast 2>&1 | tee "${tmp}/cargo.log"
cargo_status=${PIPESTATUS[0]}
set -e

echo
echo "nextest exit=${nextest_status}，cargo test exit=${cargo_status}"

python3 - "${tmp}/nextest.log" "${tmp}/cargo.log" "${nextest_status}" "${cargo_status}" <<'PY'
import re
import sys

nextest_log, cargo_log, nextest_status, cargo_status = sys.argv[1:5]
nextest_status = int(nextest_status)
cargo_status = int(cargo_status)

nextest_text = open(nextest_log, encoding="utf-8", errors="replace").read()
summary = re.search(
    r"Summary \[[^\]]*\] (\d+) tests run: (\d+) passed(?: \([^)]*\))?"
    r"(?:, (\d+) failed)?, (\d+) skipped",
    nextest_text,
)
if summary is None:
    print("找不到 nextest 的 Summary，建置或執行可能失敗。")
    sys.exit(1)

nextest_run = int(summary.group(1))
nextest_passed = int(summary.group(2))
nextest_failed = int(summary.group(3) or 0)
nextest_skipped = int(summary.group(4))

cargo_passed = cargo_failed = cargo_ignored = 0
in_doctest = False
result_re = re.compile(
    r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored"
)
for line in open(cargo_log, encoding="utf-8", errors="replace"):
    stripped = line.strip()
    if stripped.startswith("Doc-tests "):
        in_doctest = True
        continue
    match = result_re.match(stripped)
    if match is None:
        continue
    if not in_doctest:
        cargo_passed += int(match.group(1))
        cargo_failed += int(match.group(2))
        cargo_ignored += int(match.group(3))
    # 每個 test result 都結束一個 target；下一個 Doc-tests 會重新設定。
    in_doctest = False

cargo_run = cargo_passed + cargo_failed
print(
    f"nextest：run={nextest_run} passed={nextest_passed} "
    f"failed={nextest_failed} skipped={nextest_skipped}"
)
print(
    f"cargo test（非 doctest）：run={cargo_run} passed={cargo_passed} "
    f"failed={cargo_failed} ignored={cargo_ignored}"
)

problems = []
if nextest_status != 0 or cargo_status != 0:
    problems.append(f"exit code 不同或非 0：nextest={nextest_status} cargo={cargo_status}")
if nextest_run != cargo_run:
    problems.append(f"執行數不同：nextest={nextest_run} cargo={cargo_run}")
if nextest_passed != cargo_passed:
    problems.append(f"passed 數不同：nextest={nextest_passed} cargo={cargo_passed}")
if nextest_failed != cargo_failed:
    problems.append(f"failed 數不同：nextest={nextest_failed} cargo={cargo_failed}")
if nextest_skipped != cargo_ignored:
    problems.append(f"skipped／ignored 數不同：nextest={nextest_skipped} cargo={cargo_ignored}")

if problems:
    print("相容性守門 FAIL：")
    for problem in problems:
        print(f"  - {problem}")
    sys.exit(1)

print("相容性守門 PASS：nextest 與 cargo test 的統計一致。")
PY
