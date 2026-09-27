#!/usr/bin/env python3
"""
Phase A benchmark harness for the inline AI spike.

For each GGUF model:
  1. Boot llama-server with GPU offload + the model.
  2. Wait for /health to report ready.
  3. Run a warm-up FIM request (to prime the KV cache so first-token
     latency reflects warm conditions, which is what the user experiences
     after the first few keystrokes).
  4. Run each benchmark prompt N times through /infill, measuring:
       - time to first token (TTFT) in ms
       - full response latency in ms
       - tokens generated
       - throughput (tok/s, measured after first token)
  5. Record the generated completion for later quality review.
  6. Shut the server down.

Writes JSON to .cache/inline-ai/results.json; the summary table in
Vibecoding/inline-ai-benchmarks.md is generated from that file.

Usage:
  scripts/inline-ai-benchmark.py [--only MODEL_ID] [--runs N]
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path
from typing import Iterator

import urllib.request
import urllib.error


ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / ".cache/inline-ai"
BIN = CACHE / "bin/llama-server"
MODELS_DIR = CACHE / "models"
RESULTS = CACHE / "results.json"


# ─── Models under test ────────────────────────────────────────────────────
@dataclasses.dataclass
class Model:
    id: str
    display: str
    gguf: str
    ctx: int = 4096
    # n_gpu_layers=-1 means offload all. llama.cpp interprets 999 the same way.
    gpu_layers: int = 999


MODELS: list[Model] = [
    Model("qwen-1_5b", "Qwen2.5-Coder-1.5B Q4_K_M",
          "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf"),
    Model("qwen-3b", "Qwen2.5-Coder-3B Q4_K_M",
          "qwen2.5-coder-3b-instruct-q4_k_m.gguf"),
    Model("qwen-7b", "Qwen2.5-Coder-7B Q4_K_M",
          "qwen2.5-coder-7b-instruct-q4_k_m.gguf"),
    Model("deepseek-16b", "DeepSeek-Coder-V2-Lite Q4_K_M",
          "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf", ctx=4096),
]


# ─── Prompts ──────────────────────────────────────────────────────────────
# These target the bookstore demo schema (see scripts/seed-demo-db.sh) so
# they're representative of queries a SQail user actually writes.
SCHEMA_CONTEXT = """\
-- Dialect: SQLite
-- Tables in scope:
-- authors(id INT PK, name TEXT NOT NULL, country TEXT NOT NULL, birth_year INT NOT NULL)
-- books(id INT PK, title TEXT NOT NULL, author_id INT NOT NULL REFERENCES authors(id),
--       genre TEXT NOT NULL, published_year INT NOT NULL, price REAL NOT NULL, stock INT NOT NULL)
-- customers(id INT PK, name TEXT NOT NULL, email TEXT NOT NULL UNIQUE, city TEXT NOT NULL, joined_at TEXT NOT NULL)
-- orders(id INT PK, customer_id INT REFERENCES customers(id), book_id INT REFERENCES books(id),
--        quantity INT NOT NULL, order_date TEXT NOT NULL, total_price REAL NOT NULL)
-- reviews(id INT PK, book_id INT REFERENCES books(id), customer_id INT REFERENCES customers(id),
--         rating INT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL)
"""


@dataclasses.dataclass
class Prompt:
    id: str
    scenario: str
    prefix: str
    suffix: str = ""


PROMPTS: list[Prompt] = [
    Prompt(
        "p01_select_start",
        "Start a SELECT from a known table",
        SCHEMA_CONTEXT + "\n-- List all books with their author name\nSELECT b.title, ",
    ),
    Prompt(
        "p02_join_clause",
        "Complete a JOIN clause",
        SCHEMA_CONTEXT
        + "\n-- Top 10 best-selling books\nSELECT b.title, SUM(o.quantity) AS sold\nFROM books b\nJOIN orders o ",
        suffix="\nGROUP BY b.id\nORDER BY sold DESC\nLIMIT 10;",
    ),
    Prompt(
        "p03_where_filter",
        "Complete WHERE filter with a column",
        SCHEMA_CONTEXT
        + "\n-- Fantasy books published after 2000\nSELECT title, published_year\nFROM books\nWHERE genre = 'Fantasy'\n  AND ",
        suffix=";",
    ),
    Prompt(
        "p04_group_by_agg",
        "Complete GROUP BY aggregation",
        SCHEMA_CONTEXT
        + "\n-- Revenue per genre\nSELECT b.genre, ",
        suffix="\nFROM books b\nJOIN orders o ON o.book_id = b.id\nGROUP BY b.genre;",
    ),
    Prompt(
        "p05_cte",
        "Complete a CTE body",
        SCHEMA_CONTEXT
        + "\n-- Customers who spent more than $100\nWITH customer_totals AS (\n  SELECT ",
        suffix="\n)\nSELECT c.name, ct.total\nFROM customer_totals ct\nJOIN customers c ON c.id = ct.customer_id\nWHERE ct.total > 100;",
    ),
    Prompt(
        "p06_subquery",
        "Complete an IN subquery",
        SCHEMA_CONTEXT
        + "\n-- Books that have never been ordered\nSELECT title\nFROM books\nWHERE id NOT IN (\n  SELECT ",
        suffix="\n);",
    ),
    Prompt(
        "p07_update_set",
        "Complete an UPDATE SET clause",
        SCHEMA_CONTEXT
        + "\n-- Apply 10% discount to books older than 1990\nUPDATE books\nSET price = ",
        suffix="\nWHERE published_year < 1990;",
    ),
    Prompt(
        "p08_insert_values",
        "Complete INSERT column list",
        SCHEMA_CONTEXT
        + "\n-- Add a new review (book 1, customer 3, 5 stars)\nINSERT INTO reviews (",
        suffix=")\nVALUES (NULL, 1, 3, 5, 'Loved it', '2025-01-20');",
    ),
    Prompt(
        "p09_order_limit",
        "Complete ORDER BY + LIMIT",
        SCHEMA_CONTEXT
        + "\n-- 5 most recent orders\nSELECT id, customer_id, order_date, total_price\nFROM orders\n",
    ),
    Prompt(
        "p10_window",
        "Complete a window function",
        SCHEMA_CONTEXT
        + "\n-- Rank customers by lifetime spend\nSELECT\n  c.name,\n  SUM(o.total_price) AS lifetime_spend,\n  ",
        suffix="\nFROM customers c\nJOIN orders o ON o.customer_id = c.id\nGROUP BY c.id;",
    ),
]


# ─── llama-server lifecycle ───────────────────────────────────────────────
def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def start_server(model: Model) -> tuple[subprocess.Popen, int]:
    port = free_port()
    env = os.environ.copy()
    ld = CACHE / "bin"
    env["LD_LIBRARY_PATH"] = f"{ld}:{env.get('LD_LIBRARY_PATH', '')}"

    cmd = [
        str(BIN),
        "-m", str(MODELS_DIR / model.gguf),
        "--port", str(port),
        "--host", "127.0.0.1",
        "--ctx-size", str(model.ctx),
        "--n-gpu-layers", str(model.gpu_layers),
        "--parallel", "1",
        "--no-mmap",
        "--log-disable",
    ]
    print(f"[{model.id}] launching: {' '.join(cmd)}", flush=True)
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        env=env,
        start_new_session=True,
    )

    # Wait for /health to report ok (up to 120s for big model loads)
    deadline = time.time() + 120
    url = f"http://127.0.0.1:{port}/health"
    while time.time() < deadline:
        if proc.poll() is not None:
            err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
            raise RuntimeError(f"llama-server died early:\n{err}")
        try:
            with urllib.request.urlopen(url, timeout=2) as r:
                if r.status == 200:
                    print(f"[{model.id}] ready on :{port}", flush=True)
                    return proc, port
        except (urllib.error.URLError, ConnectionResetError):
            pass
        time.sleep(0.5)
    proc.terminate()
    raise RuntimeError(f"llama-server did not become healthy in 120s")


def stop_server(proc: subprocess.Popen) -> None:
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        proc.wait()


# ─── Single request ───────────────────────────────────────────────────────
def infill_stream(port: int, prompt: Prompt, n_predict: int = 48,
                  temperature: float = 0.2) -> dict:
    """Send a streaming /infill request, return timing + generated text."""
    body = json.dumps({
        "input_prefix": prompt.prefix,
        "input_suffix": prompt.suffix,
        "n_predict": n_predict,
        "temperature": temperature,
        "top_p": 0.9,
        "repeat_penalty": 1.05,
        "stream": True,
        "stop": [";", "\n\n"],
        "cache_prompt": True,
    }).encode("utf-8")

    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/infill",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )

    start = time.perf_counter()
    ttft: float | None = None
    pieces: list[str] = []
    token_times: list[float] = []
    tokens = 0
    stop_reason = "unknown"

    with urllib.request.urlopen(req, timeout=60) as resp:
        for raw in resp:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            # Server-Sent Events: "data: {...}"
            if line.startswith("data: "):
                line = line[6:]
            if line == "[DONE]":
                break
            try:
                chunk = json.loads(line)
            except json.JSONDecodeError:
                continue
            piece = chunk.get("content", "")
            if piece and ttft is None:
                ttft = time.perf_counter() - start
            if piece:
                pieces.append(piece)
                token_times.append(time.perf_counter() - start)
                tokens += 1
            if chunk.get("stop"):
                stop_reason = chunk.get("stop_type") or chunk.get(
                    "stopped_word", "eos"
                ) or "eos"
                break

    total = time.perf_counter() - start
    # tokens/sec after the first token (steady state)
    if tokens >= 2 and ttft is not None:
        tps = (tokens - 1) / max(token_times[-1] - ttft, 1e-6)
    else:
        tps = 0.0
    return {
        "ttft_ms": round((ttft or total) * 1000, 1),
        "total_ms": round(total * 1000, 1),
        "tokens": tokens,
        "tok_per_s": round(tps, 1),
        "stop_reason": stop_reason,
        "text": "".join(pieces),
    }


# ─── Orchestration ────────────────────────────────────────────────────────
def bench_model(model: Model, runs: int) -> dict:
    if not (MODELS_DIR / model.gguf).exists():
        print(f"[{model.id}] skipping — {model.gguf} not found", flush=True)
        return {"model": dataclasses.asdict(model), "skipped": True}

    proc, port = start_server(model)
    try:
        # Warm-up: one throwaway request so KV cache/CUDA kernels are hot.
        print(f"[{model.id}] warmup …", flush=True)
        try:
            infill_stream(port, PROMPTS[0], n_predict=16)
        except Exception as e:
            print(f"[{model.id}] warmup failed: {e}", flush=True)

        per_prompt = []
        for p in PROMPTS:
            runs_data = []
            for r in range(runs):
                try:
                    res = infill_stream(port, p)
                except Exception as e:
                    res = {"error": str(e)}
                runs_data.append(res)
                print(
                    f"[{model.id}] {p.id} run {r+1}/{runs}: "
                    f"ttft={res.get('ttft_ms','?')}ms "
                    f"tok={res.get('tokens','?')} "
                    f"tps={res.get('tok_per_s','?')}",
                    flush=True,
                )
            per_prompt.append({
                "id": p.id,
                "scenario": p.scenario,
                "runs": runs_data,
            })
        return {
            "model": dataclasses.asdict(model),
            "prompts": per_prompt,
        }
    finally:
        stop_server(proc)


def summarise(results: list[dict]) -> None:
    print("\n=== Summary ===")
    print(f"{'Model':<35} {'TTFT p50':>10} {'TTFT p95':>10} {'Tok/s p50':>10}")
    for r in results:
        if r.get("skipped"):
            print(f"{r['model']['display']:<35}  (skipped — model not downloaded)")
            continue
        ttfts, tps = [], []
        for p in r["prompts"]:
            for run in p["runs"]:
                if "ttft_ms" in run:
                    ttfts.append(run["ttft_ms"])
                    tps.append(run["tok_per_s"])
        if not ttfts:
            continue
        ttfts.sort(); tps.sort()
        def pct(xs, q):
            return xs[min(len(xs) - 1, int(len(xs) * q))]
        print(
            f"{r['model']['display']:<35} "
            f"{pct(ttfts, 0.5):>9.1f}ms "
            f"{pct(ttfts, 0.95):>9.1f}ms "
            f"{pct(tps, 0.5):>9.1f}"
        )


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", help="Run only a specific model id")
    ap.add_argument("--runs", type=int, default=3,
                    help="Runs per prompt (default: 3)")
    args = ap.parse_args(argv)

    if not BIN.exists():
        print(f"llama-server not found at {BIN}. Run scripts/fetch-llama-cpp.sh first.",
              file=sys.stderr)
        return 2

    picked = [m for m in MODELS if (args.only is None or m.id == args.only)]
    if not picked:
        print(f"no model matches --only={args.only!r}", file=sys.stderr)
        return 2

    out = []
    for m in picked:
        out.append(bench_model(m, args.runs))

    RESULTS.write_text(json.dumps(out, indent=2))
    print(f"\nwrote {RESULTS.relative_to(ROOT)}")
    summarise(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
