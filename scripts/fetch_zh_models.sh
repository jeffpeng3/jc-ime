#!/usr/bin/env bash
# 下載注音選字與注音標點 ONNX 模型，重跑會重新驗證大小與 SHA-256。
#
# 來源固定 Hugging Face 的 commit，所有檔案雜湊寫死在下面。任何既有檔案
# 若大小或雜湊不符，腳本直接失敗，不會覆寫。缺少的檔案才下載到 `.part`
# 再驗證後改名。
#
# 用法：scripts/fetch_zh_models.sh [模型根目錄]
# 預設模型根目錄：$JC_IME_DATA/models 或 /mnt/work/jc/jc-ime-data/models
set -eu

DATA_ROOT=${JC_IME_DATA:-/mnt/work/jc/jc-ime-data}
OUT_DIR=${1:-$DATA_ROOT/models}
CACHE_DIR=${ZH_MODELS_CACHE:-/tmp/opencode/zh-models}

mkdir -p "$CACHE_DIR"

# 角色｜repo｜revision｜短 rev｜授權｜檔名｜SHA-256｜大小
cat > "$CACHE_DIR/files.tsv" <<'TSV'
zh-select|Funmula/0c-sequence-d512|3533e34ef25896afeacc928d136ba7f35783bba7|3533e34e|MIT|model.onnx|4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261|130723137
zh-select|Funmula/0c-sequence-d512|3533e34ef25896afeacc928d136ba7f35783bba7|3533e34e|MIT|vocab.json|556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3|312708
zh-select|Funmula/0c-sequence-d512|3533e34ef25896afeacc928d136ba7f35783bba7|3533e34e|MIT|manifest.json|6545fed2d7092aec70f929c2fce8360e1456778e99f6eeadc9d8299e5ad91763|1165
zh-select|Funmula/0c-sequence-d512|3533e34ef25896afeacc928d136ba7f35783bba7|3533e34e|MIT|LICENSE|ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f|1077
zh-select|Funmula/0c-sequence-d512|3533e34ef25896afeacc928d136ba7f35783bba7|3533e34e|MIT|README.md|c1cda3b9ab51321317bfafc12a634b404ca393611cdbc97ac0c98b229648a027|8365
zh-punct|Funmula/0c-punct-d512|105c2770e0b7cf4ce44a792b3b186000dd1727d9|105c2770|MIT|model.onnx|4db3ed40df777347d9c455da01285b01de192d4c210c5f7cd9f0646c727880d8|130584075
zh-punct|Funmula/0c-punct-d512|105c2770e0b7cf4ce44a792b3b186000dd1727d9|105c2770|MIT|vocab.json|556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3|312708
zh-punct|Funmula/0c-punct-d512|105c2770e0b7cf4ce44a792b3b186000dd1727d9|105c2770|MIT|manifest.json|70909d272d81fe8556efb5a0e2301c872f3c6fbdb9932a6651fedd214e39527c|556
zh-punct|Funmula/0c-punct-d512|105c2770e0b7cf4ce44a792b3b186000dd1727d9|105c2770|MIT|LICENSE|ab1525e5aba48f95223228d1fcd045820783e6b3846972fbec67970314177e4f|1077
zh-punct|Funmula/0c-punct-d512|105c2770e0b7cf4ce44a792b3b186000dd1727d9|105c2770|MIT|README.md|0402fdc8c81618f6f9d51284ec0b0bef1a5444ff8f74e25280e2436c73a12f77|5596
TSV

sha256_of() {
  sha256sum "$1" | cut -d' ' -f1
}

# 驗證檔案的實際大小與 SHA-256；不符就回傳非零。
verify_file() {
  local file=$1 want_sha=$2 want_size=$3 got_size got_sha
  if [ ! -f "$file" ]; then
    echo "缺少檔案：$file" >&2
    return 1
  fi
  got_size=$(stat -c %s "$file")
  if [ "$got_size" != "$want_size" ]; then
    echo "大小不符：$file 期望 $want_size，實際 $got_size" >&2
    return 1
  fi
  got_sha=$(sha256_of "$file")
  if [ "$got_sha" != "$want_sha" ]; then
    echo "sha256 不符：$file 期望 $want_sha，實際 $got_sha" >&2
    return 1
  fi
}

