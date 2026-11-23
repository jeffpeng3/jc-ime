#!/usr/bin/env python3
"""檢查 Inno Setup 專案引用的檔案與暫存 payload 是否一致。

用途：`ISCC` 之前先擋掉「安裝包漏檔」與「manifest 與檔案不符」。
這是開發與 CI 用的檢查，不進安裝包。Linux 與 Windows 都能跑。

用法：
    uv run --no-project installer/check-payload.py target/installer/payload

檢查項目：
1. `installer/jc.iss` 的 `[Files]` 每個 `Source:` 指到的檔案都存在且不是
   0 位元組。
2. `data/manifest.json` 的每個 `shipped: true` 條目都在 payload 裡，大小與
   SHA-256 都與 manifest 相同。
3. 每個 `shipped: true` 的 manifest 條目都被 `jc.iss` 引用；少一個就是安裝包漏檔。
4. 授權文件清單（repo 的 `LICENSE`、`NOTICE`、`legal/THIRD-PARTY-NOTICES.md` 與
   `legal/licenses/` 全部）每一份都在 payload、且內容與 repo 相同；repo 有的檔案
   不能在 `jc.iss` 缺席。`legal/licenses/` 新增檔案而沒進安裝腳本時會失敗。
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import sys
from pathlib import Path


def force_utf8_stdio() -> None:
    """讓 Windows 的 stdout／stderr 用 UTF-8，避免 cp1252 遇到中文就爆。

    Python 3.7 以上有 TextIOWrapper.reconfigure；更舊的版本改用 buffer 重新
    包一層。stdout 被替換成沒有 buffer 的物件時（例如測試工具）就維持原狀。
    """
    for name in ("stdout", "stderr"):
        stream = getattr(sys, name)
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is not None:
            try:
                reconfigure(encoding="utf-8", errors="replace")
                continue
            except (AttributeError, OSError, ValueError):
                pass
        buffer = getattr(stream, "buffer", None)
        detach = getattr(stream, "detach", None)
        if detach is not None:
            try:
                buffer = detach()
            except (AttributeError, OSError, ValueError):
                pass
        if buffer is None:
            continue
        line_buffering = getattr(stream, "line_buffering", False)
        setattr(
            sys,
            name,
            io.TextIOWrapper(
                buffer,
                encoding="utf-8",
                errors="replace",
                line_buffering=line_buffering,
            ),
        )


force_utf8_stdio()

PAYLOAD_VAR = "{#PayloadDir}"
REPO_ROOT = Path(__file__).resolve().parent.parent

# `[Files]` 的 Source 行；只認 payload 變數開頭的路徑。
SOURCE_RE = re.compile(
    r"^\s*Source:\s*\""
    + re.escape(PAYLOAD_VAR)
    + r"[\\/]+([^\"]+)\"",
    re.MULTILINE,
)

# 授權文件在 repo 與 payload 使用同一個相對路徑；jc.iss 也照這個路徑引用。
LEGAL_FIXED = ("LICENSE", "NOTICE", "legal/THIRD-PARTY-NOTICES.md")


def iss_files(iss: Path) -> list[tuple[str, str]]:
    """回傳 (相對路徑, 原始 Source 片段)。"""
    text = iss.read_text(encoding="utf-8-sig")
    result: list[tuple[str, str]] = []
    for match in SOURCE_RE.finditer(text):
        relative = match.group(1).replace("\\", "/").strip()
        result.append((relative, match.group(0).strip()))
    return result


def legal_files() -> list[str]:
    """授權文件的完整清單；`legal/licenses/` 以目錄實際內容為準。"""
    licenses_dir = REPO_ROOT / "legal" / "licenses"
    nested = sorted(
        path.relative_to(REPO_ROOT).as_posix()
        for path in licenses_dir.rglob("*")
        if path.is_file()
    )
    return list(LEGAL_FIXED) + nested


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_legal(payload: Path, referenced: set[str], errors: list[str]) -> int:
    """比對 repo、payload 與 jc.iss 三邊的授權文件清單。回傳檢查過的檔案數。"""
    required = legal_files()
    checked = 0
    for relative in required:
        checked += 1
        source = REPO_ROOT / relative
        if not source.is_file():
            errors.append(f"repo 缺少授權文件 {relative}")
            continue
        if relative not in referenced:
            errors.append(f"授權文件 {relative} 沒有被 jc.iss 引用")
        staged = payload / relative
        if not staged.is_file():
            errors.append(f"payload 缺少授權文件 {relative}")
            continue
        if staged.stat().st_size != source.stat().st_size:
            errors.append(
                f"授權文件 {relative} 大小不符：repo {source.stat().st_size}，"
                f"payload {staged.stat().st_size}"
            )
            continue
        if sha256(staged) != sha256(source):
            errors.append(f"授權文件 {relative} 內容與 repo 不同")
    return checked


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "payload",
        nargs="?",
        default="target/installer/payload",
        help="暫存的 payload 根目錄（預設 target/installer/payload）",
    )
    parser.add_argument(
        "--iss",
        default=str(REPO_ROOT / "installer" / "jc.iss"),
        help="Inno Setup 腳本（預設 installer/jc.iss）",
    )
    args = parser.parse_args()

    payload = Path(args.payload)
    iss = Path(args.iss)
    errors: list[str] = []
    checked = 0

    if not payload.is_dir():
        print(f"找不到 payload 目錄：{payload}", file=sys.stderr)
        return 2
    if not iss.is_file():
        print(f"找不到 Inno Setup 腳本：{iss}", file=sys.stderr)
        return 2

    referenced: set[str] = set()
    for relative, source in iss_files(iss):
        referenced.add(relative)
        path = payload / relative
        checked += 1
        if not path.is_file():
            errors.append(f"缺少 {relative}（{source}）")
        elif path.stat().st_size == 0:
            errors.append(f"{relative} 是 0 位元組（{source}）")

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
                    errors.append(f"data/{relative} 在 manifest 內隨附，但 jc.iss 沒有引用")

    legal_checked = check_legal(payload, referenced, errors)

    print(f"jc.iss 引用檔案：{checked}")
    print(f"授權文件：{legal_checked}")
    if errors:
        for error in errors:
            print(f"錯誤：{error}", file=sys.stderr)
        print(f"共 {len(errors)} 個錯誤", file=sys.stderr)
        return 1
    print("payload 檢查通過")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
