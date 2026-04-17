# Inline AI Assistance — Architecture & Implementation Plan

*Ghost-text SQL completion powered by a small local LLM, running next to the
existing deterministic schema-aware completion.*

---

## TL;DR

- **Viable?** Yes. A 4080 Super (16 GB VRAM, ~700 GB/s) runs a 3B–7B code model
  comfortably at 100–300 tok/s with sub-100 ms first-token latency. That is
  well inside the budget for perceived real-time inline completion.
- **Recommended stack:** `llama.cpp` (llama-server) bundled as a Tauri sidecar,
  serving a **Qwen2.5-Coder-3B** (default) or **7B** (optional) model in GGUF
  `Q4_K_M` via the OpenAI-compatible `/infill` (FIM) endpoint.
- **Integration:** A new Monaco `InlineCompletionsProvider` runs alongside the
  existing `createSqlCompletionProvider` in `@/run/media/bart/Development/dev/codeberg/sqail/src/lib/sqlCompletions.ts:459`.
  Deterministic popup completions stay as-is. The LLM only produces
  **ghost-text** suggestions (Copilot-style) that accept with `Tab`.
- **No cloud calls.** Feature is opt-in, fully local, no telemetry. Falls back
  to a user-supplied endpoint (Ollama / LM Studio / vLLM) for users who
  already run one.

---

## 1. Why this works on a 4080 Super

| Model                       | Size (Q4_K_M) | VRAM   | First tok  | Throughput     | FIM  |
| --------------------------- | ------------- | ------ | ---------- | -------------- | ---- |
| Qwen2.5-Coder-1.5B          | ~1.1 GB       | ~2 GB  | ~20–40 ms  | 300–500 tok/s  | yes  |
| **Qwen2.5-Coder-3B**        | ~2.0 GB       | ~3 GB  | ~30–60 ms  | 180–300 tok/s  | yes  |
| Qwen2.5-Coder-7B            | ~4.4 GB       | ~6 GB  | ~50–90 ms  | 80–140 tok/s   | yes  |
| DeepSeek-Coder-V2-Lite-16B  | ~9 GB         | ~12 GB | ~80–130 ms | 40–70 tok/s    | yes  |
| StarCoder2-3B / 7B          | ~2 / ~4.5 GB  | ~3–6GB | ~30–90 ms  | 100–250 tok/s  | yes  |

All numbers are rough llama.cpp/CUDA ballparks for RTX 40-class hardware. A
typical inline completion is 10–30 tokens, so:

```
perceived latency = debounce (150 ms) + first tok (~50 ms) + 15 tok × 5 ms
                  ≈ 275 ms
```

which is well inside the "near real-time" bar users associate with Copilot.
The 4080 Super is more than enough — the bottleneck will be prompt
construction and model choice, not the GPU.

**Default pick: Qwen2.5-Coder-3B Q4_K_M**
- Strong SQL knowledge (trained on a lot of code, including SQL).
- Native FIM tokens (`<|fim_prefix|>` / `<|fim_suffix|>` / `<|fim_middle|>`).
- Fits comfortably alongside the Tauri app, WebView2/WebKit, and the user's
  DB client libraries, even on smaller GPUs.
- Upgrade path to 7B for users with 12+ GB VRAM.

---

## 2. Where this fits in SQail today

