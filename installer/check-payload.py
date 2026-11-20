#!/usr/bin/env python3
"""檢查 WiX 專案引用的檔案與暫存 payload 是否一致。

用途：`wix build` 之前先擋掉「安裝包漏檔」與「manifest 與檔案不符」。
這是開發與 CI 用的檢查，不進安裝包。Linux 與 Windows 都能跑。

用法：
    uv run --no-project installer/check-payload.py target/installer/payload

檢查項目：
1. `installer/jc-ime.wxs` 裡每個 `File/@Source` 指到的檔案都存在且不是 0 位元組。
2. `data/manifest.json` 的每個 `shipped: true` 條目都在 payload 裡，大小與
   SHA-256 都與 manifest 相同。
3. 每個 `shipped: true` 的 manifest 條目都被 WXS 引用；少一個就是安裝包漏檔。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

# WiX v5 的命名空間；v4 之後都是這個字串。
WIX_NS = "{http://wixtoolset.org/schemas/v4/wxs}"
PAYLOAD_VAR = "$(var.PayloadDir)"


def payload_relative(source: str) -> str | None:
    """把 `$(var.PayloadDir)/a/b` 轉成 `a/b`；其他變數回 None。"""
    if not source.startswith(PAYLOAD_VAR):
        return None
    relative = source[len(PAYLOAD_VAR) :].lstrip("/\\")
    return relative.replace("\\", "/")


def wxs_files(wxs: Path) -> list[tuple[str, str, str]]:
    """回傳 (File/@Id, 相對路徑, 原始 Source)。"""
    tree = ET.parse(wxs)
    result = []
    for element in tree.getroot().iter(f"{WIX_NS}File"):
        source = element.get("Source")
        if not source:
            continue
        relative = payload_relative(source)
        if relative is None:
            continue
        result.append((element.get("Id", "?"), relative, source))
    return result


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "payload",
        nargs="?",
        default="target/installer/payload",
        help="暫存的 payload 根目錄（預設 target/installer/payload）",
    )
    parser.add_argument(
        "--wxs",
        default="installer/jc-ime.wxs",
        help="WiX 專案檔（預設 installer/jc-ime.wxs）",
    )
    args = parser.parse_args()

    payload = Path(args.payload)
    wxs = Path(args.wxs)
    errors: list[str] = []
    checked = 0

    if not payload.is_dir():
        print(f"找不到 payload 目錄：{payload}", file=sys.stderr)
        return 2
    if not wxs.is_file():
        print(f"找不到 WiX 專案：{wxs}", file=sys.stderr)
        return 2

    referenced: set[str] = set()
    for file_id, relative, _source in wxs_files(wxs):
        referenced.add(relative)
        path = payload / relative
        checked += 1
        if not path.is_file():
            errors.append(f"缺少 {relative}（File Id {file_id}）")
        elif path.stat().st_size == 0:
            errors.append(f"{relative} 是 0 位元組（File Id {file_id}）")

    manifest_path = payload / "data" / "manifest.json"
    if not manifest_path.is_file():
        errors.append("缺少 data/manifest.json")
    else:
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"data/manifest.json 無法解析：{error}")
            manifest = None
        if manifest is not None:
            shipped = [entry for entry in manifest.get("files", []) if entry.get("shipped")]
            for entry in shipped:
                relative = entry["path"]
                path = payload / "data" / relative
                if not path.is_file():
                    errors.append(f"缺少隨附檔 data/{relative}")
                    continue
                size = path.stat().st_size
                if size != entry["bytes"]:
                    errors.append(
                        f"data/{relative} 大小不符：manifest {entry['bytes']}，實際 {size}"
                    )
                    continue
                actual = sha256(path)
                if actual != entry["sha256"]:
                    errors.append(
                        f"data/{relative} SHA-256 不符：manifest {entry['sha256']}，實際 {actual}"
                    )
                if f"data/{relative}" not in referenced:
                    errors.append(f"data/{relative} 在 manifest 內隨附，但 WiX 沒有引用")

    print(f"WXS 引用檔案：{checked}")
    if errors:
        for error in errors:
            print(f"錯誤：{error}", file=sys.stderr)
        print(f"共 {len(errors)} 個錯誤", file=sys.stderr)
        return 1
    print("payload 檢查通過")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
