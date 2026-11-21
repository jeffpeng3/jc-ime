#!/usr/bin/env python3
"""以 Python 重現 `jc-cli data verify <root>` 的輸出與 exit code。

讀 <root>/manifest.json，對每個檔案做 Full 驗證：先比大小，再比 SHA-256。
輸出（人讀與 `--json`）和錯誤訊息對齊 crates/jc-data/src/manifest.rs 與
crates/jc-cli/src/manifest_cmd.rs。CI 的 scripts/test-verify-bundle.sh 會逐情境
比對兩邊的 stdout 與 exit code，做為 schema 漂移的防線。

等價範圍（VerifyLevel::Full）：

- 檢查 manifest schema、路徑、sha256 格式、授權代號；順序與 Rust 的 validate 一致。
- `shipped: true` 缺席算錯誤，`shipped: false` 缺席算警告。
- 任何已存在檔案的大小或雜湊不符都算錯誤。
- 人讀輸出略過缺席的下載項，摘要也只算顯示出來的檢查；`--json` 輸出完整 report。

不等價範圍：VerifyLevel::Startup（只看大小）、`data manifest` 產生、下載項 URL
與授權內容檢查、installer/check-payload.py 的 payload 檢查；這些仍由各自的工具負責。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

SCHEMA = "0c.data-pack/1"
MANIFEST_FILE = "manifest.json"
ROLES = {
    "ja_lexicon",
    "zh_lexicon",
    "zh_lm",
    "english",
    "ja_neural",
    "ja_tokenizer",
    "zh_select",
}
HEX64 = re.compile(r"^[0-9a-f]{64}$")
LICENSE_ID = re.compile(r"^L-[0-9]{2}$")


class ManifestError(Exception):
    """manifest 讀取或格式錯誤，對應 Rust 的 ManifestError 顯示文字。"""


def sha256_file(path: Path) -> str:
    """以 64 KiB 區塊計算 SHA-256，與 Rust 的 sha256_file 相同。"""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1 << 16):
            digest.update(chunk)
    return digest.hexdigest()


def is_safe_rel_path(value: str) -> bool:
    """對應 Rust 的 is_safe_rel_path：相對、無反斜線、無 .／.. 元件。"""
    if not value or "\\" in value or value.startswith("/"):
        return False
    return all(part not in ("", ".", "..") for part in value.split("/"))


def validate_manifest(data: object) -> None:
    """對應 DataManifest::validate；順序與 Rust 一致，影響先報哪個錯。"""
    if not isinstance(data, dict):
        raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：JSON 根節點不是物件")
    if data.get("schema") != SCHEMA:
        raise ManifestError(
            f"manifest schema 不符：程式需要 {SCHEMA}，實際是 {data.get('schema')}"
        )
    files = data.get("files")
    if not isinstance(files, list) or not files:
        raise ManifestError("manifest 沒有列出任何檔案")
    seen: set[str] = set()
    for entry in files:
        if not isinstance(entry, dict):
            raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：files 元素不是物件")
        path = entry.get("path")
        if not isinstance(path, str) or not is_safe_rel_path(path):
            raise ManifestError(f"manifest 路徑不合法：{path}")
        if path in seen:
            raise ManifestError(f"manifest 路徑重複：{path}")
        seen.add(path)
        sha = entry.get("sha256")
        if not isinstance(sha, str) or not HEX64.fullmatch(sha):
            raise ManifestError(f"manifest 的 sha256 格式不合法：{path}")
        licenses = entry.get("license")
        if not isinstance(licenses, list) or not licenses:
            raise ManifestError(f"manifest 的授權條目不合法：{path} 的 ")
        for lic in licenses:
            if not isinstance(lic, str) or not LICENSE_ID.fullmatch(lic):
                raise ManifestError(f"manifest 的授權條目不合法：{path} 的 {lic}")
        if entry.get("role") not in ROLES:
            # Rust 由 serde 在解析階段拒絕未知 role；這裡 fail closed。
            raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：未知 role {entry.get('role')}")
        if not isinstance(entry.get("shipped"), bool):
            raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：shipped 不是布林值：{path}")
        if not isinstance(entry.get("bytes"), int) or entry["bytes"] < 0:
            raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：bytes 不是整數：{path}")


def load_manifest(path: Path) -> dict:
    """讀取並驗證 manifest。"""
    try:
        raw = path.read_bytes()
    except OSError as err:
        raise ManifestError(f"讀取 {path} 失敗：{err}") from err
    try:
        data = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as err:
        raise ManifestError(f"解析 {MANIFEST_FILE} 失敗：{err}") from err
    validate_manifest(data)
    return data


def check_file(root: Path, entry: dict) -> dict:
    """對應 check_file：產生一筆 FileCheck。"""
    rel = entry["path"]
    full = root / rel
    try:
        size = full.stat().st_size
    except FileNotFoundError:
        status = {"status": "missing"}
    except OSError as err:
        status = {"status": "read", "message": str(err)}
    else:
        if size != entry["bytes"]:
            status = {"status": "size", "expected": entry["bytes"], "actual": size}
        else:
            try:
                actual = sha256_file(full)
            except OSError as err:
                status = {"status": "read", "message": str(err)}
            else:
                status = (
                    {"status": "ok"}
                    if actual.lower() == entry["sha256"].lower()
                    else {
                        "status": "hash",
                        "expected": entry["sha256"],
                        "actual": actual,
                    }
                )
    return {
        "path": rel,
        "role": entry["role"],
        "shipped": entry["shipped"],
        **status,
    }


def is_error(check: dict) -> bool:
    """對應 FileCheck::is_error。"""
    status = check["status"]
    if status == "ok":
        return False
    if status == "missing":
        return check["shipped"]
    return True


def is_absent_download(check: dict) -> bool:
    """對應 is_absent_download。"""
    return check["status"] == "missing" and not check["shipped"]


def describe(check: dict) -> str:
    """對應 describe；空格數與 Rust 格式字串完全一致。"""
    status = check["status"]
    path = check["path"]
    role = check["role"]
    if status == "ok":
        return f"通過      {path}（{role}）"
    if status == "missing":
        if check["shipped"]:
            return f"缺少      {path}（{role}，隨附檔，錯誤）"
        return f"缺少      {path}（{role}，下載項，警告）"
    if status == "size":
        return (
            f"大小不符  {path}：manifest {check['expected']} 位元組，"
            f"實際 {check['actual']} 位元組"
        )
    if status == "hash":
        return f"雜湊不符  {path}：manifest {check['expected']}，實際 {check['actual']}"
    return f"讀取失敗  {path}：{check['message']}"


def to_report(checks: list[dict]) -> dict:
    """轉成 VerifyReport 的 JSON 形狀；serde 的 tag 一定產生嵌套 status 物件。"""
    result = []
    for check in checks:
        entry = {
            "path": check["path"],
            "role": check["role"],
            "shipped": check["shipped"],
        }
        status = check["status"]
        if status == "ok" or status == "missing":
            entry["status"] = {"status": status}
        elif status == "size":
            entry["status"] = {
                "status": "size",
                "expected": check["expected"],
                "actual": check["actual"],
            }
        elif status == "hash":
            entry["status"] = {
                "status": "hash",
                "expected": check["expected"],
                "actual": check["actual"],
            }
        else:
            entry["status"] = {"status": "read", "message": check["message"]}
        result.append(entry)
    return {"checks": result}


def summarize(checks: list[dict]) -> str:
    """對應 VerifyReport::summary。"""
    passed = sum(c["status"] == "ok" for c in checks)
    errors = sum(is_error(c) for c in checks)
    warnings = sum(
        c["status"] == "missing" and not c["shipped"] for c in checks
    )
    return f"通過 {passed}/{len(checks)}，錯誤 {errors}，警告 {warnings}"


def verify(root: Path, manifest_path: Path, as_json: bool, out) -> None:
    """對應 manifest_cmd::verify；失敗時丟出要給 main 印的訊息。"""
    data = load_manifest(manifest_path)
    checks = [check_file(root, entry) for entry in data["files"]]
    shown = [c for c in checks if not is_absent_download(c)]
    summary = summarize(shown)
    if as_json:
        print(json.dumps(to_report(checks), ensure_ascii=False, indent=2), file=out)
    else:
        for check in shown:
            print(describe(check), file=out)
        print(f"驗證 {manifest_path}：{summary}", file=out)
    if any(is_error(c) for c in checks):
        raise ManifestError(f"資料驗證失敗：{summary}")


def main(argv: list[str] | None = None) -> int:
    # Rust 的 stdout/stderr 不做換行変換，Windows 上 print() 預設會把 \n 寫成 \r\n。
    # 強制用 LF，確保與 jc-cli 的輸出位元組一致。
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(newline="\n")
    parser = argparse.ArgumentParser(description="驗證資料包 manifest")
    parser.add_argument("root", type=Path, help="資料根目錄")
    parser.add_argument("--manifest", type=Path, help="manifest 路徑，預設 <root>/manifest.json")
    parser.add_argument("--json", action="store_true", help="輸出完整 JSON report")
    args = parser.parse_args(argv)
    try:
        if not args.root.is_dir():
            raise ManifestError(f"資料根目錄不存在：{args.root}")
        manifest_path = args.manifest or args.root / MANIFEST_FILE
        verify(args.root, manifest_path, args.json, sys.stdout)
    except ManifestError as err:
        print(f"jc-cli: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