# 下載單一檔案；manifest.json 留在快取，其餘直接放進角色目錄。
fetch_file() {
  local role=$1 repo=$2 rev=$3 file=$4 want_sha=$5 want_size=$6
  local dest
  if [ "$file" = manifest.json ]; then
    dest="$CACHE_DIR/$role/manifest.json"
    mkdir -p "$CACHE_DIR/$role"
  else
    dest="$OUT_DIR/$role/$file"
    mkdir -p "$OUT_DIR/$role"
  fi

  if [ -f "$dest" ]; then
    verify_file "$dest" "$want_sha" "$want_size"
    echo "ok $role/$file"
    return
  fi

  local url="https://huggingface.co/$repo/resolve/$rev/$file"
  local tmp="$dest.part"
  rm -f "$tmp"
  echo "下載 $url" >&2
  curl -fL --retry 3 --retry-delay 2 --connect-timeout 20 -o "$tmp" "$url"
  verify_file "$tmp" "$want_sha" "$want_size"
  mv "$tmp" "$dest"
  echo "ok $role/$file"
}

while IFS='|' read -r role repo rev rev_short license file want_sha want_size; do
  [ -z "$role" ] && continue
  fetch_file "$role" "$repo" "$rev" "$file" "$want_sha" "$want_size"
done < "$CACHE_DIR/files.tsv"

# 把上游 runtime manifest 加上 download 區塊，寫成角色目錄的 manifest.json。
uv run --no-project python - "$OUT_DIR" "$CACHE_DIR" "$CACHE_DIR/files.tsv" <<'PY'
import hashlib
import json
import os
import sys
from collections import OrderedDict

out_dir, cache_dir, table_path = sys.argv[1], sys.argv[2], sys.argv[3]

roles = OrderedDict()
with open(table_path, encoding="utf-8") as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        role, repo, rev, rev_short, license, name, sha, size = line.split("|")
        roles.setdefault(
            role,
            {
                "repository": repo,
                "revision": rev,
                "revision_short": rev_short,
                "license": license,
                "files": OrderedDict(),
            },
        )
        roles[role]["files"][name] = {"sha256": sha, "size": int(size)}


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


for role, meta in roles.items():
    upstream_path = os.path.join(cache_dir, role, "manifest.json")
    with open(upstream_path, encoding="utf-8") as f:
        runtime = json.load(f, object_pairs_hook=OrderedDict)

    repo, rev = meta["repository"], meta["revision"]
    base = f"https://huggingface.co/{repo}/resolve/{rev}"
    files = OrderedDict()
    for name, pinned in meta["files"].items():
        files[name] = OrderedDict(
            [
                ("sha256", pinned["sha256"]),
                ("size", pinned["size"]),
                ("url", f"{base}/{name}"),
            ]
        )

    # 上游 manifest 自己宣告的雜湊要與下載表一致。
    for name, digest in runtime.get("files", {}).items():
        if name in meta["files"] and meta["files"][name]["sha256"] != digest:
            raise SystemExit(
                f"{role}/{name} 的上游 manifest 雜湊 {digest} 與下載表不符"
            )

    runtime["download"] = OrderedDict(
        [
            ("repository", repo),
            ("revision", rev),
            ("revision_short", meta["revision_short"]),
            ("url", f"https://huggingface.co/{repo}/tree/{rev}"),
            ("license", meta["license"]),
            ("upstream_manifest_sha256", sha256_file(upstream_path)),
            ("files", files),
        ]
    )

    role_dir = os.path.join(out_dir, role)
    with open(os.path.join(role_dir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(runtime, f, ensure_ascii=False, indent=2)
        f.write("\n")

    notice = [
        f"{role} model",
        f"Source: {repo}",
        f"Revision: {rev} ({meta['revision_short']})",
        f"URL: https://huggingface.co/{repo}/tree/{rev}",
        f"License: {meta['license']}; see LICENSE in this directory.",
        "Files:",
    ]
    for name, pinned in meta["files"].items():
        notice.append(f"  {name}  sha256={pinned['sha256']}  size={pinned['size']}")
    with open(os.path.join(role_dir, "NOTICE"), "w", encoding="utf-8") as f:
        f.write("\n".join(notice) + "\n")

    print(f"manifest ok {role}/manifest.json")
PY

echo "完成：$OUT_DIR/zh-select、$OUT_DIR/zh-punct" >&2
