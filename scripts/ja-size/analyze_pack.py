#!/usr/bin/env python3
"""量測 0CJADIC v2 pack 的內部組成，並產生可重跑的實驗變體。

用法（唯讀分析）：

    uv run --with zstandard scripts/ja-size/analyze_pack.py \
        /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack

產生變體 pack 給 `jc-cli eval ja --pack` 用：

    # 只留成本最低的 30 萬筆
    uv run --with zstandard scripts/ja-size/analyze_pack.py <pack> \
        --write-trim /tmp/opencode/ja-trim-300k.pack --keep 300000
    # 矩陣改成全域 8-bit 位移量化
    uv run --with zstandard scripts/ja-size/analyze_pack.py <pack> \
        --write-quant /tmp/opencode/ja-conn-u8.pack --quant global-u8
    # 每列各自 min/max 的 8-bit 量化（多存 row table）
    uv run --with zstandard scripts/ja-size/analyze_pack.py <pack> \
        --write-quant /tmp/opencode/ja-conn-row-u8.pack --quant row-u8

`--json` 把完整數字寫成 JSON，方便報告引用。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from array import array
from collections import Counter
from dataclasses import dataclass

HEADER_SIZE = 96
ENTRY_SIZE = 24
INDEX_ENTRY_SIZE = 8
TRAILER_SIZE = 32
MIN_COST = -32767

SECTIONS = (
    "surface_blob",
    "reading_blob",
    "entries",
    "index",
    "pos",
    "conj",
    "connection",
)


@dataclass
class Pack:
    raw: bytes
    entry_count: int
    surface_count: int
    pos_count: int
    conj_type_count: int
    conj_form_count: int
    context_count: int
    sections: dict[str, tuple[int, int]]

    def bytes_of(self, name: str) -> bytes:
        off, length = self.sections[name]
        return self.raw[off : off + length]

    def entries(self):
        off, _ = self.sections["entries"]
        for i in range(self.entry_count):
            base = off + i * ENTRY_SIZE
            surface_off, surface_len, reading_off, reading_len = struct.unpack_from(
                "<IHIH", self.raw, base
            )
            pos_id, conj_type, conj_form, cost, flags, left, right = struct.unpack_from(
                "<HBBhHHH", self.raw, base + 12
            )
            yield {
                "index": i,
                "surface_off": surface_off,
                "surface_len": surface_len,
                "reading_off": reading_off,
                "reading_len": reading_len,
                "pos_id": pos_id,
                "conj_type": conj_type,
                "conj_form": conj_form,
                "cost": cost,
                "flags": flags,
                "left": left,
                "right": right,
            }

    def surface(self, entry) -> str:
        blob = self.bytes_of("surface_blob")
        return blob[
            entry["surface_off"] : entry["surface_off"] + entry["surface_len"]
        ].decode("utf-8")

    def reading(self, entry) -> str:
        blob = self.bytes_of("reading_blob")
        return blob[
            entry["reading_off"] : entry["reading_off"] + entry["reading_len"]
        ].decode("utf-8")

    def matrix(self) -> array:
        data = self.bytes_of("connection")
        values = array("h")
        values.frombytes(data)
        if sys.byteorder != "little":
            values.byteswap()
        return values


def load_pack(path: str) -> Pack:
    raw = open(path, "rb").read()
    if raw[:8] != b"0CJADIC\0":
        raise SystemExit(f"{path}: magic 不是 0CJADIC")
    version, header_size = struct.unpack_from("<HH", raw, 8)
    if version != 2 or header_size != HEADER_SIZE:
        raise SystemExit(f"{path}: 只支援 v2，檔案是 v{version}/{header_size}")
    entry_count, surface_count = struct.unpack_from("<II", raw, 16)
    pos_count, conj_type_count, conj_form_count, context_count = struct.unpack_from(
        "<HHHH", raw, 24
    )
    sections = {}
    for i, name in enumerate(SECTIONS):
        off, length = struct.unpack_from("<II", raw, 32 + i * 8)
        sections[name] = (off, length)

    total = HEADER_SIZE + sum(length for _, length in sections.values()) + TRAILER_SIZE
    if total != len(raw):
        raise SystemExit(f"{path}: 版面加總 {total} 與檔案大小 {len(raw)} 不符")
    digest = hashlib.sha256(raw[:-TRAILER_SIZE]).digest()
    if digest != raw[-TRAILER_SIZE:]:
        raise SystemExit(f"{path}: trailer SHA-256 不符")
    return Pack(
        raw=raw,
        entry_count=entry_count,
        surface_count=surface_count,
        pos_count=pos_count,
        conj_type_count=conj_type_count,
        conj_form_count=conj_form_count,
        context_count=context_count,
        sections=sections,
    )


def cost_stats(entries: list[dict]) -> dict:
    costs = sorted(e["cost"] for e in entries)
    n = len(costs)
    thresholds = (0, 1000, 2000, 3000, 4000, 5000, 7000, 10000, 15000, 20000)
    out = {
        "n": n,
        "min": costs[0],
        "max": costs[-1],
        "mean": round(sum(costs) / n, 1),
        "p50": costs[n // 2],
        "p90": costs[int(n * 0.9)],
        "p99": costs[int(n * 0.99)],
        "thresholds": {},
        "duplicate_costs": len(costs) - len(set(costs)),
    }
    for threshold in thresholds:
        import bisect

        count = bisect.bisect_right(costs, threshold)
        out["thresholds"][str(threshold)] = count
    return out


def size_of_selection(pack: Pack, entries: list[dict]) -> dict:
    """留下 entries 時，依 pack 版面重算各 section 大小。

    surface blob 只留用到的唯一 surface（原始順序已排序），reading blob 逐筆
    累加，entries 固定 24 bytes，index 每個唯一 surface 8 bytes；POS／活用／
    矩陣不隨詞條減少。
    """
    seen_surface: set[int] = set()
    surface_bytes = 0
    reading_bytes = 0
    previous = None
    for e in entries:
        if e["surface_off"] != previous:
            # 同一 surface 的詞條共用 offset；只有第一次遇到才放進 blob。
            if e["surface_off"] not in seen_surface:
                seen_surface.add(e["surface_off"])
                surface_bytes += e["surface_len"]
            previous = e["surface_off"]
        reading_bytes += e["reading_len"]
    sections = {
        "surface_blob": surface_bytes,
        "reading_blob": reading_bytes,
        "entries": len(entries) * ENTRY_SIZE,
        "index": len(seen_surface) * INDEX_ENTRY_SIZE,
        "pos": pack.sections["pos"][1],
        "conj": pack.sections["conj"][1],
        "connection": pack.sections["connection"][1],
    }
    total = HEADER_SIZE + sum(sections.values()) + TRAILER_SIZE
    return {
        "entries": len(entries),
        "surfaces": len(seen_surface),
        "sections": sections,
        "total": total,
    }


def matrix_stats(matrix: array, context_count: int) -> dict:
    values = list(matrix)
    n = len(values)
    counts = Counter(values)
    top = counts.most_common(12)
    nonzero = [v for v in values if v != 0]
    pos = [v for v in values if v > 0]
    neg = [v for v in values if v < 0]

    def pct(part: int) -> float:
        return round(part * 100 / n, 3)

    return {
        "cells": n,
        "context_count": context_count,
        "bytes_i16": n * 2,
        "min": min(values),
        "max": max(values),
        "distinct_values": len(counts),
        "zero_cells": counts.get(0, 0),
        "zero_pct": pct(counts.get(0, 0)),
        "positive_cells": len(pos),
        "positive_pct": pct(len(pos)),
        "negative_cells": len(neg),
        "negative_pct": pct(len(neg)),
        "nonzero_min": min(nonzero) if nonzero else None,
        "nonzero_max": max(nonzero) if nonzero else None,
        "top_values": [
            {"value": value, "count": count, "pct": pct(count)} for value, count in top
        ],
        "within_i8_direct": len([v for v in values if -128 <= v <= 127]),
        "within_i8_direct_pct": pct(len([v for v in values if -128 <= v <= 127])),
    }


def quant_global_u8(matrix: array) -> tuple[array, dict]:
    lo, hi = min(matrix), max(matrix)
    scale = (hi - lo) / 255.0
    quant = bytearray(len(matrix))
    out = array("h", bytes(2 * len(matrix)))
    errors = []
    for i, v in enumerate(matrix):
        q = round((v - lo) / scale)
        q = 0 if q < 0 else 255 if q > 255 else q
        quant[i] = q
        deq = round(lo + q * scale)
        out[i] = deq
        errors.append(abs(deq - v))
    stats = error_stats(errors, "global-u8", extra={"lo": lo, "hi": hi, "scale": round(scale, 4)})
    stats["stored_bytes"] = len(quant) + 8
    return out, stats


def quant_row_u8(matrix: array, n: int) -> tuple[array, dict]:
    out = array("h", bytes(2 * len(matrix)))
    quant = bytearray(len(matrix))
    rows = []
    errors = []
    for row in range(n):
        base = row * n
        values = matrix[base : base + n]
        lo, hi = min(values), max(values)
        if hi == lo:
            rows.append((lo, 0.0))
            for col in range(n):
                out[base + col] = lo
            continue
        scale = (hi - lo) / 255.0
        rows.append((lo, scale))
        for col in range(n):
            v = values[col]
            q = round((v - lo) / scale)
            q = 0 if q < 0 else 255 if q > 255 else q
            quant[base + col] = q
            deq = round(lo + q * scale)
            out[base + col] = deq
            errors.append(abs(deq - v))
    stats = error_stats(errors, "row-u8", extra={})
    # 每列存 i16 offset + f32 scale = 6 bytes；實際格式可壓成 i16 + u16。
    stats["row_table_bytes"] = n * 6
    stats["stored_bytes"] = len(quant) + n * 6
    return out, stats


def quant_direct_i8(matrix: array) -> tuple[array, dict]:
    out = array("h", bytes(2 * len(matrix)))
    errors = []
    clamped = 0
    for i, v in enumerate(matrix):
        q = -128 if v < -128 else 127 if v > 127 else v
        if q != v:
            clamped += 1
        out[i] = q
        errors.append(abs(q - v))
    stats = error_stats(errors, "direct-i8", extra={"clamped_cells": clamped})
    stats["stored_bytes"] = len(matrix)
    return out, stats


def error_stats(errors: list[int], label: str, extra: dict) -> dict:
    n = len(errors)
    errors_sorted = sorted(errors)
    nonzero = [e for e in errors if e]
    out = {
        "scheme": label,
        "mean_abs": round(sum(errors) / n, 3),
        "max_abs": errors_sorted[-1],
        "p50_abs": errors_sorted[n // 2],
        "p99_abs": errors_sorted[int(n * 0.99)],
        "cells_changed": len(nonzero),
        "cells_changed_pct": round(len(nonzero) * 100 / n, 2),
        "mean_abs_changed": round(sum(nonzero) / len(nonzero), 3) if nonzero else 0.0,
    }
    out.update(extra)
    return out


def encode_pack(
    pack: Pack,
    entries: list[dict],
    matrix: array | None,
    connection_extra_bytes: int,
) -> bytes:
    """依原 pack 的版面重寫一份。

    entries 必須維持原本順序（surface 已排序）。surface blob 用原始 offset
    重建；reading 逐筆重排。matrix 給 None 時沿用原始位元組。
    """
    surface_blob = bytearray()
    reading_blob = bytearray()
    raw_entries = bytearray()
    index = bytearray()
    previous_surface_off = None
    current_surface_off = 0
    source_surfaces = pack.bytes_of("surface_blob")
    source_readings = pack.bytes_of("reading_blob")

    for e in entries:
        if e["surface_off"] != previous_surface_off:
            current_surface_off = len(surface_blob)
            surface_blob += source_surfaces[
                e["surface_off"] : e["surface_off"] + e["surface_len"]
            ]
            index += struct.pack("<II", current_surface_off, e.get("_new_index", e["index"]))
            previous_surface_off = e["surface_off"]
        reading_off = len(reading_blob)
        reading_blob += source_readings[
            e["reading_off"] : e["reading_off"] + e["reading_len"]
        ]
        raw_entries += struct.pack(
            "<IHIHHBBhHHH",
            current_surface_off,
            e["surface_len"],
            reading_off,
            e["reading_len"],
            e["pos_id"],
            e["conj_type"],
            e["conj_form"],
            e["cost"],
            e["flags"],
            e["left"],
            e["right"],
        )

    if matrix is None:
        connection = bytearray(pack.bytes_of("connection"))
    else:
        connection = bytearray(struct.pack(f"<{len(matrix)}h", *matrix))
        assert len(connection) == len(matrix) * 2

    payload = bytearray()
    offsets = {}
    for name, data in (
        ("surface_blob", surface_blob),
        ("reading_blob", reading_blob),
        ("entries", raw_entries),
        ("index", index),
        ("pos", pack.bytes_of("pos")),
        ("conj", pack.bytes_of("conj")),
        ("connection", connection),
    ):
        offsets[name] = (HEADER_SIZE + len(payload), len(data))
        payload += data

    header = bytearray(HEADER_SIZE)
    header[0:8] = b"0CJADIC\0"
    struct.pack_into("<HH", header, 8, 2, HEADER_SIZE)
    struct.pack_into("<I", header, 12, 0)
    struct.pack_into("<II", header, 16, len(entries), len(index) // INDEX_ENTRY_SIZE)
    struct.pack_into(
        "<HHHH",
        header,
        24,
        pack.pos_count,
        pack.conj_type_count,
        pack.conj_form_count,
        pack.context_count,
    )
    for i, name in enumerate(SECTIONS):
        off, length = offsets[name]
        struct.pack_into("<II", header, 32 + i * 8, off, length)

    body = bytes(header) + bytes(payload)
    return body + hashlib.sha256(body).digest()


def section_report(pack: Pack) -> list[dict]:
    total = len(pack.raw)
    rows = []
    labels = {
        "surface_blob": "詞面表",
        "reading_blob": "讀音表",
        "entries": "詞條固定欄位",
        "index": "詞面索引",
        "pos": "POS 表",
        "conj": "活用表",
        "connection": "連接成本矩陣",
    }
    for name in SECTIONS:
        off, length = pack.sections[name]
        rows.append(
            {
                "section": name,
                "label": labels[name],
                "offset": off,
                "bytes": length,
                "mb": round(length / 1_000_000, 3),
                "pct": round(length * 100 / total, 2),
            }
        )
    rows.append(
        {
            "section": "trailer",
            "label": "SHA-256 trailer",
            "offset": total - TRAILER_SIZE,
            "bytes": TRAILER_SIZE,
            "mb": 0.0,
            "pct": 0.0,
        }
    )
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("pack")
    parser.add_argument("--json")
    parser.add_argument("--keep", type=int, help="只留成本最低的 N 筆，寫出 trimmed pack")
    parser.add_argument("--write-trim")
    parser.add_argument("--write-quant")
    parser.add_argument(
        "--quant", choices=("global-u8", "row-u8", "direct-i8"), default="global-u8"
    )
    args = parser.parse_args()

    pack = load_pack(args.pack)
    entries = list(pack.entries())
    matrix = pack.matrix()
    n = pack.context_count

    report = {
        "pack": args.pack,
        "bytes": len(pack.raw),
        "mb": round(len(pack.raw) / 1_000_000, 3),
        "header": {
            "format_version": 2,
            "entries": pack.entry_count,
            "unique_surfaces": pack.surface_count,
            "pos_count": pack.pos_count,
            "conj_type_count": pack.conj_type_count,
            "conj_form_count": pack.conj_form_count,
            "context_count": pack.context_count,
        },
        "sections": section_report(pack),
        "cost": cost_stats(entries),
        "matrix": matrix_stats(matrix, n),
    }

    # surface／reading 實際字元數與共用情形。
    surface_lengths = Counter()
    reading_lengths = Counter()
    seen_surface = set()
    shared = 0
    for e in entries:
        if e["surface_off"] in seen_surface:
            shared += 1
        seen_surface.add(e["surface_off"])
        surface_lengths[e["surface_len"]] += 1
        reading_lengths[e["reading_len"]] += 1
    report["strings"] = {
        "entries_sharing_surface": shared,
        "unique_surfaces": len(seen_surface),
        "surface_bytes": pack.sections["surface_blob"][1],
        "reading_bytes": pack.sections["reading_blob"][1],
        "avg_surface_bytes": round(pack.sections["surface_blob"][1] / len(seen_surface), 2),
        "avg_reading_bytes": round(pack.sections["reading_blob"][1] / len(entries), 2),
    }

    # 只留成本最低 N 筆的尺寸。
    by_cost = sorted(entries, key=lambda e: (e["cost"], e["surface_off"]))
    trims = {}
    for keep in (100_000, 300_000, 500_000, 800_000):
        selection = by_cost[:keep]
        stats = size_of_selection(pack, selection)
        stats["saved_mb"] = round((len(pack.raw) - stats["total"]) / 1_000_000, 3)
        stats["cost_max"] = selection[-1]["cost"]
        trims[str(keep)] = stats
    report["trim"] = trims

    # 成本門檻以下全留。
    for threshold in (1000, 3000, 5000, 10000):
        selection = [e for e in entries if e["cost"] <= threshold]
        stats = size_of_selection(pack, selection)
        stats["saved_mb"] = round((len(pack.raw) - stats["total"]) / 1_000_000, 3)
        report.setdefault("threshold_trim", {})[str(threshold)] = stats

    # 量化誤差。
    quantized, global_stats = quant_global_u8(matrix)
    row_quantized, row_stats = quant_row_u8(matrix, n)
    direct_quantized, direct_stats = quant_direct_i8(matrix)
    report["quantization"] = {
        "global-u8": global_stats,
        "row-u8": row_stats,
        "direct-i8": direct_stats,
    }

    if args.write_trim:
        keep = args.keep or 300_000
        selection = sorted(by_cost[:keep], key=lambda e: e["index"])
        for i, e in enumerate(selection):
            e["_new_index"] = i
        data = encode_pack(pack, selection, None, 0)
        open(args.write_trim, "wb").write(data)
        print(f"寫出 trimmed pack：{args.write_trim}（{len(data)/1_000_000:.2f} MB）")

    if args.write_quant:
        if args.quant == "global-u8":
            replacement, qstats = global_stats, quantized
        elif args.quant == "row-u8":
            replacement, qstats = row_stats, row_quantized
        else:
            replacement, qstats = direct_stats, direct_quantized
        # 變體仍存成 i16 矩陣，才能直接餵給現有引擎；額外 row table 不寫。
        data = encode_pack(pack, entries, replacement, 0)
        open(args.write_quant, "wb").write(data)
        print(
            f"寫出 {args.quant} pack：{args.write_quant}（{len(data)/1_000_000:.2f} MB，"
            f"mean_abs={qstats['mean_abs']}，max_abs={qstats['max_abs']}）"
        )

    # 人看的摘要。
    print(f"pack: {args.pack}  {report['mb']:.1f} MB  {pack.entry_count:,} 詞條")
    print(f"{'section':<16} {'MB':>8} {'pct':>7}")
    for row in section_report(pack):
        print(f"{row['label']:<16} {row['mb']:>8.2f} {row['pct']:>6.2f}%")
    cost = report["cost"]
    print(
        f"cost min/max/mean/p50/p90 = {cost['min']}/{cost['max']}/{cost['mean']}/"
        f"{cost['p50']}/{cost['p90']}"
    )
    for threshold, count in cost["thresholds"].items():
        print(f"  cost <= {threshold:>6}: {count:>9,} 筆")
    print("只留成本最低 N 筆：")
    for keep, stats in trims.items():
        print(
            f"  {keep:>7}: {stats['total']/1_000_000:>7.2f} MB，"
            f"省 {stats['saved_mb']:>6.2f} MB，成本上限 {stats['cost_max']}"
        )
    print("矩陣：")
    m = report["matrix"]
    print(
        f"  {m['cells']:,} 格，min/max={m['min']}/{m['max']}，"
        f"0 佔 {m['zero_pct']}%，不同值 {m['distinct_values']}"
    )
    for scheme, stats in report["quantization"].items():
        print(
            f"  {scheme:<10} mean_abs={stats['mean_abs']:<7} max_abs={stats['max_abs']:<6} "
            f"p99={stats['p99_abs']:<6} 變動 {stats['cells_changed_pct']}% "
            f"儲存 {stats['stored_bytes']/1_000_000:.2f} MB"
        )

    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
        print(f"JSON：{args.json}")


if __name__ == "__main__":
    main()
