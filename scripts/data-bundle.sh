#!/usr/bin/env bash
# 把 jc-ime-data 裡 manifest 標成 shipped: true 的檔案打成可重現的資料包。
#
# 用法：
#   scripts/data-bundle.sh [資料根目錄] [輸出檔]
#
# 資料根目錄省略時讀 JC_DATA_ROOT，預設 /mnt/work/jc/jc-ime-data。
# 輸出檔省略時寫到 OUT_DIR（預設目前目錄）的
# jc-ime-data-<manifest version>.tar.zst；找不到 zstd 時改成 .tar.gz。
#
# 環境：
#   JC_DATA_MANIFEST    manifest 路徑；預設 <資料根目錄>/manifest.json
#   OUT_DIR             輸出目錄；預設 .
#   ZSTD_LEVEL          zstd 壓縮等級；預設 19
#   SOURCE_DATE_EPOCH   tar 內所有檔案的 mtime；預設 0（1970-01-01 UTC）
#
# 可重現性：
#   - tar 用 --sort=name、固定 mtime、owner/group 0 與數字 UID/GID。
#   - gzip 用 -n，不寫入檔名與時間戳。
#   - zstd 用 -n 與單執行緒，避免執行緒數改變輸出。
#   - 每個檔案模式固定 0644；封存檔只列檔案，不列目錄，解壓時由 tar 建立
#     目錄。
#   同一份 manifest 與同樣的壓縮工具版本會得到相同 SHA-256。
#
# 注意：只打包 manifest 的 shipped: true 項目與 manifest.json；下載項
# （Funmula 的 zh-select、zh-punct）不會進資料包。
set -euo pipefail

export LC_ALL=C
export TZ=UTC

usage() {
    sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

root=${1:-${JC_DATA_ROOT:-/mnt/work/jc/jc-ime-data}}
manifest=${JC_DATA_MANIFEST:-${root}/manifest.json}
out_dir=${OUT_DIR:-.}
source_date_epoch=${SOURCE_DATE_EPOCH:-0}
zstd_level=${ZSTD_LEVEL:-19}
explicit_out=${2:-}

if [[ ! -d ${root} ]]; then
    echo "資料根目錄不存在：${root}" >&2
    exit 2
fi
if [[ ! -f ${manifest} ]]; then
    echo "manifest 不存在：${manifest}" >&2
    echo "先跑：cargo run -p jc-cli --release -- data manifest ${root} --out ${manifest}" >&2
    exit 2
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

work=$(mktemp -d)
stage=${work}/stage
list=${work}/files.txt
meta=${work}/meta
mkdir -p "${stage}"
tmp_out=
cleanup() {
    if [[ -n ${tmp_out} ]]; then
        rm -f "${tmp_out}"
    fi
    rm -rf "${work}"
}
trap cleanup EXIT

# 建立暫存目錄：manifest.json 用複本，shipped 檔用符號連結，tar 再用 -h
# 解引用。這樣原始資料根目錄可以唯讀，manifest 也可以放在 repo 外。
python3 - "${root}" "${manifest}" "${stage}" "${list}" "${meta}" <<'PY'
import json
import os
import shutil
import sys
from pathlib import Path, PurePosixPath

root = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])
stage = Path(sys.argv[3])
list_path = Path(sys.argv[4])
meta_path = Path(sys.argv[5])

try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"讀取 manifest 失敗：{error}")

version = str(manifest.get("version", "")).strip()
if not version:
    raise SystemExit("manifest 沒有 version")

entries = [entry for entry in manifest.get("files", []) if entry.get("shipped")]
if not entries:
    raise SystemExit("manifest 沒有任何 shipped: true 的檔案")

stage_manifest = stage / "manifest.json"
shutil.copy2(manifest_path, stage_manifest)