Existing pieces we build on (don't duplicate):

- `@/run/media/bart/Development/dev/codeberg/sqail/src/components/SqlEditor.tsx:59-66` —
  Monaco mount + completion provider registration.
- `@/run/media/bart/Development/dev/codeberg/sqail/src/lib/sqlCompletions.ts:459`
  — deterministic popup completion (keywords, tables, columns, functions,
  snippets). Stays as-is.
- `@/run/media/bart/Development/dev/codeberg/sqail/src/lib/sqlCompletions.ts:417-451`
  — `extractReferencedTables` / `stripStringsAndComments`. Re-use for
  trigger gating and context retrieval.
- `@/run/media/bart/Development/dev/codeberg/sqail/src/lib/schemaContext.ts:4`
  — `buildSchemaContext` (full-schema dump, too big for inline; we'll add a
  scoped version).
- `@/run/media/bart/Development/dev/codeberg/sqail/src-tauri/src/ai/client.rs:656-726`
  — existing OpenAI-compatible streaming (LM Studio). We reuse the streaming
  helpers but route to a new inline endpoint.

New surfaces:

- Frontend: `src/lib/inlineAi.ts`, `src/stores/inlineAiStore.ts`, a new
  "Inline completion" section in the AI settings.
- Backend: `src-tauri/src/ai/inline/` (sidecar manager, FIM prompt builder,
  streaming completer) + new Tauri commands.

Deterministic completions and LLM completions coexist peacefully:

- **Popup list** (deterministic): triggered on `.`, `,`, `(`, or typing an
  identifier. Shows tables, columns, keywords, snippets. Unchanged.
- **Ghost text** (LLM): triggered on pause-in-typing at the end of a line.
  Shows a dimmed inline suggestion. `Tab` accepts, any keystroke rejects.

They never collide because Monaco renders them through different mechanisms
(`CompletionItemProvider` vs `InlineCompletionsProvider`).

---

## 3. Architecture

```
┌───────────────────────── React / Monaco ─────────────────────────────┐
│                                                                      │
│   SqlEditor.tsx                                                      │
│     ├── registerCompletionItemProvider (existing, deterministic)     │
│     └── registerInlineCompletionsProvider  ◄── NEW                   │
│                │                                                     │
│                ▼                                                     │
│       inlineAi.ts                                                    │
│         ├── triggerGate()        (skip in string/comment/after dot)  │
│         ├── debounce 150 ms                                          │
│         ├── LRU cache (prefix/suffix/schemaHash → completion)        │
│         ├── AbortController per request                              │
│         └── invoke("inline_complete_start", …) → request_id          │
│                                                                      │
│       inlineAiStore.ts  (zustand)                                    │
│         ├── enabled, model, endpoint, maxTokens                      │
│         ├── sidecar status (running / downloading / error)           │
│         └── listens for  inline:chunk  /  inline:done  /  inline:err │
│                                                                      │
└──────────────────────┬───────────────────────────────────────────────┘
                       │ Tauri invoke / event
┌──────────────────────▼─────────── Rust (Tauri) ──────────────────────┐
│                                                                      │
│   commands.rs                                                        │
│     ├── inline_complete_start(prefix, suffix, ctx) -> req_id         │
│     ├── inline_complete_cancel(req_id)                               │
│     ├── inline_sidecar_status / start / stop                         │
│     └── inline_model_list / download                                 │
│                                                                      │
│   ai/inline/                                                         │
│     ├── sidecar.rs   (spawns llama-server, health checks)            │
│     ├── models.rs    (catalog, GGUF download with progress events)   │
│     ├── fim.rs       (per-model FIM token templates, stop tokens)    │
│     ├── context.rs   (compact DDL builder, token-budget-aware)       │
│     └── completer.rs (HTTP streaming, cancellation registry)         │
│                                                                      │
└──────────────────────┬───────────────────────────────────────────────┘
                       │ http://127.0.0.1:PORT
┌──────────────────────▼─────────── llama.cpp sidecar ─────────────────┐
│   llama-server --model qwen2.5-coder-3b-q4_k_m.gguf                  │
│                --ctx-size 8192 --n-gpu-layers 99 --port 49331        │
│   Endpoints:  /infill   /completion   /health                        │
└──────────────────────────────────────────────────────────────────────┘
```

### 3.1 Request lifecycle

1. User types. Monaco fires `onDidChangeCursorPosition` /
   `provideInlineCompletions`.
2. `triggerGate()` runs fast, synchronous checks on the prefix:
   - cursor is inside a string literal / comment → **skip**
   - last non-space char is `.` → **skip** (popup completions own this)
   - current statement already ends with `;` → **skip**
   - line is blank and previous statement just ended → allow
   - selection is non-empty → **skip**
3. Debounce `150 ms` (configurable). On any new keystroke, the pending
   request is aborted (`AbortController`).
4. Build cache key: `hash(last 512 chars of prefix | first 128 chars of
   suffix | schema_context_hash | model_id)`. Hit → return immediately.
5. Miss → `invoke('inline_complete_start', { prefix, suffix, context,
   maxTokens: 48 })`. Tauri returns a `request_id` immediately.
6. Rust builds the FIM prompt, opens a streaming POST to `llama-server`,
   emits `inline:chunk` events as tokens arrive.
7. The React provider accumulates chunks and updates the ghost text.
8. Stop conditions (enforced on both sides):
   - Model emits FIM EOT / EOS token (model-specific list in `fim.rs`).
   - Output contains a `;` — we truncate at the first one (completions
     should not span statements).
   - Output contains two consecutive newlines (end of logical block).
   - `maxTokens` reached (default 48, configurable).
   - Client cancels (new keystroke / cursor move / Esc).
9. `inline:done` event carries the final string. The provider returns it to
   Monaco as the single inline completion item. `Tab` accepts.

### 3.2 FIM prompt format

llama.cpp's `/infill` endpoint takes `input_prefix` / `input_suffix` and
handles tokenization itself — no manual template juggling needed. We only
need a small per-model config for:

- model id / GGUF filename
- stop tokens (FIM end sentinel, EOS, newline policy)
- recommended sampler (`temperature: 0.2`, `top_p: 0.9`, `repeat_penalty: 1.05`)
- max context (typically 4–8 K for inline; model-dependent)

Request body example:

```json
POST /infill
{
  "input_prefix": "<schema context>\n\n-- SQL:\n<code before cursor>",
  "input_suffix": "<code after cursor>",
  "n_predict": 48,
  "temperature": 0.2,
  "top_p": 0.9,
  "stream": true,
  "stop": [";", "\n\n"]
}
```

If we ever need to run against a model whose runtime only exposes
`/completion` (not `/infill`), the same `fim.rs` module assembles the prompt
manually using the model's own FIM special tokens.

### 3.3 Schema context retrieval (token-budget aware)

`buildSchemaContext()` in `src/lib/schemaContext.ts:4` dumps the whole
database — fine for Ctrl+K one-shot prompts, way too big for inline.

New helper `buildInlineContext(prefix, fullText)`:

1. Re-use `extractReferencedTables(statementText)` from
   `src/lib/sqlCompletions.ts:418` to find `FROM` / `JOIN` / `UPDATE`
   targets in the current statement.
2. For each referenced table, pull its columns from the `useSchemaStore`
   cache and render a compact line:
   ```
   -- mas.equipment_group(id INT PK, plant_cd VARCHAR, description TEXT NOT NULL, ...)
   ```
3. If no tables are referenced (user is starting a query), pull the
   **last N recently used tables** from the query history store instead.
4. Hard cap: **1500 characters** of schema context. If over budget, drop
   non-essential columns (keep PK, FK, NOT NULL; drop TEXT/BLOB bodies).
5. Prepend the connection's dialect:
   ```
   -- Dialect: PostgreSQL 15
   -- Tables in scope:
   -- mas.equipment_group(...)
   -- mas.plant(id INT PK, code VARCHAR)
   ```
6. Hash the rendered context; used for cache keys.

This is important for both speed (smaller prompt = faster first token) and
quality (less irrelevant schema = less hallucinated JOINs).

### 3.4 Trigger gating (avoid unnecessary LLM calls)

Every LLM call costs ~100–300 ms and ~5 W of GPU. We gate hard:

| Condition                                 | Action  |
| ----------------------------------------- | ------- |
| Feature disabled in settings              | skip    |
| Sidecar not ready                         | skip    |
| Cursor inside a `'` / `"` string literal  | skip    |
| Cursor inside `--` or `/* */` comment     | skip    |
| Char immediately before cursor is `.`     | skip    |
| Non-empty selection                       | skip    |
| Current statement already ends in `;`     | skip    |
| Prefix hasn't changed since last suggest  | reuse   |
| Prefix is a strict prefix of cached hit   | reuse & trim |
| All gates pass                            | request |

Implementation re-uses `stripStringsAndComments` from
`src/lib/sqlCompletions.ts:323` for literal/comment detection.

### 3.5 Cancellation

- **Client side:** an `AbortController` per in-flight request. A new
  keystroke aborts the previous fetch; the aborted fetch sends a
  `inline_complete_cancel(request_id)` tauri invoke as cleanup.
- **Rust side:** an in-memory `HashMap<request_id, oneshot::Sender>`
  registry. `inline_complete_cancel` fires the oneshot; the streaming task
  selects between the HTTP stream and the cancel signal and drops the
  stream on cancel, which closes the connection to llama-server, which
  halts token generation within ~1 token.
- **Sidecar side:** llama-server halts generation when the HTTP client
  disconnects (standard behaviour).

---

## 4. Deployment modes

Two mutually-exclusive endpoints, user-selectable in settings:

### 4.1 Bundled sidecar (default, recommended)

- Ship `llama-server` binaries alongside the Tauri app:
  - Linux x86_64 (CUDA 12 build)
  - Linux x86_64 (Vulkan/CPU fallback build)
  - Windows x86_64 (CUDA 12 build)
  - macOS aarch64 (Metal build)
  - macOS x86_64 (Metal/CPU build)
- Binaries are ~20–40 MB each gzipped. Do **not** commit them to git; they
  are pulled from the llama.cpp GitHub release assets at package time
  (`scripts/fetch-llama-binaries.sh`, mirrors the pattern of
  `scripts/build-*.sh`).
- Registered with Tauri via `tauri.conf.json`'s `bundle.externalBin`.
- Spawned with `tauri-plugin-shell` or directly via `tokio::process::Command`
  (same pattern as the claude CLI integration at
  `src-tauri/src/ai/client.rs:563`).
- Health-checked every 5 s via `GET /health`.
- Auto-restart with exponential backoff on crash (max 3 attempts).
- Port: bind to `127.0.0.1:0` (kernel picks free port), store port in app
  state, pass to frontend via event.

Model files are **not** bundled — too big. Downloaded on first use to
`<app_data>/models/` via `inline_model_download`, with progress events to
the UI. Default URL pattern:

```
https://huggingface.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF
  /resolve/main/qwen2.5-coder-3b-instruct-q4_k_m.gguf
```

Checksummed against a signed manifest shipped with the app.

### 4.2 External OpenAI-compatible endpoint

For users who already run Ollama / LM Studio / llama.cpp / vLLM:

- Extend the existing `AiProviderType::LmStudio` flow at
  `src-tauri/src/ai/provider.rs:13` with an "Inline" capability flag, OR
- Add `AiProviderType::LocalInline` with its own config.

Because the existing `stream_lm_studio` already speaks OpenAI chat format,
we can reuse it for models that do **not** expose `/infill` by building the
FIM prompt as the user message. Quality is slightly worse but it works.

---

## 5. Performance budget

Target: ghost text appears within ~300 ms of the user pausing.

| Step                                  | Budget   | Notes                              |
| ------------------------------------- | -------- | ---------------------------------- |
| Debounce window                       | 150 ms   | configurable 50–500 ms             |
| Trigger gate + cache lookup           | < 2 ms   | all sync JS                        |
| `invoke` round-trip                   | 3–5 ms   | Tauri IPC                          |
| Build schema context (cached)         | < 1 ms   | store snapshot + hash              |
| HTTP POST to llama-server             | 1–3 ms   | localhost                          |
| First token (Qwen2.5-Coder-3B Q4)     | 40–80 ms | on 4080 Super, warm model          |
| 15 tokens streamed                    | 75 ms    | at ~200 tok/s                      |
| Monaco ghost-text render              | < 5 ms   |                                    |
| **Total (cache miss, 15-tok answer)** | ~275 ms  |                                    |
| **Cache hit**                         | ~6 ms    |                                    |

Warm-up: sidecar takes ~1–3 s to load the model into VRAM on first start.
Keep it running for the app's lifetime. Optional: a tiny `/infill` warmup
request on startup to prime the KV cache.

---

## 6. Settings UX

New section in the AI settings modal (`src/components/SettingsModal.tsx`):

```
┌─ Inline Completion ─────────────────────────────────────────┐
│                                                             │
│  [x] Enable inline AI completion                            │
│                                                             │
│  Backend:  (•) Built-in (local)   ( ) External endpoint     │
│                                                             │
│  Model:    [ Qwen2.5-Coder-3B-Instruct Q4_K_M  ▼ ]          │
│            Downloaded · 1.9 GB · GPU (CUDA) ✓               │
│            [ Download another model ]                       │
│                                                             │
│  Status:   ● Running on 127.0.0.1:49331 (VRAM: 3.2 GB)      │
│            [ Stop ] [ Restart ] [ View log ]                │
│                                                             │
│  Tuning                                                     │
│    Debounce (ms):     [ 150 ]                               │
│    Max tokens:        [  48 ]                               │
│    Temperature:       [ 0.2 ]                               │
│                                                             │
│  [x] Auto-start sidecar when SQail launches                 │
│  [ ] Use CPU only (disables GPU acceleration)               │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

Editor status-bar widget (bottom right): small icon indicating
`idle / thinking / error`, click opens settings.

---

## 7. Implementation plan

Sized roughly for solo dev work; a focused ~2-week sprint for MVP.

### Phase A — Spike & model choice (1–2 days) ✅ done
- [x] A.1 Install llama.cpp locally, verify CUDA build runs on the 4080S.
  Built from source at tag `b8815`, see `scripts/fetch-llama-cpp.sh`.
- [x] A.2 Benchmark Qwen2.5-Coder 1.5B / 3B / 7B and DeepSeek-Coder-V2-Lite
  with realistic SQL FIM prompts. All candidates cleared the 100 ms TTFT
  bar by ~10× on warm runs. See `scripts/inline-ai-benchmark.py`.
- [x] A.3 Picks:
    - **Default: Qwen2.5-Coder-3B Q4_K_M** (2.5 GB VRAM, 6 ms TTFT,
      195 tok/s, perfect quality on 10/10 prompts).
    - **Performance: DeepSeek-Coder-V2-Lite Q4_K_M** (11 GB VRAM — gated
      on ≥16 GB GPUs). Beats the originally-planned Qwen 7B on both
      speed and quality thanks to its MoE architecture.
    - **Low-end / CPU fallback: Qwen2.5-Coder-1.5B Q4_K_M** (1.6 GB).
    - 7B is dropped — slower and lower quality than both the 3B and
      the 16B MoE.
- [x] A.4 Results in `Vibecoding/inline-ai-benchmarks.md`.

### Phase B — Sidecar manager (2–3 days) ✅ done
- [x] B.1 `src-tauri/src/ai/inline/` with `mod.rs`, `state.rs`.
- [x] B.2 `sidecar.rs`: spawn `llama-server`, free-port allocation,
  health-check loop (5 s poll, exponential-backoff auto-restart up to 3
  attempts), process-group cleanup on Unix, `kill_on_drop(true)` globally.
- [x] B.3 `models.rs`: hard-coded three-model catalog, resumable GGUF
  download (`.part` suffix, Range header), optional SHA-256 verification,
  progress events `inline:model-download-progress` throttled to 200 ms.
- [x] B.4 Commands: `inline_sidecar_{status,start,stop}`,
  `inline_model_{list,download,cancel_download,delete}`. Wired into
  `lib.rs invoke_handler!`.
- [x] B.5 `scripts/fetch-llama-binaries.sh` prepared for release
  bundling (covers host Linux CUDA today; Windows/macOS prebuilts
  documented as TODO). `tauri.conf.json externalBin` left to Phase G.
  Sidecar binary resolver works in dev via the `.cache/inline-ai/bin/`
  fallback and honours `SQAIL_LLAMA_SERVER_PATH` for overrides.

### Phase C — FIM completer (2 days) ✅ done
- [x] C.1 `fim.rs`: per-model sampler + stop-string config. Single
  default (`temperature=0.2, top_p=0.9, top_k=40, repeat_penalty=1.05,
  n_predict=48, stops=[";", "\n\n"]`) applied via `config_for(&entry)`
  — trivial to override per-model later.
- [x] C.2 Context: confirmed frontend-owned. Rust takes `prefix` +
  optional `suffix` straight from the `inline_complete_start` command.
- [x] C.3 `completer.rs`: streaming POST to `/infill`, SSE frame
  parser, per-request `oneshot::Sender` cancel registry (`CompletionRegistry`),
  client-side stop fence (truncates at first `;` / `\n\n`). Emits
  `inline:chunk` / `inline:done` / `inline:error`. Unit tests cover
  the stop fence.
- [x] C.4 Commands: `inline_complete_start` (non-blocking — spawns
  task, returns request_id synchronously) and `inline_complete_cancel`.
  Wired into `lib.rs invoke_handler!`. End-to-end smoke test against
  the live Qwen-3B sidecar confirmed request/stream formats match.

### Phase D — Frontend inline provider (2–3 days) ✅ done
- [x] D.1 `src/lib/inlineAi.ts`: Monaco `InlineCompletionsProvider` with
  hard gating (`shouldTrigger`), 150 ms debounce, shared module-level
  event dispatcher (`Map<requestId, handler>`), LRU cache lookup, and
  full cancellation on new keystrokes (cancels both the JS promise and
  the Rust request via `inline_complete_cancel`).
- [x] D.2 `src/stores/inlineAiStore.ts`: persisted settings (enabled,
  model, autoStart, debounce, maxTokens, temperature, cpuOnly, ctxSize)
  plus runtime state (sidecar status, model catalog, per-model
  download progress). Event-adapter methods are called by
  `useInlineAiLifecycle` on boot.
- [x] D.3 `src/lib/inlineContext.ts`: 1500-char budget builder — pulls
  referenced tables from the stripped current statement, falls back to
  the first 6 loaded tables if nothing is referenced, drops TEXT/BLOB
  bodies before truncating. Exports `extractReferencedTables` +
  `stripStringsAndComments` from `sqlCompletions.ts` for reuse.
- [x] D.4 `src/lib/lruCache.ts`: 256-entry map-backed LRU + djb2 hash.
- [x] D.5 Provider registered in `SqlEditor.tsx` for `sql` / `mysql` /
  `pgsql` alongside the existing completion provider. Monaco's
  `inlineSuggest.enabled` flipped on.
- [x] D.6 Keybindings: `Tab` / `Esc` / cursor-move are Monaco defaults
  under `inlineSuggest.enabled`. Alt+[ / Alt+] cycling deferred to v2
  (single-item responses today). `pnpm tsc --noEmit` + `pnpm build` +
  `pnpm eslint` all clean.

### Phase E — Settings UI & lifecycle (1–2 days) ✅ done
- [x] E.1 New **Inline AI** tab in `SettingsModal.tsx`. Settings tab
  type widened to include `"inline-ai"`; App-level state upgraded from
  `boolean` to `SettingsTab | null` so the indicator can deep-link
  directly to this tab.
- [x] E.2 Model picker (`InlineAiSettingsTab.tsx`): catalog rendered
  as radio-selectable rows with tier badges (default / perf / lite),
  VRAM + disk-size labels, and per-row Download / Cancel / Delete
  actions. Live progress bars driven by `inline:model-download-progress`
  events coming through `applyDownloadProgress` in the store.
- [x] E.3 Sidecar status widget (`SidecarPanel`): live dot + readout
  and Start / Stop / Restart buttons reflecting `inline:sidecar-status`.
- [x] E.4 Toolbar status indicator (`InlineAiIndicator.tsx`): small
  lightning-bolt icon with coloured dot (muted / amber-pulse /
  emerald / red). Click opens settings directly on the Inline AI tab.
- [x] E.5 Contextual onboarding: the enable toggle flips the sidecar
  on/off via a new `toggleEnabled` side-effect. When enabled without
  the selected model downloaded, an amber banner appears inside the
  settings tab with a one-click "Download now" action.

### Phase F — Polish (1–2 days) ✅ done
- [x] F.1 Extracted `stripThinkingBlocks` into
  `src/lib/stripThinking.ts`, re-used by both `aiStore.ts` and
  `inlineAi.ts`. Guards against `<think>` leaks if a reasoning
  variant is ever picked.
- [x] F.2 Client-side stop fence doubled up: the Rust completer's
  `apply_stops` + a JS mirror (`truncateAtStop`) in `inlineAi.ts` so
  an external endpoint that ignores the Rust fence still can't
  produce runaway ghost text.
- [x] F.3 `cancelAllInlineRequests` exported from `inlineAi.ts` and
  wired to zustand store subscriptions in `useInlineAiLifecycle`:
  fires on sidecar-not-ready, model change, enable→disable, and
  active-connection change. Cursor/selection changes are already
  handled by Monaco's cancellation token.
- [x] F.4 Ring buffer of the last 20 completions
  (`CompletionTelemetry`) populated from `inline:done` events.
  Toggled via a new `devMode` setting; rendered as a compact table
  below the settings tab when on.
- [x] F.5 README now has a dedicated "Inline AI completion" section
  with the model tier table, `CONTRIBUTING.md` documents the dev
  sidecar build path, and `RELEASES.md` has an Unreleased entry.

### Phase G — Release gating (0.5 day) ✅ done
- [x] G.1 CI: `release.yml` unchanged — the feature ships without
  bundled `llama-server` binaries for v0.5.0 (following the risk-table
  mitigation to avoid CUDA bloat). The dev-only fetch script at
  `scripts/fetch-llama-binaries.sh` stays as documentation for the
  follow-up that will wire `externalBin` for Windows/macOS prebuilts.
  README + CONTRIBUTING document the three supported runtime modes:
  (1) external OpenAI-compatible endpoint, (2) user-supplied
  `SQAIL_LLAMA_SERVER_PATH`, (3) Linux devs building from source.
- [x] G.2 Release bump 0.4.2 → **0.5.0** across `package.json`,
  `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`. Release notes
  moved from Unreleased to v0.5.0 in `RELEASES.md` and a matching
  entry added to `releases.json` (consumed by the in-app About tab
  and the portal).
- [x] G.3 Size check: inline AI is **disabled by default** and
  downloads nothing until the user flips the toggle. No new binaries
  bundled in this release, so the installer size delta is **0 bytes**
  for users who never enable the feature. Bundled prebuilts (~30 MB
  per platform) are a conscious non-goal for 0.5.0.

### Follow-ups tracked after 0.5.0
- [x] F1. Automated runtime download of `llama-server` — landed post-0.5.0.
  `src-tauri/src/ai/inline/binaries.rs` mirrors `models.rs`, stages the
  archive under `<app_data>/inline-ai/bin/`, extracts flattened
  alongside the exe. Catalog: Windows x64 Vulkan `.zip`, macOS
  arm64/x64 Metal `.tar.gz`, Linux x64 Vulkan `.tar.gz`, all pinned to
  llama.cpp `b8815`.
- F2. Fill in `scripts/fetch-llama-binaries.sh` for Windows / macOS
  prebuilts and register them under `bundle.externalBin` (optional —
  F1 already solves the Windows UX problem without installer bloat).
- F3. Extend `release.yml` with a `verify-inline-ai-binaries` step
  once F2 lands, so CI catches missing artefacts before shipping.

---

## 8. Risks & mitigations

| Risk                                                | Mitigation                                        |
| --------------------------------------------------- | ------------------------------------------------- |
| Bundling CUDA binaries balloons installer size      | Download llama-server on first enable, not bundle |
| No CUDA on user's Linux distro                      | Ship Vulkan + CPU fallback binary                 |
| Model download fails / partial                      | SHA-256 resume, clear error, retry button         |
| User has no GPU at all                              | CPU mode works (slower, ~15 tok/s on 3B Q4)       |
| Completions drift off-schema (hallucinated columns) | Tight context window + truncation + low temp     |
| Sidecar leaks across app restarts                   | PID file + cleanup on startup, process group kill |
| Conflicts with popup completions                    | Strict gating on `.` and identifier-only prefixes |
| Multiple editors (split view) fight for ghost text  | Request queue keyed on editor id; only active     |
|                                                     | editor's request renders                          |
| Monaco inline API differences on mobile / webview2  | Feature-detect and hide feature if unsupported    |

---

## 9. Open questions

1. **Do we support multiple models loaded at once?** No for v1. Model
   switch = sidecar restart. Keeps VRAM usage predictable.
2. **Do we fine-tune on the user's schema/queries?** No for v1. Context
   retrieval is cheaper and already good. Fine-tune could be a future
   "pro" feature if there's demand.
3. **Do inline completions contribute to AI history?** No. Too noisy. Only
   accepted completions could be logged (opt-in) for quality metrics.
4. **Multi-line completions?** Yes, up to the `\n\n` stop. But discourage
   runaway generation via `maxTokens=48` default.
5. **Licensing:** llama.cpp is MIT. Qwen2.5-Coder is Apache-2.0.
   DeepSeek-Coder-V2 is a custom license that permits redistribution.
   Bundling binaries and providing download URLs is clean. Document in
   `LICENSE` / about screen.

---

## 10. Success criteria for MVP

- First-token latency ≤ 100 ms on the default model on a 4080-class GPU.
- Accept rate ≥ 30 % on realistic SQL workloads (measured locally).
- Zero network traffic when using the bundled sidecar.
- App still launches in < 1 s when feature is disabled.
- No regression to the existing deterministic popup completion.
- Clean uninstall: deleting the app removes all models and sidecar state
  from `<app_data>/`.

