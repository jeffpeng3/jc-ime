#!/usr/bin/env python3
"""產生 `legal/RUST-THIRD-PARTY-NOTICES.md`：散布的 Rust 依賴授權清單與全文。

範圍是「真的會進到安裝包」的 crate：

- 根是實際散布的二進位所在 crate：`jc-tsf`（兩個架構的 DLL）、`jc-engine`、
  `jc-cli`、`jc-settings-ui`、`jc-tsf-smoke`。
- 只沿著**正常依賴**走（`dep_kinds` 的 `kind` 是 `null`）：dev-dependency 只在
  測試時用、build-dependency 只在建置時跑，兩者都不會出現在安裝包裡。
- 排除 proc-macro crate：它們是編譯期程式，不會連進任何一支散布的執行檔。
- 目標平台取聯集：`x86_64-pc-windows-msvc`、`i686-pc-windows-msvc`（32 位元 TSF
  DLL）、`x86_64-unknown-linux-gnu`（Linux tar.gz）。同一份 Cargo.lock 在不同平台
  會解析出不同的條件依賴，只算一種平台會漏。

授權全文直接從 cargo registry 的 `.crate` 壓縮檔讀，不讀 `registry/src`：後者要
建置過才存在，而且各機器狀態不同；`.crate` 由 `cargo fetch` 保證存在，內容與
crates.io 上的位元組相同，輸出才可重現。

用法：

    python3 scripts/gen-rust-notices.py           # 重新產生檔案
    python3 scripts/gen-rust-notices.py --check   # 只比對，不一致就回 1
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUTPUT = REPO / "legal" / "RUST-THIRD-PARTY-NOTICES.md"

# 三個散布目標；key 只用來說明，實際傳給 cargo 的就是字串本身。
TARGETS = (
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)

# 實際進安裝包的 crate（見 installer/jc.iss 的 [Files]）。
SHIPPED = ("jc-tsf", "jc-engine", "jc-cli", "jc-settings-ui", "jc-tsf-smoke")

# 條文本身不含著作權人的授權：上架包沒附授權檔時，可以安全地引用同樣條款的
# 其他 crate 的全文。MIT／BSD／ISC 的全文開頭就是著作權人，不能這樣借用。
GENERIC_LICENSES = ("Apache-2.0", "Unicode-3.0")

# 授權檔的檔名樣式；只取 crate 根目錄那一層，不管子目錄裡的範例或測試資料。
LICENSE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "UNLICENSE", "NOTICE")


def cargo(*args: str) -> str:
    """跑 cargo；沒有 cargo 在 PATH 就改用 `mise exec -- cargo`（見 AGENTS.md）。"""
    command = ["cargo", *args]
    if shutil.which("cargo") is None:
        command = ["mise", "exec", "--", *command]
    result = subprocess.run(
        command,
        cwd=REPO,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if result.returncode != 0:
        raise SystemExit(
            f"cargo {' '.join(args)} 失敗（exit {result.returncode}）：\n{result.stderr}"
        )
    return result.stdout


def cargo_home() -> Path:
    if value := os.environ.get("CARGO_HOME"):
        return Path(value)
    return Path.home() / ".cargo"


def metadata(target: str) -> dict:
    """取某個目標平台解析後的依賴圖（不建置，只讀 manifest 與 lock）。"""
    return json.loads(
        cargo("metadata", "--format-version", "1", "--locked", "--filter-platform", target)
    )


def normal_deps(node: dict) -> list[str]:
    """一個節點的正常依賴；`dep_kinds` 的 `kind` 是 None 代表正常依賴。"""
    result = []
    for dep in node.get("deps", []):
        kinds = {kind.get("kind") for kind in dep.get("dep_kinds", [])}
        if None in kinds or not kinds:
            result.append(dep["pkg"])
    return result


def is_proc_macro(package: dict) -> bool:
    return any("proc-macro" in target["kind"] for target in package["targets"])


def closure() -> dict[str, dict]:
    """所有散布 crate 的正常依賴聯集（不含 dev／build／proc-macro）。"""
    packages: dict[str, dict] = {}
    edges: dict[str, set[str]] = {}
    for target in TARGETS:
        data = metadata(target)
        for package in data["packages"]:
            packages[package["id"]] = package
        for node in data["resolve"]["nodes"]:
            edges.setdefault(node["id"], set()).update(normal_deps(node))

    roots = [pid for pid, package in packages.items() if package["name"] in SHIPPED]
    missing = set(SHIPPED) - {packages[pid]["name"] for pid in roots}
    if missing:
        raise SystemExit(f"找不到散布的 crate：{', '.join(sorted(missing))}")

    seen: set[str] = set()
    stack = list(roots)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        stack.extend(edges.get(pid, ()))

    result = {}
    for pid in seen:
        package = packages[pid]
        if not package["source"]:
            continue  # 本專案自己的 crate（AGPL-3.0-only，見 LICENSE）
        if is_proc_macro(package):
            continue
        result[pid] = package
    return result


def spdx_ids(expression: str) -> list[str]:
    """從 SPDX 運算式取出授權 id（去掉 AND／OR／WITH 的修飾）。"""
    words = expression.replace("(", " ").replace(")", " ").split()
    return [word for word in words if word not in ("AND", "OR", "WITH")]


def ident(package: dict) -> tuple[str, str]:
    """crate 的名稱與版本。"""
    return package["name"], package["version"]


def ident_key(package: dict) -> tuple[str, str]:
    """清單排序用的鍵：先名稱再版本。"""
    name, version = ident(package)
    return name, version


def crate_archive(name: str, version: str) -> Path:
    """`~/.cargo/registry/cache/<index>/<name>-<version>.crate`。"""
    root = cargo_home() / "registry" / "cache"
    matches = sorted(root.glob(f"*/{name}-{version}.crate"))
    if not matches:
        raise SystemExit(
            f"找不到 {name}-{version}.crate；先跑 `cargo fetch --target ...` 再產生"
        )
    if len(matches) > 1:
        raise SystemExit(f"{name}-{version}.crate 有多份：{matches}")
    return matches[0]


def license_texts(name: str, version: str) -> dict[str, bytes]:
    """回傳 crate 根目錄的授權檔（檔名 → 內容）。"""
    texts: dict[str, bytes] = {}
    with tarfile.open(crate_archive(name, version), mode="r:gz") as archive:
        for member in archive.getmembers():
            if not member.isfile():
                continue
            # 壓縮檔內的頂層目錄固定是 `<name>-<version>`。
            path = Path(member.name)
            if len(path.parts) != 2:
                continue
            filename = path.parts[1]
            if not filename.upper().startswith(LICENSE_PREFIXES):
                continue
            handle = archive.extractfile(member)
            if handle is None:
                continue
            texts[filename] = normalize(handle.read())
    return texts


def normalize(text: bytes) -> bytes:
    """只做排版正規化：CRLF→LF、去行尾空白、去頭尾空行。

    crates.io 上的同一份 Apache-2.0 有十幾種排版（縮排寬度、CRLF、開頭多一個空
    行）。這些差異不影響條文，卻是去重的最大障礙。條文本身一個字都不動。
    """
    text = text.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")
    lines = [line.rstrip() for line in text.split("\n")]
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return ("\n".join(lines) + "\n").encode()


def first_line(text: bytes) -> str:
    """授權全文的第一行非空內容，用來當章節標題。"""
    for raw in text.decode("utf-8", "replace").splitlines():
        line = " ".join(raw.split()).lstrip("#")
        if line.strip():
            return line.strip()[:72]
    return "（空白）"


def repository(package: dict) -> str:
    url = package.get("repository")
    if url:
        return url
    name, version = ident(package)
    return f"https://crates.io/crates/{name}/{version}"


def fetch() -> None:
    """把三個平台的 `.crate` 都抓到 `~/.cargo/registry/cache`。

    授權全文從壓縮檔讀，沒抓下來就沒有全文可讀；`--locked` 確保抓的是
    `Cargo.lock` 決定的版本。
    """
    for target in TARGETS:
        cargo("fetch", "--locked", "--target", target)


def build() -> str:
    fetch()
    packages = closure()

    # 全文去重：同一份位元組只留一次。key 是 SHA-256。
    texts: dict[str, bytes] = {}
    # 每個 crate 用了哪些全文（依檔名排序）。
    usage: dict[str, dict[str, str]] = {}
    for pid, package in packages.items():
        name, version = ident(package)
        files = license_texts(name, version)
        usage[pid] = {}
        for filename, content in sorted(files.items()):
            digest = hashlib.sha256(content).hexdigest()
            texts.setdefault(digest, content)
            usage[pid][filename] = digest

    # 每個授權 id 最常見的那份全文，用來補「上架包沒附授權檔」的 crate。
    by_id: dict[str, Counter[str]] = {}
    for pid, package in packages.items():
        if not usage[pid]:
            continue
        ids = spdx_ids(package.get("license") or "")
        for digest in set(usage[pid].values()):
            for identifier in ids:
                by_id.setdefault(identifier, Counter())[digest] += 1
    canonical = {identifier: counts.most_common(1)[0][0] for identifier, counts in by_id.items()}

    for pid, package in packages.items():
        if usage[pid]:
            continue
        for identifier in spdx_ids(package.get("license") or ""):
            if identifier not in GENERIC_LICENSES:
                continue
            digest = canonical.get(identifier)
            if digest is not None:
                usage[pid][f"（{identifier}，引用同樣條款）"] = digest

    ordered_texts = sorted(texts)
    section = {digest: index + 1 for index, digest in enumerate(ordered_texts)}

    lines: list[str] = []
    add = lines.append
    add("<!-- 由 scripts/gen-rust-notices.py 產生，不要手改。 -->")
    add("")
    add("# Rust 依賴授權")
    add("")
    add("安裝包裡的 DLL 與執行檔是靜態連結 Rust 相依後的產物，散布時要一併附上這些")
    add("相依的授權聲明。資料、模型與題庫的授權另外列在")
    add("[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md)，本專案自己的授權見")
    add("[`../LICENSE`](../LICENSE)（AGPL-3.0-only）。")
    add("")
    add("範圍：從 `Cargo.lock` 解析 `jc-tsf`、`jc-engine`、`jc-cli`、`jc-settings-ui`、")
    add("`jc-tsf-smoke` 的**正常依賴**，聯集")
    for target in TARGETS:
        add(f"`{target}`")
    add("三個目標平台。dev-dependency、build-dependency 與 proc-macro crate 不會進")
    add("安裝包，因此不列。")
    add("")
    add(f"共 {len(packages)} 個 crate、{len(texts)} 份不同的授權全文。")
    add("")
    add("上架包沒有附授權檔的 crate 只列 Cargo.toml 的宣告；條款本身不含著作權人時")
    add("（Apache-2.0、Unicode-3.0），改為引用同樣條款的其他 crate 的全文。")
    add("")
    add("重新產生：`python3 scripts/gen-rust-notices.py`；CI 用 `--check` 比對，")
    add("改了 `Cargo.toml`／`Cargo.lock` 卻沒重跑會讓 build 失敗。")
    add("")
    add("## 清單")
    add("")
    add("| crate | 版本 | 授權（Cargo.toml） | 授權全文 |")
    add("|---|---|---|---|")
    order = sorted(packages, key=lambda pid: ident_key(packages[pid]))
    for pid in order:
        package = packages[pid]
        name, version = ident(package)
        files = usage[pid]
        if files:
            # 不帶連結：標題裡有 § 與授權原文，手算 GitHub 的錨點容易失準，
            # 章節編號本身就夠查。
            refs = "、".join(
                f"§{section[digest]}" for digest in sorted(set(files.values()))
            )
        else:
            refs = "（這個 crate 的上架包沒有附授權檔）"
        add(f"| `{name}` | {version} | {package.get('license') or '未宣告'} | {refs} |")
    add("")
    add("## 授權全文")
    add("")
    add("同一份全文只列一次，因此多數 crate 共用同一個章節。標題取自全文的第一行。")
    add("全文只做排版正規化（CRLF→LF、去行尾空白、去頭尾空行），條文內容不動。")
    add("")
    for digest in ordered_texts:
        content = texts[digest]
        number = section[digest]
        add(f"### §{number} {first_line(content)}")
        add("")
        owners = [
            f"`{ident(packages[pid])[0]}` {ident(packages[pid])[1]}"
            for pid in order
            if digest in usage[pid].values()
        ]
        add(f"使用這個全文的 crate（{len(owners)} 個）：" + "、".join(owners))
        add("")
        add("```text")
        add(content.decode("utf-8", "replace").rstrip("\n"))
        add("```")
        add("")
    return "\n".join(lines).rstrip("\n") + "\n"



def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="只比對既有檔案，不一致就回 1（CI 用）",
    )
    options = parser.parse_args()

    generated = build()
    if options.check:
        if not OUTPUT.is_file():
            print(f"缺少 {OUTPUT.relative_to(REPO)}，先跑 scripts/gen-rust-notices.py", file=sys.stderr)
            return 1
        current = OUTPUT.read_text(encoding="utf-8")
        if current == generated:
            print(f"{OUTPUT.relative_to(REPO)} 與 Cargo.lock 一致")
            return 0
        print(
            f"{OUTPUT.relative_to(REPO)} 與 Cargo.lock 不一致，"
            "請跑 python3 scripts/gen-rust-notices.py 後重新提交",
            file=sys.stderr,
        )
        for old, new in zip(current.splitlines(), generated.splitlines()):
            if old != new:
                print(f"  第一個差異：\n  - {old}\n  + {new}", file=sys.stderr)
                break
        return 1

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(generated, encoding="utf-8")
    print(f"寫入 {OUTPUT.relative_to(REPO)}（{len(generated)} bytes）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
