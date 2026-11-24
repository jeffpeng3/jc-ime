#!/usr/bin/env bash
# 驗證已解開的 jc-ime 資料包內容與根 manifest 一致。
#
# 用法：
#   scripts/check-data-bundle.sh <解開後的資料根目錄> [<bundle.sha256>]
#
# 第二個參數可以是 `build-data-bundle.sh` 產生的 .sha256 檔，或 64 字元
# SHA-256 字串。給定時會重算資料包本身的 SHA-256；資料包檔案預設取
# .sha256 檔內記錄的檔名，找不到時讀 JC_DATA_BUNDLE_FILE。
# 省略時只驗內容（CI 的快取命中路徑沒有重新下載 bundle）。
#
# 檢查項目：
#   - build-zh/lexicon.bin、build-zh/manifest.json 存在
#   - build-zh-bigram/lm.bin 存在（bigram 語言模型）
#   - build-zh/lm.bin 不存在（unigram 已退役）
#   - english/words.bin、english/manifest.json、english/NOTICE.txt 存在
#   - 根 manifest.json 的每個 shipped: true 檔案都存在、size 相符
#   - 必要檔案都有列進根 manifest 的 shipped 清單
#
# 失敗時以 ::error:: 印出缺少的檔案、預期路徑與重建指令，最後 exit 1。
set -euo pipefail

export LC_ALL=C

