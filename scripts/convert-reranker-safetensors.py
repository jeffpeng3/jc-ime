#!/usr/bin/env python3
"""把日文 reranker 的 PyTorch checkpoint 轉成 candle 用的 safetensors。

checkpoint 的 state_dict 名稱與 `oc_training.model.reranker.Reranker` 一致，
candle 端照同名讀取，所以這裡只做 dtype 正規化與 metadata 記錄：

    python3 scripts/convert-reranker-safetensors.py \
        --checkpoint /mnt/work/0c/runs/p2-23f-extra-finetune/best.pt \
        --output /mnt/work/jc/jc-ime-data/models/reranker-extra/model.safetensors

輸出一律 fp32。模型檔不進 git，這個腳本只負責重現轉檔。
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import torch
from safetensors.torch import save_file


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--dtype",
        default="float32",
        choices=("float32", "float16"),
        help="輸出 dtype；candle 端用 fp32 對齊 ONNX fp32。",
    )
    args = parser.parse_args()

    payload = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    if "model" not in payload:
        raise SystemExit(f"{args.checkpoint}: checkpoint 沒有 model")
    state = payload["model"]
    dtype = getattr(torch, args.dtype)

    tensors = {}
    for name, value in state.items():
        if not isinstance(value, torch.Tensor):
            raise SystemExit(f"{name}: 不是 tensor")
        tensors[name] = value.detach().to(dtype).contiguous().clone()

    config = (payload.get("config") or {}).get("model") or {}
    metadata = {
        "format": "pt",
        "source_checkpoint": str(args.checkpoint),
        "source_sha256": _sha256(args.checkpoint),
        "model_config": json.dumps(config, sort_keys=True),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    save_file(tensors, str(args.output), metadata=metadata)

    parameters = sum(value.numel() for value in tensors.values())
    print(
        f"wrote {args.output} ({args.output.stat().st_size} bytes, "
        f"{len(tensors)} tensors, {parameters} parameters)"
    )


def _sha256(path: Path) -> str:
    import hashlib

    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


if __name__ == "__main__":
    main()
