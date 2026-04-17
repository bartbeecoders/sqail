//! Training — fine-tune a base LLM on a database's schema, metadata, and
//! sample data so it can generate SQL tailored to that database.
//!
//! The heavy lifting (actual LoRA fine-tuning) happens in a Python sidecar
//! — see `scripts/train_sql_lora.py` and the `jobs` module. Dataset
//! generation is fully in Rust so the app can always at least produce the
//! training corpus, even without Python installed.
//!
//! See `Vibecoding/llm-training.md` for the rationale and the request/
//! response shape used between the app and the Python trainer.

pub mod convert;
pub mod dataset;
pub mod env;
pub mod jobs;
pub mod models;
pub mod state;
