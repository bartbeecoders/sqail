#!/usr/bin/env python3
"""
LoRA fine-tuning for a base instruct model on a SQail-generated SQL
dataset. Invoked by the Rust training job manager; communicates progress
back to the parent via JSON lines on stdout.

CLI contract (must stay stable — see
`src-tauri/src/ai/training/jobs.rs`):

    python3 train_sql_lora.py \\
        --dataset <path/to/dataset.jsonl> \\
        --base-model <huggingface repo id> \\
        --output-dir <adapter save dir> \\
        --epochs <float> --lr <float> \\
        --lora-rank <int> --lora-alpha <int> \\
        --max-steps <int, -1 disables> \\
        --batch-size <int>

Protocol:

* **stdout**: one JSON object per line, shape
  ``{"phase": "<loading|training|saving|done|error>", "step": <int?>,
     "total_steps": <int?>, "progress": <float 0..1?>, "loss": <float?>,
     "message": <str?>, "error": <str?>}``.
  Anything that isn't valid JSON on stdout is treated as a log line by
  the Rust side.
* **stderr**: free-form logs (shown in the UI's "View log" panel).
* **exit status**: ``0`` on success, ``1`` on any failure.

Runtime requirements:

    pip install torch transformers peft trl datasets accelerate

Bitsandbytes is optional; when installed, the script enables 4-bit QLoRA
automatically.
"""

from __future__ import annotations

import argparse
import json
import sys
import traceback
from pathlib import Path


def emit(**payload) -> None:
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--dataset", required=True)
    p.add_argument("--base-model", required=True)
    p.add_argument("--output-dir", required=True)
    p.add_argument("--epochs", type=float, default=3.0)
    p.add_argument("--lr", type=float, default=2e-4)
    p.add_argument("--lora-rank", type=int, default=8)
    p.add_argument("--lora-alpha", type=int, default=16)
    p.add_argument("--max-steps", type=int, default=-1)
    p.add_argument("--batch-size", type=int, default=1)
    return p.parse_args()


