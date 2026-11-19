#!/usr/bin/env python3
"""把日文標點模型的 PyTorch checkpoint 轉成 candle 用的 safetensors。

checkpoint 的 `state_dict` 名稱與 `oc_training.model.punctuation.PunctuationTagger`
一致，candle 端照同名讀取，所以這裡只做 dtype 正規化與 metadata 記錄：

    python3 scripts/convert-punctuation-safetensors.py \
        --checkpoint /mnt/work/0c/runs/p2-24a-punctuation-public-core/best.pt \
        --output /mnt/work/jc/jc-ime-data/models/punctuation-public-core/model.safetensors

沒有 checkpoint 時可以用 `--onnx` 指定 fp32 ONNX，權重改從 initializer 取。
ONNX 匯出把 linear 權重轉置並匿名成 `onnx::MatMul_*`；它們在 initializer
清單裡的順序固定為每層 `in_proj`、`out_proj`、`linear1`、`linear2`，最後
是 `classifier`，腳本照這個順序還原成 checkpoint 名稱。輸出一律 fp32。
模型檔不進 git，這個腳本只負責重現轉檔。
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import torch
from safetensors.torch import save_file

# ONNX initializer 裡每層的 anonymous MatMul 順序與原始形狀（轉置前）。
LAYER_WEIGHTS = (
    ("self_attn.in_proj_weight", (1536, 512)),
    ("self_attn.out_proj.weight", (512, 512)),
    ("linear1.weight", (1024, 512)),
    ("linear2.weight", (512, 1024)),
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", type=Path)
    parser.add_argument("--onnx", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--dtype",
        default="float32",
        choices=("float32", "float16"),
        help="輸出 dtype；candle 端用 fp32 對齊 ONNX fp32。",
    )
    args = parser.parse_args()
    if bool(args.checkpoint) == bool(args.onnx):
        raise SystemExit("--checkpoint 與 --onnx 必須擇一")

    dtype = getattr(torch, args.dtype)
    if args.checkpoint:
        tensors, config, source = from_checkpoint(args.checkpoint, dtype)
    else:
        tensors, config, source = from_onnx(args.onnx, dtype)

    metadata = {
        "format": "pt" if args.checkpoint else "onnx",
        "source": str(source),
        "source_sha256": _sha256(source),
        "model_config": json.dumps(config, sort_keys=True),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    save_file(tensors, str(args.output), metadata=metadata)

    parameters = sum(value.numel() for value in tensors.values())
    print(
        f"wrote {args.output} ({args.output.stat().st_size} bytes, "
        f"{len(tensors)} tensors, {parameters} parameters)"
    )


def from_checkpoint(path: Path, dtype: torch.dtype):
    payload = torch.load(path, map_location="cpu", weights_only=False)
    if "model" not in payload:
        raise SystemExit(f"{path}: checkpoint 沒有 model")
    tensors = {}
    for name, value in payload["model"].items():
        if not isinstance(value, torch.Tensor):
            raise SystemExit(f"{name}: 不是 tensor")
        tensors[name] = value.detach().to(dtype).contiguous().clone()
    config = (payload.get("config") or {}).get("model") or {}
    return tensors, config, path


def from_onnx(path: Path, dtype: torch.dtype):
    import onnx
    from onnx import numpy_helper

    model = onnx.load(str(path))
    tensors: dict[str, torch.Tensor] = {}
    anonymous: list[torch.Tensor] = []
    for initializer in model.graph.initializer:
        value = torch.from_numpy(numpy_helper.to_array(initializer).copy())
        if initializer.name.startswith("onnx::MatMul_"):
            anonymous.append(value)
            continue
        tensors[initializer.name] = value.to(dtype).contiguous()

    layers = len(anonymous) // len(LAYER_WEIGHTS) - 1
    if len(anonymous) != layers * len(LAYER_WEIGHTS) + 1:
        raise SystemExit(f"anonymous MatMul 數量 {len(anonymous)} 不符預期")
    for index in range(layers):
        for offset, (suffix, shape) in enumerate(LAYER_WEIGHTS):
            value = anonymous[index * len(LAYER_WEIGHTS) + offset]
            if tuple(value.shape) != shape[::-1]:
                raise SystemExit(
                    f"layer {index} {suffix}: 形狀 {tuple(value.shape)}，預期 {shape[::-1]}"
                )
            tensors[f"encoder.layers.{index}.{suffix}"] = (
                value.t().to(dtype).contiguous()
            )
    classifier = anonymous[-1]
    if tuple(classifier.shape) != (4, 512):
        raise SystemExit(f"classifier 形狀 {tuple(classifier.shape)}，預期 (4, 512)")
    tensors["classifier.weight"] = classifier.t().to(dtype).contiguous()

    config = {
        "vocab_size": int(tensors["token_embedding.weight"].shape[0]),
        "d_model": int(tensors["token_embedding.weight"].shape[1]),
        "num_layers": layers,
        "num_heads": 8,
        "ffn_dim": int(tensors["encoder.layers.0.linear1.weight"].shape[0]),
        "max_positions": int(tensors["position_embedding.weight"].shape[0]),
    }
    return tensors, config, path


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


if __name__ == "__main__":
    main()
