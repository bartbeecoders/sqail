#!/usr/bin/env python3
"""Measure VRAM footprint for each benchmarked model.

Starts llama-server per model, lets it settle (warm-up /infill),
records `nvidia-smi --query-compute-apps=used_memory`, shuts down.
"""
from __future__ import annotations
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
# Reuse the harness's helpers so we don't duplicate boot logic.
from importlib import import_module
bench = import_module("inline-ai-benchmark")

ROOT = Path(__file__).resolve().parent.parent
RESULTS = ROOT / ".cache/inline-ai/vram.json"


def gpu_used_mib(pid: int) -> int:
    out = subprocess.check_output(
        ["nvidia-smi",
         "--query-compute-apps=pid,used_memory",
         "--format=csv,noheader,nounits"],
        text=True,
    )
    for line in out.strip().splitlines():
        line = line.strip()
        if not line:
            continue
        parts = [p.strip() for p in line.split(",")]
        if int(parts[0]) == pid:
            return int(parts[1])
    return 0


def main() -> int:
    results = []
    for m in bench.MODELS:
        gguf = bench.MODELS_DIR / m.gguf
        if not gguf.exists():
            print(f"skip {m.id}: {gguf.name} missing")
            continue
        proc, port = bench.start_server(m)
        try:
            # warm-up to fully allocate KV cache
            bench.infill_stream(port, bench.PROMPTS[0], n_predict=16)
            time.sleep(1)
            mib = gpu_used_mib(proc.pid)
            print(f"{m.id:<15} {mib} MiB")
            results.append({"id": m.id, "display": m.display, "vram_mib": mib})
        finally:
            bench.stop_server(proc)
            time.sleep(2)

    RESULTS.write_text(json.dumps(results, indent=2))
    print(f"wrote {RESULTS.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
