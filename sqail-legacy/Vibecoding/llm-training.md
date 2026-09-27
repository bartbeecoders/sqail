# LLM Training — Schema-Tuned LoRA Adapters

## Motivation

Would it make sense to take a small AI model (open weights) and train it
on the SQL database schema and sample data to generate SQL queries? For
the right workload — large, idiosyncratic schemas that the base model
hasn't seen — **yes**. Fine-tuning a LoRA adapter on a corpus derived
from the schema + generated metadata + sample data reliably lifts SQL
quality for that specific database, on top of the existing inline-AI
schema-context retrieval.

The v1 flow:

1. User picks one of their configured database connections and a base
   model from the inline-AI catalog.
2. SQaiL extracts schema, metadata, and sample rows and emits a JSONL
   training corpus.
3. A Python trainer sidecar (local GPU, e.g. RTX 4080 Super) runs a LoRA
   fine-tune and writes the adapter next to the base model.
4. The adapter shows up in the "Trained models" list, ready to be loaded
   by the existing llama.cpp sidecar via `--lora` (follow-up — see §5).

## Architecture

```
┌───────────────────── React ──────────────────────┐
│  SettingsModal → "Model Training" tab            │
│    ├── TrainingSettingsTab.tsx                   │
│    ├── trainingStore.ts  (zustand + events)      │
│    └── types/training.ts                         │
└──────────────────┬───────────────────────────────┘
                   │ invoke + `training:*` events
┌──────────────────▼─────────── Rust (Tauri) ──────┐
│  ai/training/                                    │
│    ├── dataset.rs — schema/metadata/sample → JSONL│
│    ├── env.rs     — python + deps probe          │
│    ├── jobs.rs    — Python sidecar orchestrator  │
│    ├── models.rs  — trained-adapter catalog      │
│    └── state.rs   — job registry                 │
└──────────────────┬───────────────────────────────┘
                   │ spawn
┌──────────────────▼────────── Python sidecar ─────┐
│  scripts/train_sql_lora.py                       │
│    HF transformers + peft + trl + datasets       │
│    stdout: JSONL progress events                 │
└──────────────────────────────────────────────────┘
```

Data flow at a glance:

* `training_preview_dataset(connection_id, options)` →
  `DatasetStats` (non-destructive dry-run).
* `training_start(connection_id, base_model_id, options,
  hyperparams)` → `job_id`; emits `training:update`, `training:log`,
  `training:done` events.
* `training_list_jobs()` / `training_cancel(job_id)` — lifecycle.
* `training_list_models()` / `training_delete_model(id)` — adapter
  catalog under `<app_data>/inline-ai/trained/<id>/`.

## 1. Dataset generation (Rust, deterministic)

`src-tauri/src/ai/training/dataset.rs` walks the connection's schemas
and emits:

* **Schema-level examples** — the database dialect, per-table column
  listings, canonical `SELECT *` / `COUNT(*)` templates.
* **Metadata examples** — descriptions, example usage, and per-column
  explanations from the existing `ObjectMetadata` store (the same
  records the Ctrl+K palette already ships as context).
* **Sample-row WHERE clauses** — for each table we `SELECT … LIMIT 5`
  and convert concrete values into `WHERE col = <value>` examples.
  Numeric vs. string quoting is dialect-aware; quotes are escaped.
* **Join heuristics** — every `*_id` column gets matched against the
  singularised table names on the same connection, yielding an
  `INNER JOIN` template.

Output goes to
`<app_data>/inline-ai/training/<conn>/<job>/dataset.jsonl` in the
instruction-tuning shape `{instruction, input, output}` — directly
consumable by `trl.SFTTrainer` via its `formatting_func`.

Knobs (surface in the UI):

| option              | default | notes                                    |
| ------------------- | ------- | ---------------------------------------- |
| `sampleRows`        | 5       | rows per table — 0 disables              |
| `maxTables`         | 200     | hard cap for large DBs                   |
| `includeMetadata`   | true    | pulls from generated-metadata store      |
| `includeSamples`    | true    | WHERE-clause examples from real rows     |
| `includeJoins`      | true    | PK ← FK heuristic join templates         |
| `schemas`           | all     | optional whitelist                       |

## 2. Python trainer sidecar

`scripts/train_sql_lora.py` is a single file so we don't ship a
requirements.txt; the user manages their own venv. Runtime deps:

```
pip install torch transformers peft trl datasets accelerate
# optional, enables 4-bit QLoRA on 16 GB GPUs:
pip install bitsandbytes
```

CLI contract (stable — see `ai/training/jobs.rs`):

```
python3 train_sql_lora.py \
    --dataset <jsonl>       --base-model <hf repo id> \
    --output-dir <dir>      --epochs <float> \
    --lr <float>            --lora-rank <int> \
    --lora-alpha <int>      --max-steps <int> \
    --batch-size <int>
```

Protocol:

* **stdout** — one JSON object per line, e.g.
  `{"phase":"training","step":42,"total_steps":200,"loss":0.41,"progress":0.21}`.
  The Rust side maps this directly to `training:update` events.