def main() -> int:
    args = parse_args()
    emit(phase="loading", message=f"Loading base model {args.base_model}")

    try:
        import torch  # noqa: F401 — we need to import before transformers
        from datasets import Dataset
        from peft import LoraConfig, get_peft_model, prepare_model_for_kbit_training
        from transformers import (
            AutoModelForCausalLM,
            AutoTokenizer,
            TrainerCallback,
            TrainingArguments,
        )
        from trl import SFTTrainer
    except Exception as e:  # noqa: BLE001
        emit(phase="error", error=f"missing dependency: {e}")
        return 1

    # 4-bit quant speeds things up enormously on a 16 GB card. Optional.
    quant_config = None
    try:
        import bitsandbytes  # noqa: F401
        from transformers import BitsAndBytesConfig

        quant_config = BitsAndBytesConfig(
            load_in_4bit=True,
            bnb_4bit_quant_type="nf4",
            bnb_4bit_use_double_quant=True,
            bnb_4bit_compute_dtype="bfloat16",
        )
    except Exception:
        pass

    dataset_path = Path(args.dataset)
    if not dataset_path.is_file():
        emit(phase="error", error=f"dataset not found: {dataset_path}")
        return 1

    # Load the JSONL examples and format as chat-style conversations.
    examples: list[dict[str, str]] = []
    with dataset_path.open("r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                examples.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    emit(
        phase="loading",
        message=f"Loaded {len(examples)} examples",
    )
    if not examples:
        emit(phase="error", error="dataset is empty")
        return 1

    # Prefer bf16 on cards that support it (Ampere+ / Ada / MI200+).
    # Mixed-precision choice also flows into TrainingArguments below.
    try:
        bf16_ok = bool(
            torch.cuda.is_available() and torch.cuda.is_bf16_supported()
        )
    except Exception:
        bf16_ok = False
    torch_dtype = torch.bfloat16 if bf16_ok else torch.float16

    try:
        tokenizer = AutoTokenizer.from_pretrained(args.base_model, use_fast=True)
        if tokenizer.pad_token is None:
            tokenizer.pad_token = tokenizer.eos_token

        model = AutoModelForCausalLM.from_pretrained(
            args.base_model,
            quantization_config=quant_config,
            device_map="auto",
            trust_remote_code=True,
            torch_dtype=torch_dtype,
        )
        # Gradient checkpointing trades compute for VRAM — on a 1.5B
        # model this is the difference between fitting in 4 GB and OOM.
        # Must be off-config during training (use_cache incompatible).
        model.config.use_cache = False
        if hasattr(model, "gradient_checkpointing_enable"):
            model.gradient_checkpointing_enable(
                gradient_checkpointing_kwargs={"use_reentrant": False}
            )
        if quant_config is not None:
            model = prepare_model_for_kbit_training(model)
    except Exception as e:  # noqa: BLE001
        emit(phase="error", error=f"model load: {e}")
        return 1

    lora = LoraConfig(
        r=args.lora_rank,
        lora_alpha=args.lora_alpha,
        lora_dropout=0.05,
        bias="none",
        task_type="CAUSAL_LM",
    )
    model = get_peft_model(model, lora)

    def format_example(ex: dict[str, str]) -> str:
        instruction = ex.get("instruction", "")
        ctx = ex.get("input", "")
        answer = ex.get("output", "")
        prompt = instruction
        if ctx:
            prompt = f"{instruction}\n\n{ctx}"
        # Chat-template format — tokenizer.apply_chat_template will pick
        # the right delimiters for the base model. Fall back to plain
        # prompt/response if there's no template.
        if tokenizer.chat_template:
            return tokenizer.apply_chat_template(
                [
                    {"role": "user", "content": prompt},
                    {"role": "assistant", "content": answer},
                ],
                tokenize=False,
            )
        return f"### Instruction:\n{prompt}\n\n### Response:\n{answer}\n"

    ds = Dataset.from_list(
        [{"text": format_example(ex)} for ex in examples]
    )

    class ProgressCallback(TrainerCallback):
        """Forward training loop progress to the Rust parent."""

        def on_step_end(self, args, state, control, **kwargs):
            total = state.max_steps or 1
            progress = state.global_step / max(total, 1)
            loss = None
            if state.log_history:
                # Last entry may or may not include a loss.
                for entry in reversed(state.log_history):
                    if "loss" in entry:
                        loss = float(entry["loss"])
                        break
            emit(
                phase="training",
                step=int(state.global_step),
                total_steps=int(total),
                progress=float(progress),
                loss=loss,
            )

    # TRL's SFTTrainer API changed in 0.12: `tokenizer` became
    # `processing_class`, and `dataset_text_field` + `max_seq_length`
    # moved into `SFTConfig` (a thin subclass of `TrainingArguments`).
    # Build the right flavour depending on which TRL is installed so this
    # script works across the common pip-installed versions.
    use_sft_config = False
    try:
        from trl import SFTConfig  # noqa: F401 — presence is the probe
        use_sft_config = True
    except Exception:
        pass

    common_kwargs = dict(
        output_dir=args.output_dir,
        num_train_epochs=args.epochs,
        per_device_train_batch_size=args.batch_size,
        gradient_accumulation_steps=max(1, 4 // max(args.batch_size, 1)),
        learning_rate=args.lr,
        logging_steps=5,
        save_strategy="no",
        max_steps=args.max_steps if args.max_steps > 0 else -1,
        report_to=[],
        # Mixed-precision picked up from the probe above.
        fp16=not bf16_ok and not quant_config,
        bf16=bf16_ok and not quant_config,
        optim="paged_adamw_8bit" if quant_config else "adamw_torch",
        warmup_ratio=0.03,
        # VRAM knobs — matters a lot on ≤16 GB cards shared with other
        # processes. Gradient checkpointing halves activation memory;
        # the empty_cache_steps reduces fragmentation between steps.
        gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
        torch_empty_cache_steps=10,
    )

    if use_sft_config:
        import inspect

        from trl import SFTConfig

        # TRL keeps shuffling these kwargs between releases:
        #   <0.12  — on SFTTrainer as `dataset_text_field` + `max_seq_length`
        #   0.12   — moved to SFTConfig, still `max_seq_length`
        #   ≥0.14  — renamed to `max_length` on SFTConfig
        # Introspect the signature so we pass whatever this install takes.
        sft_params = set(inspect.signature(SFTConfig).parameters.keys())
        extra = {}
        if "dataset_text_field" in sft_params:
            extra["dataset_text_field"] = "text"
        # 512 is plenty for SQL Q&A and halves the logits-tensor memory
        # footprint — important because recent TRL versions compute a
        # full-vocab entropy per token in compute_loss (~0.6 GB at
        # seq=1024 on Qwen's 150k vocab).
        if "max_length" in sft_params:
            extra["max_length"] = 512
        elif "max_seq_length" in sft_params:
            extra["max_seq_length"] = 512
        # Disable entropy/token-accuracy side computations when the
        # installed TRL lets us — they blow up logits memory on
        # large-vocab models.
        for name, val in (
            ("compute_entropy", False),
            ("entropy_coeff", 0.0),
            ("compute_token_accuracy", False),
            ("average_tokens_across_devices", False),
        ):
            if name in sft_params:
                extra[name] = val

        training_args = SFTConfig(**common_kwargs, **extra)
    else:
        training_args = TrainingArguments(**common_kwargs)

    emit(phase="training", message="Starting training loop", progress=0.0)

    try:
        trainer_kwargs = dict(
            model=model,
            train_dataset=ds,
            args=training_args,
            callbacks=[ProgressCallback()],
        )
        if use_sft_config:
            # New API: tokenizer is now `processing_class`, and the
            # dataset/seq-length knobs live on SFTConfig.
            trainer_kwargs["processing_class"] = tokenizer
        else:
            trainer_kwargs["tokenizer"] = tokenizer
            trainer_kwargs["dataset_text_field"] = "text"
            trainer_kwargs["max_seq_length"] = 512

        trainer = SFTTrainer(**trainer_kwargs)
        trainer.train()
    except Exception as e:  # noqa: BLE001
        emit(phase="error", error=f"training: {e}")
        traceback.print_exc(file=sys.stderr)
        return 1

    emit(phase="saving", message="Saving LoRA adapter")
    try:
        model.save_pretrained(args.output_dir)
        tokenizer.save_pretrained(args.output_dir)
    except Exception as e:  # noqa: BLE001
        emit(phase="error", error=f"save: {e}")
        return 1

    emit(phase="done", message="Training complete", progress=1.0)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001
        emit(phase="error", error=str(e))
        traceback.print_exc(file=sys.stderr)
        sys.exit(1)
