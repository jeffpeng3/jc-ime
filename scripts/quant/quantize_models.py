#!/usr/bin/env python3
"""產生 int8／int4 量化模型到 models/quant/<model>/<precision>/。

原始 fp32 與官方 int8 檔不動；輸出只寫 quant 目錄。
int4 用 ORT 的 MatMulNBitsQuantizer（舊名 MatMul4BitsQuantizer）。
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

import onnx
from onnxruntime.quantization import QuantType, quantize_dynamic
from onnxruntime.quantization.matmul_nbits_quantizer import MatMulNBitsQuantizer

QUANT_ROOT = Path("/mnt/work/jc/jc-ime-data/models/quant")
MODELS_ROOT = Path("/mnt/work/jc/jc-ime-data/models")


def copy_sidecars(model: str) -> None:
    src = MODELS_ROOT / model
    dst = QUANT_ROOT / model
    for name in ("vocab.json", "manifest.json"):
        if (src / name).is_file():
            shutil.copy2(src / name, dst / name)


def quantize_int8(model: str, src: Path, dst: Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    quantize_dynamic(
        model_input=str(src),
        model_output=str(dst),
        weight_type=QuantType.QInt8,
        per_channel=False,
        reduce_range=False,
        extra_options={"EnableSubgraph": True},
    )


def quantize_int4(src: Path, dst: Path, block_size: int = 128) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    quantizer = MatMulNBitsQuantizer(
        model=str(src),
        bits=4,
        block_size=block_size,
        is_symmetric=False,
        op_types_to_quantize=("MatMul", "Gather"),
    )
    quantizer.process()
    quantizer.model.save_model_to_file(str(dst), use_external_data_format=False)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", choices=["ja-rerank", "ja-punct", "zh-select", "all"], default="all")
    parser.add_argument("--precision", choices=["int8", "int4", "all"], default="all")
    parser.add_argument("--block-size", type=int, default=128)
    args = parser.parse_args()

    models = ["ja-rerank", "ja-punct", "zh-select"] if args.model == "all" else [args.model]
    precisions = ["int8", "int4"] if args.precision == "all" else [args.precision]

    for model in models:
        copy_sidecars(model)
        src_fp32 = QUANT_ROOT / model / "fp32" / "model.onnx"
        src_int8 = QUANT_ROOT / model / "int8" / "model.onnx"
        if not src_fp32.is_file():
            print(f"跳過 {model}：缺少 {src_fp32}", file=sys.stderr)
            continue
        for precision in precisions:
            dst = QUANT_ROOT / model / precision / "model.onnx"
            if precision == "int8":
                if model in ("ja-rerank", "ja-punct"):
                    # 官方 int8 已是動態量化，直接沿用（已複製到 int8/）。
                    print(f"{model}/int8：沿用官方檔案 {src_int8}")
                    continue
                quantize_int8(model, src_fp32, dst)
            else:
                quantize_int4(src_fp32, dst, args.block_size)
            print(f"寫出 {dst}（{dst.stat().st_size:,} bytes）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
