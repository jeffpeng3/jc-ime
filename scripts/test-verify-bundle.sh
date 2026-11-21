#!/usr/bin/env bash
# 比對 scripts/verify-bundle.py 與 `jc-cli data verify` 的 Human／JSON 輸出與 exit code。
# 六個情境都從固定內容的小資料根目錄產生，不依賴正式資料包。
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "${repo_root}"

tmp=$(mktemp -d /tmp/opencode/verify-bundle-test.XXXXXX)
trap 'rm -rf "${tmp}"' EXIT

# 覆寫 JC_CLI_BIN 可以直接用編好的 jc-cli，跳過 cargo run 的身分檢查。
if [[ -n "${JC_CLI_BIN:-}" ]]; then
  rust_cmd=("${JC_CLI_BIN}")
else
  rust_cmd=(cargo run -q -p jc-cli --)
fi

# 造一個小資料根目錄：一個 shipped 檔與一個下載項。
# 內容固定，manifest 的 bytes／sha256 都從內容算出，只有被測情境刻意改壞。
make_fixture() {
  local scenario=$1 target=$2
  python3 - "${scenario}" "${target}" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

scenario, target = sys.argv[1], Path(sys.argv[2])
target.mkdir(parents=True, exist_ok=True)

lex_path = "build-zh/lexicon.bin"
dl_path = "models/zh-select/model.onnx"
lex_good = b"zh-lexicon"
dl_good = b"onnx-model"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write(rel: str, data: bytes | None) -> None:
    if data is None:
        return
    path = target / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


lex_actual = {
    "ok": lex_good,
    "hash_mismatch": b"zh-lexicoX",  # 同長度，內容不同
    "size_mismatch": b"zh-lexicon-longer",
    "shipped_missing": None,
    "download_missing": lex_good,
    "json": lex_good,
}[scenario]
dl_actual = None if scenario in {"download_missing", "json"} else dl_good

write(lex_path, lex_actual)
write(dl_path, dl_actual)

files = [
    {
        "path": lex_path,
        "bytes": len(lex_good),
        "sha256": sha256(lex_good),
        "role": "zh_lexicon",
        "license": ["L-04"],
        "shipped": True,
    },
    {
        "path": dl_path,
        "bytes": len(dl_good),
        "sha256": sha256(dl_good),
        "role": "zh_select",
        "license": ["L-11"],
        "shipped": False,
        "url": "https://example.invalid/model.onnx",
    },
]
manifest = {
    "schema": "0c.data-pack/1",
    "version": "test",
    "generated_at": "2026-11-21T00:00:00Z",
    "files": files,
}
(target / "manifest.json").write_text(
    json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
)
PY
}

failures=0

compare() {
  local scenario=$1
  shift
  local extra=("$@")
  local dir="${tmp}/${scenario}"
  make_fixture "${scenario}" "${dir}"

  local py_out="${tmp}/${scenario}.py.out" py_err="${tmp}/${scenario}.py.err"
  local rs_out="${tmp}/${scenario}.rs.out" rs_err="${tmp}/${scenario}.rs.err"
  local py_rc=0 rs_rc=0

  python3 "${repo_root}/scripts/verify-bundle.py" "${dir}" "${extra[@]}" \
    >"${py_out}" 2>"${py_err}" || py_rc=$?
  "${rust_cmd[@]}" data verify "${dir}" "${extra[@]}" \
    >"${rs_out}" 2>"${rs_err}" || rs_rc=$?

  if [[ "${py_rc}" -ne "${rs_rc}" ]]; then
    echo "FAIL ${scenario}：exit code 不同（Python ${py_rc}，Rust ${rs_rc}）" >&2
    echo "  Python stderr: $(cat "${py_err}")" >&2
    echo "  Rust stderr: $(cat "${rs_err}")" >&2
    return 1
  fi
  if ! diff -u "${rs_out}" "${py_out}" >"${tmp}/${scenario}.diff"; then
    echo "FAIL ${scenario}：stdout 不同" >&2
    cat "${tmp}/${scenario}.diff" >&2
    return 1
  fi
  echo "ok   ${scenario}：exit ${py_rc}，stdout 一致"
}

compare ok || failures=$((failures + 1))
compare hash_mismatch || failures=$((failures + 1))
compare size_mismatch || failures=$((failures + 1))
compare shipped_missing || failures=$((failures + 1))
compare download_missing || failures=$((failures + 1))
compare json --json || failures=$((failures + 1))

if [[ "${failures}" -ne 0 ]]; then
  echo "${failures} 個情境失敗" >&2
  exit 1
fi
echo "6/6 情境通過"