* **stderr** — free-form log (shown in the UI's "View log" drawer).
* Exit code `0` success, `1` on any failure.

## 3. Tauri commands

All registered in `lib.rs invoke_handler!`:

* `training_check_env()` — probes Python + package availability + CUDA.
* `training_preview_dataset(connectionId, options)` — returns
  `DatasetStats` without kicking off training.
* `training_start(connectionId, baseModelId, options, hyperparams)` —
  returns `jobId`.
* `training_cancel(jobId)` / `training_list_jobs()` — lifecycle.
* `training_list_models()` / `training_delete_model(modelId)` — catalog.

Events:

* `training:update` — full `TrainingJob` snapshot on every state
  transition or progress tick.
* `training:log` — `{id, line}` for each stderr line.
* `training:done` — emitted once when the job exits (success or failure).

## 4. UX

New **Model Training** tab in Settings. Four sections:

1. **Python environment** — status card with pass/fail per required
   package, a one-line `pip install …` suggestion for the missing
   bits, and a Re-check button.
2. **Dataset source** — connection picker (gated on being actively
   connected) + base-model picker (reuses the inline-AI catalog).
3. **Dataset options** + **Hyperparameters** — knobs described above.
4. **Run** — preview, start, cancel; a live progress panel with
   loss/step readout and an expandable stderr log; below that, the
   recent-job history and the trained-model catalog with delete.

The Python-env panel means the feature fails gracefully on machines
without PyTorch — the user sees exactly what's missing instead of a
cryptic spawn error.

## 5. Known limitations & follow-ups

* **Adapter activation is out of scope for v1.** The sidecar doesn't
  yet load `--lora <path>` at startup; hooking the catalog entry into
  the existing inline-AI `start` flow is the next step. (The adapter
  on disk is already in peft's default layout, so llama.cpp's
  `convert_lora_to_gguf.py` works unchanged once we wire it in.)
* **MSSQL / DbService**: schema + metadata + join heuristics work but
  sample-row extraction is skipped. Re-add once we have a generic
  row-sampling helper that doesn't rely on native sqlx pools.
* **No bundled Python**: we probe for `python3` on PATH (overridable
  via `SQAIL_PYTHON`). Bundling Python + PyTorch would bloat the
  installer by ~2 GB — a deliberate non-goal.
* **No fine-tune cancellation mid-step**: `Cancel` sends SIGTERM to
  the trainer, which tears down the process; mid-step checkpointing
  would need more plumbing and isn't blocking for v1.
* **No hyperparameter validation UX beyond the spin-boxes.** Bad
  values surface as a trainer error in the log — acceptable for now.
* **llama.cpp LoRA hot-swap**: `llama-server --lora` requires
  restarting the sidecar with the new adapter. That plumbing lives in
  the follow-up PR.

## 6. References

* Inline AI (the local sidecar this feature builds on) — `inline-ai.md`.
* Trained-adapter storage layout — `src-tauri/src/ai/training/models.rs`.
* Trainer CLI — `scripts/train_sql_lora.py`.



## 7. Anti-hallucination: column grounding

### The failure mode

Prompts like *"list of running process orders"* used to come back as

```sql
SELECT plant_cd, process_order_nr, material_cd,
       mfg_date, exp_date, created_date, last_edit_date
FROM   sub.process_order
WHERE  process_order_status_cd = 'Running';
```

Several of those columns (`mfg_date`, `exp_date`, `created_date`,
`last_edit_date`) do not exist on the real table. The root cause was a
mix of training and prompting:

1. **Training shape was wrong.** The corpus emitted `SELECT *` and
   `COUNT(*)` per table, but never a `{instruction, input, output}` pair
   where `input = schema` and `output = SELECT <specific columns>`
   picked *from that schema*. The model learned table vocabulary but
   never learned to read a column list and pick from it — so at
   inference it fell back on its base-model prior ("process-order
   tables usually have mfg/exp/created dates").
2. **System prompt was weak.** `build_system_prompt` only said "use
   correct table and column names". Small local models need a hard
   "columns not in the list must not appear" rule and a self-check
   step, not a vague nudge.
3. **Schema context could arrive empty.** The frontend's
   `buildSchemaContext` only serialised columns for tables the user had
   already expanded in the sidebar. Going straight to Ctrl+K often
   produced a schema block with table names and no column lists.

### Fixes shipped

**Prompting (`src-tauri/src/ai/prompt.rs`)**

* The schema-context block is now prefixed with "This is the complete
  and authoritative list of tables and columns available — treat it as
  the ONLY source of truth."
* `generate_sql` and `fix_query` have explicit column-grounding rules:
  only use columns that appear verbatim, prefer `SELECT *` over
  guessing, and silently verify every referenced column before emitting.

**Schema context (`src/lib/schemaContext.ts`)**

* `buildSchemaContext` is now async and calls `schemaStore.loadAllColumns`
  per schema before serialising, so every table in the context is
  guaranteed to carry its column list even if the user never expanded
  the tree. Cached columns are skipped, so repeat calls are cheap.

**Training corpus (`src-tauri/src/ai/training/dataset.rs`)**

Two new example blocks per table:

* **NL→SQL with schema in `input`** — `derive_nl_to_sql_examples`
  emits "List all …", "Show the first 10 …", two-column projections,
  "How many …", and per-primary-key lookup examples. Every one of
  these has the authoritative column list in `input` and an `output`
  that references only columns from that list.
* **Contrastive "only these columns exist"** —
  `derive_exclusion_example` (for tables with ≥3 columns) spells out
  in the `input` that columns like `created_at`, `mfg_date`,
  `exp_date`, `description` do NOT exist unless listed. Teaches the
  *exclusion* rule, not just the inclusion rule.
* The existing WHERE-clause samples now also carry the column-list
  input, for consistency with the new block.

Net effect: the model sees, thousands of times, the exact pattern it
needs at inference — "here is an authoritative column list, compose a
SELECT using only those columns." Combined with the tightened system
prompt, this removes the fabrication vector in both the hosted and
local-LoRA paths.
