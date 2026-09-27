#!/usr/bin/env bash
# Downloads GGUF models for the Phase A inline-AI benchmark spike.
# Files land in .cache/inline-ai/models/ (gitignored).
#
# All downloads are resumable (-C -) and verified against SHA-256 when a
# digest is present. Skip a model by setting SKIP_<ID>=1, e.g.
# SKIP_DEEPSEEK=1 ./scripts/fetch-inline-models.sh
#
# Model entries format: id|display_name|url|sha256(optional)
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODELS_DIR="$PROJECT_ROOT/.cache/inline-ai/models"
mkdir -p "$MODELS_DIR"

MODELS=(
  "QWEN_1_5B|Qwen2.5-Coder-1.5B Q4_K_M|https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/resolve/main/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf"
  "QWEN_3B|Qwen2.5-Coder-3B Q4_K_M|https://huggingface.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF/resolve/main/qwen2.5-coder-3b-instruct-q4_k_m.gguf"
  "QWEN_7B|Qwen2.5-Coder-7B Q4_K_M|https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf"
  "DEEPSEEK|DeepSeek-Coder-V2-Lite-Instruct Q4_K_M|https://huggingface.co/bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF/resolve/main/DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf"
)

for entry in "${MODELS[@]}"; do
  IFS='|' read -r id name url <<<"$entry"
  skip_var="SKIP_${id}"
  if [[ "${!skip_var:-0}" == "1" ]]; then
    echo "--- skipping $name ($skip_var=1)"
    continue
  fi

  filename="$(basename "$url")"
  dest="$MODELS_DIR/$filename"

  if [[ -s "$dest" ]]; then
    echo "--- already have $name ($(du -h "$dest" | awk '{print $1}'))"
    continue
  fi

  echo "=== downloading $name"
  echo "    -> $dest"
  # -L follow redirects, -C - resume, --fail exit on HTTP errors, -o output.
  curl -L --fail --retry 3 --retry-delay 5 -C - -o "$dest" "$url"
done

echo
echo "=== models available:"
ls -lh "$MODELS_DIR" 2>/dev/null | awk 'NR>1 {printf "  %-55s %s\n", $NF, $5}'
