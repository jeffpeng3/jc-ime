#!/usr/bin/env python3
"""產生 Big5 → Unicode 查表資料。

來源是 CPython 的 ``big5`` codec（``Modules/cjkcodecs``）。這個 codec 對
Unicode 官方 BIG5.TXT 標成「NO MAPPING」的 7 個碼位改用 CP950 的映射
（A15A、A1C3、A1C5、A1FE、A240、A2CC、A2CE），和微軟 Big5 產品一致。

輸出是 fixed-width 的 u16 小端陣列，索引算法在
``crates/jc-core/src/big5.rs``：

    lead 0xA1..=0xF9，每列 157 個 trail
    trail 0x40..=0x7E（63 個）接著 0xA1..=0xFE（94 個）
    每格 2 bytes，0 代表沒有對應

只收 Big5 合法區間 0xA140–0xF9FE 內、可解碼成單一 BMP 字元的碼位。
輸出固定排序、沒有時間戳，重跑會得到相同 bytes。

用法：
    scripts/gen-big5-table.py            # 寫入資料檔
    scripts/gen-big5-table.py --check    # 只比對現有檔案，不同就 exit 1
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

LEAD_MIN = 0xA1
LEAD_MAX = 0xF9
TRAIL_SLOTS = 63 + 94
SLOT_COUNT = (LEAD_MAX - LEAD_MIN + 1) * TRAIL_SLOTS

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "jc-core" / "data" / "big5" / "big5.bin"


def trail_column(trail: int) -> int | None:
    if 0x40 <= trail <= 0x7E:
        return trail - 0x40
    if 0xA1 <= trail <= 0xFE:
        return 63 + (trail - 0xA1)
    return None


def build_table() -> tuple[bytes, int]:
    slots = [0] * SLOT_COUNT
    mapped = 0
    for lead in range(LEAD_MIN, LEAD_MAX + 1):
        for trail in list(range(0x40, 0x7F)) + list(range(0xA1, 0xFF)):
            col = trail_column(trail)
            assert col is not None
            code = bytes((lead, trail))
            try:
                text = code.decode("big5")
            except UnicodeDecodeError:
                continue
            if len(text) != 1:
                raise SystemExit(f"{code.hex()} 解碼成 {len(text)} 個字元，預期 1 個")
            scalar = ord(text)
            if scalar > 0xFFFF:
                raise SystemExit(f"{code.hex()} → U+{scalar:04X}，超出 BMP，資料格式要改")
            if scalar == 0:
                raise SystemExit(f"{code.hex()} → U+0000，和未對應的 sentinel 衝突")
            index = (lead - LEAD_MIN) * TRAIL_SLOTS + col
            slots[index] = scalar
            mapped += 1

    data = bytearray()
    for scalar in slots:
        data += scalar.to_bytes(2, "little")
    return bytes(data), mapped


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="只比對現有檔案；內容不同就 exit 1",
    )
    args = parser.parse_args()

    data, mapped = build_table()
    digest = sha256(data)
    rel = OUT.relative_to(ROOT)

    if args.check:
        try:
            current = OUT.read_bytes()
        except FileNotFoundError:
            print(f"{rel} 不存在", file=sys.stderr)
            return 1
        if current != data:
            print(
                f"{rel} 和產生結果不同（現有 {len(current)} bytes，"
                f"預期 {len(data)} bytes）",
                file=sys.stderr,
            )
            return 1
        print(f"{rel} OK：{mapped} 個映射，{len(data)} bytes，sha256 {digest}")
        return 0

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(data)
    print(f"寫入 {rel}：{mapped} 個映射，{len(data)} bytes，sha256 {digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
