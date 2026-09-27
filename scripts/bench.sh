#!/usr/bin/env bash
# Service throughput/latency and UI frame times on the podman databases.
# Numbers are recorded in docs/benchmarks.md.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
"$ROOT/scripts/db.sh" up >/dev/null
info "service (1M rows per engine)"
cargo run -q --release -p sqail-client --example bench
info "UI (release build, headless)"
cargo test -q --release -p sqail-ui --test ui large_results_and_all_engines -- --ignored --nocapture 2>&1 \
    | grep -E "streaming|streamed|scrolling|editor:"
info "highlighter (5,000-line script)"
cargo test -q --release -p sqail-ui --lib highlight_and_layout -- --ignored --nocapture 2>&1 \
    | grep -E "tokenize|layout job|galley"