usage() {
    sed -n '2,21p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

root=${1:-}
sha_arg=${2:-}
failures=0

if [[ -z ${root} ]]; then
    usage >&2
    exit 2
fi
if [[ ! -d ${root} ]]; then
    echo "::error::資料根目錄不存在：${root}" >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "::error::找不到 python3；scripts/check-data-bundle.sh 需要 python3 讀 manifest.json" >&2
    exit 2
fi

root=$(cd "${root}" && pwd -P)

fail() {
    echo "::error::$*" >&2
    failures=$((failures + 1))
}

rebuild_hint() {
    echo "請在資料根目錄重跑 scripts/build-data-bundle.sh <資料根目錄> <輸出.tar.zst>，再更新 repo variable JC_DATA_BUNDLE_URL 與 JC_DATA_BUNDLE_SHA256。" >&2
}

# 資料包本身的 SHA-256。
if [[ -n ${sha_arg} ]]; then
    bundle_file=
    expected=
    if [[ -f ${sha_arg} ]]; then
        read -r expected recorded_name < <(awk 'NF { print $1, $2; exit }' "${sha_arg}")
        if [[ -n ${recorded_name} ]]; then
            if [[ ${recorded_name} = /* ]]; then
                bundle_file=${recorded_name}
            else
                bundle_file=$(dirname "${sha_arg}")/${recorded_name}
            fi
        fi
    elif [[ ${sha_arg} =~ ^[0-9A-Fa-f]{64}$ ]]; then
        expected=${sha_arg}
    else
        echo "::error::第二個參數既不是存在的 .sha256 檔，也不是 64 字元 SHA-256：${sha_arg}" >&2
        exit 2
    fi

    if [[ -z ${bundle_file} && -n ${JC_DATA_BUNDLE_FILE:-} ]]; then
        bundle_file=${JC_DATA_BUNDLE_FILE}
    fi
    if [[ -z ${expected} || ! ${expected} =~ ^[0-9A-Fa-f]{64}$ ]]; then
        echo "::error::${sha_arg} 沒有可用的 SHA-256（預期 64 個十六進位字元）" >&2
        exit 2
    fi
    expected=${expected,,}

    if [[ -z ${bundle_file} || ! -f ${bundle_file} ]]; then
        fail "找不到資料包檔案，無法重算 SHA-256；用 JC_DATA_BUNDLE_FILE=<bundle> 指定，或改成只驗內容（省略第二個參數）。"
    else
        actual=$(sha256sum "${bundle_file}" | awk '{print $1}')
        if [[ ${actual} != "${expected}" ]]; then
            fail "資料包 SHA-256 不符：預期 ${expected}，實際 ${actual}（檔案：${bundle_file}）"
        else
            echo "資料包 SHA-256 通過：${actual}"
        fi
    fi
fi

require_file() {
    local relative=$1
    local path="${root}/${relative}"
    if [[ ! -f ${path} ]]; then
        fail "缺少必要檔案：${relative}（預期路徑：${path}）"
    fi
}

for relative in \
    build-zh/lexicon.bin \
    build-zh/manifest.json \
    build-zh-bigram/lm.bin \
    english/words.bin \
    english/manifest.json \
    english/NOTICE.txt
do
    require_file "${relative}"
done

if [[ -e "${root}/build-zh/lm.bin" ]]; then
    fail "資料包不應包含 build-zh/lm.bin（unigram 已退役，注音只帶 build-zh-bigram/lm.bin）；請重新產生根 manifest 與資料包。"
fi

manifest="${root}/manifest.json"
if [[ ! -f ${manifest} ]]; then
    fail "資料包缺少根 manifest：manifest.json（預期路徑：${manifest}）"
else
    set +e
    manifest_errors=$(python3 - "${manifest}" "${root}" <<'PY'
import json
import sys
from pathlib import Path, PurePosixPath

manifest_path = Path(sys.argv[1])
root = Path(sys.argv[2])
required = [
    "build-zh/lexicon.bin",
    "build-zh/manifest.json",
    "build-zh-bigram/lm.bin",
    "english/words.bin",
    "english/manifest.json",
    "english/NOTICE.txt",
]

try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except (OSError, ValueError) as error:
    print(f"::error::讀取根 manifest 失敗：{manifest_path}：{error}")
    raise SystemExit(1)

files = manifest.get("files")
if not isinstance(files, list) or not files:
    print(f"::error::根 manifest 沒有列出任何檔案：{manifest_path}")
    raise SystemExit(1)

errors = 0
shipped = set()
for entry in files:
    if not isinstance(entry, dict) or entry.get("shipped") is not True:
        continue
    relative = entry.get("path")
    if not isinstance(relative, str) or not relative:
        print("::error::根 manifest 的 shipped 項目沒有合法 path")
        errors += 1
        continue
    pure = PurePosixPath(relative)
    if pure.is_absolute() or any(part in ("", ".", "..") for part in pure.parts):
        print(f"::error::根 manifest 的 shipped 路徑不合法：{relative}")
        errors += 1
        continue
    shipped.add(relative)

    target = root / Path(*pure.parts)
    if not target.is_file():
        print(
            f"::error::根 manifest 的 shipped 檔案不存在：{relative}"
            f"（預期路徑：{target}）"
        )
        errors += 1
        continue

    expected_bytes = entry.get("bytes")
    try:
        expected_bytes = int(expected_bytes)
    except (TypeError, ValueError):
        print(
            f"::error::根 manifest 的 shipped 檔案沒有合法 bytes：{relative}"
            f"（值：{entry.get('bytes')!r}）"
        )
        errors += 1
        continue
    actual_bytes = target.stat().st_size
    if actual_bytes != expected_bytes:
        print(
            f"::error::根 manifest 的 shipped 檔案 size 不符：{relative}"
            f"（預期 {expected_bytes} bytes，實際 {actual_bytes} bytes）"
        )
        errors += 1

for relative in required:
    if relative not in shipped:
        print(
            f"::error::根 manifest 沒有把必要檔案列成 shipped: true：{relative}"
            f"（預期路徑：{root / relative}）"
        )
        errors += 1

raise SystemExit(1 if errors else 0)
PY
)
    manifest_status=$?
    set -e
    if [[ -n ${manifest_errors} ]]; then
        printf '%s\n' "${manifest_errors}" >&2
        failures=$((failures + 1))
    fi
    if [[ ${manifest_status} -ne 0 && -z ${manifest_errors} ]]; then
        fail "驗證根 manifest 失敗：${manifest}（python3 exit ${manifest_status}）"
    fi
fi

if (( failures > 0 )); then
    echo "::error::資料包檢查失敗：共 ${failures} 類問題（資料根目錄：${root}）" >&2
    rebuild_hint
    exit 1
fi

echo "資料包檢查通過：${root}"
