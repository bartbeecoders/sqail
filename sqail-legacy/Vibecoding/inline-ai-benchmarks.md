# Inline AI — Phase A Benchmarks

*Model selection spike for the inline AI feature planned in
[`inline-ai.md`](./inline-ai.md). Measures first-token latency, throughput,
VRAM footprint, and subjective completion quality on realistic SQL FIM
prompts against the bookstore demo schema.*

> Status: **complete** — decisions below in §Decision.

---

## Methodology

### Hardware
- CPU: AMD Ryzen 9 7950X (16-core, Zen 4)
- GPU: NVIDIA GeForce RTX 4080 SUPER (16 GB VRAM, compute capability 8.9)
- NVIDIA driver: 595.58.03
- CUDA toolkit: release 12.8 (V12.8.93)

### Software
- llama.cpp built locally at tag **`b8815`** (`ae2d348`) with
  `GGML_CUDA=ON` and `CMAKE_CUDA_ARCHITECTURES=89`
  (see `scripts/fetch-llama-cpp.sh`).
- Quantisation: all models **Q4_K_M** (GGUF).
- llama-server flags: `--ctx-size 4096 --n-gpu-layers 999 --parallel 1
  --no-mmap --log-disable`.
- Sampler: `temperature=0.2, top_p=0.9, repeat_penalty=1.05`.
- Stop tokens: `[";", "\n\n"]`, `n_predict=48`, `cache_prompt=true`.

### Prompts
Ten FIM prompts exercising common completion scenarios against the
bookstore demo schema (`scripts/seed-demo-db.sh`). Defined in
`scripts/inline-ai-benchmark.py`:

| Id  | Scenario                                  |
| --- | ----------------------------------------- |
| p01 | Start a SELECT from a known table         |
| p02 | Complete a JOIN clause                    |
| p03 | Complete WHERE filter with a column       |
| p04 | Complete GROUP BY aggregation             |
| p05 | Complete a CTE body                       |
| p06 | Complete an IN subquery                   |
| p07 | Complete an UPDATE SET clause             |
| p08 | Complete INSERT column list               |
| p09 | Complete ORDER BY + LIMIT                 |
| p10 | Complete a window function                |

### Procedure
For each model, in a single `llama-server` session:

1. Wait for `/health` to go green.
2. Warm-up `/infill` on p01 (primes CUDA kernels + KV cache).
3. 10 prompts × **3 runs** of `/infill` (30 samples), recording TTFT
   and tok/s.
4. VRAM captured by a second script (`scripts/measure-vram.py`)
   that reads `nvidia-smi --query-compute-apps` after warmup.

**Caveat on TTFT:** `cache_prompt=true` means the shared schema prefix
is cached across prompts, so reported TTFT reflects the *warm* condition
users experience after the first few keystrokes. The first p95 spike on
each model is visible in run 1 of each prompt (e.g. DeepSeek 22–32 ms
cold vs ~6 ms warm); steady-state TTFT is what matters for the feature
and that's what the p50 captures.

Reproduce:

```bash
./scripts/fetch-llama-cpp.sh
./scripts/fetch-inline-models.sh
./scripts/inline-ai-benchmark.py --runs 3
./scripts/measure-vram.py
```

Raw output lands in `.cache/inline-ai/{results,vram}.json`.

---

## Results — latency, throughput, VRAM

30 samples per model (10 prompts × 3 runs).

| Model                             | TTFT p50 | TTFT p95 | Tok/s mean | VRAM    | File size |
| --------------------------------- | -------: | -------: | ---------: | ------: | --------: |
| Qwen2.5-Coder-1.5B Q4_K_M         |  4.4 ms  |   8.3 ms |      297   | 1.6 GB  |   1.1 GB  |
| **Qwen2.5-Coder-3B Q4_K_M**       |  6.1 ms  |  11.8 ms |      195   | 2.5 GB  |   2.0 GB  |
| Qwen2.5-Coder-7B Q4_K_M           |  9.7 ms  |  16.3 ms |      113   | 4.9 GB  |   4.4 GB  |
| **DeepSeek-Coder-V2-Lite Q4_K_M** |  6.4 ms  |  29.0 ms |      200   | 11.1 GB |   9.7 GB  |

Target from `inline-ai.md` §5: first token ≤ 100 ms, 15-token completion
inside ~275 ms total. **Every candidate clears the TTFT bar by a full
order of magnitude on warm runs.** The bottleneck will be debounce +
schema-context assembly, not inference.