rows = []
total = stage_manifest.stat().st_size
for entry in sorted(entries, key=lambda item: item["path"]):
    relative = PurePosixPath(str(entry["path"]))
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise SystemExit(f"manifest 路徑不合法：{entry['path']}")
    source = root / Path(*relative.parts)
    if not source.is_file():
        raise SystemExit(f"manifest 列出的檔案不存在：{source}")
    size = source.stat().st_size
    expected = int(entry.get("bytes", -1))
    if size != expected:
        raise SystemExit(
            f"{relative} 大小不符：manifest {expected}，實際 {size}；請先重新產生 manifest"
        )
    target = stage / Path(*relative.parts)
    target.parent.mkdir(parents=True, exist_ok=True)
    os.symlink(source.resolve(), target)
    rows.append((str(relative), size, str(entry.get("sha256", ""))))
    total += size

paths = ["manifest.json"] + [row[0] for row in rows]
list_path.write_text("\n".join(paths) + "\n", encoding="utf-8")
meta_path.write_text(f"{version}\t{total}\t{len(rows)}\n", encoding="utf-8")

print(f"資料版本：{version}")
print(f"隨附檔案：{len(rows)} 個，另加 manifest.json")
for path, size, digest in rows:
    print(f"  {path}\t{size:,} bytes\tsha256 {digest}")
print(f"  manifest.json\t{stage_manifest.stat().st_size:,} bytes")
print(f"總大小：{total:,} bytes（{total / 1024 / 1024:.1f} MiB）")
PY

IFS=$'\t' read -r data_version _ < "${meta}"

use_zstd=1
if ! command -v zstd >/dev/null 2>&1; then
    use_zstd=0
    echo "找不到 zstd；改用 gzip。gzip 一樣用 -n，但壓縮率與 .tar.zst 不同。" >&2
fi

safe_version=$(printf '%s' "${data_version}" | tr -c 'A-Za-z0-9._-' '_')
if [[ -n ${explicit_out} ]]; then
    out=${explicit_out}
else
    out="${out_dir}/jc-ime-data-${safe_version}.tar.zst"
fi

if [[ ${use_zstd} -eq 0 ]]; then
    case ${out} in
        *.tar.zst) out=${out%.tar.zst}.tar.gz ;;
        *.tzst) out=${out%.tzst}.tar.gz ;;
    esac
else
    case ${out} in
        *.tar.gz | *.tgz) use_zstd=0 ;;
    esac
fi

out_parent=$(dirname "${out}")
mkdir -p "${out_parent}"
tmp_out="${out}.tmp.$$"

echo "輸出：${out}"
echo "壓縮：$(if [[ ${use_zstd} -eq 1 ]]; then echo "zstd -n -T1 -${zstd_level}"; else echo "gzip -n -9"; fi)"

tar_args=(
    --sort=name
    --format=gnu
    --mtime="@${source_date_epoch}"
    --owner=0
    --group=0
    --numeric-owner
    --mode='0644'
    --dereference
    -C "${stage}"
    -cf -
    -T "${list}"
)

if [[ ${use_zstd} -eq 1 ]]; then
    tar "${tar_args[@]}" | zstd -n -T1 "-${zstd_level}" -c > "${tmp_out}"
else
    tar "${tar_args[@]}" | gzip -n -9 > "${tmp_out}"
fi

if command -v sha256sum >/dev/null 2>&1; then
    bundle_sha=$(sha256sum "${tmp_out}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    bundle_sha=$(shasum -a 256 "${tmp_out}" | awk '{print $1}')
else
    echo "找不到 sha256sum 或 shasum，無法計算 bundle SHA-256" >&2
    exit 2
fi

bundle_bytes=$(wc -c < "${tmp_out}")
echo "bundle 大小：${bundle_bytes} bytes（$((bundle_bytes / 1024 / 1024)) MiB）"
echo "bundle SHA-256：${bundle_sha}"
mv -f "${tmp_out}" "${out}"
tmp_out=
echo "完成：${out}"
