#!/usr/bin/env bash
# 從資料根目錄產生可安裝的 jc-ime 資料包（tar.zst）與 SHA-256。
#
# 用法：
#   scripts/build-data-bundle.sh <資料根目錄> <輸出.tar.zst>
#
# 只收 jc-data catalog 標成 shipped: true 的檔案。注音部分固定是：
#   build-zh/lexicon.bin           詞典
#   build-zh/manifest.json         詞典驗證用 manifest
#   build-zh-bigram/lm.bin         bigram 語言模型（不再收 build-zh/lm.bin）
# 另外收 english/、models/、packs/ 的 shipped 檔案。
#
# 為什麼 bigram 不用同目錄 manifest.json：引擎用 `ZhLm::open` 直接讀
# build-zh-bigram/lm.bin，驗證檔內 magic、版本、CRC32 與索引版面；這個檔案
# 沒有與 lexicon 共用的 manifest 驗證。root manifest.json 由本腳本產生，
# 引擎啟動時的 check_data_manifest 會驗它。
#
# 環境：
#   JC_CLI_BIN         已編好的 jc-cli；未設定時用 cargo run -p jc-cli
#   DATA_VERSION       根 manifest 的 version；預設 local
#   SOURCE_DATE_EPOCH  tar mtime 與 manifest generated_at；預設 0
#   OUT_DIR            未給第二個參數時的輸出目錄；預設 .
#
# 輸出：
#   <輸出.tar.zst>
#   <輸出.tar.zst>.sha256（`sha256sum` 格式）
set -euo pipefail

export LC_ALL=C
export TZ=UTC

usage() {
    sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

root=${1:-}
out=${2:-}
out_dir=${OUT_DIR:-.}
data_version=${DATA_VERSION:-local}
source_date_epoch=${SOURCE_DATE_EPOCH:-0}

if [[ -z ${root} ]]; then
    echo "用法：$0 <資料根目錄> <輸出.tar.zst>" >&2
    exit 2
fi
if [[ ! -d ${root} ]]; then
    echo "資料根目錄不存在：${root}" >&2
    exit 2
fi
if [[ -z ${out} ]]; then
    out="${out_dir}/jc-ime-data-${data_version}.tar.zst"
fi
if ! [[ ${source_date_epoch} =~ ^[0-9]+$ ]]; then
    echo "SOURCE_DATE_EPOCH 必須是整數秒：${source_date_epoch}" >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "找不到 python3" >&2
    exit 2
fi
if ! command -v tar >/dev/null 2>&1; then
    echo "找不到 tar" >&2
    exit 2
fi
if ! command -v zstd >/dev/null 2>&1; then
    echo "找不到 zstd；本腳本固定輸出 tar.zst" >&2
    exit 2
fi

work=$(mktemp -d)
stage=${work}/stage
list=${work}/files.txt
tmp_out=
cleanup() {
    if [[ -n ${tmp_out} ]]; then
        rm -f "${tmp_out}"
    fi
    rm -rf "${work}"
}
trap cleanup EXIT

mkdir -p "${stage}"

# manifest 由 catalog 產生，避免手動維護路徑、role 與授權代號。
cli=()
if [[ -n ${JC_CLI_BIN:-} ]]; then
    cli=("${JC_CLI_BIN}")
else
    cli=(cargo run -q -p jc-cli --)
fi

echo "產生 root manifest：${stage}/manifest.json"
"${cli[@]}" data manifest "${root}" --out "${stage}/manifest.json" \
    --version "${data_version}" >/dev/null

# generated_at 固定用 SOURCE_DATE_EPOCH，讓同一份資料產生相同 manifest。
python3 - "${stage}/manifest.json" "${source_date_epoch}" <<'PY'
import datetime
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
epoch = int(sys.argv[2])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["generated_at"] = (
    datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc)
    .strftime("%Y-%m-%dT%H:%M:%SZ")
)
path.write_text(
    json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY

# 只把 shipped 檔案放進 stage；manifest 自己也在 tar 裡。
python3 - "${root}" "${stage}" "${list}" <<'PY'
import hashlib
import json
import os
import sys
from pathlib import Path, PurePosixPath

root = Path(sys.argv[1])
stage = Path(sys.argv[2])
list_path = Path(sys.argv[3])
manifest = json.loads((stage / "manifest.json").read_text(encoding="utf-8"))

entries = sorted(
    (entry for entry in manifest.get("files", []) if entry.get("shipped")),
    key=lambda item: item["path"],
)
if not entries:
    raise SystemExit("manifest 沒有任何 shipped: true 的檔案")

rows = []
for entry in entries:
    relative = PurePosixPath(str(entry["path"]))
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise SystemExit(f"manifest 路徑不合法：{entry['path']}")
    source = root / Path(*relative.parts)
    if not source.is_file():
        raise SystemExit(f"manifest 列出的檔案不存在：{source}")
    size = source.stat().st_size
    if size != int(entry["bytes"]):
        raise SystemExit(
            f"{relative} 大小不符：manifest {entry['bytes']}，實際 {size}"
        )
    digest = hashlib.sha256()
    with source.open("rb") as handle:
        while chunk := handle.read(1 << 20):
            digest.update(chunk)
    actual = digest.hexdigest()
    if actual.lower() != str(entry["sha256"]).lower():
        raise SystemExit(f"{relative} SHA-256 不符：manifest {entry['sha256']}，實際 {actual}")
    target = stage / Path(*relative.parts)
    target.parent.mkdir(parents=True, exist_ok=True)
    os.symlink(source.resolve(), target)
    rows.append((str(relative), size))

if any(row[0] == "build-zh/lm.bin" for row in rows):
    raise SystemExit("資料包不可包含 unigram-only 的 build-zh/lm.bin")
if not any(row[0] == "build-zh-bigram/lm.bin" for row in rows):
    raise SystemExit("資料包缺少 build-zh-bigram/lm.bin")

paths = ["manifest.json"] + [row[0] for row in rows]
list_path.write_text("\n".join(paths) + "\n", encoding="utf-8")

print(f"隨附檔案：{len(rows)} 個，另加 manifest.json")
for path, size in rows:
    print(f"  {path}\t{size:,} bytes")
PY

# 解引用 stage 內的符號連結後用 Full 驗證，確認 tar 會帶到正確內容。
echo "驗證 staged 資料包"
"${cli[@]}" data verify "${stage}" >/dev/null

out_parent=$(dirname "${out}")
mkdir -p "${out_parent}"
tmp_out="${out}.tmp.$$"

echo "輸出：${out}"
tar \
    --sort=name \
    --format=gnu \
    --mtime="@${source_date_epoch}" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --mode='0644' \
    --dereference \
    --zstd \
    -C "${stage}" \
    -cf "${tmp_out}" \
    -T "${list}"

bundle_sha=$(sha256sum "${tmp_out}" | awk '{print $1}')
bundle_bytes=$(wc -c < "${tmp_out}")
mv -f "${tmp_out}" "${out}"
tmp_out=
sha256sum "${out}" > "${out}.sha256"

echo "bundle 大小：${bundle_bytes} bytes（$((bundle_bytes / 1024 / 1024)) MiB）"
echo "bundle SHA-256：${bundle_sha}"
echo "完成：${out}"
echo "校驗檔：${out}.sha256"