DeepSeek-Coder-V2-Lite is a 16B **Mixture-of-Experts** with only ~2.4B
active parameters per token — that's why it holds 200 tok/s despite the
model size. The flip side is the VRAM: it parks 11 GB, leaving only
4.5 GB free on a 16 GB GPU. Not viable on ≤12 GB cards.

---

## Results — completion quality

All four models produced syntactically correct, on-schema SQL for every
prompt. Legend: ✅ plausible and on-schema · ⚠️ stylistically weak or
over-verbose · ❌ wrong schema / runaway / off-topic.

| Prompt | 1.5B | 3B | 7B | DS-Lite | Notes                                          |
| ------ | :--: | :-: | :-: | :-----: | ---------------------------------------------- |
| p01    | ✅   | ✅  | ✅  | ✅     | All JOIN authors + books correctly             |
| p02    | ✅   | ✅  | ✅  | ✅     | `ON b.id = o.book_id`, all identical           |
| p03    | ✅   | ✅  | ✅  | ✅     | All produced `published_year > 2000`           |
| p04    | ✅   | ✅  | ✅  | ✅     | 1.5B used `b.price*o.quantity`, others used `o.total_price` — both valid |
| p05    | ✅   | ✅  | ✅  | ✅     | 1.5B took the direct path, 3B went via JOIN — both correct |
| p06    | ✅   | ✅  | ✅  | ✅     | All picked `book_id FROM orders`               |
| p07    | ✅   | ✅  | ✅  | ✅     | All produced `price * 0.9` or `price * 0.90`   |
| p08    | ✅   | ✅  | ✅  | ✅     | All column lists match the schema              |
| p09    | ✅   | ✅  | ✅  | ✅     | All `ORDER BY order_date DESC LIMIT 5`         |
| p10    | ✅   | ✅  | ❌  | ✅     | 7B returned `c.joined_at` — unrelated to the rank-by-spend task |

Score: **1.5B 10/10 · 3B 10/10 · 7B 9/10 · DS-Lite 10/10**.

The 7B miss on p10 is a reminder that a larger model is not
automatically better on narrow tasks. Plausibly fixable by tightening
the FIM context or lowering the sampler's temperature further, but we
shouldn't ship a default that needs that kind of hand-holding.

---

## Decision

| Tier             | Pick                            | Rationale                                  |
| ---------------- | ------------------------------- | ------------------------------------------ |
| **Default**      | Qwen2.5-Coder-3B Q4_K_M         | 2.5 GB VRAM, 6 ms TTFT, 195 tok/s, 10/10 quality. Fits any modern GPU (≥4 GB), ships a 2 GB download. Native FIM tokens, Apache-2.0. |
| **Performance**  | DeepSeek-Coder-V2-Lite Q4_K_M   | Same quality as 3B, same-ish speed, but a 16B MoE model behind it. Offer only when ≥16 GB VRAM detected. 9.7 GB download. |
| **Low-end / CPU**| Qwen2.5-Coder-1.5B Q4_K_M       | 1.6 GB VRAM, 300 tok/s, still 10/10 quality on our prompts. Good fallback when 3B would crowd out the user's other GPU workloads. |
| ~~Mid~~          | ~~Qwen2.5-Coder-7B Q4_K_M~~     | Dropped. Slower than both the 3B *and* the 16B DS-Lite MoE, lower quality than both. Nothing it does that something else doesn't do better. |

**Action for `inline-ai.md`:** replace the planned "performance mode:
Qwen 7B" recommendation with **DeepSeek-Coder-V2-Lite**, noting the
11 GB VRAM requirement.

---

## Notes & gotchas

- **`cache_prompt: true` matters a lot.** Without it, TTFT on the 3B
  jumps from ~6 ms to ~35 ms on cold runs (schema context re-tokenised
  each call). With it, the editor's typical workflow (prefix grows one
  character at a time) stays in the sub-10-ms range.
- **Stop tokens**: `[";", "\n\n"]` worked cleanly for all four models.
  No runaway generations observed across 120 runs.
- **Streaming output quirk**: Qwen-1.5B/3B/7B use OpenAI-style `data:
  {...}` SSE; DeepSeek-V2-Lite emits the same format. Harness treats
  both uniformly.
- **Warm-up is non-optional**. First-request TTFT on a cold model is
  500–2500 ms (kernel compilation + KV alloc). Ship a startup
  `/infill` ping in the sidecar manager to eat that cost before the
  user types.
- **DeepSeek's p95 (29 ms)** is almost entirely cold-run artefacts in
  the streaming API — the first request of each prompt takes ~25 ms,
  subsequent ones ~6 ms. A second warm-up loop would flatten it.
