//! Python environment detection.
//!
//! Fine-tuning needs `python3` on PATH (or overridden via the
//! `SQAIL_PYTHON` env var) with `torch`, `peft`, `transformers`, `trl`,
//! and `datasets` importable. We don't bundle any of this — the sidecar
//! binary is on the order of gigabytes. Instead we probe and report.

use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheck {
    pub python_path: Option<String>,
    pub python_version: Option<String>,
    pub torch_available: bool,
    pub transformers_available: bool,
    pub peft_available: bool,
    pub trl_available: bool,
    pub datasets_available: bool,
    pub cuda_available: bool,
    pub cuda_device: Option<String>,
    pub missing: Vec<String>,
}

pub fn python_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SQAIL_PYTHON") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for name in ["python3", "python"] {
        if let Ok(out) = std::process::Command::new("which").arg(name).output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
        #[cfg(windows)]
        if let Ok(out) = std::process::Command::new("where").arg(name).output() {
            if out.status.success() {
                let first = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !first.is_empty() {
                    return Some(PathBuf::from(first));
                }
            }
        }
    }
    None
}

const PROBE: &str = r#"
import json, importlib, sys
report = {"python_version": sys.version.split()[0]}
for mod in ("torch", "transformers", "peft", "trl", "datasets"):
    try:
        importlib.import_module(mod)
        report[mod] = True
    except Exception:
        report[mod] = False
try:
    import torch
    report["cuda"] = bool(torch.cuda.is_available())
    if report["cuda"] and torch.cuda.device_count() > 0:
        report["device"] = torch.cuda.get_device_name(0)
    else:
        report["device"] = None
except Exception:
    report["cuda"] = False
    report["device"] = None
print(json.dumps(report))
"#;

pub async fn check() -> EnvCheck {
    let Some(py) = python_path() else {
        return EnvCheck {
            python_path: None,
            python_version: None,
            torch_available: false,
            transformers_available: false,
            peft_available: false,
            trl_available: false,
            datasets_available: false,
            cuda_available: false,
            cuda_device: None,
            missing: vec!["python3".into()],
        };
    };

    let mut cmd = Command::new(&py);
    cmd.arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return EnvCheck {
                python_path: Some(py.to_string_lossy().to_string()),
                python_version: None,
                torch_available: false,
                transformers_available: false,
                peft_available: false,
                trl_available: false,
                datasets_available: false,
                cuda_available: false,
                cuda_device: None,
                missing: vec!["python3 failed to start".into()],
            };
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(PROBE.as_bytes()).await;
    }
    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(_) => {
            return EnvCheck {
                python_path: Some(py.to_string_lossy().to_string()),
                python_version: None,
                torch_available: false,
                transformers_available: false,
                peft_available: false,
                trl_available: false,
                datasets_available: false,
                cuda_available: false,
                cuda_device: None,
                missing: vec!["probe failed".into()],
            };
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or(serde_json::Value::Null);

    let mut missing = Vec::new();
    let torch = parsed.get("torch").and_then(|v| v.as_bool()).unwrap_or(false);
    let transformers = parsed.get("transformers").and_then(|v| v.as_bool()).unwrap_or(false);
    let peft = parsed.get("peft").and_then(|v| v.as_bool()).unwrap_or(false);
    let trl = parsed.get("trl").and_then(|v| v.as_bool()).unwrap_or(false);
    let datasets = parsed.get("datasets").and_then(|v| v.as_bool()).unwrap_or(false);
    let cuda = parsed.get("cuda").and_then(|v| v.as_bool()).unwrap_or(false);
    if !torch {
        missing.push("torch".into());
    }
    if !transformers {
        missing.push("transformers".into());
    }
    if !peft {
        missing.push("peft".into());
    }
    if !trl {
        missing.push("trl".into());
    }
    if !datasets {
        missing.push("datasets".into());
    }

    EnvCheck {
        python_path: Some(py.to_string_lossy().to_string()),
        python_version: parsed
            .get("python_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        torch_available: torch,
        transformers_available: transformers,
        peft_available: peft,
        trl_available: trl,
        datasets_available: datasets,
        cuda_available: cuda,
        cuda_device: parsed
            .get("device")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        missing,
    }
}
